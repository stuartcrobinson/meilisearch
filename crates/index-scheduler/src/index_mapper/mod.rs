use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::{fs, thread};

// Import version constants directly
use meilisearch_types::versioning::{VERSION_MAJOR, VERSION_MINOR, VERSION_PATCH};
// Remove incorrect import from crate root: use meilisearch_types::VERSION_STRING;
use meilisearch_types::heed::types::{SerdeJson, Str};
use meilisearch_types::heed::{Database, Env, RoTxn, RwTxn, WithoutTls};
use meilisearch_types::milli;
use meilisearch_types::milli::database_stats::DatabaseStats;
use meilisearch_types::milli::update::IndexerConfig;
use meilisearch_types::milli::{FieldDistribution, Index};
use serde::{Deserialize, Serialize};
use meilisearch_types::settings::Settings;
// Duplicate serde import removed
use time::OffsetDateTime;
use tracing::error;
use uuid::Uuid;

use self::index_map::IndexMap; // Keep IndexMap import
use self::IndexStatus::{Available, BeingDeleted, Closing, Missing};
// InsertionOutcome is no longer needed here after refactor in 8a13e19
// use crate::lru::InsertionOutcome;
use crate::uuid_codec::UuidCodec;
// Remove unused versioning import
use crate::{Error, IndexBudget, IndexSchedulerOptions, Result};

mod index_map;

// [meilisearchfj] Added env field (already imported above)
// Duplicate Env import removed
// [meilisearchfj] Added Path import
use std::path::Path;

/// Structure holding the deserialized content of `metadata.json` from a snapshot.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FjParsedSnapshotMetadata {
    pub meilisearch_version: String,
    pub settings: Settings<meilisearch_types::settings::Unchecked>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// The number of database used by index mapper
const NUMBER_OF_DATABASES: u32 = 2;
/// Database const names for the `IndexMapper`.
mod db_name {
    pub const INDEX_MAPPING: &str = "index-mapping";
    pub const INDEX_STATS: &str = "index-stats";
}

/// Structure managing meilisearch's indexes.
///
/// It is responsible for:
/// 1. Creating new indexes
/// 2. Opening indexes and storing references to these opened indexes
/// 3. Accessing indexes through their uuid
/// 4. Mapping a user-defined name to each index uuid.
///
/// # Implementation notes
///
/// An index exists as 3 bits of data:
/// 1. The index data on disk, that can exist in 3 states: Missing, Present, or BeingDeleted.
/// 2. The persistent database containing the association between the index' name and its UUID,
///    that can exist in 2 states: Missing or Present.
/// 3. The state of the index in the in-memory `IndexMap`, that can exist in multiple states:
///   - Missing
///   - Available
///   - Closing (because an index needs resizing or was evicted from the cache)
///   - BeingDeleted
///
/// All of this data should be kept consistent between index operations, which is achieved by the `IndexMapper`
/// with the use of the following primitives:
/// - A RwLock on the `IndexMap`.
/// - Transactions on the association database.
/// - ClosingEvent signals emitted when closing an environment.
#[derive(Clone)]
pub struct IndexMapper {
    /// Keep track of the opened indexes. Used mainly by the index resolver.
    index_map: Arc<RwLock<IndexMap>>,

    /// Map an index name with an index uuid currently available on disk.
    pub(crate) index_mapping: Database<Str, UuidCodec>,
    /// Map an index UUID with the cached stats associated to the index.
    ///
    /// Using an UUID forces to use the index_mapping table to recover the index behind a name, ensuring
    /// consistency wrt index swapping.
    pub(crate) index_stats: Database<UuidCodec, SerdeJson<IndexStats>>,

    /// Path to the folder where the LMDB environments of each index are.
    base_path: PathBuf,
    /// The map size an index is opened with on the first time.
    index_base_map_size: usize,
    /// The quantity by which the map size of an index is incremented upon reopening, in bytes.
    index_growth_amount: usize,
    /// Whether we open a meilisearch index with the MDB_WRITEMAP option or not.
    enable_mdb_writemap: bool,
    pub indexer_config: Arc<IndexerConfig>,

    /// A few types of long running batches of tasks that act on a single index set this field
    /// so that a handle to the index is available from other threads (search) in an optimized manner.
    currently_updating_index: Arc<RwLock<Option<(String, Index)>>>,

    // [meilisearchfj] Added env field for direct access
    env: Env<WithoutTls>,
    // [meilisearchfj] Added snapshots_path field for explicit configuration
    snapshots_path: PathBuf,
}

/// Whether the index is available for use or is forbidden to be inserted back in the index map
#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub enum IndexStatus {
    /// Not currently in the index map.
    Missing,
    /// Do not insert it back in the index map as it is currently being deleted.
    BeingDeleted,
    /// Temporarily do not insert the index in the index map as it is currently being resized/evicted from the map.
    Closing(index_map::ClosingIndex),
    /// You can use the index without worrying about anything.
    Available(Index),
}

impl IndexStatus {
    /// [meilisearchfj] Helper to check if the status is Available.
    #[allow(dead_code)] // Silence warning until used
    pub fn fj_is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }

    /// [meilisearchfj] Helper to check if the status is Closing.
    #[allow(dead_code)] // Silence warning until used
    pub fn fj_is_closing(&self) -> bool {
        matches!(self, Self::Closing(_))
    }
}

/// The statistics that can be computed from an `Index` object.
#[derive(Serialize, Deserialize, Debug)]
pub struct IndexStats {
    /// Stats of the documents database.
    #[serde(default)]
    pub documents_database_stats: DatabaseStats,

    #[serde(default, skip_serializing)]
    pub number_of_documents: Option<u64>,

    /// Size taken up by the index' DB, in bytes.
    ///
    /// This includes the size taken by both the used and free pages of the DB, and as the free pages
    /// are not returned to the disk after a deletion, this number is typically larger than
    /// `used_database_size` that only includes the size of the used pages.
    pub database_size: u64,
    /// Number of embeddings in the index.
    /// Option: retrocompatible with the stats of the pre-v1.13.0 versions of meilisearch
    pub number_of_embeddings: Option<u64>,
    /// Number of embedded documents in the index.
    /// Option: retrocompatible with the stats of the pre-v1.13.0 versions of meilisearch
    pub number_of_embedded_documents: Option<u64>,
    /// Size taken by the used pages of the index' DB, in bytes.
    ///
    /// As the DB backend does not return to the disk the pages that are not currently used by the DB,
    /// this value is typically smaller than `database_size`.
    pub used_database_size: u64,
    /// The primary key of the index
    pub primary_key: Option<String>,
    /// Association of every field name with the number of times it occurs in the documents.
    pub field_distribution: FieldDistribution,
    /// Creation date of the index.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Date of the last update of the index.
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl IndexStats {
    /// Compute the stats of an index
    ///
    /// # Parameters
    ///
    /// - rtxn: a RO transaction for the index, obtained from `Index::read_txn()`.
    pub fn new(index: &Index, rtxn: &RoTxn) -> milli::Result<Self> {
        let arroy_stats = index.arroy_stats(rtxn)?;
        Ok(IndexStats {
            number_of_embeddings: Some(arroy_stats.number_of_embeddings),
            number_of_embedded_documents: Some(arroy_stats.documents.len()),
            documents_database_stats: index.documents_stats(rtxn)?.unwrap_or_default(),
            number_of_documents: None,
            database_size: index.on_disk_size()?,
            used_database_size: index.used_size()?,
            primary_key: index.primary_key(rtxn)?.map(|s| s.to_string()),
            field_distribution: index.field_distribution(rtxn)?,
            created_at: index.created_at(rtxn)?,
            updated_at: index.updated_at(rtxn)?,
        })
    }
}

impl IndexMapper {
    pub(crate) const fn nb_db() -> u32 {
        NUMBER_OF_DATABASES
    }

    pub fn new(
        env: &Env<WithoutTls>,
        wtxn: &mut RwTxn,
        options: &IndexSchedulerOptions,
        budget: IndexBudget,
    ) -> Result<Self> {
        Ok(Self {
            env: env.clone(), // [meilisearchfj] Clone env
            snapshots_path: options.snapshots_path.clone(), // [meilisearchfj] Store snapshots_path
            index_map: Arc::new(RwLock::new(IndexMap::new(budget.index_count))),
            index_mapping: env.create_database(wtxn, Some(db_name::INDEX_MAPPING))?,
            index_stats: env.create_database(wtxn, Some(db_name::INDEX_STATS))?,
            base_path: options.indexes_path.clone(),
            index_base_map_size: budget.map_size,
            index_growth_amount: options.index_growth_amount,
            enable_mdb_writemap: options.enable_mdb_writemap,
            indexer_config: options.indexer_config.clone(),
            currently_updating_index: Default::default(),
        })
    }

    /// Get or create the index.
    pub fn create_index(
        &self,
        mut wtxn: RwTxn,
        name: &str,
        date: Option<(OffsetDateTime, OffsetDateTime)>,
    ) -> Result<Index> {
        match self.index(&wtxn, name) {
            Ok(index) => {
                wtxn.commit()?;
                Ok(index)
            }
            Err(Error::IndexNotFound(_)) => {
                let uuid = Uuid::new_v4();
                self.index_mapping.put(&mut wtxn, name, &uuid)?;

                let index_path = self.base_path.join(uuid.to_string());
                fs::create_dir_all(&index_path)?;

                // Error if the UUIDv4 somehow already exists in the map, since it should be fresh.
                // This is very unlikely to happen in practice.
                // TODO: it would be better to lazily create the index. But we need an Index::open function for milli.
                let index = self
                    .index_map
                    .write()
                    .unwrap()
                    .create(
                        &uuid,
                        &index_path,
                        date,
                        self.enable_mdb_writemap,
                        self.index_base_map_size,
                        true,
                    )
                    .map_err(|e| Error::from_milli(e, Some(uuid.to_string())))?;
                let index_rtxn = index.read_txn()?;
                let stats = crate::index_mapper::IndexStats::new(&index, &index_rtxn)
                    .map_err(|e| Error::from_milli(e, Some(name.to_string())))?;
                self.store_stats_of(&mut wtxn, name, &stats)?;
                drop(index_rtxn);

                wtxn.commit()?;

                Ok(index)
            }
            error => error,
        }
    }

    /// Removes the index from the mapping table and the in-memory index map
    /// but keeps the associated tasks.
    pub fn delete_index(&self, mut wtxn: RwTxn, name: &str) -> Result<()> {
        let uuid = self
            .index_mapping
            .get(&wtxn, name)?
            .ok_or_else(|| Error::IndexNotFound(name.to_string()))?;

        // Not an error if the index had no stats in cache.
        self.index_stats.delete(&mut wtxn, &uuid)?;

        // Once we retrieved the UUID of the index we remove it from the mapping table.
        assert!(self.index_mapping.delete(&mut wtxn, name)?);

        wtxn.commit()?;

        let mut tries = 0;
        // Attempts to remove the index from the in-memory index map in a loop.
        //
        // If the index is currently being closed, we will wait for it to be closed and retry getting it in a subsequent
        // loop iteration.
        //
        // We make 100 attempts before giving up.
        // This could happen in the following situations:
        //
        // 1. There is a bug preventing the index from being correctly closed, or us from detecting this.
        // 2. A user of the index is keeping it open for more than 600 seconds. This could happen e.g. during a pathological search.
        //    This can not be caused by indexation because deleting an index happens in the scheduler itself, so cannot be concurrent with indexation.
        //
        // In these situations, reporting the error through a panic is in order.
        let closing_event = loop {
            let mut lock = self.index_map.write().unwrap();
            match lock.start_deletion(&uuid) {
                Ok(env_closing) => break env_closing,
                Err(Some(reopen)) => {
                    // drop the lock here so that we don't synchronously wait for the index to close.
                    drop(lock);
                    tries += 1;
                    if tries >= 100 {
                        panic!("Too many attempts to close index {name} prior to deletion.")
                    }
                    let reopen = if let Some(reopen) = reopen.wait_timeout(Duration::from_secs(6)) {
                        reopen
                    } else {
                        continue;
                    };
                    reopen.close(&mut self.index_map.write().unwrap());
                    continue;
                }
                Err(None) => return Ok(()),
            }
        };

        let index_map = self.index_map.clone();
        let index_path = self.base_path.join(uuid.to_string());
        let index_name = name.to_string();
        thread::Builder::new()
            .name(String::from("index_deleter"))
            .spawn(move || {
                // We first wait to be sure that the previously opened index is effectively closed.
                // This can take a lot of time, this is why we do that in a separate thread.
                if let Some(closing_event) = closing_event {
                    closing_event.wait();
                }

                // Then we remove the content from disk.
                if let Err(e) = fs::remove_dir_all(&index_path) {
                    error!(
                        "An error happened when deleting the index {} ({}): {}",
                        index_name, uuid, e
                    );
                }

                // Finally we remove the entry from the index map.
                index_map.write().unwrap().end_deletion(&uuid);
            })
            .unwrap();

        Ok(())
    }

    pub fn exists(&self, rtxn: &RoTxn, name: &str) -> Result<bool> {
        Ok(self.index_mapping.get(rtxn, name)?.is_some())
    }

    /// [meilisearchfj] Returns the base path for indexes.
    #[allow(dead_code)] // Silence warning until used
    pub fn fj_base_path(&self) -> &Path {
        &self.base_path
    }

    /// [meilisearchfj] Returns the number of indexes currently available in the LRU cache.
    #[allow(dead_code)] // Silence warning until used
    pub fn fj_available_cache_len(&self) -> usize {
        self.index_map.read().unwrap().fj_available_len()
    }

    /// [meilisearchfj] Returns the number of indexes currently marked as unavailable (closing or deleting).
    #[allow(dead_code)] // Silence warning until used
    pub fn fj_unavailable_cache_len(&self) -> usize {
        self.index_map.read().unwrap().fj_unavailable_len()
    }

    /// [meilisearchfj] Returns the status of an index in the cache.
    #[allow(dead_code)] // Silence warning until used
    pub fn fj_index_status(&self, uuid: &Uuid) -> IndexStatus {
        self.index_map.read().unwrap().get(uuid)
    }

    /// Resizes the maximum size of the specified index to the double of its current maximum size.
    ///
    /// This operation involves closing the underlying environment and so can take a long time to complete.
    ///
    /// # Panics
    ///
    /// - If the Index corresponding to the passed name is concurrently being deleted/resized or cannot be found in the
    ///   in memory hash map.
    pub fn resize_index(&self, rtxn: &RoTxn, name: &str) -> Result<()> {
        let uuid = self
            .index_mapping
            .get(rtxn, name)?
            .ok_or_else(|| Error::IndexNotFound(name.to_string()))?;

        // We remove the index from the in-memory index map.
        self.index_map.write().unwrap().close_for_resize(
            &uuid,
            self.enable_mdb_writemap,
            self.index_growth_amount,
        );

        Ok(())
    }

    /// Return an index, may open it if it wasn't already opened.
    pub fn index(&self, rtxn: &RoTxn, name: &str) -> Result<Index> {
        if let Some((current_name, current_index)) =
            self.currently_updating_index.read().unwrap().as_ref()
        {
            if current_name == name {
                return Ok(current_index.clone());
            }
        }

        let uuid = self
            .index_mapping
            .get(rtxn, name)?
            .ok_or_else(|| Error::IndexNotFound(name.to_string()))?;

        let mut tries = 0;
        // attempts to open the index in a loop.
        //
        // If the index is currently being closed, we will wait for it to be closed and retry getting it in a subsequent
        // loop iteration.
        //
        // We make 100 attempts before giving up.
        // This could happen in the following situations:
        //
        // 1. There is a bug preventing the index from being correctly closed, or us from detecting it was.
        // 2. A user of the index is keeping it open for more than 600 seconds. This could happen e.g. during a long indexation,
        //    a pathological search, and so on.
        //
        // In these situations, reporting the error through a panic is in order.
        let index = loop {
            tries += 1;
            if tries > 100 {
                panic!("Too many spurious wake ups while trying to open the index {name}");
            }

            // we get the index here to drop the lock before entering the match
            let index = self.index_map.read().unwrap().get(&uuid);

            match index {
                Available(index) => break index,
                Closing(reopen) => {
                    // Avoiding deadlocks: no lock taken while doing this operation.
                    let reopen = if let Some(reopen) = reopen.wait_timeout(Duration::from_secs(6)) {
                        reopen
                    } else {
                        continue;
                    };
                    let index_path = self.base_path.join(uuid.to_string());
                    // take the lock to reopen the environment.
                    reopen
                        .reopen(&mut self.index_map.write().unwrap(), &index_path)
                        .map_err(|e| Error::from_milli(e, Some(uuid.to_string())))?;
                    continue;
                }
                BeingDeleted => return Err(Error::IndexNotFound(name.to_string())),
                // since we're lazy, it's possible that the index has not been opened yet.
                Missing => {
                    let mut index_map = self.index_map.write().unwrap();
                    // between the read lock and the write lock it's not impossible
                    // that someone already opened the index (eg if two searches happen
                    // at the same time), thus before opening it we check a second time
                    // if it's not already there.
                    match index_map.get(&uuid) {
                        Missing => {
                            let index_path = self.base_path.join(uuid.to_string());

                            break index_map
                                .create(
                                    &uuid,
                                    &index_path,
                                    None,
                                    self.enable_mdb_writemap,
                                    self.index_base_map_size,
                                    false,
                                )
                                .map_err(|e| Error::from_milli(e, Some(uuid.to_string())))?;
                        }
                        Available(index) => break index,
                        Closing(_) => {
                            // the reopening will be handled in the next loop operation
                            continue;
                        }
                        BeingDeleted => return Err(Error::IndexNotFound(name.to_string())),
                    }
                }
            }
        };

        Ok(index)
    }

    /// Attempts `f` for each index that exists in the index mapper.
    ///
    /// It is preferable to use this function rather than a loop that opens all indexes, as a way to avoid having all indexes opened,
    /// which is unsupported in general.
    ///
    /// Since `f` is allowed to return a result, and `Index` is cloneable, it is still possible to wrongly build e.g. a vector of
    /// all the indexes, but this function makes it harder and so less likely to do accidentally.
    pub fn try_for_each_index<U, V>(
        &self,
        rtxn: &RoTxn,
        mut f: impl FnMut(&str, &Index) -> Result<U>,
    ) -> Result<V>
    where
        V: FromIterator<U>,
    {
        self.index_mapping
            .iter(rtxn)?
            .map(|res| {
                res.map_err(Error::from)
                    .and_then(|(name, _)| self.index(rtxn, name).and_then(|index| f(name, &index)))
            })
            .collect()
    }

    /// Return the name of all indexes without opening them.
    pub fn index_names(&self, rtxn: &RoTxn) -> Result<Vec<String>> {
        self.index_mapping
            .iter(rtxn)?
            .map(|res| res.map_err(Error::from).map(|(name, _)| name.to_string()))
            .collect()
    }

    /// Swap two index names.
    pub fn swap(&self, wtxn: &mut RwTxn, lhs: &str, rhs: &str) -> Result<()> {
        let lhs_uuid = self
            .index_mapping
            .get(wtxn, lhs)?
            .ok_or_else(|| Error::IndexNotFound(lhs.to_string()))?;
        let rhs_uuid = self
            .index_mapping
            .get(wtxn, rhs)?
            .ok_or_else(|| Error::IndexNotFound(rhs.to_string()))?;

        self.index_mapping.put(wtxn, lhs, &rhs_uuid)?;
        self.index_mapping.put(wtxn, rhs, &lhs_uuid)?;

        Ok(())
    }

    /// The stats of an index.
    ///
    /// If available in the cache, they are directly returned.
    /// Otherwise, the `Index` is opened to compute the stats on the fly (the result is not cached).
    /// The stats for an index are cached after each `Index` update.
    pub fn stats_of(&self, rtxn: &RoTxn, index_uid: &str) -> Result<IndexStats> {
        let uuid = self
            .index_mapping
            .get(rtxn, index_uid)?
            .ok_or_else(|| Error::IndexNotFound(index_uid.to_string()))?;

        match self.index_stats.get(rtxn, &uuid)? {
            Some(stats) => Ok(stats),
            None => {
                let index = self.index(rtxn, index_uid)?;
                let index_rtxn = index.read_txn()?;
                IndexStats::new(&index, &index_rtxn)
                    .map_err(|e| Error::from_milli(e, Some(uuid.to_string())))
            }
        }
    }

    /// Stores the new stats for an index.
    ///
    /// Expected usage is to compute the stats the index using `IndexStats::new`, the pass it to this function.
    pub fn store_stats_of(
        &self,
        wtxn: &mut RwTxn,
        index_uid: &str,
        stats: &IndexStats,
    ) -> Result<()> {
        let uuid = self
            .index_mapping
            .get(wtxn, index_uid)?
            .ok_or_else(|| Error::IndexNotFound(index_uid.to_string()))?;

        self.index_stats.put(wtxn, &uuid, stats)?;
        Ok(())
    }

    pub fn index_exists(&self, rtxn: &RoTxn, name: &str) -> Result<bool> {
        Ok(self.index_mapping.get(rtxn, name)?.is_some())
    }

    pub fn indexer_config(&self) -> &IndexerConfig {
        &self.indexer_config
    }

    pub fn set_currently_updating_index(&self, index: Option<(String, Index)>) {
        *self.currently_updating_index.write().unwrap() = index;
    }

    /// Imports an index from a snapshot file.
    ///
    /// This method handles unpacking the snapshot, validating its contents,
    /// creating the index directory, opening the index, and registering it
    /// with the IndexMapper.
    ///
    /// # Arguments
    ///
    /// * `target_index_uid`: The desired user-facing name for the imported index.
    /// * `snapshot_path`: The path to the snapshot file (`.snapshot.tar.gz`).
    ///
    /// # Returns
    ///
    /// A tuple containing the opened `Index` handle and the parsed metadata from the snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * The `snapshot_path` is invalid or outside the configured snapshots directory.
    /// * The `target_index_uid` already exists.
    /// * The snapshot file is corrupted or has an invalid format.
    /// * The snapshot version is incompatible with the current Meilisearch version.
    /// * An I/O error occurs during unpacking or file operations.
    /// * An internal error occurs within Heed or Milli.
    pub fn fj_import_index_from_snapshot(
        &self,
        target_index_uid: &str,
        snapshot_path: &Path,
    ) -> Result<(Index, FjParsedSnapshotMetadata)> {
        // 1. Validate Request & Path
        // [meilisearchfj] Use configured snapshots_path
        if !snapshot_path.starts_with(&self.snapshots_path) || !snapshot_path.exists() {
            return Err(Error::InvalidSnapshotPath { path: snapshot_path.to_path_buf() });
        }

        // Check if target_index_uid already exists
        // [meilisearchfj] Use self.env for transaction
        let rtxn = self.env.read_txn()?;
        if self.index_exists(&rtxn, target_index_uid)? {
            return Err(Error::SnapshotImportTargetIndexExists {
                target_index_uid: target_index_uid.to_string(),
            });
        }
        drop(rtxn); // Release read transaction before potential writes

        // 2. Unpack & Validate Snapshot
        // Create temporary directory within the main data path for atomicity
        let temp_dir_base = self.base_path.parent().ok_or_else(|| {
            Error::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not determine parent directory for temporary snapshot import",
            ))
        })?;
        let temp_dir = tempfile::Builder::new()
            .prefix("tmp_snapshot_import_")
            .tempdir_in(temp_dir_base)?;

        let snapshot_file = fs::File::open(snapshot_path)?;
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(snapshot_file));
        archive.unpack(temp_dir.path())?;

        let data_mdb_path = temp_dir.path().join("data.mdb");
        let metadata_path = temp_dir.path().join("metadata.json");

        if !data_mdb_path.exists() || !metadata_path.exists() {
            return Err(Error::InvalidSnapshotFormat { path: snapshot_path.to_path_buf() });
        }

        // 3. Parse Metadata & Version Check
        let metadata_file = fs::File::open(&metadata_path)?;
        let metadata: FjParsedSnapshotMetadata = serde_json::from_reader(metadata_file)
            .map_err(|_e| Error::InvalidSnapshotFormat { path: snapshot_path.to_path_buf() })?; // Prefix unused 'e'

        // Use the imported version constants directly
        let (current_major, current_minor, _) = (
            VERSION_MAJOR.parse().unwrap_or(0),
            VERSION_MINOR.parse().unwrap_or(0),
            VERSION_PATCH.parse().unwrap_or(0),
        );
        let snapshot_version_str = &metadata.meilisearch_version;
        let snapshot_version_parts: Vec<&str> = snapshot_version_str.split('.').collect();
        // Construct current version string for comparison
        let current_version_string = format!("{}.{}.{}", VERSION_MAJOR, VERSION_MINOR, VERSION_PATCH); // Use parsed patch

        // Add logging here
        tracing::info!(target: "snapshot_import", "Version Check: Snapshot='{}', Current='{}'", snapshot_version_str, current_version_string);
        tracing::info!(target: "snapshot_import", "Parsed Versions: Snapshot Major={}, Minor={:?}. Current Major={}, Minor={}",
                       snapshot_version_parts.get(0).unwrap_or(&"?"),
                       snapshot_version_parts.get(1), // Log minor part before parsing
                       current_major, current_minor);


        if snapshot_version_parts.len() < 2 {
            tracing::error!(target: "snapshot_import", "Version Check Failed: Snapshot version has too few parts.");
            return Err(Error::SnapshotVersionMismatch {
                path: snapshot_path.to_path_buf(),
                snapshot_version: snapshot_version_str.clone(),
                current_version: current_version_string,
            });
        }
        let snapshot_major: u32 = snapshot_version_parts[0].parse().map_err(|e| { // Add error logging
            tracing::error!(target: "snapshot_import", "Version Check Failed: Could not parse snapshot major version: {}", e);
            Error::SnapshotVersionMismatch {
                path: snapshot_path.to_path_buf(),
                snapshot_version: snapshot_version_str.clone(),
                current_version: current_version_string.clone(),
            }
        })?;
        let snapshot_minor: u32 = snapshot_version_parts[1].parse().map_err(|e| { // Add error logging
             tracing::error!(target: "snapshot_import", "Version Check Failed: Could not parse snapshot minor version: {}", e);
            Error::SnapshotVersionMismatch {
                path: snapshot_path.to_path_buf(),
                snapshot_version: snapshot_version_str.clone(),
                current_version: current_version_string.clone(),
            }
        })?;

        tracing::info!(target: "snapshot_import", "Comparing Parsed Versions: Snapshot Major={}, Minor={}. Current Major={}, Minor={}",
                       snapshot_major, snapshot_minor, current_major, current_minor);

        // Directly return the specific error if versions mismatch
        if snapshot_major != current_major || snapshot_minor != current_minor {
            tracing::error!(target: "snapshot_import", "Version Check Failed: Major or Minor version mismatch.");
            return Err(Error::SnapshotVersionMismatch { // Return the specific error directly
                path: snapshot_path.to_path_buf(),
                snapshot_version: snapshot_version_str.to_string(), // Use to_string() for clarity
                current_version: current_version_string,
            });
        }

        tracing::info!(target: "snapshot_import", "Version Check Passed."); // Log success

        // 4. Prepare Index Directory & Data
        let new_uuid = Uuid::new_v4();
        let index_path = self.base_path.join(new_uuid.to_string());
        fs::create_dir_all(&index_path)?;
        fs::rename(data_mdb_path, index_path.join("data.mdb"))?; // Move data.mdb

        // 5. Register, Open, and Map Index
        // [meilisearchfj] Use self.env for transaction
        let mut wtxn = self.env.write_txn()?;
        let mut index_map = self.index_map.write().unwrap(); // Lock the index map

        // Open the index using milli::Index::new_with_creation_dates
        let options = milli::heed::EnvOpenOptions::new();
        let mut options = options.read_txn_without_tls();
        options.map_size(crate::utils::clamp_to_page_size(self.index_base_map_size));
        if self.enable_mdb_writemap {
            unsafe { options.flags(milli::heed::EnvFlags::WRITE_MAP) };
        }

        let _index = milli::Index::new_with_creation_dates( // Prefix unused 'index'
            options,
            &index_path,
            metadata.created_at,
            metadata.updated_at,
            false, // `creation` is false because we are importing existing data
        )
        .map_err(|e| Error::from_milli(e, Some(target_index_uid.to_string())))?;

        // Update the mapping table
        self.index_mapping.put(&mut wtxn, target_index_uid, &new_uuid)?;

        // [meilisearchfj] Use the new method to insert the already opened index
        // Assume fj_insert_opened_index now returns InsertionOutcome
        let outcome = index_map.fj_insert_opened_index(
            new_uuid,
            _index.clone(), // Clone index for potential return
            self.enable_mdb_writemap,
        );

        // Handle potential eviction
        // Use the full path to InsertionOutcome::Evicted
        if let crate::lru::InsertionOutcome::Evicted(evicted_uuid, evicted_index) = outcome {
            // Use the public method specifically for closing evicted indexes
            index_map.fj_close_evicted_index(evicted_uuid, evicted_index, self.enable_mdb_writemap);
        }

        // The index we use is the one passed to fj_insert_opened_index
        let index = _index;

        // Store initial stats after successful import and insertion into map.
        // Compute fresh stats after import.
        let index_rtxn = index.read_txn()?;
        let stats = crate::index_mapper::IndexStats::new(&index, &index_rtxn)
            .map_err(|e| Error::from_milli(e, Some(target_index_uid.to_string())))?;
        self.index_stats.put(&mut wtxn, &new_uuid, &stats)?;
        drop(index_rtxn);

        drop(index_map); // Release lock before committing

        wtxn.commit()?; // Commit changes to mapping and stats

        // 6. Cleanup is handled by tempfile's Drop trait

        Ok((index, metadata))
    }
}

// Removed unused BTreeMap, BTreeSet imports
use std::fs::File;
use std::path::Path;

use flate2::write::GzEncoder;
use flate2::Compression;
// Remove unused imports related to manual settings construction
use meilisearch_types::{
    heed::{self},
    milli::{self, Index},
    // Removed unused settings imports: Settings as ApiSettings, Unchecked
};
use tar::Builder;
use tempfile::tempdir_in;
// Remove OffsetDateTime import
use uuid::Uuid;

use crate::error::Error;
use crate::fj_snapshot_metadata::SnapshotMetadata;

/// Creates a snapshot of a single index.
///
/// The snapshot is a gzipped tarball containing the `data.mdb` file of the index
/// and a `metadata.json` file with index settings and other metadata.
///
/// # Arguments
///
/// * `index_uid`: The UID of the index being snapshotted.
/// * `index`: A handle to the `milli::Index`.
/// * `metadata`: Pre-read metadata of the index.
/// * `snapshots_path`: The directory where the final snapshot file will be stored.
///
/// # Returns
///
/// A `Result` containing the unique snapshot identifier (UID) on success,
/// or an `Error` on failure.
// Remove unused IndexSchedulerHandle import
// Remove unused IndexScheduler import if no longer needed
// use crate::IndexScheduler;

pub fn create_index_snapshot(
    index_uid: &str,
    index: &Index, // Revert to accepting Index handle
    metadata: SnapshotMetadata, // Accept pre-read metadata
    snapshots_path: &Path,
) -> crate::Result<String> {
    let snapshot_uid = Uuid::new_v4().to_string();
    let snapshot_filename = format!("{}-{}.snapshot.tar.gz", index_uid, snapshot_uid);
    let snapshot_filepath = snapshots_path.join(&snapshot_filename);

    // Use a temporary directory within the main data directory if possible,
    // otherwise fallback to the system default. This helps ensure atomicity
    // and efficiency if snapshots_path is on the same filesystem.
    let temp_dir_base = snapshots_path.parent().unwrap_or(snapshots_path);
    tracing::info!(target: "snapshot_creation", "Using temp base dir: {:?}", temp_dir_base);
    let temp_dir = tempdir_in(temp_dir_base).map_err(|e| Error::IoError(e))?;
    let temp_path = temp_dir.path();
    tracing::info!(target: "snapshot_creation", "Created temp dir: {:?}", temp_path);

    // Metadata is now passed in, no need to read it here.

    // Write the provided metadata to metadata.json in the temp directory
    tracing::info!(target: "snapshot_creation", "Writing metadata.json to temp dir");
    let metadata_path = temp_path.join("metadata.json");
    let metadata_file = File::create(&metadata_path).map_err(|e| Error::IoError(e))?;
    serde_json::to_writer(metadata_file, &metadata)
        .map_err(|e| Error::SnapshotCreationFailed {
            index_uid: index_uid.to_string(),
            source: Box::new(e),
        })?;
    tracing::info!(target: "snapshot_creation", "Successfully wrote metadata.json");

    // Copy data.mdb to the temp directory using index.copy_to_path
    let temp_data_path = temp_path.join("data.mdb");
    tracing::info!(target: "snapshot_creation", "Copying data.mdb to {:?}", temp_data_path);
    // Use compaction for potentially smaller snapshots
    index
        .copy_to_path(&temp_data_path, heed::CompactionOption::Enabled)
        .map_err(|e| {
            Error::SnapshotCreationFailed {
                index_uid: index_uid.to_string(),
                source: Box::new(e),
            }
        })?;
    tracing::info!(target: "snapshot_creation", "Successfully copied data.mdb");

    // Ensure the target directory exists before creating the file
    tracing::info!(target: "snapshot_creation", "Ensuring target snapshot directory exists: {:?}", snapshots_path);
    std::fs::create_dir_all(snapshots_path).map_err(|e| Error::IoError(e))?;

    // Create the final gzipped tarball
    tracing::info!(target: "snapshot_creation", "Creating final tarball at: {:?}", snapshot_filepath);
    let snapshot_file = File::create(&snapshot_filepath).map_err(|e| Error::IoError(e))?;
    let gz_encoder = GzEncoder::new(snapshot_file, Compression::default());
    let mut tar_builder = Builder::new(gz_encoder);

    // Add data.mdb and metadata.json to the archive
    tar_builder.append_path_with_name(&temp_data_path, "data.mdb").map_err(|e| Error::IoError(e))?; // Use temp_data_path
    tar_builder
        .append_path_with_name(&metadata_path, "metadata.json")
        .map_err(|e| Error::IoError(e))?;

    // Finish writing the archive
    tar_builder.finish().map_err(|e| Error::IoError(e))?;
    tracing::info!(target: "snapshot_creation", "Finished tar builder");
    let gz_encoder = tar_builder.into_inner().map_err(|e| Error::IoError(e))?;
    let file = gz_encoder.finish().map_err(|e| Error::IoError(e))?;
    tracing::info!(target: "snapshot_creation", "Finished Gzip encoder");
    // Explicitly sync data to disk
    file.sync_all().map_err(|e| Error::IoError(e))?;
    tracing::info!(target: "snapshot_creation", "Synced file to disk");
    drop(file); // Ensure file handle is closed
    tracing::info!(target: "snapshot_creation", "Closed snapshot file handle");

    // Verify the final snapshot file exists and has content
    tracing::info!(target: "snapshot_creation", "Verifying final snapshot file existence and content at: {:?}", snapshot_filepath);
    if !snapshot_filepath.exists() {
        return Err(Error::SnapshotCreationFailed {
            index_uid: index_uid.to_string(),
            source: Box::new(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Snapshot file does not exist after creation process completed.",
            )),
        });
    }
    let file_meta = std::fs::metadata(&snapshot_filepath).map_err(|e| Error::IoError(e))?;
    if file_meta.len() == 0 {
         return Err(Error::SnapshotCreationFailed {
            index_uid: index_uid.to_string(),
            source: Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Snapshot file was created but is empty.",
            )),
        });
    }


    // Temp dir is automatically cleaned up when `temp_dir` goes out of scope here.

    Ok(snapshot_uid)
}

/// Reads the necessary metadata and settings from the index using an existing transaction.
pub(crate) fn read_metadata_inner( // Make the function visible within the crate
    index_uid: &str,
    index: &Index,
    rtxn: &heed::RoTxn,
) -> crate::Result<SnapshotMetadata> {
    // Helper closure to simplify error mapping for milli operations that return heed::Result
    let _from_milli_heed = |e: milli::heed::Error| Error::from_milli(milli::Error::from(e), Some(index_uid.to_string())); // Prefix with _
    // Helper closure for milli operations returning milli::Error directly
    let from_milli = |e: milli::Error| Error::from_milli(e, Some(index_uid.to_string()));
    // Helper closure to simplify error mapping for milli operations that return heed::Result
    let from_milli_heed = |e: milli::heed::Error| Error::from_milli(milli::Error::from(e), Some(index_uid.to_string())); // Keep this one as it's used below

    // Use the existing settings function to retrieve all settings at once
    let settings_checked = meilisearch_types::settings::settings(
        index,
        rtxn,
        meilisearch_types::settings::SecretPolicy::RevealSecrets, // Reveal secrets for snapshot
    )
    .map_err(from_milli)?;

    // Convert to Unchecked for SnapshotMetadata
    let settings = settings_checked.into_unchecked();


    let primary_key = index.primary_key(&rtxn).map_err(from_milli_heed)?.map(String::from);
    // Use from_milli directly for created_at/updated_at as they return milli::Error
    let created_at = index.created_at(&rtxn).map_err(from_milli)?;
    let updated_at = index.updated_at(&rtxn).map_err(from_milli)?;

    Ok(SnapshotMetadata {
        meilisearch_version: env!("CARGO_PKG_VERSION").to_string(),
        primary_key,
        settings,
        created_at,
        updated_at,
    })
}

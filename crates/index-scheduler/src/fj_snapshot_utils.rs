// Removed unused BTreeMap, BTreeSet imports
use std::fs::File;
use std::path::Path;

use std::fs; // Add fs import for cleanup

use flate2::write::GzEncoder;
use flate2::Compression;
use meilisearch_types::{
    heed::{self},
    milli::{self, Index},
};
use tar::Builder;
// Remove tempfile import: use tempfile::tempdir_in;
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
    snapshots_path: &Path, // This should be the *directory*
) -> crate::Result<PathBuf> { // Return the full path
    let snapshot_uid = Uuid::new_v4().to_string();
    let snapshot_filename = format!("{}-{}.snapshot.tar.gz", index_uid, snapshot_uid);
    // Construct the final path within the provided directory
    let snapshot_filepath = snapshots_path.join(&snapshot_filename);

    // Ensure the target directory exists *before* any file operations within it
    tracing::info!(target: "snapshot_creation", "Ensuring target snapshot directory exists: {:?}", snapshots_path);
    fs::create_dir_all(snapshots_path).map_err(|e| Error::IoError(e))?;

    // Define temporary paths *within* the final snapshot directory
    let temp_metadata_path = snapshots_path.join(format!("metadata-{}.json.tmp", snapshot_uid));
    let temp_data_path = snapshots_path.join(format!("data-{}.mdb.tmp", snapshot_uid));

    // Defer cleanup of temporary files using a scope guard or similar pattern
    // For simplicity here, we'll use explicit cleanup in success/error paths,
    // but a guard is more robust.
    let cleanup = |path1: &Path, path2: &Path| {
        let _ = fs::remove_file(path1); // Ignore errors during cleanup
        let _ = fs::remove_file(path2);
    };

    // Write metadata directly to the temporary path in the snapshot directory
    tracing::info!(target: "snapshot_creation", "Writing metadata to temp file: {:?}", temp_metadata_path);
    match File::create(&temp_metadata_path) {
        Ok(metadata_file) => {
            if let Err(e) = serde_json::to_writer(metadata_file, &metadata) {
                cleanup(&temp_metadata_path, &temp_data_path);
                return Err(Error::SnapshotCreationFailed {
                    index_uid: index_uid.to_string(),
                    source: Box::new(e),
                });
            }
        }
        Err(e) => {
            cleanup(&temp_metadata_path, &temp_data_path);
            return Err(Error::IoError(e));
        }
    }
    tracing::info!(target: "snapshot_creation", "Successfully wrote temp metadata");

    // Copy data.mdb directly to the temporary path in the snapshot directory
    tracing::info!(target: "snapshot_creation", "Copying data.mdb to temp file: {:?}", temp_data_path);
    if let Err(e) = index.copy_to_path(&temp_data_path, heed::CompactionOption::Enabled) {
        cleanup(&temp_metadata_path, &temp_data_path);
        return Err(Error::SnapshotCreationFailed {
            index_uid: index_uid.to_string(),
            source: Box::new(e),
        });
    }
    tracing::info!(target: "snapshot_creation", "Successfully copied temp data.mdb");

    // Create the final gzipped tarball directly
    tracing::info!(target: "snapshot_creation", "Creating final tarball at: {:?}", snapshot_filepath);
    match File::create(&snapshot_filepath) {
        Ok(snapshot_file) => {
            let gz_encoder = GzEncoder::new(snapshot_file, Compression::default());
            let mut tar_builder = Builder::new(gz_encoder);

            // Add files from their temporary paths within snapshots_path
            if let Err(e) = tar_builder.append_path_with_name(&temp_data_path, "data.mdb") {
                cleanup(&temp_metadata_path, &temp_data_path);
                let _ = fs::remove_file(&snapshot_filepath); // Attempt cleanup of partial tarball
                return Err(Error::IoError(e));
            }
            if let Err(e) = tar_builder.append_path_with_name(&temp_metadata_path, "metadata.json")
            {
                cleanup(&temp_metadata_path, &temp_data_path);
                let _ = fs::remove_file(&snapshot_filepath);
                return Err(Error::IoError(e));
            }

            // Finish writing the archive
            if let Err(e) = tar_builder.finish() {
                cleanup(&temp_metadata_path, &temp_data_path);
                let _ = fs::remove_file(&snapshot_filepath);
                return Err(Error::IoError(e));
            }
            tracing::info!(target: "snapshot_creation", "Finished tar builder");
            let gz_encoder = match tar_builder.into_inner() {
                Ok(enc) => enc,
                Err(e) => {
                    cleanup(&temp_metadata_path, &temp_data_path);
                    let _ = fs::remove_file(&snapshot_filepath);
                    return Err(Error::IoError(e));
                }
            };
            let file = match gz_encoder.finish() {
                Ok(f) => f,
                Err(e) => {
                    cleanup(&temp_metadata_path, &temp_data_path);
                    let _ = fs::remove_file(&snapshot_filepath);
                    return Err(Error::IoError(e));
                }
            };
            tracing::info!(target: "snapshot_creation", "Finished Gzip encoder");

            // Explicitly sync data to disk
            if let Err(e) = file.sync_all() {
                cleanup(&temp_metadata_path, &temp_data_path);
                let _ = fs::remove_file(&snapshot_filepath);
                return Err(Error::IoError(e));
            }
            tracing::info!(target: "snapshot_creation", "Synced file to disk");
            drop(file); // Ensure file handle is closed
            tracing::info!(target: "snapshot_creation", "Closed snapshot file handle");
        }
        Err(e) => {
            cleanup(&temp_metadata_path, &temp_data_path);
            return Err(Error::IoError(e));
        }
    }

    // Cleanup temporary files *after* successful tarball creation and sync
    cleanup(&temp_metadata_path, &temp_data_path);
    tracing::info!(target: "snapshot_creation", "Cleaned up temporary files");


    // Attempt to sync the parent directory as well (remains the same)
    if let Some(parent_dir) = snapshot_filepath.parent() {
        tracing::info!(target: "snapshot_creation", "Attempting to sync parent directory: {:?}", parent_dir);
        match File::open(parent_dir) {
            Ok(dir_handle) => {
                if let Err(e) = dir_handle.sync_all() {
                    tracing::warn!(target: "snapshot_creation", "Failed to sync parent directory {:?}: {}", parent_dir, e);
                    // Continue anyway, maybe it wasn't necessary
                } else {
                    tracing::info!(target: "snapshot_creation", "Successfully synced parent directory");
                }
                drop(dir_handle); // Close directory handle
            }
            Err(e) => {
                tracing::warn!(target: "snapshot_creation", "Failed to open parent directory {:?} for syncing: {}", parent_dir, e);
                // Continue anyway
            }
        }
    } else {
        tracing::warn!(target: "snapshot_creation", "Could not get parent directory for snapshot path: {:?}", snapshot_filepath);
    }

    // Final attempt to sync the data to disk using File::sync_data
    match File::open(&snapshot_filepath) {
        Ok(file_handle) => {
            if let Err(e) = file_handle.sync_data() {
                tracing::warn!(target: "snapshot_creation", "Failed to sync_data for snapshot file {:?}: {}", snapshot_filepath, e);
                // Continue anyway
            } else {
                tracing::info!(target: "snapshot_creation", "Successfully sync_data for snapshot file");
            }
            drop(file_handle); // Close the handle
        }
        Err(e) => {
             tracing::warn!(target: "snapshot_creation", "Failed to re-open snapshot file {:?} for sync_data: {}", snapshot_filepath, e);
             // Continue anyway
        }
    }


    // Verification moved to the caller (`create_test_snapshot`)

    // Add internal verification checks just before returning Ok
    tracing::info!(target: "snapshot_creation", "[Internal Check] Verifying snapshot file state before returning: {:?}", snapshot_filepath);
    let internal_exists = snapshot_filepath.exists();
    let internal_is_file = snapshot_filepath.is_file();
    let internal_metadata_result = std::fs::metadata(&snapshot_filepath);
    tracing::info!(target: "snapshot_creation", "[Internal Check] Pre-return state: exists={}, is_file={}, metadata={:?}", internal_exists, internal_is_file, internal_metadata_result);


    // Log final path before returning
    tracing::info!(target: "snapshot_creation", "Successfully created snapshot: {:?}", snapshot_filepath);
    tracing::info!(target: "snapshot_creation", "Exiting create_index_snapshot successfully for: {:?}", snapshot_filepath);

    Ok(snapshot_filepath) // Return the full path
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

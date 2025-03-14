use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use meilisearch_types::heed::CompactionOption;
use meilisearch_types::milli::progress::Progress;
use meilisearch_types::tasks::{Status, Task};
use meilisearch_types::{compression, VERSION_FILE_NAME};

use crate::processing::SingleIndexSnapshotCreationProgress;
use crate::{Error, IndexScheduler, Result};

impl IndexScheduler {
    #[allow(dead_code)]
    pub(super) fn process_single_index_snapshot_creation(
        &self,
        progress: Progress,
        mut tasks: Vec<Task>,
    ) -> Result<Vec<Task>> {
        // We expect exactly one task for single index snapshot creation
        let task = &tasks[0];
        let index_uid = task.index_uid().unwrap();
        
        // Extract the snapshot path from the task kind
        let snapshot_path = if let meilisearch_types::tasks::KindWithContent::SingleIndexSnapshotCreation { snapshot_path, .. } = &task.kind {
            snapshot_path.clone()
        } else {
            // Generate a default path if not specified
            format!("{}-{}.snapshot", index_uid, SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs())
        };

        tracing::info!(target: "index_scheduler", "Creating snapshot for index '{}'", index_uid);
        progress.update_progress(SingleIndexSnapshotCreationProgress::StartingSnapshot);

        // Ensure snapshots directory exists
        fs::create_dir_all(&self.scheduler.snapshots_path)?;

        // Create temporary directory for snapshot
        tracing::debug!(target: "index_scheduler", "Creating temporary directory for snapshot");
        progress.update_progress(SingleIndexSnapshotCreationProgress::CreatingTempDirectory);
        let temp_snapshot_dir = tempfile::tempdir()?;

        // Copy version file to ensure compatibility when importing
        tracing::debug!(target: "index_scheduler", "Copying version file");
        progress.update_progress(SingleIndexSnapshotCreationProgress::CopyingVersionFile);
        let dst = temp_snapshot_dir.path().join(VERSION_FILE_NAME);
        fs::copy(&self.scheduler.version_file_path, dst)?;

        // Snapshot the index LMDB environment
        tracing::debug!(target: "index_scheduler", "Snapshotting index environment");
        progress.update_progress(SingleIndexSnapshotCreationProgress::SnapshotIndexEnvironment);
        
        // Get a read transaction to access the index
        let rtxn = self.env.read_txn()?;
        
        // Get the index from the index mapper
        let index = self.index_mapper.index(&rtxn, index_uid)?;
        
        // Create the directory for the index in the snapshot
        let index_snapshot_dir = temp_snapshot_dir.path().join("index");
        fs::create_dir_all(&index_snapshot_dir)?;
        
        // Copy the index LMDB environment to the snapshot directory
        index
            .copy_to_path(index_snapshot_dir.join("data.mdb"), CompactionOption::Enabled)
            .map_err(|e| Error::from_milli(e, Some(index_uid.to_string())))?;

        // Create metadata file with index details
        tracing::debug!(target: "index_scheduler", "Creating snapshot metadata");
        progress.update_progress(SingleIndexSnapshotCreationProgress::CreatingMetadata);
        
        // Create a simple metadata file with index information
        let metadata = serde_json::json!({
            "index_uid": index_uid,
            "created_at": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(),
            "meilisearch_version": env!("CARGO_PKG_VERSION"),
        });
        
        let metadata_path = temp_snapshot_dir.path().join("metadata.json");
        fs::write(metadata_path, serde_json::to_string_pretty(&metadata)?)?;
        
        // Release the read transaction
        drop(rtxn);

        // Create tarball from the snapshot directory
        tracing::debug!(target: "index_scheduler", "Creating snapshot tarball");
        progress.update_progress(SingleIndexSnapshotCreationProgress::CreatingTarball);
        
        // Determine the final snapshot path
        let final_snapshot_path = if Path::new(&snapshot_path).is_absolute() {
            PathBuf::from(snapshot_path)
        } else {
            self.scheduler.snapshots_path.join(snapshot_path)
        };
        
        // Create a temporary file for the tarball
        let temp_snapshot_file = tempfile::NamedTempFile::new_in(final_snapshot_path.parent().unwrap_or(Path::new(".")))?;
        
        // Compress the snapshot directory to the temporary file
        compression::to_tar_gz(temp_snapshot_dir.path(), temp_snapshot_file.path())?;

        // Move the snapshot to its final location
        tracing::debug!(target: "index_scheduler", "Moving snapshot to final location");
        progress.update_progress(SingleIndexSnapshotCreationProgress::MovingSnapshot);
        
        // Create parent directories if they don't exist
        if let Some(parent) = final_snapshot_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        // Persist the temporary file to its final location
        let file = temp_snapshot_file.persist(&final_snapshot_path)?;

        // Set permissions on the snapshot file
        tracing::debug!(target: "index_scheduler", "Setting snapshot file permissions");
        progress.update_progress(SingleIndexSnapshotCreationProgress::SettingPermissions);
        
        let mut permissions = file.metadata()?.permissions();
        permissions.set_readonly(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            #[allow(clippy::non_octal_unix_permissions)]
            //                     rwxrwxrwx
            permissions.set_mode(0b100100100);
        }
        
        file.set_permissions(permissions)?;

        tracing::info!(target: "index_scheduler", "Successfully created snapshot for index '{}' at '{}'", 
            index_uid, final_snapshot_path.display());

        // Mark all tasks as succeeded
        for task in &mut tasks {
            task.status = Status::Succeeded;
        }

        Ok(tasks)
    }
}

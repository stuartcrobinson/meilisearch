use std::fs;
use std::path::{Path, PathBuf};

use meilisearch_types::heed::types::Str;
use meilisearch_types::heed::{Database, RoTxn};
use meilisearch_types::milli::progress::Progress;
use meilisearch_types::tasks::{Status, Task};
use meilisearch_types::{compression, VERSION_FILE_NAME};
use uuid::Uuid;

use crate::processing::SingleIndexSnapshotImportProgress;
use crate::{Error, IndexScheduler, Result};

impl IndexScheduler {
    pub(super) fn process_single_index_snapshot_import(
        &self,
        progress: Progress,
        mut tasks: Vec<Task>,
    ) -> Result<Vec<Task>> {
        // We expect exactly one task for single index snapshot import
        let task = &tasks[0];
        let source_path = task.source_path().unwrap();
        let index_uid = task.index_uid().unwrap();
        let target_index_uid = task.target_index_uid().unwrap_or(index_uid);

        tracing::info!(target: "index_scheduler", 
            "Importing snapshot from '{}' to index '{}'", 
            source_path, target_index_uid);
        
        progress.update_progress(SingleIndexSnapshotImportProgress::StartingImport);

        // Create temporary directory for extraction
        tracing::debug!(target: "index_scheduler", "Creating temporary directory for extraction");
        progress.update_progress(SingleIndexSnapshotImportProgress::CreatingTempDirectory);
        let temp_dir = tempfile::tempdir()?;

        // Determine the full path to the snapshot file
        let snapshot_path = if Path::new(source_path).is_absolute() {
            PathBuf::from(source_path)
        } else {
            // If relative, assume it's relative to the snapshots directory
            self.scheduler.snapshots_path.join(source_path)
        };

        // Extract the snapshot
        tracing::debug!(target: "index_scheduler", "Extracting snapshot");
        progress.update_progress(SingleIndexSnapshotImportProgress::ExtractingSnapshot);
        compression::from_tar_gz(&snapshot_path, temp_dir.path())?;

        // Validate version compatibility
        tracing::debug!(target: "index_scheduler", "Validating version compatibility");
        progress.update_progress(SingleIndexSnapshotImportProgress::ValidatingVersion);
        
        let version_file_path = temp_dir.path().join(VERSION_FILE_NAME);
        if !version_file_path.exists() {
            return Err(Error::SnapshotVersionFileNotFound);
        }
        
        // Read metadata
        tracing::debug!(target: "index_scheduler", "Reading snapshot metadata");
        progress.update_progress(SingleIndexSnapshotImportProgress::ReadingMetadata);
        
        let metadata_path = temp_dir.path().join("metadata.json");
        let metadata = if metadata_path.exists() {
            let metadata_content = fs::read_to_string(&metadata_path)?;
            Some(serde_json::from_str::<serde_json::Value>(&metadata_content)?)
        } else {
            None
        };
        
        if let Some(metadata) = &metadata {
            tracing::debug!(target: "index_scheduler", 
                "Snapshot metadata: {}", 
                serde_json::to_string_pretty(metadata).unwrap_or_default());
        }

        // Validate snapshot integrity
        tracing::debug!(target: "index_scheduler", "Validating snapshot integrity");
        progress.update_progress(SingleIndexSnapshotImportProgress::ValidatingIntegrity);
        
        let index_dir = temp_dir.path().join("index");
        if !index_dir.exists() || !index_dir.join("data.mdb").exists() {
            return Err(Error::SnapshotMissingIndexData);
        }

        // Create or replace the target index
        tracing::debug!(target: "index_scheduler", "Creating target index");
        progress.update_progress(SingleIndexSnapshotImportProgress::CreatingTargetIndex);
        
        // Start a write transaction
        let mut wtxn = self.env.write_txn()?;
        
        // Generate a new UUID for the index
        let index_uuid = Uuid::new_v4();
        
        // Create the index directory
        let index_path = self.index_mapper.index_path(index_uuid);
        fs::create_dir_all(&index_path)?;
        
        // Copy index data from snapshot
        tracing::debug!(target: "index_scheduler", "Copying index data from snapshot");
        progress.update_progress(SingleIndexSnapshotImportProgress::CopyingIndexData);
        
        // Copy the LMDB files from the snapshot to the new index location
        fs::copy(
            index_dir.join("data.mdb"),
            index_path.join("data.mdb"),
        )?;
        
        // Update index mappings in the scheduler
        tracing::debug!(target: "index_scheduler", "Updating index mappings");
        progress.update_progress(SingleIndexSnapshotImportProgress::UpdatingIndexMappings);
        
        // Add the index to the index mapping
        self.index_mapper.index_mapping.put(&mut wtxn, target_index_uid, &index_uuid)?;
        
        // Commit the transaction
        wtxn.commit()?;
        
        // Perform final verification
        tracing::debug!(target: "index_scheduler", "Verifying import");
        progress.update_progress(SingleIndexSnapshotImportProgress::VerifyingImport);
        
        // Open a read transaction to verify the index was properly imported
        let rtxn = self.env.read_txn()?;
        let index = self.index_mapper.index(&rtxn, target_index_uid)?;
        
        // Get document count to include in the task result
        let document_count = index.number_of_documents(&rtxn)
            .map_err(|e| Error::from_milli(e, Some(target_index_uid.to_string())))?;
        
        drop(rtxn);

        tracing::info!(target: "index_scheduler", 
            "Successfully imported snapshot to index '{}' with {} documents", 
            target_index_uid, document_count);

        // Mark all tasks as succeeded and include the document count
        for task in &mut tasks {
            task.status = Status::Succeeded;
            
            // Update the task with the number of imported documents
            if let Some(details) = &mut task.details {
                details.set_imported_documents(Some(document_count));
            }
        }

        Ok(tasks)
    }
}

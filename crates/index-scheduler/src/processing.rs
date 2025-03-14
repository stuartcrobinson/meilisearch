use std::sync::Arc;

use meilisearch_types::milli::progress::{AtomicSubStep, NamedStep, Progress, ProgressView};
use meilisearch_types::milli::{make_atomic_progress, make_enum_progress};
use roaring::RoaringBitmap;

use crate::utils::ProcessingBatch;

#[derive(Clone, Default)]
pub struct ProcessingTasks {
    pub batch: Option<Arc<ProcessingBatch>>,
    /// The list of tasks ids that are currently running.
    pub processing: Arc<RoaringBitmap>,
    /// The progress on processing tasks
    pub progress: Option<Progress>,
}

impl ProcessingTasks {
    /// Creates an empty `ProcessingAt` struct.
    pub fn new() -> ProcessingTasks {
        ProcessingTasks::default()
    }

    pub fn get_progress_view(&self) -> Option<ProgressView> {
        Some(self.progress.as_ref()?.as_progress_view())
    }

    /// Stores the currently processing tasks, and the date time at which it started.
    pub fn start_processing(
        &mut self,
        processing_batch: ProcessingBatch,
        processing: RoaringBitmap,
    ) -> Progress {
        self.batch = Some(Arc::new(processing_batch));
        self.processing = Arc::new(processing);
        let progress = Progress::default();
        progress.update_progress(BatchProgress::ProcessingTasks);
        self.progress = Some(progress.clone());

        progress
    }

    /// Set the processing tasks to an empty list
    pub fn stop_processing(&mut self) -> Self {
        self.progress = None;

        Self {
            batch: std::mem::take(&mut self.batch),
            processing: std::mem::take(&mut self.processing),
            progress: None,
        }
    }

    /// Returns `true` if there, at least, is one task that is currently processing that we must stop.
    pub fn must_cancel_processing_tasks(&self, canceled_tasks: &RoaringBitmap) -> bool {
        !self.processing.is_disjoint(canceled_tasks)
    }
}

make_enum_progress! {
    pub enum BatchProgress {
        ProcessingTasks,
        WritingTasksToDisk,
    }
}

make_enum_progress! {
    pub enum TaskCancelationProgress {
        RetrievingTasks,
        UpdatingTasks,
    }
}

make_enum_progress! {
    pub enum TaskDeletionProgress {
        DeletingTasksDateTime,
        DeletingTasksMetadata,
        DeletingTasks,
        DeletingBatches,
    }
}

make_enum_progress! {
    pub enum SnapshotCreationProgress {
        StartTheSnapshotCreation,
        SnapshotTheIndexScheduler,
        SnapshotTheUpdateFiles,
        SnapshotTheIndexes,
        SnapshotTheApiKeys,
        CreateTheTarball,
    }
}

make_enum_progress! {
    pub enum SingleIndexSnapshotCreationProgress {
        StartingSnapshot,
        CreatingTempDirectory,
        CopyingVersionFile,
        SnapshotIndexEnvironment,
        CreatingMetadata,
        CreatingTarball,
        MovingSnapshot,
        SettingPermissions,
    }
}

make_enum_progress! {
    pub enum SingleIndexSnapshotImportProgress {
        StartingImport,
        CreatingTempDirectory,
        ExtractingSnapshot,
        ValidatingVersion,
        ReadingMetadata,
        ValidatingIntegrity,
        CreatingTargetIndex,
        CopyingIndexData,
        UpdatingIndexMappings,
        VerifyingImport,
    }
}

make_enum_progress! {
    pub enum DumpCreationProgress {
        StartTheDumpCreation,
        DumpTheApiKeys,
        DumpTheTasks,
        DumpTheBatches,
        DumpTheIndexes,
        DumpTheExperimentalFeatures,
        CompressTheDump,
    }
}

make_enum_progress! {
    pub enum CreateIndexProgress {
        CreatingTheIndex,
    }
}

make_enum_progress! {
    pub enum UpdateIndexProgress {
        UpdatingTheIndex,
    }
}

make_enum_progress! {
    pub enum DeleteIndexProgress {
        DeletingTheIndex,
    }
}

make_enum_progress! {
    pub enum SwappingTheIndexes {
        EnsuringCorrectnessOfTheSwap,
        SwappingTheIndexes,
    }
}

make_enum_progress! {
    pub enum InnerSwappingTwoIndexes {
        RetrieveTheTasks,
        UpdateTheTasks,
        UpdateTheIndexesMetadata,
    }
}

make_enum_progress! {
    pub enum DocumentOperationProgress {
        RetrievingConfig,
        ComputingDocumentChanges,
        Indexing,
    }
}

make_enum_progress! {
    pub enum DocumentEditionProgress {
        RetrievingConfig,
        ComputingDocumentChanges,
        Indexing,
    }
}

make_enum_progress! {
    pub enum DocumentDeletionProgress {
        RetrievingConfig,
        DeleteDocuments,
        Indexing,
    }
}

make_enum_progress! {
    pub enum SettingsProgress {
        RetrievingAndMergingTheSettings,
        ApplyTheSettings,
    }
}

make_atomic_progress!(Task alias AtomicTaskStep => "task" );
make_atomic_progress!(Document alias AtomicDocumentStep => "document" );
make_atomic_progress!(Batch alias AtomicBatchStep => "batch" );
make_atomic_progress!(UpdateFile alias AtomicUpdateFileStep => "update file" );

#[cfg(test)]
mod test {
    use std::sync::atomic::Ordering;

    use meili_snap::{json_string, snapshot};

    use super::*;

    #[test]
    fn one_level() {
        let mut processing = ProcessingTasks::new();
        processing.start_processing(ProcessingBatch::new(0), RoaringBitmap::new());
        snapshot!(json_string!(processing.get_progress_view()), @r#"
        {
          "steps": [
            {
              "currentStep": "processing tasks",
              "finished": 0,
              "total": 2
            }
          ],
          "percentage": 0.0
        }
        "#);
        processing.progress.as_ref().unwrap().update_progress(BatchProgress::WritingTasksToDisk);
        snapshot!(json_string!(processing.get_progress_view()), @r#"
        {
          "steps": [
            {
              "currentStep": "writing tasks to disk",
              "finished": 1,
              "total": 2
            }
          ],
          "percentage": 50.0
        }
        "#);
    }

    #[test]
    fn task_progress() {
        let mut processing = ProcessingTasks::new();
        processing.start_processing(ProcessingBatch::new(0), RoaringBitmap::new());
        let (atomic, tasks) = AtomicTaskStep::new(10);
        processing.progress.as_ref().unwrap().update_progress(tasks);
        snapshot!(json_string!(processing.get_progress_view()), @r#"
        {
          "steps": [
            {
              "currentStep": "processing tasks",
              "finished": 0,
              "total": 2
            },
            {
              "currentStep": "task",
              "finished": 0,
              "total": 10
            }
          ],
          "percentage": 0.0
        }
        "#);
        atomic.fetch_add(6, Ordering::Relaxed);
        snapshot!(json_string!(processing.get_progress_view()), @r#"
        {
          "steps": [
            {
              "currentStep": "processing tasks",
              "finished": 0,
              "total": 2
            },
            {
              "currentStep": "task",
              "finished": 6,
              "total": 10
            }
          ],
          "percentage": 30.000002
        }
        "#);
        processing.progress.as_ref().unwrap().update_progress(BatchProgress::WritingTasksToDisk);
        snapshot!(json_string!(processing.get_progress_view()), @r#"
        {
          "steps": [
            {
              "currentStep": "writing tasks to disk",
              "finished": 1,
              "total": 2
            }
          ],
          "percentage": 50.0
        }
        "#);
        let (atomic, tasks) = AtomicTaskStep::new(5);
        processing.progress.as_ref().unwrap().update_progress(tasks);
        atomic.fetch_add(4, Ordering::Relaxed);
        snapshot!(json_string!(processing.get_progress_view()), @r#"
        {
          "steps": [
            {
              "currentStep": "writing tasks to disk",
              "finished": 1,
              "total": 2
            },
            {
              "currentStep": "task",
              "finished": 4,
              "total": 5
            }
          ],
          "percentage": 90.0
        }
        "#);
    }

    #[cfg(test)]
    mod single_index_snapshot_progress_tests {
        use super::*;
        use meilisearch_types::milli::progress::Progress;

        #[test]
        fn test_single_index_snapshot_creation_progress_ordering() {
            // Test that steps are correctly ordered
            assert!(SingleIndexSnapshotCreationProgress::StartingSnapshot as usize < 
                    SingleIndexSnapshotCreationProgress::CreatingTarball as usize);
            
            // Test that first() returns the first step
            assert_eq!(
                SingleIndexSnapshotCreationProgress::first(),
                SingleIndexSnapshotCreationProgress::StartingSnapshot
            );
            
            // Test that last() returns the last step
            assert_eq!(
                SingleIndexSnapshotCreationProgress::last(),
                SingleIndexSnapshotCreationProgress::SettingPermissions
            );
        }

        #[test]
        fn test_single_index_snapshot_import_progress_ordering() {
            // Test that steps are correctly ordered
            assert!(SingleIndexSnapshotImportProgress::StartingImport as usize < 
                    SingleIndexSnapshotImportProgress::CopyingIndexData as usize);
            
            // Test that first() returns the first step
            assert_eq!(
                SingleIndexSnapshotImportProgress::first(),
                SingleIndexSnapshotImportProgress::StartingImport
            );
            
            // Test that last() returns the last step
            assert_eq!(
                SingleIndexSnapshotImportProgress::last(),
                SingleIndexSnapshotImportProgress::VerifyingImport
            );
        }

        #[test]
        fn test_progress_update_integration() {
            // Create a progress instance that we'll use for testing
            let progress = Progress::new();
            
            // Test that we can update progress with our custom enum
            progress.update_progress(SingleIndexSnapshotCreationProgress::StartingSnapshot);
            assert_eq!(
                progress.snapshot().map(|s| s.to_string()),
                Some("StartingSnapshot".to_string())
            );
            
            // Test updating to a different step
            progress.update_progress(SingleIndexSnapshotCreationProgress::CreatingMetadata);
            assert_eq!(
                progress.snapshot().map(|s| s.to_string()),
                Some("CreatingMetadata".to_string())
            );
            
            // Test import progress updates
            progress.update_progress(SingleIndexSnapshotImportProgress::StartingImport);
            assert_eq!(
                progress.snapshot().map(|s| s.to_string()),
                Some("StartingImport".to_string())
            );
        }

        #[test]
        fn test_progress_step_names() {
            // Test that step names are correctly formatted for display
            // This is important for logs and user-facing progress reporting
            assert_eq!(
                format!("{}", SingleIndexSnapshotCreationProgress::StartingSnapshot),
                "StartingSnapshot"
            );
            
            assert_eq!(
                format!("{}", SingleIndexSnapshotImportProgress::ExtractingSnapshot),
                "ExtractingSnapshot"
            );
        }
    }
}

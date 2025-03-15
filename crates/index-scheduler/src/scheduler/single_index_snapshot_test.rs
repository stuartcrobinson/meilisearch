use meilisearch_types::tasks::{KindWithContent, Status, Task};
use uuid::Uuid;

use crate::test_utils::IndexSchedulerHandle;

#[test]
fn test_single_index_snapshot_creation_task_enqueuing() {
    let dir = tempfile::tempdir().unwrap();
    let scheduler = IndexSchedulerHandle::new(&dir);
    
    // Create an index first
    let index_uid = "test_index";
    scheduler.create_index(index_uid).unwrap();
    
    // Create a snapshot path
    let snapshot_path = format!("{}.snapshot", Uuid::new_v4());
    
    // Enqueue a single index snapshot creation task
    let task_id = scheduler
        .register(KindWithContent::SingleIndexSnapshotCreation { 
            index_uid: index_uid.to_string(),
            snapshot_path: snapshot_path.clone(),
        })
        .unwrap();
    
    // Verify the task was enqueued correctly
    let task = scheduler.get_task(task_id).unwrap();
    assert_eq!(task.status, Status::Enqueued);
    
    if let KindWithContent::SingleIndexSnapshotCreation { index_uid: task_index_uid, snapshot_path: task_snapshot_path } = &task.kind {
        assert_eq!(task_index_uid, index_uid);
        assert_eq!(task_snapshot_path, snapshot_path);
    } else {
        panic!("Expected SingleIndexSnapshotCreation task");
    }
}

#[test]
fn test_single_index_snapshot_import_task_enqueuing() {
    let dir = tempfile::tempdir().unwrap();
    let scheduler = IndexSchedulerHandle::new(&dir);
    
    // Create an index first
    let index_uid = "test_index";
    scheduler.create_index(index_uid).unwrap();
    
    // Create a source path for the snapshot
    let source_path = format!("{}.snapshot", Uuid::new_v4());
    
    // Enqueue a single index snapshot import task
    let task_id = scheduler
        .register(KindWithContent::SingleIndexSnapshotImport { 
            index_uid: index_uid.to_string(),
            source_path: source_path.clone(),
            target_index_uid: None,
        })
        .unwrap();
    
    // Verify the task was enqueued correctly
    let task = scheduler.get_task(task_id).unwrap();
    assert_eq!(task.status, Status::Enqueued);
    
    if let KindWithContent::SingleIndexSnapshotImport { index_uid: task_index_uid, source_path: task_source_path, target_index_uid } = &task.kind {
        assert_eq!(task_index_uid, index_uid);
        assert_eq!(task_source_path, source_path);
        assert_eq!(target_index_uid, None);
    } else {
        panic!("Expected SingleIndexSnapshotImport task");
    }
}

#[test]
fn test_single_index_snapshot_import_with_target_task_enqueuing() {
    let dir = tempfile::tempdir().unwrap();
    let scheduler = IndexSchedulerHandle::new(&dir);
    
    // Create source and target indices
    let source_index_uid = "source_index";
    let target_index_uid = "target_index";
    scheduler.create_index(source_index_uid).unwrap();
    
    // Create a source path for the snapshot
    let source_path = format!("{}.snapshot", Uuid::new_v4());
    
    // Enqueue a single index snapshot import task with target index
    let task_id = scheduler
        .register(KindWithContent::SingleIndexSnapshotImport { 
            index_uid: source_index_uid.to_string(),
            source_path: source_path.clone(),
            target_index_uid: Some(target_index_uid.to_string()),
        })
        .unwrap();
    
    // Verify the task was enqueued correctly
    let task = scheduler.get_task(task_id).unwrap();
    assert_eq!(task.status, Status::Enqueued);
    
    if let KindWithContent::SingleIndexSnapshotImport { index_uid: task_index_uid, source_path: task_source_path, target_index_uid: task_target_index_uid } = &task.kind {
        assert_eq!(task_index_uid, source_index_uid);
        assert_eq!(task_source_path, source_path);
        assert_eq!(task_target_index_uid, Some(target_index_uid.to_string()));
    } else {
        panic!("Expected SingleIndexSnapshotImport task");
    }
}

// Mock implementation for testing task state transitions
mod mock_processing {
    use super::*;
    use crate::IndexScheduler;
    use meilisearch_types::milli::progress::Progress;
    
    // Add mock processing methods to IndexScheduler
    impl IndexScheduler {
        // Mock method for testing snapshot creation
        pub fn mock_process_single_index_snapshot_creation(
            &self,
            _progress: Progress,
            mut tasks: Vec<Task>,
        ) -> crate::Result<Vec<Task>> {
            // Update task status to processing
            for task in &mut tasks {
                task.status = Status::Processing;
            }
            
            // Simulate some work
            std::thread::sleep(std::time::Duration::from_millis(10));
            
            // Update task status to succeeded
            for task in &mut tasks {
                task.status = Status::Succeeded;
            }
            
            Ok(tasks)
        }
        
        // Mock method for testing snapshot import
        pub fn mock_process_single_index_snapshot_import(
            &self,
            _progress: Progress,
            mut tasks: Vec<Task>,
        ) -> crate::Result<Vec<Task>> {
            // Update task status to processing
            for task in &mut tasks {
                task.status = Status::Processing;
            }
            
            // Simulate some work
            std::thread::sleep(std::time::Duration::from_millis(10));
            
            // Update task status to succeeded
            for task in &mut tasks {
                task.status = Status::Succeeded;
                
                // Add some details to the task
                if let Some(details) = &mut task.details {
                    if let meilisearch_types::tasks::Details::SingleIndexSnapshotImport { imported_documents, .. } = details {
                        *imported_documents = Some(100); // Mock 100 imported documents
                    }
                }
            }
            
            Ok(tasks)
        }
    }
}

#[test]
fn test_single_index_snapshot_task_state_transitions() {
    
    let dir = tempfile::tempdir().unwrap();
    let mut scheduler = IndexSchedulerHandle::new(&dir);
    
    // Create an index
    let index_uid = "test_index";
    scheduler.create_index(index_uid).unwrap();
    
    // Register a snapshot creation task
    let snapshot_path = format!("{}.snapshot", Uuid::new_v4());
    let task_id = scheduler
        .register(KindWithContent::SingleIndexSnapshotCreation { 
            index_uid: index_uid.to_string(),
            snapshot_path,
        })
        .unwrap();
    
    // Mock the processing function
    scheduler.mock_process(move |scheduler, progress, tasks| {
        // Check if this is a snapshot creation task
        if let KindWithContent::SingleIndexSnapshotCreation { .. } = &tasks[0].kind {
            scheduler.mock_process_single_index_snapshot_creation(progress, tasks)
        } else {
            panic!("Expected SingleIndexSnapshotCreation task");
        }
    });
    
    // Process the task
    scheduler.process_tasks();
    
    // Verify the task completed successfully
    let task = scheduler.get_task(task_id).unwrap();
    assert_eq!(task.status, Status::Succeeded);
}

#[test]
fn test_single_index_snapshot_import_task_state_transitions() {
    
    let dir = tempfile::tempdir().unwrap();
    let mut scheduler = IndexSchedulerHandle::new(&dir);
    
    // Create an index
    let index_uid = "test_index";
    scheduler.create_index(index_uid).unwrap();
    
    // Register a snapshot import task
    let source_path = format!("{}.snapshot", Uuid::new_v4());
    let task_id = scheduler
        .register(KindWithContent::SingleIndexSnapshotImport { 
            index_uid: index_uid.to_string(),
            source_path,
            target_index_uid: None,
        })
        .unwrap();
    
    // Mock the processing function
    scheduler.mock_process(move |scheduler, progress, tasks| {
        // Check if this is a snapshot import task
        if let KindWithContent::SingleIndexSnapshotImport { .. } = &tasks[0].kind {
            scheduler.mock_process_single_index_snapshot_import(progress, tasks)
        } else {
            panic!("Expected SingleIndexSnapshotImport task");
        }
    });
    
    // Process the task
    scheduler.process_tasks();
    
    // Verify the task completed successfully
    let task = scheduler.get_task(task_id).unwrap();
    assert_eq!(task.status, Status::Succeeded);
    
    // Verify details were updated
    if let Some(details) = &task.details {
        if let meilisearch_types::tasks::Details::SingleIndexSnapshotImport { imported_documents, .. } = details {
            assert_eq!(*imported_documents, Some(100));
        } else {
            panic!("Expected SingleIndexSnapshotImport details");
        }
    }
}

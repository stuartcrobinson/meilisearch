use super::*;
use tempfile::tempdir;
use meilisearch_types::tasks::{KindWithContent, Status};
use crate::{IndexScheduler, test_utils::FailureLocation};

#[test]
fn test_single_index_snapshot_validation() {
    // Setup test environment
    let dir = tempfile::tempdir().unwrap();
    let (mut scheduler, _handle) = IndexScheduler::test(true, vec![]);
    let test_index = "test-index";
    
    // Create an index for testing
    let wtxn = scheduler.env.write_txn().unwrap();
    scheduler.index_mapper.create_index(wtxn, test_index, None).unwrap();
    
    // Test valid snapshot creation
    let valid_creation = scheduler.register_task(
        KindWithContent::SingleIndexSnapshotCreation { 
            index_uid: test_index.to_string(),
            snapshot_path: "valid-path.snapshot".to_string()
        }
    ).unwrap();
    assert_eq!(valid_creation.status, Status::Enqueued);
    
    // Test non-existent index for snapshot creation
    let err = scheduler.register_task(
        KindWithContent::SingleIndexSnapshotCreation { 
            index_uid: "non-existent-index".to_string(),
            snapshot_path: "path.snapshot".to_string()
        }
    ).unwrap_err();
    assert!(matches!(err, Error::IndexNotFound(name) if name == "non-existent-index"));
    
    // Create a mock snapshot file for import testing
    let temp_dir = tempdir().unwrap();
    let valid_snapshot = temp_dir.path().join("valid.snapshot");
    std::fs::write(&valid_snapshot, "mock data").unwrap();
    
    // Test valid snapshot import
    let valid_import = scheduler.register_task(
        KindWithContent::SingleIndexSnapshotImport { 
            index_uid: "new-index".to_string(),
            source_path: valid_snapshot.to_string_lossy().to_string(),
            target_index_uid: None
        }
    ).unwrap();
    assert_eq!(valid_import.status, Status::Enqueued);
    
    // Test invalid snapshot file path
    let err = scheduler.register_task(
        KindWithContent::SingleIndexSnapshotImport { 
            index_uid: "new-index".to_string(),
            source_path: "/non-existent-file.snapshot".to_string(),
            target_index_uid: None
        }
    ).unwrap_err();
    assert!(matches!(err, Error::IoError(_)));
    
    // Test invalid file extension
    let invalid_file = temp_dir.path().join("invalid.txt");
    std::fs::write(&invalid_file, "mock data").unwrap();
    let err = scheduler.register_task(
        KindWithContent::SingleIndexSnapshotImport { 
            index_uid: "new-index".to_string(),
            source_path: invalid_file.to_string_lossy().to_string(),
            target_index_uid: None
        }
    ).unwrap_err();
    assert!(matches!(err, Error::InvalidSnapshotFormat(_)));
}

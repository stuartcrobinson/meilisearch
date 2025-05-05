//! Tests for the integration of Single Index Snapshot tasks into the scheduler.
//! Corresponds to Step 4 of the sis_guide.md.

// Removed unused ErrorCode import
use meilisearch_types::tasks::{Details, KindWithContent, Status};
use tokio; // Import tokio for the test macro

// Remove unused import: use crate::test_utils::IndexSchedulerHandle;
use crate::fj_test_utils::FjIndexSchedulerHandleExt; // Import the extension trait
use crate::IndexScheduler; // Import IndexScheduler to call ::test()
use crate::Result; // Import Result for async fn return type

#[tokio::test] // Mark test as async
async fn test_single_index_snapshot_creation_success() -> Result<()> { // Return Result
    let (_scheduler, mut handle) = IndexScheduler::test(true, vec![]); // Prefix unused scheduler
    let index_uid = "test_snapshot_success";

    // 1. Create an index and add some data
    let task_creation = handle.fj_register_task( // Use handle extension method
        KindWithContent::IndexCreation {
            index_uid: index_uid.to_string(),
            primary_key: None,
        },
    ).await?; // Added .await
    handle.advance_one_successful_batch();
    let task_info = handle.fj_get_task(task_creation.uid).await?; // Use handle extension method
    assert_eq!(task_info.status, Status::Succeeded);

    // Create content file using handle extension method
    let json_content = serde_json::json!([{ "id": 1, "field": "value" }]);
    let (content_uuid, documents_count) = handle.fj_create_update_file(json_content)?;

    let task_add = handle.fj_register_task( // Use handle extension method
        KindWithContent::DocumentAdditionOrUpdate {
            index_uid: index_uid.to_string(),
            primary_key: Some("id".to_string()),
                method: meilisearch_types::milli::update::IndexDocumentsMethod::ReplaceDocuments,
                content_file: content_uuid,
                documents_count,
                allow_index_creation: false,
        },
    ).await?; // Added .await
    handle.advance_one_successful_batch();
    let task_info = handle.fj_get_task(task_add.uid).await?; // Use handle extension method
    assert_eq!(task_info.status, Status::Succeeded);

    // 2. Register the snapshot creation task
    let snapshot_task = handle.fj_register_task( // Use handle extension method
        KindWithContent::SingleIndexSnapshotCreation { index_uid: index_uid.to_string() },
    ).await?; // Added .await

    // 3. Process the snapshot task
    handle.advance_one_successful_batch();

    // 4. Verify the task outcome
    let completed_task = handle.fj_get_task(snapshot_task.uid).await?; // Use handle extension method
    assert_eq!(completed_task.status, Status::Succeeded, "Task failed: {:?}", completed_task.error);
    assert!(completed_task.error.is_none());

    let snapshot_uid = match completed_task.details {
        Some(Details::SingleIndexSnapshotCreation { snapshot_uid: Some(uid) }) => uid,
        _ => panic!("Task details are incorrect: {:?}", completed_task.details),
    };

    // 5. Verify the snapshot file exists
    let expected_filename = format!("{}-{}.snapshot.tar.gz", index_uid, snapshot_uid);
    // Access snapshots_path via handle extension method
    let snapshot_path = handle.fj_snapshots_path().join(expected_filename);
    assert!(snapshot_path.exists(), "Snapshot file not found at {:?}", snapshot_path);

    // Optional: Basic verification of snapshot content (more thorough checks in Step 3 tests)
    let file = std::fs::File::open(&snapshot_path).unwrap();
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let entries = archive.entries().unwrap();
    let mut found_mdb = false;
    let mut found_meta = false;
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path().unwrap();
        if path.file_name().unwrap() == "data.mdb" {
            found_mdb = true;
        }
        if path.file_name().unwrap() == "metadata.json" {
            found_meta = true;
        }
    }
    assert!(found_mdb, "data.mdb not found in snapshot archive");
    assert!(found_meta, "metadata.json not found in snapshot archive");
    Ok(()) // Return Ok for Result
}

// Remove duplicate import: use crate::IndexScheduler;

#[tokio::test] // Mark test as async
async fn test_single_index_snapshot_creation_index_not_found() -> Result<()> { // Return Result
    let (_scheduler, mut handle) = IndexScheduler::test(true, vec![]); // Prefix unused scheduler
    let index_uid = "non_existent_index";

    // 1. Register the snapshot creation task for a non-existent index
    let snapshot_task = handle.fj_register_task( // Use handle extension method
        KindWithContent::SingleIndexSnapshotCreation { index_uid: index_uid.to_string() },
    ).await?; // Added .await

    // 2. Process the snapshot task
    handle.advance_one_successful_batch();

    // 3. Verify the task outcome
    let completed_task = handle.fj_get_task(snapshot_task.uid).await?; // Use handle extension method
    assert_eq!(completed_task.status, Status::Failed);
    assert!(completed_task.error.is_some());

    // Check the error type (adjust based on the exact error implementation)
    let error = completed_task.error.unwrap();
    // Check the public message field for expected content related to index not found.
    // This is less precise than comparing codes but uses the available public API.
    assert!(
        error.message.contains(index_uid), // Check if index UID is mentioned
        "Error message does not contain index UID: {}",
        error.message
    );
    assert!(
        error.message.to_lowercase().contains("not found") || error.message.to_lowercase().contains("doesn't exist"), // Check for "not found" text
        "Error message does not contain index UID: {}",
        error.message // Use public 'message' field
    );

    // Verify details indicate failure
    match completed_task.details {
        Some(Details::SingleIndexSnapshotCreation { snapshot_uid: None }) => (), // Expected
        _ => panic!("Task details are incorrect for failed task: {:?}", completed_task.details),
    }
    Ok(()) // Return Ok for Result
}

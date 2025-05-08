use actix_web::StatusCode;
use meilisearch_types::snapshot::FjSingleIndexSnapshotImportPayload;
use meilisearch_types::tasks::{Details, Status, TaskId};
use serde_json::json;
use std::fs;
use tempfile::TempDir;

// Assuming Server, Client, default_settings_for_test, Value are available via common
// e.g. use crate::common::{Server, Client, default_settings_for_test, Value};
// For the purpose of this standalone file, we'll assume they are brought into scope by a higher-level `mod.rs`
// or that the test runner handles this. If these tests were in `meilisearch/tests/tests/`
// they would typically use `use super::common::*;`
// Since this is `meilisearch/tests/msfj_sis_snapshot_api.rs`, it's a separate crate.
// We need to reference the `common` module from the `meilisearch` crate's test helpers.
// This usually means `meilisearch::tests::common::*` if common is pub.
// Or, more typically, tests in `meilisearch/tests/` have their own `common.rs` or use `super::common` if structured that way.
// For now, let's assume `meilisearch_integration_tests::common::*` is the way if this file is treated as part of an integration test suite.
// Given the project structure, it's likely `meilisearch::tests::common` is not directly accessible like a normal dependency.
// The `common` module in `meilisearch/tests/common/` is for tests *within* the `meilisearch` crate (`#[cfg(test)]` integration tests).
// For a file in `meilisearch/tests/`, it's an external integration test.
// It would need to use `meilisearch_test_macro::meilisearch_test` and helpers provided by the test setup.

// Let's use the common pattern seen in other Meilisearch integration tests:
use super::common::{default_settings_for_test, Server, Value}; // Adjust if common is structured differently

async fn create_server_with_temp_snapshots_path() -> (Server<TempDir>, TempDir) {
    let snapshot_dir = TempDir::new().expect("Failed to create temp snapshot directory");
    let mut opt = default_settings_for_test();
    opt.snapshot_dir = snapshot_dir.path().to_path_buf();
    // In a real scenario, ensure the feature is enabled if behind a flag
    // opt.experimental_enable_single_index_snapshots = true;

    let server = Server::new_with_options(opt).await;
    (server, snapshot_dir)
}

#[actix_rt::test]
async fn test_single_index_snapshot_creation_success() {
    let (server, snapshot_temp_dir) = create_server_with_temp_snapshots_path().await;
    let client = server.service.as_client();
    let index_uid = "test_creation_success";

    // 1. Create a test index
    let (response, code) = client.create_index(json!({ "uid": index_uid })).await;
    assert_eq!(code, StatusCode::ACCEPTED, "Failed to create index: {}", response);
    let task_id = response.uid();
    server.wait_task(task_id).await;

    // Add some documents
    let documents = json!([
        { "id": 1, "field1": "hello" },
        { "id": 2, "field1": "world" }
    ]);
    let (response, code) = client.add_documents(index_uid, documents, Some("id")).await;
    assert_eq!(code, StatusCode::ACCEPTED, "Failed to add documents: {}", response);
    server.wait_task(response.uid()).await;

    // 2. Call POST /indexes/{index_uid}/snapshots
    let snapshot_url = format!("/indexes/{}/snapshots", index_uid);
    let (response, code) = client.post(snapshot_url, json!({})).await;

    // 3. Verify 202 Accepted and valid SummarizedTaskView
    assert_eq!(code, StatusCode::ACCEPTED, "Snapshot creation failed: {}", response);
    let task_id = response.uid();
    assert!(response["type"].as_str().unwrap().contains("singleIndexSnapshotCreation"));

    // 4. Wait for the task to complete (Succeeded)
    let task_response = server.wait_task(task_id).await;
    assert_eq!(task_response["status"], "succeeded", "Snapshot task did not succeed: {}", task_response);
    let details = task_response.get("details").expect("Task details missing");
    let snapshot_uid = details.get("snapshotUid").expect("snapshotUid missing in details").as_str().expect("snapshotUid not a string");
    assert!(!snapshot_uid.is_empty(), "snapshotUid is empty");

    // 5. Verify the snapshot file exists
    let snapshot_filename = format!("{}-{}.snapshot.tar.gz", index_uid, snapshot_uid);
    let snapshot_file_path = snapshot_temp_dir.path().join(&snapshot_filename);
    assert!(snapshot_file_path.exists(), "Snapshot file not found at {:?}", snapshot_file_path);
}

#[actix_rt::test]
async fn test_single_index_snapshot_import_success() {
    let (server, snapshot_temp_dir) = create_server_with_temp_snapshots_path().await;
    let client = server.service.as_client();
    let source_index_uid = "test_import_source";
    let target_index_uid = "test_import_target";

    // 1. Create a source index and snapshot it
    let (response, code) = client.create_index(json!({ "uid": source_index_uid })).await;
    assert_eq!(code, StatusCode::ACCEPTED);
    server.wait_task(response.uid()).await;

    let documents = json!([ { "id": 1, "data": "content1" }, { "id": 2, "data": "content2" } ]);
    let (response, code) = client.add_documents(source_index_uid, documents.clone(), Some("id")).await;
    assert_eq!(code, StatusCode::ACCEPTED);
    server.wait_task(response.uid()).await;

    let settings_payload = json!({ "displayedAttributes": ["id", "data"], "searchableAttributes": ["data"] });
    let (response, code) = client.update_settings(source_index_uid, settings_payload.clone()).await;
    assert_eq!(code, StatusCode::ACCEPTED);
    server.wait_task(response.uid()).await;

    let snapshot_url = format!("/indexes/{}/snapshots", source_index_uid);
    let (response, code) = client.post(snapshot_url, json!({})).await;
    assert_eq!(code, StatusCode::ACCEPTED);
    let creation_task_response = server.wait_task(response.uid()).await;
    assert_eq!(creation_task_response["status"], "succeeded");
    let snapshot_uid = creation_task_response["details"]["snapshotUid"].as_str().unwrap();
    let snapshot_filename = format!("{}-{}.snapshot.tar.gz", source_index_uid, snapshot_uid);
    assert!(snapshot_temp_dir.path().join(&snapshot_filename).exists());

    // 2. Call POST /snapshots/import
    let import_payload = FjSingleIndexSnapshotImportPayload {
        source_snapshot_filename: snapshot_filename.clone(),
        target_index_uid: target_index_uid.to_string(),
    };
    let (response, code) = client.post("/snapshots/import", serde_json::to_value(import_payload).unwrap()).await;

    // 3. Verify 202 Accepted
    assert_eq!(code, StatusCode::ACCEPTED, "Import request failed: {}", response);
    assert!(response["type"].as_str().unwrap().contains("singleIndexSnapshotImport"));

    // 4. Wait for task to complete
    let import_task_response = server.wait_task(response.uid()).await;
    assert_eq!(import_task_response["status"], "succeeded", "Import task did not succeed: {}", import_task_response);

    // 5. Verify new index exists with data and settings
    let (_target_index_info, code) = client.get_index(target_index_uid).await;
    assert_eq!(code, StatusCode::OK, "Target index not found after import");

    let (target_docs, code) = client.get_all_documents(target_index_uid).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(target_docs["results"].as_array().unwrap().len(), 2);

    let (target_settings, code) = client.get_settings(target_index_uid).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(target_settings["displayedAttributes"], settings_payload["displayedAttributes"]);
    assert_eq!(target_settings["searchableAttributes"], settings_payload["searchableAttributes"]);
}

#[actix_rt::test]
async fn test_single_index_snapshot_import_target_exists() {
    let (server, snapshot_temp_dir) = create_server_with_temp_snapshots_path().await;
    let client = server.service.as_client();
    let source_index_uid = "import_err_source_exists";
    let target_index_uid = "import_err_target_already_exists";

    client.create_index_with_uid(source_index_uid).await;
    let snapshot_url = format!("/indexes/{}/snapshots", source_index_uid);
    let (response, code) = client.post(snapshot_url, json!({})).await;
    assert_eq!(code, StatusCode::ACCEPTED);
    let creation_task_response = server.wait_task(response.uid()).await;
    let snapshot_uid = creation_task_response["details"]["snapshotUid"].as_str().unwrap();
    let snapshot_filename = format!("{}-{}.snapshot.tar.gz", source_index_uid, snapshot_uid);

    client.create_index_with_uid(target_index_uid).await; // Target already exists

    let import_payload = FjSingleIndexSnapshotImportPayload {
        source_snapshot_filename: snapshot_filename,
        target_index_uid: target_index_uid.to_string(),
    };
    let (response, code) = client.post("/snapshots/import", serde_json::to_value(import_payload).unwrap()).await;
    assert_eq!(code, StatusCode::ACCEPTED);

    let task_response = server.wait_task(response.uid()).await;
    assert_eq!(task_response["status"], "failed", "Task should have failed: {}", task_response);
    assert_eq!(task_response["error"]["code"], "index_already_exists");
    assert_eq!(task_response["error"]["type"], "invalid_request");
}

#[actix_rt::test]
async fn test_single_index_snapshot_import_source_not_found() {
    let (server, _snapshot_temp_dir) = create_server_with_temp_snapshots_path().await;
    let client = server.service.as_client();
    let target_index_uid = "import_err_target_source_missing";
    let non_existent_snapshot_filename = "this_snapshot_does_not_exist.snapshot.tar.gz";

    let import_payload = FjSingleIndexSnapshotImportPayload {
        source_snapshot_filename: non_existent_snapshot_filename.to_string(),
        target_index_uid: target_index_uid.to_string(),
    };
    let (response, code) = client.post("/snapshots/import", serde_json::to_value(import_payload).unwrap()).await;
    assert_eq!(code, StatusCode::ACCEPTED);

    let task_response = server.wait_task(response.uid()).await;
    assert_eq!(task_response["status"], "failed", "Task should have failed: {}", task_response);
    // Assuming the error code for a missing snapshot file, as per guide's error handling section.
    assert_eq!(task_response["error"]["code"], "invalid_snapshot_path");
    assert_eq!(task_response["error"]["type"], "invalid_request");
    assert!(task_response["error"]["message"].as_str().unwrap().contains(non_existent_snapshot_filename));
}

#[actix_rt::test]
async fn test_single_index_snapshot_import_invalid_payload() {
    let (server, _snapshot_temp_dir) = create_server_with_temp_snapshots_path().await;
    let client = server.service.as_client();

    // Missing source_snapshot_filename
    let (response, code) = client.post("/snapshots/import", json!({ "target_index_uid": "test_target" })).await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "Response: {}", response);
    assert_eq!(response["code"], "missing_field", "Response: {}", response); // deserr typically gives "missing_field"
    assert!(response["message"].as_str().unwrap().contains("source_snapshot_filename"));


    // Missing target_index_uid
    let (response, code) = client.post("/snapshots/import", json!({ "source_snapshot_filename": "test.snapshot.tar.gz" })).await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "Response: {}", response);
    assert_eq!(response["code"], "missing_field", "Response: {}", response);
    assert!(response["message"].as_str().unwrap().contains("target_index_uid"));


    // Invalid target_index_uid format (e.g., contains spaces)
    let (response, code) = client.post("/snapshots/import", json!({
        "source_snapshot_filename": "test.snapshot.tar.gz",
        "target_index_uid": "invalid uid with spaces"
    })).await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "Response: {}", response);
    assert_eq!(response["code"], "invalid_index_uid", "Response: {}", response);
}

#[actix_rt::test]
async fn test_single_index_snapshot_import_invalid_filename_path_traversal() {
    let (server, _snapshot_temp_dir) = create_server_with_temp_snapshots_path().await;
    let client = server.service.as_client();

    let import_payload = FjSingleIndexSnapshotImportPayload {
        source_snapshot_filename: "../../../etc/hosts.snapshot.tar.gz".to_string(), // Path traversal attempt
        target_index_uid: "exploit_target_path_traversal".to_string(),
    };

    // This check could happen at the API handler (resulting in immediate 400)
    // or at the scheduler level (task enqueued then fails).
    // Step 11 suggests security checks in the handler. If so, it would be a direct 400.
    // If the check is deferred to the scheduler, the task would fail.
    // The guide's Step 5 (IndexMapper) also mentions validating snapshot_path.
    // Let's assume the task gets enqueued and then fails due to path validation in the scheduler.
    let (response, code) = client.post("/snapshots/import", serde_json::to_value(import_payload).unwrap()).await;
    assert_eq!(code, StatusCode::ACCEPTED, "Import task for path traversal should be enqueued: {}", response);

    let task_response = server.wait_task(response.uid()).await;
    assert_eq!(task_response["status"], "failed", "Task should have failed due to invalid path: {}", task_response);
    assert_eq!(task_response["error"]["code"], "invalid_snapshot_path", "Error code mismatch: {}", task_response);
    assert_eq!(task_response["error"]["type"], "invalid_request", "Error type mismatch: {}", task_response);
}

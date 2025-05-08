use actix_web::http::StatusCode;
use meilisearch_types::fj_snapshot::FjSingleIndexSnapshotImportPayload;
// Unused imports Details, Status, TaskId removed.
// Unused import serde_json::json removed; common::json! will be used.
// std::fs::File is used with its FQN later, so no 'use std::fs;' is strictly needed,
// and the previous comment about it being unused was misleading.
use tempfile::TempDir;

// Assuming Server, Client, default_settings, Value are available via common
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

// Correctly include and use the common test utilities for an integration test file.
#[allow(dead_code)] // Allow dead code from the common module for this specific test file
mod common;
// Unused import Value removed from this line.
// Removed `json` from this use statement as it's a macro available via `mod common;`
use common::{default_settings, Server, Owned, GetAllDocumentsOptions};

async fn create_server_with_temp_snapshots_path() -> (Server<Owned>, TempDir) {
    let snapshot_dir = TempDir::new().expect("Failed to create temp snapshot directory");
    // Provide the required directory argument to default_settings
    let mut opt = default_settings(snapshot_dir.path());
    opt.snapshot_dir = snapshot_dir.path().to_path_buf();
    // In a real scenario, ensure the feature is enabled if behind a flag
    // opt.experimental_enable_single_index_snapshots = true;

    // Unwrap the Result from Server::new_with_options
    let server = Server::new_with_options(opt).await.expect("Failed to create server");
    (server, snapshot_dir)
}

#[actix_rt::test]
async fn test_single_index_snapshot_creation_success() {
    let (server, snapshot_temp_dir) = create_server_with_temp_snapshots_path().await;
    let index_uid = "test_creation_success";

    // 1. Create a test index
    // Call server.create_index with a common::Value payload
    let (response, code) = server.create_index(json!({"uid": index_uid})).await;
    assert_eq!(code, StatusCode::ACCEPTED, "Failed to create index: {}", response);
    let task_id = response.uid();
    server.wait_task(task_id).await;

    // Add some documents
    // Use common::json! macro
    let documents = json!([
        { "id": 1, "field1": "hello" },
        { "id": 2, "field1": "world" }
    ]);
    let index = server.index(index_uid);
    let (response, code) = index.add_documents(documents, Some("id")).await;
    assert_eq!(code, StatusCode::ACCEPTED, "Failed to add documents: {}", response);
    server.wait_task(response.uid()).await;

    // 2. Call POST /indexes/{index_uid}/snapshots
    let snapshot_url = format!("/indexes/{}/snapshots", index_uid);
    // Use common::json! macro and wrap with common::Value
    let (response, code) = server.service.post(snapshot_url, json!({})).await;

    // 3. Verify 202 Accepted and valid SummarizedTaskView
    assert_eq!(code, StatusCode::ACCEPTED, "Snapshot creation failed: {}", response);
    let task_id = response.uid();
    assert!(response["type"].as_str().unwrap().contains("singleIndexSnapshotCreation"));

    // 4. Wait for the task to complete (Succeeded)
    let task_response = server.wait_task(task_id).await;
    assert_eq!(task_response["status"], "succeeded", "Snapshot task did not succeed: {}", task_response);
    let details = task_response.get("details").expect("Task details missing");
    // Expect "dumpUid" as per current DetailsView conversion for SingleIndexSnapshotCreation
    let snapshot_uid = details.get("dumpUid").expect("dumpUid missing in details").as_str().expect("dumpUid not a string");
    assert!(!snapshot_uid.is_empty(), "dumpUid is empty");

    // 5. Verify the snapshot file exists
    let snapshot_filename = format!("{}-{}.snapshot.tar.gz", index_uid, snapshot_uid);
    let snapshot_file_path = snapshot_temp_dir.path().join(&snapshot_filename);
    assert!(snapshot_file_path.exists(), "Snapshot file not found at {:?}", snapshot_file_path);
}

#[actix_rt::test]
async fn test_single_index_snapshot_import_success() {
    let (server, snapshot_temp_dir) = create_server_with_temp_snapshots_path().await;
    let source_index_uid = "test_import_source";
    let target_index_uid = "test_import_target";

    // 1. Create a source index and snapshot it
    // Call server.create_index with a common::Value payload
    let (response, code) = server.create_index(json!({"uid": source_index_uid})).await;
    assert_eq!(code, StatusCode::ACCEPTED);
    server.wait_task(response.uid()).await;

    let source_index = server.index(source_index_uid);
    // Use common::json! macro
    let documents = json!([ { "id": 1, "data": "content1" }, { "id": 2, "data": "content2" } ]);
    let (response, code) = source_index.add_documents(documents.clone(), Some("id")).await;
    assert_eq!(code, StatusCode::ACCEPTED);
    server.wait_task(response.uid()).await;

    // Use common::json! macro
    let settings_payload = json!({
        "displayedAttributes": ["id", "data"],
        "searchableAttributes": ["data"],
        "stopWords": ["a", "the", "of"]
    });
    let (response, code) = source_index.update_settings(settings_payload.clone()).await;
    assert_eq!(code, StatusCode::ACCEPTED);
    server.wait_task(response.uid()).await;

    let snapshot_url = format!("/indexes/{}/snapshots", source_index_uid);
    // Use common::json! macro and wrap with common::Value
    let (response, code) = server.service.post(snapshot_url, json!({})).await;
    assert_eq!(code, StatusCode::ACCEPTED);
    let creation_task_response = server.wait_task(response.uid()).await;
    assert_eq!(creation_task_response["status"], "succeeded");
    // Corrected to use "dumpUid" as per DetailsView for SingleIndexSnapshotCreation
    let snapshot_uid = creation_task_response["details"]["dumpUid"].as_str().expect("dumpUid should be a string");
    let snapshot_filename = format!("{}-{}.snapshot.tar.gz", source_index_uid, snapshot_uid);
    assert!(snapshot_temp_dir.path().join(&snapshot_filename).exists());

    // 2. Call POST /snapshots/import
    let import_payload = FjSingleIndexSnapshotImportPayload {
        source_snapshot_filename: snapshot_filename.clone(),
        target_index_uid: target_index_uid.to_string(),
    };
    // Wrap serde_json::Value with common::Value
    let (response, code) = server.service.post("/snapshots/import", common::Value(serde_json::to_value(import_payload).unwrap())).await;

    // 3. Verify 202 Accepted
    assert_eq!(code, StatusCode::ACCEPTED, "Import request failed: {}", response);
    assert!(response["type"].as_str().unwrap().contains("singleIndexSnapshotImport"));

    // 4. Wait for task to complete
    let import_task_response = server.wait_task(response.uid()).await;
    assert_eq!(import_task_response["status"], "succeeded", "Import task did not succeed: {}", import_task_response);

    // 5. Verify new index exists with data and settings
    let target_index = server.index(target_index_uid);
    let (_target_index_info, code) = target_index.get().await; // get_index info
    assert_eq!(code, StatusCode::OK, "Target index not found after import");

    // Provide default options for get_all_documents
    let (target_docs, code) = target_index.get_all_documents(GetAllDocumentsOptions::default()).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(target_docs["results"].as_array().unwrap().len(), 2);

    // Use target_index.settings()
    let (target_settings, code) = target_index.settings().await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(target_settings["displayedAttributes"], settings_payload["displayedAttributes"]);
    assert_eq!(target_settings["searchableAttributes"], settings_payload["searchableAttributes"]);

    // Sort stopWords before comparison as their order might not be preserved
    let mut target_stopwords: Vec<String> = target_settings["stopWords"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    target_stopwords.sort_unstable();

    let mut payload_stopwords: Vec<String> = settings_payload["stopWords"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    payload_stopwords.sort_unstable();

    assert_eq!(target_stopwords, payload_stopwords);
}

#[actix_rt::test]
async fn test_single_index_snapshot_import_target_exists() {
    let (server, _snapshot_temp_dir) = create_server_with_temp_snapshots_path().await;
    let source_index_uid = "import_err_source_exists";
    let target_index_uid = "import_err_target_already_exists";

    // Create source index and snapshot
    // Call server.create_index with a common::Value payload
    let (task_response, code) = server.create_index(json!({"uid": source_index_uid})).await;
    assert_eq!(code, StatusCode::ACCEPTED);
    server.wait_task(task_response.uid()).await;
    let snapshot_url = format!("/indexes/{}/snapshots", source_index_uid);
    // Use common::json! macro and wrap with common::Value
    let (response, code) = server.service.post(snapshot_url, json!({})).await;
    assert_eq!(code, StatusCode::ACCEPTED);
    let creation_task_response = server.wait_task(response.uid()).await;
    // Corrected to use "dumpUid" for consistency with other tests in this file
    let snapshot_uid = creation_task_response["details"]["dumpUid"].as_str().expect("dumpUid should be a string");
    let snapshot_filename = format!("{}-{}.snapshot.tar.gz", source_index_uid, snapshot_uid);

    // Create target index (so it already exists)
    // Call server.create_index with a common::Value payload
    let (task_response, code) = server.create_index(json!({"uid": target_index_uid})).await;
    assert_eq!(code, StatusCode::ACCEPTED);
    server.wait_task(task_response.uid()).await;

    let import_payload = FjSingleIndexSnapshotImportPayload {
        source_snapshot_filename: snapshot_filename,
        target_index_uid: target_index_uid.to_string(),
    };
    // Wrap serde_json::Value with common::Value
    let (response, code) = server.service.post("/snapshots/import", common::Value(serde_json::to_value(import_payload).unwrap())).await;
    assert_eq!(code, StatusCode::ACCEPTED);

    let task_response = server.wait_task(response.uid()).await;
    assert_eq!(task_response["status"], "failed", "Task should have failed: {}", task_response);
    assert_eq!(task_response["error"]["code"], "index_already_exists");
    assert_eq!(task_response["error"]["type"], "invalid_request");
}

#[actix_rt::test]
async fn test_single_index_snapshot_import_source_not_found() {
    let (server, _snapshot_temp_dir) = create_server_with_temp_snapshots_path().await;
    let target_index_uid = "import_err_target_source_missing";
    let non_existent_snapshot_filename = "this_snapshot_does_not_exist.snapshot.tar.gz";

    let import_payload = FjSingleIndexSnapshotImportPayload {
        source_snapshot_filename: non_existent_snapshot_filename.to_string(),
        target_index_uid: target_index_uid.to_string(),
    };
    // Wrap serde_json::Value with common::Value
    let (response, code) = server.service.post("/snapshots/import", common::Value(serde_json::to_value(import_payload).unwrap())).await;
    
    // Expect a direct error from the API handler as the snapshot file does not exist.
    assert_eq!(code, StatusCode::BAD_REQUEST, "Import request should fail immediately for non-existent snapshot: {}", response);
    
    // Verify the error response structure
    assert_eq!(response["code"], "invalid_snapshot_path", "Response: {}", response);
    assert_eq!(response["type"], "invalid_request", "Response: {}", response);
    assert!(response["message"].as_str().unwrap().contains("not found or not accessible"), "Response: {}", response);
    assert!(response["message"].as_str().unwrap().contains(non_existent_snapshot_filename), "Response: {}", response);
}

#[actix_rt::test]
async fn test_single_index_snapshot_import_invalid_payload() {
    let (server, _snapshot_temp_dir) = create_server_with_temp_snapshots_path().await;

    // Missing source_snapshot_filename
    // Use common::json! macro
    // Send other fields in correct camelCase format to isolate the "missing field" error.
    let (response, code) = server.service.post("/snapshots/import", json!({ "targetIndexUid": "test_target" })).await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "Response: {}", response);
    assert_eq!(response["code"], "bad_request", "Response: {}", response); // Error code for missing field
    assert!(response["message"].as_str().unwrap().contains("sourceSnapshotFilename"), "Response: {}", response);


    // Missing target_index_uid
    // Use common::json! macro
    // Send other fields in correct camelCase format.
    let (response, code) = server.service.post("/snapshots/import", json!({ "sourceSnapshotFilename": "test.snapshot.tar.gz" })).await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "Response: {}", response);
    assert_eq!(response["code"], "bad_request", "Response: {}", response); // Error code for missing field
    assert!(response["message"].as_str().unwrap().contains("targetIndexUid"), "Response: {}", response);


    // Invalid target_index_uid format (e.g., contains spaces)
    // Use common::json! macro
    // Send all fields in correct camelCase format to test validation of targetIndexUid's content.
    // Create a dummy snapshot file so that the file system checks pass and validation can proceed to targetIndexUid.
    let dummy_snapshot_filename = "test.snapshot.tar.gz";
    let dummy_snapshot_path = _snapshot_temp_dir.path().join(dummy_snapshot_filename);
    std::fs::File::create(&dummy_snapshot_path).expect("Failed to create dummy snapshot file for test");

    let (response, code) = server.service.post("/snapshots/import", json!({
        "sourceSnapshotFilename": dummy_snapshot_filename,
        "targetIndexUid": "invalid uid with spaces"
    })).await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "Response: {}", response);
    assert_eq!(response["code"], "invalid_index_uid", "Response: {}", response);
}

#[actix_rt::test]
async fn test_single_index_snapshot_import_invalid_filename_path_traversal() {
    let (server, _snapshot_temp_dir) = create_server_with_temp_snapshots_path().await;

    let import_payload = FjSingleIndexSnapshotImportPayload {
        source_snapshot_filename: "../../../etc/hosts.snapshot.tar.gz".to_string(), // Path traversal attempt
        target_index_uid: "exploit_target_path_traversal".to_string(),
    };

    // This check could happen at the API handler (resulting in immediate 400)
    // or at the scheduler level (task enqueued then fails).
    // Step 11 suggests security checks in the handler. If so, it would be a direct 400.
    // If the check is deferred to the scheduler, the task would fail.
    // The guide's Step 5 (IndexMapper) also mentions validating snapshot_path.
    // The API handler should immediately reject path traversal attempts.
    // Wrap serde_json::Value with common::Value
    let (response, code) = server.service.post("/snapshots/import", common::Value(serde_json::to_value(import_payload).unwrap())).await;
    
    // Expect a direct error from the API handler.
    assert_eq!(code, StatusCode::BAD_REQUEST, "Import request should fail immediately for path traversal attempt: {}", response);

    // Verify the error response structure
    assert_eq!(response["code"], "invalid_snapshot_path", "Response: {}", response);
    assert_eq!(response["type"], "invalid_request", "Response: {}", response);
    assert!(response["message"].as_str().unwrap().contains("cannot contain '..' or be an absolute path"), "Response: {}", response);
}

#[actix_rt::test]
async fn test_single_index_snapshot_empty_index_create_import() {
    let (server, snapshot_temp_dir) = create_server_with_temp_snapshots_path().await;
    let source_index_uid = "test_empty_source";
    let target_index_uid = "test_empty_target";

    // 1. Create an empty source index
    let (response, code) = server.create_index(json!({"uid": source_index_uid})).await;
    assert_eq!(code, StatusCode::ACCEPTED, "Failed to create source index: {}", response);
    server.wait_task(response.uid()).await;

    // 2. Snapshot the empty source index
    let snapshot_url = format!("/indexes/{}/snapshots", source_index_uid);
    let (response, code) = server.service.post(snapshot_url, json!({})).await;
    assert_eq!(code, StatusCode::ACCEPTED, "Snapshot creation failed: {}", response);
    let creation_task_response = server.wait_task(response.uid()).await;
    assert_eq!(creation_task_response["status"], "succeeded", "Snapshot task did not succeed: {}", creation_task_response);
    let snapshot_uid = creation_task_response["details"]["dumpUid"].as_str().expect("dumpUid should be a string");
    let snapshot_filename = format!("{}-{}.snapshot.tar.gz", source_index_uid, snapshot_uid);
    assert!(snapshot_temp_dir.path().join(&snapshot_filename).exists(), "Snapshot file not found");

    // 3. Import the snapshot to a new target index
    let import_payload = FjSingleIndexSnapshotImportPayload {
        source_snapshot_filename: snapshot_filename.clone(),
        target_index_uid: target_index_uid.to_string(),
    };
    let (response, code) = server.service.post("/snapshots/import", common::Value(serde_json::to_value(import_payload).unwrap())).await;
    assert_eq!(code, StatusCode::ACCEPTED, "Import request failed: {}", response);
    let import_task_response = server.wait_task(response.uid()).await;
    assert_eq!(import_task_response["status"], "succeeded", "Import task did not succeed: {}", import_task_response);

    // 4. Verify the target index exists and is empty
    let target_index = server.index(target_index_uid);
    let (_target_index_info, code) = target_index.get().await;
    assert_eq!(code, StatusCode::OK, "Target index not found after import");

    let (target_docs, code) = target_index.get_all_documents(GetAllDocumentsOptions::default()).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(target_docs["results"].as_array().unwrap().len(), 0, "Target index should be empty");

    // 5. Verify default settings (or any specific settings if they were applied to the empty index)
    // For an empty index with no settings updates, it should have default settings.
    let (target_settings, code) = target_index.settings().await;
    assert_eq!(code, StatusCode::OK);
    // Example: Check a few default settings. This can be expanded.
    assert_eq!(target_settings["displayedAttributes"], json!(["*"]));
    assert_eq!(target_settings["searchableAttributes"], json!(["*"]));
    assert_eq!(target_settings["filterableAttributes"].as_array().unwrap().len(), 0);
    assert_eq!(target_settings["sortableAttributes"].as_array().unwrap().len(), 0);
    assert_eq!(target_settings["stopWords"].as_array().unwrap().len(), 0);
}

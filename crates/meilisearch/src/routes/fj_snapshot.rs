// Removed unused import: use std::path::Path;

use actix_web::web::{self, Data};
use actix_web::{HttpRequest, HttpResponse};
use index_scheduler::IndexScheduler;
use meilisearch_types::error::{Code, ResponseError};
use meilisearch_types::fj_snapshot::FjSingleIndexSnapshotImportPayload;
use meilisearch_types::keys::actions;
use meilisearch_types::tasks::{KindWithContent, TaskId};
use crate::extractors::authentication::{policies::ActionPolicy, GuardedData};
// TODO: Verify the actual path for these helper functions.
// They might be in `crate::routes::helpers` or directly in `crate::routes::mod`
// or exposed as extension traits on HttpRequest.
use crate::routes::{get_task_id, is_dry_run, SummarizedTaskView};
use crate::Opt;

pub async fn fj_create_index_snapshot(
    index_scheduler: GuardedData<ActionPolicy<{ actions::SNAPSHOTS_CREATE }>, Data<IndexScheduler>>,
    index_uid_path: web::Path<String>,
    req: HttpRequest,
    opt: web::Data<Opt>,
) -> Result<HttpResponse, ResponseError> {
    let index_uid = index_uid_path.into_inner();

    let task_kind = KindWithContent::SingleIndexSnapshotCreation { index_uid };

    let uid: Option<TaskId> = get_task_id(&req, &opt)?;
    let dry_run = is_dry_run(&req, &opt)?;
    
    let task = index_scheduler.register(task_kind, uid, dry_run)?;
    println!("[HANDLER_CREATE_SNAPSHOT] Registered task UID: {:?}, Dry Run: {}, Status: {:?}", task.uid, dry_run, task.status);
    
    // Use SummarizedTaskView::from(task) or task.into() if From<Task> for SummarizedTaskView is implemented
    Ok(HttpResponse::Accepted().json(SummarizedTaskView::from(task)))
}

#[cfg(test)]
mod msfj_sis_api_handler_tests {
    use super::*;
    // Removed: use crate::extractors::authentication::policies::ActionGuard; // Import ActionGuard
    use actix_web::{http::StatusCode, test, web, App}; // Import App and StatusCode
    // Removed: use async_trait::async_trait;
    use clap::Parser;
    use index_scheduler::IndexSchedulerOptions; // Added IndexSchedulerOptions
    use meilisearch_auth::AuthController;
    use meilisearch_types::error::deserr_codes; // Add this line
    use meilisearch_types::heed::{EnvOpenOptions, WithoutTls}; // Removed Env
    use meilisearch_types::milli::update::IndexerConfig; // Added IndexerConfig
    use meilisearch_types::tasks::{Status, TaskId, Kind}; // Removed Task
    use meilisearch_types::error::ErrorCode; // Add this line
    // Removed: use core::marker::PhantomData; // For ActionGuard if Default is not derived
    // Removed: use serde_json::Value; // Add this line
    use meilisearch_types::versioning; // Added for version constants
    use meilisearch_types::VERSION_FILE_NAME; // Added for version file name
    use serde::Deserialize; // Add Deserialize for TestSummarizedTaskView
    use serde_json::Value; // Add this line
    use std::fs::{self, File}; // Add File
    use std::io::Write; // Add Write
    use std::path::PathBuf; // Add PathBuf
    use std::sync::Arc; // Added Arc for IndexerConfig
    // use std::sync::{Arc, Mutex}; // No longer needed for MockIndexScheduler fields
    use tempfile::tempdir;
    use time::OffsetDateTime;

    // Local struct for deserializing task view in tests
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    struct TestSummarizedTaskView {
        task_uid: TaskId,
        _index_uid: Option<String>,
        _status: Status,
        #[serde(rename = "type")]
        _kind: Kind,
        #[serde(with = "time::serde::rfc3339")] // Ensure OffsetDateTime can be deserialized
        _enqueued_at: OffsetDateTime,
    }

    // MockIndexScheduler is no longer needed as we will use a real IndexScheduler.
    // Helper function to create a real IndexScheduler for tests
    fn test_index_scheduler(opt: &Opt) -> IndexScheduler {
        // Create auth_env
        let auth_path = opt.db_path.join("auth_test_fj_snapshot");
        fs::create_dir_all(&auth_path).unwrap();
        let mut auth_env_options = EnvOpenOptions::new().read_txn_without_tls(); // Use read_txn_without_tls
        auth_env_options.map_size(100 * 1024 * 1024); // 100MB
        auth_env_options.max_dbs(10); // Default for auth
        let auth_env = unsafe { auth_env_options.open(&auth_path).unwrap() };

        // Define tasks_path (used by IndexSchedulerOptions)
        let tasks_path = opt.db_path.join("tasks_test_fj_snapshot");
        fs::create_dir_all(&tasks_path).unwrap();

        let update_file_path = opt.db_path.join("updates_test_fj_snapshot");
        fs::create_dir_all(&update_file_path).unwrap();
        let indexes_path = opt.db_path.join("indexes_test_fj_snapshot");
        fs::create_dir_all(&indexes_path).unwrap();
        let dumps_path = opt.db_path.join("dumps_test_fj_snapshot");
        fs::create_dir_all(&dumps_path).unwrap();


        let indexer_config = IndexerConfig { skip_index_budget: true, ..Default::default() };

        let options = IndexSchedulerOptions {
            version_file_path: opt.db_path.join(VERSION_FILE_NAME),
            auth_path,
            tasks_path,
            update_file_path,
            indexes_path,
            snapshots_path: opt.snapshot_dir.clone(), // snapshot_dir is already a PathBuf
            dumps_path,
            webhook_url: None,
            webhook_authorization_header: None,
            task_db_size: 10 * 1024 * 1024, // 10MB
            index_base_map_size: 1024 * 1024, // 1MB
            enable_mdb_writemap: false,
            index_growth_amount: 100 * 1024 * 1024, // 100MB, smaller for tests
            index_count: 5, // Default from test_utils
            indexer_config: Arc::new(indexer_config),
            autobatching_enabled: true, // Default from test_utils
            cleanup_enabled: true,     // Default from test_utils
            max_number_of_tasks: 1_000_000,
            max_number_of_batched_tasks: usize::MAX,
            batched_tasks_size_limit: u64::MAX,
            instance_features: Default::default(),
            auto_upgrade: true,
            embedding_cache_cap: 10,
        };

        let from_db_version = (
            versioning::VERSION_MAJOR.parse().unwrap(),
            versioning::VERSION_MINOR.parse().unwrap(),
            versioning::VERSION_PATCH.parse().unwrap(),
        );

        IndexScheduler::new(options, auth_env, from_db_version).unwrap()
    }


    fn default_opt_with_replication() -> Opt {
        // Use Opt::parse_from to allow clap to apply defaults, then override.
        let mut opt = Opt::parse_from(std::iter::empty::<&str>());
        opt.experimental_replication_parameters = true;
        // Ensure db_path and snapshot_dir are temporary and unique for tests
        opt.db_path = tempdir().unwrap().into_path();
        opt.snapshot_dir = tempdir().unwrap().into_path();
        // master_key will be None by default, which is fine for the first step of fixing 500s.
        opt
    }

    #[actix_rt::test]
    async fn test_fj_create_index_snapshot_success() {
        let opt = default_opt_with_replication();
        let app_opt = web::Data::new(opt.clone()); // Clone opt for auth controller and scheduler

        // Setup real IndexScheduler
        let index_scheduler_instance = test_index_scheduler(&opt);
        // Pass index_scheduler_instance directly, Data will handle cloning its wrapper
        let app_data_scheduler = web::Data::new(index_scheduler_instance);


        // Setup real AuthController
        let env_options_default = EnvOpenOptions::new();
        let mut env_options: EnvOpenOptions<WithoutTls> = env_options_default.read_txn_without_tls();
        env_options.map_size(100 * 1024 * 1024); // 100MB
        env_options.max_dbs(10); // Set max_dbs for AuthController
        let auth_db_path = opt.db_path.join("auth_db_test_create_snapshot");
        fs::create_dir_all(&auth_db_path).unwrap();
        let auth_env = unsafe { env_options.open(auth_db_path).unwrap() };
        let auth_controller = AuthController::new(auth_env, &opt.master_key).unwrap();
        let app_data_auth = web::Data::new(auth_controller);


        let index_uid_value = "test_index_create".to_string();
        let url = format!("/indexes/{}/snapshots", index_uid_value);

        // Setup Actix test service
        let app = test::init_service(
            App::new()
                .app_data(app_data_scheduler.clone())
                .app_data(app_opt.clone())
                .app_data(app_data_auth.clone())
                .service(
                    web::resource("/indexes/{index_uid}/snapshots")
                        .route(web::post().to(fj_create_index_snapshot)),
                ),
        )
        .await;

        // Create and send request
        let task_id_header_value = 123;
        let req = test::TestRequest::post()
            .uri(&url)
            .insert_header(("TaskId", task_id_header_value.to_string()))
            .to_request();

        let response = test::call_service(&app, req).await;

        // Assert response status
        assert_eq!(response.status(), StatusCode::ACCEPTED, "Response: {:?}", response);

        // Assert response body
        let response_task_view: TestSummarizedTaskView = test::read_body_json(response).await;
        
        // Fetch the actual task from the scheduler to verify its properties
        // Use get_tasks_from_authorized_indexes to fetch the task
        let query = index_scheduler::Query {
            uids: Some(vec![response_task_view.task_uid]), // Convert RoaringBitmap to Vec<u32>
            ..Default::default()
        };
        let (tasks, _) = app_data_scheduler // Use app_data_scheduler which holds the IndexScheduler
            .get_tasks_from_authorized_indexes(&query, &meilisearch_auth::AuthFilter::default())
            .unwrap();
        println!("[TEST_DRY_RUN] Found {} tasks with UID {:?}.", tasks.len(), response_task_view.task_uid);
        let registered_task = tasks.first().expect("Task not found after creation").clone();

        assert_eq!(registered_task.uid, response_task_view.task_uid);
        assert_eq!(registered_task.status, Status::Enqueued); // Initially enqueued

        match registered_task.kind {
            KindWithContent::SingleIndexSnapshotCreation { index_uid } => {
                assert_eq!(index_uid, index_uid_value);
            }
            _ => panic!("Incorrect task kind registered for snapshot creation. Got: {:?}", registered_task.kind),
        }
        
        assert_eq!(response_task_view.task_uid, task_id_header_value);
        // Note: The assertion for original_query.uid is implicitly covered by the above checks.
    }

    #[actix_rt::test]
    async fn test_fj_create_index_snapshot_dry_run() {
        let opt = default_opt_with_replication();
        let app_opt = web::Data::new(opt.clone());

        let index_scheduler_instance = test_index_scheduler(&opt);
        let app_data_scheduler = web::Data::new(index_scheduler_instance);

        let env_options_default = EnvOpenOptions::new();
        let mut env_options: EnvOpenOptions<WithoutTls> = env_options_default.read_txn_without_tls();
        env_options.map_size(100 * 1024 * 1024); 
        env_options.max_dbs(10);
        let auth_db_path = opt.db_path.join("auth_db_test_create_dry_run");
        fs::create_dir_all(&auth_db_path).unwrap();
        let auth_env = unsafe { env_options.open(auth_db_path).unwrap() };
        let auth_controller = AuthController::new(auth_env, &opt.master_key).unwrap();
        let app_data_auth = web::Data::new(auth_controller);

        let index_uid_value = "test_index_dry_run".to_string();
        let url = format!("/indexes/{}/snapshots", index_uid_value);

        let app = test::init_service(
            App::new()
                .app_data(app_data_scheduler.clone())
                .app_data(app_opt.clone())
                .app_data(app_data_auth.clone())
                .service(
                    web::resource("/indexes/{index_uid}/snapshots")
                        .route(web::post().to(fj_create_index_snapshot)),
                ),
        )
        .await;

        let req = test::TestRequest::post()
            .uri(&url)
            .insert_header(("DryRun", "true"))
            .to_request();

        let response = test::call_service(&app, req).await;

        assert_eq!(response.status(), StatusCode::ACCEPTED, "Response: {:?}", response);

        let response_task_view: TestSummarizedTaskView = test::read_body_json(response).await;
        println!("[TEST_DRY_RUN] Response Task UID: {:?}", response_task_view.task_uid);
            
        // For dry_run tasks, the task is not actually persisted in the queue by IndexScheduler.
        // So, we cannot fetch it afterwards. We can only check the response.
        assert_eq!(response_task_view._status, Status::Enqueued); // It's enqueued "virtually"
        assert_eq!(response_task_view._index_uid, Some(index_uid_value.clone()));
        match response_task_view._kind {
            Kind::SingleIndexSnapshotCreation => (), // Correct kind
            _ => panic!("Incorrect task kind for dry run snapshot creation. Got: {:?}", response_task_view._kind),
        }
        // If a TaskId header was sent, it would be checked against response_task_view.task_uid.
        // Since no header is sent, a new UID (likely 0) is generated and returned, which is expected.
    }

    #[actix_rt::test]
    async fn test_fj_import_index_snapshot_success() {
        let opt = default_opt_with_replication();
        let snapshot_dir_path = opt.snapshot_dir.clone(); // Use snapshot_dir from opt
        fs::create_dir_all(&snapshot_dir_path).unwrap();


        let snapshot_filename = "valid_snapshot-uid123.snapshot.tar.gz";
        let snapshot_file_full_path = snapshot_dir_path.join(snapshot_filename);
        File::create(&snapshot_file_full_path).unwrap().write_all(b"content").unwrap();

        let app_opt = web::Data::new(opt.clone());

        let index_scheduler_instance = test_index_scheduler(&opt);
        let app_data_scheduler = web::Data::new(index_scheduler_instance);
        
        let env_options_default = EnvOpenOptions::new();
        let mut env_options: EnvOpenOptions<WithoutTls> = env_options_default.read_txn_without_tls();
        env_options.map_size(100 * 1024 * 1024);
        env_options.max_dbs(10);
        let auth_db_path = opt.db_path.join("auth_db_test_import_success");
        fs::create_dir_all(&auth_db_path).unwrap();
        let auth_env = unsafe { env_options.open(auth_db_path).unwrap() };
        let auth_controller = AuthController::new(auth_env, &opt.master_key).unwrap();
        let app_data_auth = web::Data::new(auth_controller);


        let import_payload = FjSingleIndexSnapshotImportPayload {
            source_snapshot_filename: snapshot_filename.to_string(),
            target_index_uid: "imported_target_index".to_string(),
        };

        let app = test::init_service(
            App::new()
                .app_data(app_data_scheduler.clone())
                .app_data(app_opt.clone())
                .app_data(app_data_auth.clone())
                .service(
                    web::resource("/snapshots/import")
                        .route(web::post().to(fj_import_index_snapshot)),
                ),
        )
        .await;

        let task_id_header_value = 456;
        let req = test::TestRequest::post()
            .uri("/snapshots/import")
            .insert_header(("TaskId", task_id_header_value.to_string()))
            .set_json(&import_payload)
            .to_request();

        let response = test::call_service(&app, req).await;

        assert_eq!(response.status(), StatusCode::ACCEPTED, "Response: {:?}", response);

        let response_task_view: TestSummarizedTaskView = test::read_body_json(response).await;
        
        let query = index_scheduler::Query {
            uids: Some(vec![response_task_view.task_uid]),
            ..Default::default()
        };
        let (tasks, _) = app_data_scheduler
            .get_tasks_from_authorized_indexes(&query, &meilisearch_auth::AuthFilter::default())
            .unwrap();
        let registered_task = tasks.first().expect("Task not found after creation").clone();
        
        assert_eq!(registered_task.uid, response_task_view.task_uid);
        // Note: Cannot assert registered_task.original_query.uid or .dry_run as `original_query` is not a field on `Task`.
        // The check for task_id_header_value against registered_task.uid is implicitly covered by:
        // response_task_view.task_uid == task_id_header_value (from test setup)
        // registered_task.uid == response_task_view.task_uid (asserted above)

        match registered_task.kind {
            KindWithContent::SingleIndexSnapshotImport { source_snapshot_path, target_index_uid } => {
                assert_eq!(
                    PathBuf::from(source_snapshot_path),
                    snapshot_file_full_path.canonicalize().unwrap()
                );
                assert_eq!(target_index_uid, "imported_target_index");
            }
            _ => panic!("Incorrect task kind registered for snapshot import. Got: {:?}", registered_task.kind),
        }
    }

    #[actix_rt::test]
    async fn test_fj_import_index_snapshot_invalid_filename_traversal() {
        let opt = default_opt_with_replication();
        let app_opt = web::Data::new(opt.clone());

        let index_scheduler_instance = test_index_scheduler(&opt);
        let app_data_scheduler = web::Data::new(index_scheduler_instance);

        let env_options_default = EnvOpenOptions::new();
        let mut env_options: EnvOpenOptions<WithoutTls> = env_options_default.read_txn_without_tls();
        env_options.map_size(100 * 1024 * 1024);
        env_options.max_dbs(10);
        let auth_db_path = opt.db_path.join("auth_db_test_import_trav");
        fs::create_dir_all(&auth_db_path).unwrap();
        let auth_env = unsafe { env_options.open(auth_db_path).unwrap() };
        let auth_controller = AuthController::new(auth_env, &opt.master_key).unwrap();
        let app_data_auth = web::Data::new(auth_controller);


        let import_payload = FjSingleIndexSnapshotImportPayload {
            source_snapshot_filename: "../prohibited.snapshot.tar.gz".to_string(),
            target_index_uid: "target".to_string(),
        };

        let app = test::init_service(
            App::new()
                .app_data(app_data_scheduler.clone())
                .app_data(app_opt.clone())
                .app_data(app_data_auth.clone())
                .service(
                    web::resource("/snapshots/import")
                        .route(web::post().to(fj_import_index_snapshot)),
                ),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/snapshots/import")
            .set_json(&import_payload)
            .to_request();

        let response = test::call_service(&app, req).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    
        let err: ResponseError = test::read_body_json(response).await;
        println!("[TEST_IMPORT_ABS_PATH] Actual error message: '{}'", err.message);
        println!("[TEST_IMPORT_ABS_PATH] Actual error code: '{:?}'", err.code);
        let err_value: Value = serde_json::to_value(&err).expect("Failed to serialize ResponseError");
        let actual_code = err_value.get("code").and_then(Value::as_str).expect("Serialized ResponseError missing 'code' field");
        let expected_code = deserr_codes::InvalidSnapshotPath::default().error_name();
        assert_eq!(actual_code, expected_code);
        assert!(err.message.contains("cannot contain '..'"));
    }

    #[actix_rt::test]
    async fn test_fj_import_index_snapshot_invalid_filename_absolute_path() {
        let opt = default_opt_with_replication();
        let app_opt = web::Data::new(opt.clone());

        let index_scheduler_instance = test_index_scheduler(&opt);
        let app_data_scheduler = web::Data::new(index_scheduler_instance);
        
        let env_options_default = EnvOpenOptions::new();
        let mut env_options: EnvOpenOptions<WithoutTls> = env_options_default.read_txn_without_tls();
        env_options.map_size(100 * 1024 * 1024);
        env_options.max_dbs(10);
        let auth_db_path = opt.db_path.join("auth_db_test_import_abs");
        fs::create_dir_all(&auth_db_path).unwrap();
        let auth_env = unsafe { env_options.open(auth_db_path).unwrap() };
        let auth_controller = AuthController::new(auth_env, &opt.master_key).unwrap();
        let app_data_auth = web::Data::new(auth_controller);

        let import_payload = FjSingleIndexSnapshotImportPayload {
            source_snapshot_filename: "/abs/path.snapshot.tar.gz".to_string(),
            target_index_uid: "target".to_string(),
        };

        let app = test::init_service(
            App::new()
                .app_data(app_data_scheduler.clone())
                .app_data(app_opt.clone())
                .app_data(app_data_auth.clone())
                .service(
                    web::resource("/snapshots/import")
                        .route(web::post().to(fj_import_index_snapshot)),
                ),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/snapshots/import")
            .set_json(&import_payload)
            .to_request();

        let response = test::call_service(&app, req).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let err: ResponseError = test::read_body_json(response).await;
        println!("[TEST_IMPORT_ABS_PATH] Actual error message for absolute_path test: '{}'", err.message);
        println!("[TEST_IMPORT_ABS_PATH] Actual error code for absolute_path test: '{:?}'", err.code);
        let err_value: Value = serde_json::to_value(&err).expect("Failed to serialize ResponseError");
        let actual_code = err_value.get("code").and_then(Value::as_str).expect("Serialized ResponseError missing 'code' field");
        let expected_code = deserr_codes::InvalidSnapshotPath::default().error_name();
        assert_eq!(actual_code, expected_code);
        let substring_to_check = "absolute path"; // Corrected substring
        let message_contains_substring = err.message.contains(substring_to_check);
        println!("[TEST_IMPORT_ABS_PATH] err.message: '{}'", err.message);
        println!("[TEST_IMPORT_ABS_PATH] err.message (bytes): {:?}", err.message.as_bytes());
        println!("[TEST_IMPORT_ABS_PATH] substring_to_check: '{}'", substring_to_check);
        println!("[TEST_IMPORT_ABS_PATH] substring_to_check (bytes): {:?}", substring_to_check.as_bytes());
        println!("[TEST_IMPORT_ABS_PATH] message_contains_substring_result: {}", message_contains_substring);
        assert!(message_contains_substring, "Assertion failed: err.message ('{}') does not contain '{}'", err.message, substring_to_check);
    }

    #[actix_rt::test]
    async fn test_fj_import_index_snapshot_invalid_extension() {
        let opt = default_opt_with_replication();
        let app_opt = web::Data::new(opt.clone());

        let index_scheduler_instance = test_index_scheduler(&opt);
        let app_data_scheduler = web::Data::new(index_scheduler_instance);

        let env_options_default = EnvOpenOptions::new();
        let mut env_options: EnvOpenOptions<WithoutTls> = env_options_default.read_txn_without_tls();
        env_options.map_size(100 * 1024 * 1024);
        env_options.max_dbs(10);
        let auth_db_path = opt.db_path.join("auth_db_test_import_ext");
        fs::create_dir_all(&auth_db_path).unwrap();
        let auth_env = unsafe { env_options.open(auth_db_path).unwrap() };
        let auth_controller = AuthController::new(auth_env, &opt.master_key).unwrap();
        let app_data_auth = web::Data::new(auth_controller);

        let import_payload = FjSingleIndexSnapshotImportPayload {
            source_snapshot_filename: "backup.zip".to_string(),
            target_index_uid: "target".to_string(),
        };

        let app = test::init_service(
            App::new()
                .app_data(app_data_scheduler.clone())
                .app_data(app_opt.clone())
                .app_data(app_data_auth.clone())
                .service(
                    web::resource("/snapshots/import")
                        .route(web::post().to(fj_import_index_snapshot)),
                ),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/snapshots/import")
            .set_json(&import_payload)
            .to_request();

        let response = test::call_service(&app, req).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let err: ResponseError = test::read_body_json(response).await;
        let err_value: Value = serde_json::to_value(&err).expect("Failed to serialize ResponseError");
        let actual_code = err_value.get("code").and_then(Value::as_str).expect("Serialized ResponseError missing 'code' field");
        let expected_code = deserr_codes::InvalidSnapshotPath::default().error_name();
        assert_eq!(actual_code, expected_code);
        assert!(err.message.contains("must end with '.snapshot.tar.gz'"));
    }

    #[actix_rt::test]
    async fn test_fj_import_index_snapshot_file_not_found() {
        let opt = default_opt_with_replication();
        let snapshot_dir_path = opt.snapshot_dir.clone();
        fs::create_dir_all(&snapshot_dir_path).unwrap();
        let app_opt = web::Data::new(opt.clone());

        let index_scheduler_instance = test_index_scheduler(&opt);
        let app_data_scheduler = web::Data::new(index_scheduler_instance);

        let env_options_default = EnvOpenOptions::new();
        let mut env_options: EnvOpenOptions<WithoutTls> = env_options_default.read_txn_without_tls();
        env_options.map_size(100 * 1024 * 1024);
        env_options.max_dbs(10);
        let auth_db_path = opt.db_path.join("auth_db_test_import_nf");
        fs::create_dir_all(&auth_db_path).unwrap();
        let auth_env = unsafe { env_options.open(auth_db_path).unwrap() };
        let auth_controller = AuthController::new(auth_env, &opt.master_key).unwrap();
        let app_data_auth = web::Data::new(auth_controller);

        let import_payload = FjSingleIndexSnapshotImportPayload {
            source_snapshot_filename: "non_existent.snapshot.tar.gz".to_string(),
            target_index_uid: "target".to_string(),
        };

        let app = test::init_service(
            App::new()
                .app_data(app_data_scheduler.clone())
                .app_data(app_opt.clone())
                .app_data(app_data_auth.clone())
                .service(
                    web::resource("/snapshots/import")
                        .route(web::post().to(fj_import_index_snapshot)),
                ),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/snapshots/import")
            .set_json(&import_payload)
            .to_request();

        let response = test::call_service(&app, req).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let err: ResponseError = test::read_body_json(response).await;
        let err_value: Value = serde_json::to_value(&err).expect("Failed to serialize ResponseError");
        let actual_code = err_value.get("code").and_then(Value::as_str).expect("Serialized ResponseError missing 'code' field");
        let expected_code = deserr_codes::InvalidSnapshotPath::default().error_name(); 
        assert_eq!(actual_code, expected_code);
        assert!(err.message.contains("not found or not accessible"));
    }

    #[actix_rt::test]
    async fn test_fj_import_index_snapshot_path_outside_snapshot_dir() {
        let opt = default_opt_with_replication();
        let temp_dir_root = tempdir().unwrap(); // Use a fresh tempdir for this test's structure
        let actual_snapshots_dir = temp_dir_root.path().join("actual_snapshots");
        let sibling_dir = temp_dir_root.path().join("sibling_dir");
        fs::create_dir_all(&actual_snapshots_dir).unwrap();
        fs::create_dir_all(&sibling_dir).unwrap();

        let malicious_snapshot_filename = "external.snapshot.tar.gz";
        let malicious_file_path = sibling_dir.join(malicious_snapshot_filename);
        File::create(&malicious_file_path).unwrap().write_all(b"malicious").unwrap();
        
        // Override opt.snapshot_dir for this test
        let mut opt_modified = opt.clone();
        opt_modified.snapshot_dir = actual_snapshots_dir.clone();
        let app_opt = web::Data::new(opt_modified.clone());


        let index_scheduler_instance = test_index_scheduler(&opt_modified); // Use modified opt
        let app_data_scheduler = web::Data::new(index_scheduler_instance);
        
        let env_options_default = EnvOpenOptions::new();
        let mut env_options: EnvOpenOptions<WithoutTls> = env_options_default.read_txn_without_tls();
        env_options.map_size(100 * 1024 * 1024);
        env_options.max_dbs(10);
        let auth_db_path = opt.db_path.join("auth_db_test_import_outside"); // opt.db_path is fine
        fs::create_dir_all(&auth_db_path).unwrap();
        let auth_env = unsafe { env_options.open(auth_db_path).unwrap() };
        let auth_controller = AuthController::new(auth_env, &opt.master_key).unwrap();
        let app_data_auth = web::Data::new(auth_controller);

        let import_payload = FjSingleIndexSnapshotImportPayload {
            source_snapshot_filename: format!("../sibling_dir/{}", malicious_snapshot_filename),
            target_index_uid: "target".to_string(),
        };

        let app = test::init_service(
            App::new()
                .app_data(app_data_scheduler.clone())
                .app_data(app_opt.clone())
                .app_data(app_data_auth.clone())
                .service(
                    web::resource("/snapshots/import")
                        .route(web::post().to(fj_import_index_snapshot)),
                ),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/snapshots/import")
            .set_json(&import_payload)
            .to_request();

        let response = test::call_service(&app, req).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let err: ResponseError = test::read_body_json(response).await;
        let err_value: Value = serde_json::to_value(&err).expect("Failed to serialize ResponseError");
        let actual_code = err_value.get("code").and_then(Value::as_str).expect("Serialized ResponseError missing 'code' field");
        let expected_code = deserr_codes::InvalidSnapshotPath::default().error_name();
        assert_eq!(actual_code, expected_code);
        assert!(err.message.contains("Path is outside the configured snapshots directory") || err.message.contains("cannot contain '..'"));
    }

    #[actix_rt::test]
    async fn test_fj_import_index_snapshot_path_is_directory() {
        let opt = default_opt_with_replication();
        let snapshot_dir_path = opt.snapshot_dir.clone();
        fs::create_dir_all(&snapshot_dir_path).unwrap();

        let directory_as_snapshot_filename = "a_directory.snapshot.tar.gz";
        fs::create_dir_all(snapshot_dir_path.join(directory_as_snapshot_filename)).unwrap();

        let app_opt = web::Data::new(opt.clone());

        let index_scheduler_instance = test_index_scheduler(&opt);
        let app_data_scheduler = web::Data::new(index_scheduler_instance);

        let env_options_default = EnvOpenOptions::new();
        let mut env_options: EnvOpenOptions<WithoutTls> = env_options_default.read_txn_without_tls();
        env_options.map_size(100 * 1024 * 1024);
        env_options.max_dbs(10);
        let auth_db_path = opt.db_path.join("auth_db_test_import_isdir");
        fs::create_dir_all(&auth_db_path).unwrap();
        let auth_env = unsafe { env_options.open(auth_db_path).unwrap() };
        let auth_controller = AuthController::new(auth_env, &opt.master_key).unwrap();
        let app_data_auth = web::Data::new(auth_controller);

        let import_payload = FjSingleIndexSnapshotImportPayload {
            source_snapshot_filename: directory_as_snapshot_filename.to_string(),
            target_index_uid: "target".to_string(),
        };

        let app = test::init_service(
            App::new()
                .app_data(app_data_scheduler.clone())
                .app_data(app_opt.clone())
                .app_data(app_data_auth.clone())
                .service(
                    web::resource("/snapshots/import")
                        .route(web::post().to(fj_import_index_snapshot)),
                ),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/snapshots/import")
            .set_json(&import_payload)
            .to_request();

        let response = test::call_service(&app, req).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let err: ResponseError = test::read_body_json(response).await;
        let err_value: Value = serde_json::to_value(&err).expect("Failed to serialize ResponseError");
        let actual_code = err_value.get("code").and_then(Value::as_str).expect("Serialized ResponseError missing 'code' field");
        let expected_code = deserr_codes::InvalidSnapshotPath::default().error_name();
        assert_eq!(actual_code, expected_code);
        assert!(err.message.contains("does not point to a file"));
    }
}

pub async fn fj_import_index_snapshot(
    index_scheduler: GuardedData<ActionPolicy<{ actions::SNAPSHOTS_CREATE }>, Data<IndexScheduler>>,
    payload: web::Json<FjSingleIndexSnapshotImportPayload>,
    req: HttpRequest,
    opt: web::Data<Opt>,
) -> Result<HttpResponse, ResponseError> {
    let FjSingleIndexSnapshotImportPayload { source_snapshot_filename, target_index_uid } =
        payload.into_inner();
    
    println!("[HANDLER_IMPORT_SNAPSHOT] Received source_snapshot_filename: '{}'", source_snapshot_filename);
    // Security Check for Snapshot Path
    if source_snapshot_filename.contains("..")
        || source_snapshot_filename.starts_with('/')
        || source_snapshot_filename.starts_with('\\')
    {
        println!("[HANDLER_IMPORT_SNAPSHOT] Detected absolute path or traversal for: '{}'", source_snapshot_filename);
        return Err(ResponseError::from_msg(
            format!(
                "Invalid snapshot filename provided: '{}'. Filename cannot contain '..' or be an absolute path.",
                source_snapshot_filename
            ),
            Code::InvalidSnapshotPath,
        ));
    }
    println!("[HANDLER_IMPORT_SNAPSHOT] Passed absolute path/traversal check for: '{}'", source_snapshot_filename);

    if !source_snapshot_filename.ends_with(".snapshot.tar.gz") {
        // Or any other configured/expected extension
        return Err(ResponseError::from_msg(
            format!(
                "Invalid snapshot filename provided: '{}'. Filename must end with '.snapshot.tar.gz'.",
                source_snapshot_filename
            ),
            Code::InvalidSnapshotPath,
        ));
    }

    let snapshot_dir_path = &opt.snapshot_dir;
    let source_snapshot_full_path = snapshot_dir_path.join(&source_snapshot_filename);

    let canonical_snapshot_dir = match std::fs::canonicalize(snapshot_dir_path) {
        Ok(path) => path,
        Err(e) => {
            return Err(ResponseError::from_msg(
                format!("Failed to access snapshot directory: {}. Error: {}", snapshot_dir_path.display(), e),
                Code::Internal, // Or a more specific error if available
            ));
        }
    };

    let canonical_source_path = match std::fs::canonicalize(&source_snapshot_full_path) {
        Ok(path) => path,
        Err(_) => {
            return Err(ResponseError::from_msg(
                format!("Snapshot file '{}' not found or not accessible.", source_snapshot_filename),
                Code::InvalidSnapshotPath, // Or NotFound if more appropriate and available
            ));
        }
    };

    if !canonical_source_path.starts_with(&canonical_snapshot_dir)
        || canonical_source_path == canonical_snapshot_dir
    {
        return Err(ResponseError::from_msg(
            format!(
                "Invalid snapshot file path provided: '{}'. Path is outside the configured snapshots directory.",
                source_snapshot_filename
            ),
            Code::InvalidSnapshotPath,
        ));
    }

    if !canonical_source_path.is_file() {
        return Err(ResponseError::from_msg(
            format!("Snapshot path '{}' does not point to a file.", source_snapshot_filename),
            Code::InvalidSnapshotPath,
        ));
    }

    let task_kind = KindWithContent::SingleIndexSnapshotImport {
        source_snapshot_path: canonical_source_path.to_string_lossy().into_owned(),
        target_index_uid,
    };

    let uid: Option<TaskId> = get_task_id(&req, &opt)?;
    let dry_run = is_dry_run(&req, &opt)?;

    let task = index_scheduler.register(task_kind, uid, dry_run)?;

    Ok(HttpResponse::Accepted().json(SummarizedTaskView::from(task)))
}
// Removed duplicated function definition that was here

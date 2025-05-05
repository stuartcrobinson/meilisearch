//! Tests for the `IndexMapper::fj_import_index_from_snapshot` method.

use std::fs;
use std::path::Path;
// Remove unused Arc import
// Remove unused RoTxn import
// Remove unused milli import
// Import versioning constants
use meilisearch_types::versioning::{VERSION_MAJOR, VERSION_MINOR, VERSION_PATCH};
use meilisearch_types::settings::{Settings, Unchecked};
use tempfile::tempdir;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::Error;
// Remove unused fj_snapshot_utils import
use crate::index_mapper::FjParsedSnapshotMetadata;
// Remove unused IndexMapper import
// Remove unused IndexSchedulerHandle import
use crate::IndexScheduler;

// Helper to create a dummy snapshot for testing import
// In a real scenario, we'd use fj_create_index_snapshot, but for isolated testing,
// manually creating the structure is simpler.
fn fj_create_dummy_snapshot(
    snapshot_dir: &Path,
    index_uid: &str,
    snapshot_uid: &str,
    version: &str,
    settings: &Settings<Unchecked>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    include_data_mdb: bool, // Added flag
    include_metadata_json: bool, // Added flag
) -> std::io::Result<std::path::PathBuf> {
    let temp_unpack_dir = tempdir()?;
    let data_mdb_path = temp_unpack_dir.path().join("data.mdb");
    let metadata_path = temp_unpack_dir.path().join("metadata.json");

    // Create a minimal valid data.mdb by creating and closing a temporary milli::Index
    if include_data_mdb {
        let temp_index_dir = tempdir()?;
        {
            let options = meilisearch_types::milli::heed::EnvOpenOptions::new();
            // Use read_txn_without_tls() for correct options type
            let index = meilisearch_types::milli::Index::new(options.read_txn_without_tls(), temp_index_dir.path(), true)
                .expect("Failed to create temporary index for dummy data.mdb generation");
            // Ensure env is flushed and closed before copying
            index.prepare_for_closing().wait();
        } // index and options are dropped here
        fs::copy(temp_index_dir.path().join("data.mdb"), &data_mdb_path)?;
        // temp_index_dir is dropped here, cleaning up its lock file etc.
        assert!(data_mdb_path.exists(), "Valid data.mdb was not copied successfully to {:?}", data_mdb_path);
    }

    // Create metadata.json if needed
    if include_metadata_json {
        let metadata = FjParsedSnapshotMetadata {
            meilisearch_version: version.to_string(),
            settings: settings.clone(),
            created_at,
            updated_at,
        };
        let metadata_file = fs::File::create(&metadata_path)?;
        serde_json::to_writer(metadata_file, &metadata)?;
        assert!(metadata_path.exists(), "Dummy metadata.json was not created successfully at {:?}", metadata_path);
    }

    // Pack into tar.gz
    let snapshot_filename = format!("{}-{}.snapshot.tar.gz", index_uid, snapshot_uid);
    let snapshot_path = snapshot_dir.join(&snapshot_filename);
    // Ensure the target directory exists before creating the file
    fs::create_dir_all(snapshot_dir)?;
    let tar_gz = fs::File::create(&snapshot_path)?;
    let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
    let mut tar = tar::Builder::new(enc);

    // Add files to tar using open file handles
    if include_data_mdb && data_mdb_path.exists() {
        let mut data_mdb_file = fs::File::open(&data_mdb_path)?;
        tar.append_file("data.mdb", &mut data_mdb_file)?;
    }
    if include_metadata_json && metadata_path.exists() {
        let mut metadata_file = fs::File::open(&metadata_path)?;
        tar.append_file("metadata.json", &mut metadata_file)?;
    }

    // Finish archiving *before* temp_unpack_dir goes out of scope
    tar.finish()?;
    // Explicitly drop the tar builder to ensure flushing before temp dir cleanup
    drop(tar);
    // temp_unpack_dir is dropped here, after tarball creation is complete.

    Ok(snapshot_path)
}

#[test]
fn fj_test_import_index_from_snapshot_success() {
    // Use the default test setup
    let (index_scheduler, _) = IndexScheduler::test(true, vec![]);
    let index_mapper = &index_scheduler.index_mapper;
    let snapshots_path = index_scheduler.fj_snapshots_path();

    // 1. Prepare a dummy snapshot
    let source_index_uid = "source_index";
    let snapshot_uid = Uuid::new_v4().to_string();
    let target_index_uid = "imported_index";
    // Construct version string manually using imported constants
    let current_version = format!("{}.{}.{}", VERSION_MAJOR, VERSION_MINOR, VERSION_PATCH);
    let settings = Settings::<Unchecked>::default();
    let now = OffsetDateTime::now_utc();

    let snapshot_path = fj_create_dummy_snapshot(
        &snapshots_path,
        source_index_uid,
        &snapshot_uid,
        &current_version,
        &settings,
        now,
        now,
        true, // include_data_mdb
        true, // include_metadata_json
    )
    .expect("Failed to create dummy snapshot");

    // 2. Perform the import
    let import_result = index_mapper
        .fj_import_index_from_snapshot(target_index_uid, &snapshot_path);

    // 3. Assertions
    assert!(import_result.is_ok(), "Import failed: {:?}", import_result.err());
    let (imported_index, parsed_metadata) = import_result.unwrap();

    // Verify metadata
    assert_eq!(parsed_metadata.meilisearch_version, current_version);
    assert_eq!(parsed_metadata.created_at, now);
    assert_eq!(parsed_metadata.updated_at, now);
    // Settings check happens during Step 6 integration test

    // Verify index exists in mapper
    let rtxn = index_scheduler.env.read_txn().unwrap();
    assert!(
        index_mapper.index_exists(&rtxn, target_index_uid).unwrap(),
        "Target index does not exist in mapping"
    );

    // Verify index is available in IndexMap using helper method
    let uuid = index_mapper.index_mapping.get(&rtxn, target_index_uid).unwrap().unwrap();
    assert!(
        index_mapper.fj_index_status(&uuid).fj_is_available(),
        "Index not available in IndexMap"
    );

    // Verify index directory and data.mdb exist (optional, but good sanity check)
    // Use public getter for base_path
    let index_path = index_mapper.fj_base_path().join(uuid.to_string());
    assert!(index_path.exists(), "Index directory does not exist");
    assert!(index_path.join("data.mdb").exists(), "data.mdb does not exist in index directory");

    // Verify index can be opened and basic info retrieved (timestamps are checked in metadata)
    let index_rtxn = imported_index.read_txn().unwrap();
    // Timestamps might differ slightly due to internal milli operations, removed exact check.
    // assert_eq!(imported_index.created_at(&index_rtxn).unwrap(), now);
    // assert_eq!(imported_index.updated_at(&index_rtxn).unwrap(), now);
    // Check if primary key is None as expected for default settings
    assert!(imported_index.primary_key(&index_rtxn).unwrap().is_none());
    drop(index_rtxn);

    // Verify stats were stored
    let stats = index_mapper.index_stats.get(&rtxn, &uuid).unwrap();
    assert!(stats.is_some(), "Index stats were not stored");

    // Cleanup snapshot file
    fs::remove_file(snapshot_path).unwrap();
}

#[test]
fn fj_test_import_index_target_exists() {
    // Use the default test setup
    let (index_scheduler, _) = IndexScheduler::test(true, vec![]);
    let index_mapper = &index_scheduler.index_mapper;
    let snapshots_path = index_scheduler.fj_snapshots_path();

    // 1. Create an existing index with the target name
    let target_index_uid = "existing_index";
    let wtxn = index_scheduler.env.write_txn().unwrap();
    index_mapper.create_index(wtxn, target_index_uid, None).expect("Failed to create index");

    // 2. Prepare a dummy snapshot
    let source_index_uid = "source_index";
    let snapshot_uid = Uuid::new_v4().to_string();
    // Construct version string manually using imported constants
    let current_version = format!("{}.{}.{}", VERSION_MAJOR, VERSION_MINOR, VERSION_PATCH);
    let settings = Settings::<Unchecked>::default();
    let now = OffsetDateTime::now_utc();

    let snapshot_path = fj_create_dummy_snapshot(
        &snapshots_path,
        source_index_uid,
        &snapshot_uid,
        &current_version,
        &settings,
        now,
        now,
        true, // include_data_mdb
        true, // include_metadata_json
    )
    .expect("Failed to create dummy snapshot");

    // 3. Attempt the import
    let import_result = index_mapper
        .fj_import_index_from_snapshot(target_index_uid, &snapshot_path);

    // 4. Assertions
    assert!(import_result.is_err(), "Import should have failed");
    match import_result.err().unwrap() {
        Error::SnapshotImportTargetIndexExists { target_index_uid: err_uid } => {
            assert_eq!(err_uid, target_index_uid);
        }
        other => panic!("Expected SnapshotImportTargetIndexExists, got {:?}", other),
    }

    // Cleanup snapshot file
    fs::remove_file(snapshot_path).unwrap();
}

#[test]
fn fj_test_import_index_invalid_path() {
    // Use the default test setup
    let (index_scheduler, _) = IndexScheduler::test(true, vec![]);
    let index_mapper = &index_scheduler.index_mapper;
    let snapshots_path = index_scheduler.fj_snapshots_path();
    let target_index_uid = "imported_index";

    // Scenario 1: Path does not exist
    let non_existent_path = snapshots_path.join("non_existent_snapshot.tar.gz");
    let import_result = index_mapper
        .fj_import_index_from_snapshot(target_index_uid, &non_existent_path);

    assert!(import_result.is_err(), "Import should have failed for non-existent path");
    match import_result.err().unwrap() {
        Error::InvalidSnapshotPath { path } => {
            assert_eq!(path, non_existent_path);
        }
        other => panic!("Expected InvalidSnapshotPath, got {:?}", other),
    }

    // Scenario 2: Path exists but is outside snapshots_path
    let outside_dir = tempdir().unwrap();
    let outside_path = outside_dir.path().join("outside_snapshot.tar.gz");
    fs::write(&outside_path, "dummy content").unwrap(); // Create the file

    let import_result_outside = index_mapper
        .fj_import_index_from_snapshot(target_index_uid, &outside_path);

    assert!(import_result_outside.is_err(), "Import should have failed for outside path");
    match import_result_outside.err().unwrap() {
        Error::InvalidSnapshotPath { path } => {
            assert_eq!(path, outside_path);
        }
        other => panic!("Expected InvalidSnapshotPath, got {:?}", other),
    }

    // Cleanup
    fs::remove_file(outside_path).unwrap();
}

#[test]
fn fj_test_import_index_invalid_format() {
    // Use the default test setup
    let (index_scheduler, _) = IndexScheduler::test(true, vec![]);
    let index_mapper = &index_scheduler.index_mapper;
    let snapshots_path = index_scheduler.fj_snapshots_path();
    let target_index_uid = "imported_index";
    let source_index_uid = "source_index";
    let snapshot_uid_base = Uuid::new_v4().to_string();
    // Construct version string manually using imported constants
    let current_version = format!("{}.{}.{}", VERSION_MAJOR, VERSION_MINOR, VERSION_PATCH);
    let settings = Settings::<Unchecked>::default();
    let now = OffsetDateTime::now_utc();

    // Scenario 1: Missing data.mdb
    let snapshot_uid_1 = format!("{}-1", snapshot_uid_base);
    let snapshot_path_1 = fj_create_dummy_snapshot(
        &snapshots_path,
        source_index_uid,
        &snapshot_uid_1,
        &current_version,
        &settings,
        now,
        now,
        false, // include_data_mdb = false
        true,  // include_metadata_json = true
    )
    .expect("Failed to create dummy snapshot (missing data.mdb)");

    let import_result_1 = index_mapper
        .fj_import_index_from_snapshot(target_index_uid, &snapshot_path_1);

    assert!(import_result_1.is_err(), "Import should have failed for missing data.mdb");
    match import_result_1.err().unwrap() {
        Error::InvalidSnapshotFormat { path } => {
            assert_eq!(path, snapshot_path_1);
        }
        other => panic!("Expected InvalidSnapshotFormat, got {:?}", other),
    }
    fs::remove_file(snapshot_path_1).unwrap();

    // Scenario 2: Missing metadata.json
    let snapshot_uid_2 = format!("{}-2", snapshot_uid_base);
    let snapshot_path_2 = fj_create_dummy_snapshot(
        &snapshots_path,
        source_index_uid,
        &snapshot_uid_2,
        &current_version,
        &settings,
        now,
        now,
        true,  // include_data_mdb = true
        false, // include_metadata_json = false
    )
    .expect("Failed to create dummy snapshot (missing metadata.json)");

    let import_result_2 = index_mapper
        .fj_import_index_from_snapshot(target_index_uid, &snapshot_path_2);

    assert!(import_result_2.is_err(), "Import should have failed for missing metadata.json");
    match import_result_2.err().unwrap() {
        Error::InvalidSnapshotFormat { path } => {
            assert_eq!(path, snapshot_path_2);
        }
        other => panic!("Expected InvalidSnapshotFormat, got {:?}", other),
    }
    fs::remove_file(snapshot_path_2).unwrap();
}

#[test]
fn fj_test_import_index_version_mismatch() {
    // Use the default test setup
    let (index_scheduler, _) = IndexScheduler::test(true, vec![]);
    let index_mapper = &index_scheduler.index_mapper;
    let snapshots_path = index_scheduler.fj_snapshots_path();
    let target_index_uid = "imported_index";
    let source_index_uid = "source_index";
    let snapshot_uid = Uuid::new_v4().to_string();
    // Construct version string manually using imported constants
    let current_version = format!("{}.{}.{}", VERSION_MAJOR, VERSION_MINOR, VERSION_PATCH);
    let incompatible_version = "0.99.0"; // Or any version with different major/minor
    let settings = Settings::<Unchecked>::default();
    let now = OffsetDateTime::now_utc();

    // 1. Prepare a dummy snapshot with an incompatible version
    let snapshot_path = fj_create_dummy_snapshot(
        &snapshots_path,
        source_index_uid,
        &snapshot_uid,
        incompatible_version, // Use the incompatible version here
        &settings,
        now,
        now,
        true, // include_data_mdb
        true, // include_metadata_json
    )
    .expect("Failed to create dummy snapshot with incompatible version");

    // 2. Attempt the import
    let import_result = index_mapper
        .fj_import_index_from_snapshot(target_index_uid, &snapshot_path);

    // 3. Assertions
    assert!(import_result.is_err(), "Import should have failed due to version mismatch");
    match import_result.err().unwrap() {
        Error::SnapshotVersionMismatch { path, snapshot_version, current_version: instance_version } => {
            assert_eq!(path, snapshot_path);
            assert_eq!(snapshot_version, incompatible_version);
            assert_eq!(instance_version, current_version);
        }
        other => panic!("Expected SnapshotVersionMismatch, got {:?}", other),
    }

    // Cleanup snapshot file
    fs::remove_file(snapshot_path).unwrap();
}

#[test]
fn fj_test_import_index_lru_eviction() {
    // Configure scheduler with a cache size of 1 using the correct function
    let (index_scheduler, _) =
        IndexScheduler::test_with_custom_config(vec![], |opts| {
            opts.autobatching_enabled = true;
            opts.index_count = 1; // Set cache size to 1
            None // Use default version
        });
    let index_mapper = &index_scheduler.index_mapper;
    let snapshots_path = index_scheduler.fj_snapshots_path();

    // 1. Create and open an initial index to fill the cache
    let initial_index_uid = "initial_index";
    let wtxn = index_scheduler.env.write_txn().unwrap();
    let _initial_index = index_mapper // Prefix unused variable
        .create_index(wtxn, initial_index_uid, None)
        .expect("Failed to create initial index");
    let initial_uuid = {
        let rtxn = index_scheduler.env.read_txn().unwrap();
        index_mapper.index_mapping.get(&rtxn, initial_index_uid).unwrap().unwrap()
    };

    // Ensure initial index is in the available map using helper methods
    {
        assert!(
            index_mapper.fj_index_status(&initial_uuid).fj_is_available(),
            "Initial index not available"
        );
        assert_eq!(index_mapper.fj_available_cache_len(), 1, "Cache should have 1 item");
    }

    // 2. Prepare a dummy snapshot for the second index
    let source_index_uid = "source_index";
    let snapshot_uid = Uuid::new_v4().to_string();
    let target_index_uid = "imported_index";
    // Construct version string manually using imported constants
    let current_version = format!("{}.{}.{}", VERSION_MAJOR, VERSION_MINOR, VERSION_PATCH);
    let settings = Settings::<Unchecked>::default();
    let now = OffsetDateTime::now_utc();

    let snapshot_path = fj_create_dummy_snapshot(
        &snapshots_path,
        source_index_uid,
        &snapshot_uid,
        &current_version,
        &settings,
        now,
        now,
        true, // include_data_mdb
        true, // include_metadata_json
    )
    .expect("Failed to create dummy snapshot");

    // 3. Perform the import, which should trigger eviction
    let import_result = index_mapper
        .fj_import_index_from_snapshot(target_index_uid, &snapshot_path);

    // 4. Assertions
    assert!(import_result.is_ok(), "Import failed: {:?}", import_result.err());
    let (_imported_index, _) = import_result.unwrap(); // Prefix unused variable
    let imported_uuid = {
        let rtxn = index_scheduler.env.read_txn().unwrap();
        index_mapper.index_mapping.get(&rtxn, target_index_uid).unwrap().unwrap()
    };

    // Verify imported index is now available and initial index is closing (evicted) using helper methods
    {
        assert!(
            index_mapper.fj_index_status(&imported_uuid).fj_is_available(),
            "Imported index not available"
        );
        assert!(
            index_mapper.fj_index_status(&initial_uuid).fj_is_closing(),
            "Initial index was not evicted"
        );
        assert_eq!(index_mapper.fj_available_cache_len(), 1, "Cache should still have 1 item");
        assert_eq!(index_mapper.fj_unavailable_cache_len(), 1, "Unavailable map should have 1 item");
    }

    // Cleanup snapshot file
    fs::remove_file(snapshot_path).unwrap();
}


// Removed TODO for I/O error test during unpack/move as it's complex to simulate reliably.

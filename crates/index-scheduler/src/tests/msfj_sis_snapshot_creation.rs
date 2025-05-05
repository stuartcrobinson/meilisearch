#[cfg(test)]
mod msfj_sis_snapshot_creation_tests {
    // Removed: use crate::milli::OrderBy; // This pointed to the wrong OrderBy (search::OrderBy)
    use std::collections::BTreeSet; // Keep only one BTreeSet import
    use std::fs::File; // Keep only one File import
    use meilisearch_types::facet_values_sort::FacetValuesSort; // Import the public API enum
    // Removed duplicate BTreeSet import
    // Removed duplicate File import
    // Removed duplicate BTreeSet import
    // Removed duplicate File import
    use std::io::Read;
    use std::path::Path;
    // Removed unused CompactionOption import

    use flate2::read::GzDecoder;
    use meilisearch_types::settings::WildcardSetting; // Import WildcardSetting
    // Removed the meilisearch_types::milli import block
    use tar::Archive;
    use tempfile::tempdir;

    // Removed unused BTreeMap import
    use crate::fj_snapshot_metadata::SnapshotMetadata;
    use crate::fj_snapshot_utils::create_index_snapshot;
    use crate::test_utils::index_creation_task;
    // Removed use crate::milli::order_by_map::OrderByMap;
    // Removed unused import: use crate::milli::OrderBy;
    use crate::IndexScheduler;
    use crate::error::Error;
    use time;
    // Removed crate::milli import block, will use full paths
    use meilisearch_types::{ // Keep other imports from meilisearch_types
        locales::{LocalizedAttributesRuleView, Locale}, // Keep these
        // Removed alias MilliEmbeddingSettingsEnum
        // Removed milli group import from here
        // Removed unused settings group import
        // Removed other unused imports
    };


    #[actix_rt::test]
    async fn test_create_index_snapshot_success() {
        let index_uid = "test-index-snapshot";
        // Get scheduler and handle
        let (_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        // Create the index using the scheduler inside the handle
        let _task_id = handle.register_task(index_creation_task(index_uid, Some("id"))).await.unwrap();
        // Use advance_one_successful_batch to process the index creation task
        handle.advance_one_successful_batch();

        // Get the index handle *after* waiting
        let index = handle.index(index_uid).expect("Failed to get index handle after creation");

        // === Restore settings and document addition ===
        // Add some settings and data to the index
        // Use index.write_txn() instead of handle.write_txn()
        let mut wtxn = index.write_txn().map_err(Error::HeedTransaction).unwrap();
        let mut settings_builder = meilisearch_types::milli::update::Settings::new(&mut wtxn, &index, handle.indexer_config()); // Use full path
        settings_builder.set_primary_key("id".to_string());
        settings_builder.set_displayed_fields(vec!["id".to_string(), "name".to_string()]);
        // The first closure takes only the RwTxn
        settings_builder.execute(|_wtxn| (), || false).unwrap();
        wtxn.commit().unwrap();

        let _doc_task_id = handle.add_documents(index_uid, meilisearch_types::milli::update::IndexDocumentsMethod::ReplaceDocuments, r#"[{"id": 1, "name": "test"}]"#).await.unwrap(); // Use full path
        // Use advance_one_successful_batch to process the document addition task
        handle.advance_one_successful_batch();
        // === End of restored section ===

        // Create a temporary directory for snapshots
        let snapshots_dir = tempdir().unwrap();
        let snapshots_path = snapshots_dir.path();

        // === Restore metadata reading and create_index_snapshot call ===
        // Read metadata first, using a temporary index handle
        let metadata = {
            let temp_index = handle.index(index_uid).unwrap();
            let rtxn = temp_index.read_txn().map_err(Error::HeedTransaction).unwrap();
            crate::fj_snapshot_utils::read_metadata_inner(index_uid, &temp_index, &rtxn).unwrap()
            // rtxn and temp_index dropped here
        };

        // Get a fresh index handle *after* metadata read, right before snapshotting
        // Note: Re-getting the handle might be important if the previous one held resources.
        let index_for_snapshot = handle.index(index_uid).unwrap(); // Use a distinct variable name

        // Call the function under test, passing the fresh index handle and the pre-read metadata
        let snapshot_uid = create_index_snapshot(index_uid, &index_for_snapshot, metadata.clone(), snapshots_path).unwrap();
        // === End of restored section ===


        // === Restore snapshot verification ===
        // Verify the snapshot file exists and has the correct name format
        // Use snapshot_uid.display() for formatting
        let expected_filename = format!("{}-{}.snapshot.tar.gz", index_uid, snapshot_uid.display());
        // let snapshot_filepath = snapshots_path.join(&expected_filename);
        let snapshot_filepath = snapshots_path.join(&expected_filename);
        assert!(snapshot_filepath.exists(), "Snapshot file was not created at {:?}", snapshot_filepath);

        // Unpack the snapshot and verify its contents
        let snapshot_file = File::open(&snapshot_filepath).unwrap();
        let tar_gz = GzDecoder::new(snapshot_file);
        let mut archive = Archive::new(tar_gz);
        let temp_unpack_dir = tempdir().unwrap();
        archive.unpack(temp_unpack_dir.path()).unwrap();

        // Verify data.mdb exists
        let data_mdb_path = temp_unpack_dir.path().join("data.mdb");
        assert!(data_mdb_path.exists(), "data.mdb not found in snapshot");
        // Basic check: ensure it's not empty (size might vary, just check > 0)
        assert!(data_mdb_path.metadata().unwrap().len() > 0, "data.mdb is empty");

        // Verify metadata.json exists and contains correct info
        let metadata_path = temp_unpack_dir.path().join("metadata.json");
        assert!(metadata_path.exists(), "metadata.json not found in snapshot");

        let mut metadata_file = File::open(&metadata_path).unwrap();
        let mut metadata_content = String::new();
        metadata_file.read_to_string(&mut metadata_content).unwrap();
        let deserialized_metadata: SnapshotMetadata = serde_json::from_str(&metadata_content).unwrap();

        // Compare deserialized metadata with the one read before snapshotting
        assert_eq!(deserialized_metadata.meilisearch_version, env!("CARGO_PKG_VERSION")); // Still check version
        assert_eq!(deserialized_metadata.primary_key, metadata.primary_key);
        // Compare settings (ensure all relevant fields are checked)
        assert_eq!(deserialized_metadata.settings, metadata.settings);
        assert_eq!(deserialized_metadata.created_at, metadata.created_at);
        assert_eq!(deserialized_metadata.updated_at, metadata.updated_at);

        // Verify timestamps are plausible (already compared exact values)
        assert!(metadata.created_at <= metadata.updated_at);
        // === End of restored section ===

        // Clean up temp dirs (handled by Drop)
    }

    #[actix_rt::test]
    async fn test_create_index_snapshot_invalid_path() {
        let index_uid = "test-index-invalid-path";
        // Get scheduler and handle
        let (_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        // Create the index using the scheduler inside the handle
        let _task_id = handle.register_task(index_creation_task(index_uid, None)).await.unwrap(); // No primary key needed for this test
        handle.advance_one_successful_batch();
        // Get the index handle
        let index = handle.index(index_uid).unwrap();

        // Use a non-existent path that cannot be created easily
        let invalid_snapshots_path = Path::new("/non/existent/path/for/snapshots");

        // Need dummy metadata for this test case, as it fails before using it
        let dummy_metadata = SnapshotMetadata {
            meilisearch_version: "".to_string(),
            primary_key: None,
            settings: Default::default(),
            created_at: time::OffsetDateTime::now_utc(), // Use time crate
            updated_at: time::OffsetDateTime::now_utc(), // Use time crate
        };

        // Pass index handle
        let result = create_index_snapshot(index_uid, &index, dummy_metadata, invalid_snapshots_path);

        assert!(result.is_err(), "Snapshot creation should fail for invalid path");
        // We expect an IoError when trying to create the temp dir or the final file
        match result.err().unwrap() {
            crate::Error::IoError(_) => (), // Expected
            e => panic!("Expected IoError, got {:?}", e),
        }
    }

     #[actix_rt::test]
    async fn test_create_index_snapshot_empty_index() {
        let index_uid = "test-empty-index-snapshot";
        // Get scheduler and handle
        let (_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        // Create the index using the scheduler inside the handle
        let _task_id = handle.register_task(index_creation_task(index_uid, None)).await.unwrap(); // No primary key needed for this test
        handle.advance_one_successful_batch();
        // Get the index handle
        let index = handle.index(index_uid).unwrap();

        // Create a temporary directory for snapshots
        let snapshots_dir = tempdir().unwrap();
        let snapshots_path = snapshots_dir.path();

        // Read metadata first
        let metadata = {
            let rtxn = index.read_txn().map_err(Error::HeedTransaction).unwrap();
            crate::fj_snapshot_utils::read_metadata_inner(index_uid, &index, &rtxn).unwrap()
            // rtxn is dropped here
        };

        // Call the function under test, passing the index handle
        let snapshot_path = create_index_snapshot(index_uid, &index, metadata.clone(), snapshots_path).unwrap();

        // Verify the snapshot file exists
        // Use snapshot_path directly
        assert!(snapshot_path.exists());

        // Unpack and verify basic structure
        let snapshot_file = File::open(&snapshot_path).unwrap(); // Use snapshot_path
        let tar_gz = GzDecoder::new(snapshot_file);
        let mut archive = Archive::new(tar_gz);
        let temp_unpack_dir = tempdir().unwrap();
        archive.unpack(temp_unpack_dir.path()).unwrap();

        assert!(temp_unpack_dir.path().join("data.mdb").exists());
        assert!(temp_unpack_dir.path().join("metadata.json").exists());

        // Verify metadata for empty index
        let metadata_path = temp_unpack_dir.path().join("metadata.json");
        let mut metadata_file = File::open(&metadata_path).unwrap();
        let mut metadata_content = String::new();
        metadata_file.read_to_string(&mut metadata_content).unwrap();
        let deserialized_metadata: SnapshotMetadata = serde_json::from_str(&metadata_content).unwrap();

        // Compare deserialized metadata with the one read before snapshotting
        assert_eq!(deserialized_metadata.meilisearch_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(deserialized_metadata.primary_key, metadata.primary_key);
        assert!(metadata.primary_key.is_none()); // Check original metadata had no PK
        // Compare settings
        assert_eq!(deserialized_metadata.settings, metadata.settings);

        // Optionally, verify specific default settings on the *original* metadata
        assert_eq!(
            metadata.settings.displayed_attributes,
            WildcardSetting::from(meilisearch_types::milli::update::Setting::Reset) // Use full path
        );
        assert_eq!(
            metadata.settings.searchable_attributes,
             WildcardSetting::from(meilisearch_types::milli::update::Setting::Reset) // Use full path
        );
        assert_eq!(
            metadata.settings.filterable_attributes,
            meilisearch_types::milli::update::Setting::Set(Vec::<meilisearch_types::milli::FilterableAttributesRule>::new()) // Use full paths
        );
        assert_eq!(metadata.settings.sortable_attributes, meilisearch_types::milli::update::Setting::Set(BTreeSet::new())); // Use full path for Setting

    }

    #[actix_rt::test]
    async fn test_create_index_snapshot_with_custom_settings() {
        let index_uid = "test-index-custom-settings";
        // Get scheduler and handle
        let (_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        // Create the index
        let _task_id = handle.register_task(index_creation_task(index_uid, Some("id"))).await.unwrap();
        handle.advance_one_successful_batch(); // Process index creation

        // Get the index handle
        let index = handle.index(index_uid).expect("Failed to get index handle");

        // Apply custom settings
        let mut wtxn = index.write_txn().map_err(Error::HeedTransaction).unwrap();
        // Use the full path for the Settings builder type
        let mut settings_builder = meilisearch_types::milli::update::Settings::new(&mut wtxn, &index, handle.indexer_config());

        // Typo Tolerance - Use individual setters from milli::update::Settings
        settings_builder.set_autorize_typos(false); // Use correct method: set_autorize_typos
        // Use separate setters for one_typo and two_typos
        settings_builder.set_min_word_len_one_typo(6);
        settings_builder.set_min_word_len_two_typos(10);
        // Use set_exact_words, expecting BTreeSet
        settings_builder.set_exact_words(std::collections::BTreeSet::from(["word1".to_string()])); // Use BTreeSet
        // Use set_exact_attributes, expecting HashSet
        settings_builder.set_exact_attributes(std::collections::HashSet::from(["attr1".to_string()])); // Keep HashSet for this one

        // Pagination - Use specific setter
        settings_builder.set_pagination_max_total_hits(500); // Correct method name

        // Faceting - Use individual setters
        settings_builder.set_max_values_per_facet(50);
        // Builder expects OrderByMap
        let mut sort_map = meilisearch_types::milli::order_by_map::OrderByMap::default(); // Use full path
        // Use the public API enum FacetValuesSort and call .into() for conversion
        sort_map.insert("facetA".to_string(), FacetValuesSort::Alpha.into());
        settings_builder.set_sort_facet_values_by(sort_map);


        // Embedders (Example with one embedder - Use UserProvided for testing)
        // Construct an instance of the EmbeddingSettings *struct* for UserProvided
        let inner_embedder_settings = meilisearch_types::milli::vector::settings::EmbeddingSettings {
            source: meilisearch_types::milli::update::Setting::Set(meilisearch_types::milli::vector::settings::EmbedderSource::UserProvided),
            model: meilisearch_types::milli::update::Setting::NotSet, // Not applicable for UserProvided
            api_key: meilisearch_types::milli::update::Setting::NotSet, // Not applicable for UserProvided
            dimensions: meilisearch_types::milli::update::Setting::Set(768), // Mandatory for UserProvided, use a common dimension size
            document_template: meilisearch_types::milli::update::Setting::NotSet, // Not applicable for UserProvided
            // Set other fields to NotSet or Reset as appropriate for the test's intent
            revision: meilisearch_types::milli::update::Setting::NotSet,
            pooling: meilisearch_types::milli::update::Setting::NotSet,
            document_template_max_bytes: meilisearch_types::milli::update::Setting::NotSet,
            url: meilisearch_types::milli::update::Setting::NotSet,
            request: meilisearch_types::milli::update::Setting::NotSet,
            response: meilisearch_types::milli::update::Setting::NotSet,
            headers: meilisearch_types::milli::update::Setting::NotSet,
            search_embedder: meilisearch_types::milli::update::Setting::NotSet,
            indexing_embedder: meilisearch_types::milli::update::Setting::NotSet,
            distribution: meilisearch_types::milli::update::Setting::NotSet,
            binary_quantized: meilisearch_types::milli::update::Setting::NotSet,
        };
        // The builder expects BTreeMap<String, Setting<milli::vector::settings::EmbeddingSettings>>
        // We need to wrap the inner_embedder_settings struct in Setting::Set
        let embedder_map = std::collections::BTreeMap::from([(
            "myembedder".to_string(),
            // Wrap the struct instance in Setting::Set, ensuring the type parameter is correct
            meilisearch_types::milli::update::Setting::<meilisearch_types::milli::vector::settings::EmbeddingSettings>::Set(inner_embedder_settings),
        )]);
        // Pass the correctly typed map to the builder
        settings_builder.set_embedder_settings(embedder_map); // Use set_embedder_settings
        // settings_builder.set_embedders(BTreeMap::from([("myembedder".to_string(), embedder_config)])); // Use set_embedders - Still commented out

        // Localized Attributes (Example) - Use correct builder method
        settings_builder.set_localized_attributes_rules(vec![ // Use set_localized_attributes_rules
            // Convert String to Locale and Vec<String> to AttributePatterns
            LocalizedAttributesRuleView {
                locales: vec!["en".parse::<Locale>().unwrap()],
                attribute_patterns: vec!["title_*".to_string()].into()
            }
            // Convert the View to the internal Rule type expected by the builder
            .into() // .into() should convert View to milli::LocalizedAttributesRule
        ]);

        // Facet Search - Use specific setter
        settings_builder.set_facet_search(true); // Correct method name

        // Execute settings update
        settings_builder.execute(|_wtxn| (), || false).unwrap();
        wtxn.commit().unwrap();

        // Add a document (optional, but good to have some data)
        // For UserProvided, we need to include the vector manually
        // Generate the JSON array string for the vector
        let vector_json_array = format!("[{}]", std::iter::repeat("0.1").take(768).collect::<Vec<_>>().join(","));
        let document_json = format!(
            r#"[{{
                "id": 1,
                "name": "test",
                "facetA": "value1",
                "title_en": "Hello",
                "_vectors": {{
                    "myembedder": {}
                }}
            }}]"#,
            vector_json_array
        );

        let _doc_task_id = handle.add_documents(
            index_uid,
            meilisearch_types::milli::update::IndexDocumentsMethod::ReplaceDocuments,
            &document_json // Use the generated valid JSON string
        ).await.unwrap(); // Use full path

        // Process document task (Settings were applied directly, not via task)
        // This batch should now succeed as we provide the vector and don't call OpenAI
        handle.advance_one_successful_batch(); // Process document addition

        // --- Snapshot Creation and Verification ---
        let snapshots_dir = tempdir().unwrap();
        let snapshots_path = snapshots_dir.path();

        // Read metadata *before* snapshotting to get expected values
        let expected_metadata = {
            let temp_index = handle.index(index_uid).unwrap();
            let rtxn = temp_index.read_txn().map_err(Error::HeedTransaction).unwrap();
            crate::fj_snapshot_utils::read_metadata_inner(index_uid, &temp_index, &rtxn).unwrap()
        };

        // Get a fresh handle for snapshotting
        let index_for_snapshot = handle.index(index_uid).unwrap();

        // Create the snapshot
        let snapshot_path = create_index_snapshot(index_uid, &index_for_snapshot, expected_metadata.clone(), snapshots_path).unwrap();

        // --- Verification ---
        // Use snapshot_path directly
        assert!(snapshot_path.exists(), "Snapshot file was not created");

        // Unpack
        let snapshot_file = File::open(&snapshot_path).unwrap(); // Use snapshot_path
        let tar_gz = GzDecoder::new(snapshot_file);
        let mut archive = Archive::new(tar_gz);
        let temp_unpack_dir = tempdir().unwrap();
        archive.unpack(temp_unpack_dir.path()).unwrap();

        // Verify metadata.json content
        let metadata_path = temp_unpack_dir.path().join("metadata.json");
        assert!(metadata_path.exists(), "metadata.json not found in snapshot");

        let mut metadata_file = File::open(&metadata_path).unwrap();
        let mut metadata_content = String::new();
        metadata_file.read_to_string(&mut metadata_content).unwrap();
        let deserialized_metadata: SnapshotMetadata = serde_json::from_str(&metadata_content).unwrap();

        // --- Assertions for Custom Settings ---
        // Compare the whole settings struct first
        assert_eq!(deserialized_metadata.settings, expected_metadata.settings, "Full settings mismatch");

        // Optionally, assert individual custom settings for clarity
        assert_eq!(deserialized_metadata.settings.typo_tolerance, expected_metadata.settings.typo_tolerance, "TypoTolerance mismatch");
        assert_eq!(deserialized_metadata.settings.pagination, expected_metadata.settings.pagination, "Pagination mismatch");
        assert_eq!(deserialized_metadata.settings.faceting, expected_metadata.settings.faceting, "Faceting mismatch");
        assert_eq!(deserialized_metadata.settings.embedders, expected_metadata.settings.embedders, "Embedders mismatch");
        assert_eq!(deserialized_metadata.settings.localized_attributes, expected_metadata.settings.localized_attributes, "LocalizedAttributes mismatch");
        assert_eq!(deserialized_metadata.settings.facet_search, expected_metadata.settings.facet_search, "FacetSearch mismatch");

        // Also check core info hasn't changed unexpectedly
        assert_eq!(deserialized_metadata.meilisearch_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(deserialized_metadata.primary_key, expected_metadata.primary_key);
        assert_eq!(deserialized_metadata.created_at, expected_metadata.created_at);
        assert_eq!(deserialized_metadata.updated_at, expected_metadata.updated_at);
    }

    // Removed test_minimal_index_handle_then_write_txn as it's no longer needed after fixing test utils.
}

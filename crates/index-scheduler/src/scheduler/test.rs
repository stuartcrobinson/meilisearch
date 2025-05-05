use std::collections::BTreeMap;

use big_s::S;
use meili_snap::{json_string, snapshot};
use meilisearch_auth::AuthFilter;
use meilisearch_types::milli::index::IndexEmbeddingConfig;
use meilisearch_types::milli::update::IndexDocumentsMethod::*;
use meilisearch_types::milli::{self};
use meilisearch_types::settings::SettingEmbeddingSettings;
use meilisearch_types::tasks::{IndexSwap, KindWithContent};
use roaring::RoaringBitmap;

// Removed unused import: std::fs::File;

use crate::insta_snapshot::snapshot_index_scheduler;
use crate::test_utils::Breakpoint::*;
// Use test_utils for handle_tasks and TempIndex, import handled inside module
// Removed TempIndex import from here
use crate::test_utils::{
    index_creation_task, read_json, replace_document_import_task, sample_documents,
};
// Imports moved up
use crate::IndexScheduler;
// Removed unused import: FilterableAttributesRule

#[test]
fn insert_task_while_another_task_is_processing() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    index_scheduler.register(index_creation_task("index_a", Some("id")), None, false).unwrap(); // Wrap in Some()
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_first_task");

    handle.advance_till([Start, BatchCreated]);
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "after_batch_creation");

    // while the task is processing can we register another task?
    index_scheduler.register(index_creation_task("index_b", Some("id")), None, false).unwrap(); // Wrap in Some()
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_second_task");

    index_scheduler
        .register(KindWithContent::IndexDeletion { index_uid: S("index_a") }, None, false)
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_third_task");
}

#[test]
fn test_task_is_processing() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    index_scheduler.register(index_creation_task("index_a", Some("id")), None, false).unwrap(); // Wrap in Some()
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_a_task");

    handle.advance_till([Start, BatchCreated]);
    assert!(index_scheduler.is_task_processing().unwrap());
}

/// We send a lot of tasks but notify the tasks scheduler only once as
/// we send them very fast, we must make sure that they are all processed.
#[test]
fn process_tasks_inserted_without_new_signal() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggos"), primary_key: None },
            None,
            false,
        )
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_first_task");

    index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("cattos"), primary_key: None },
            None,
            false,
        )
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_second_task");

    index_scheduler
        .register(KindWithContent::IndexDeletion { index_uid: S("doggos") }, None, false)
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_third_task");

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "processed_the_first_task");

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "processed_the_second_task");

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "processed_the_third_task");
}

#[test]
fn process_tasks_without_autobatching() {
    let (index_scheduler, mut handle) = IndexScheduler::test(false, vec![]);

    index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggos"), primary_key: None },
            None,
            false,
        )
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_first_task");

    index_scheduler
        .register(KindWithContent::DocumentClear { index_uid: S("doggos") }, None, false)
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_second_task");

    index_scheduler
        .register(KindWithContent::DocumentClear { index_uid: S("doggos") }, None, false)
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_third_task");

    index_scheduler
        .register(KindWithContent::DocumentClear { index_uid: S("doggos") }, None, false)
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_fourth_task");

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "first");

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "second");

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "third");

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "fourth");
}

#[test]
fn task_deletion_undeleteable() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let (file0, documents_count0) = sample_documents(&index_scheduler, 0, 0);
    let (file1, documents_count1) = sample_documents(&index_scheduler, 1, 1);
    file0.persist().unwrap();
    file1.persist().unwrap();

    let to_enqueue = [
        index_creation_task("catto", Some("mouse")), // Wrap in Some()
        replace_document_import_task("catto", None, 0, documents_count0),
        replace_document_import_task("doggo", Some("bone"), 1, documents_count1),
    ];

    for task in to_enqueue {
        let _ = index_scheduler.register(task, None, false).unwrap();
        index_scheduler.assert_internally_consistent();
    }

    // here we have registered all the tasks, but the index scheduler
    // has not progressed at all
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "initial_tasks_enqueued");

    index_scheduler
        .register(
            KindWithContent::TaskDeletion {
                query: "test_query".to_owned(),
                tasks: RoaringBitmap::from_iter([0, 1]),
            },
            None,
            false,
        )
        .unwrap();
    // again, no progress made at all, but one more task is registered
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "task_deletion_enqueued");

    // now we create the first batch
    handle.advance_till([Start, BatchCreated]);

    // the task deletion should now be "processing"
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "task_deletion_processing");

    handle.advance_till([InsideProcessBatch, ProcessBatchSucceeded, AfterProcessing]);
    // after the task deletion is processed, no task should actually have been deleted,
    // because the tasks with ids 0 and 1 were still "enqueued", and thus undeleteable
    // the "task deletion" task should be marked as "succeeded" and, in its details, the
    // number of deleted tasks should be 0
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "task_deletion_done");
}

#[test]
fn task_deletion_deleteable() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let (file0, documents_count0) = sample_documents(&index_scheduler, 0, 0);
    let (file1, documents_count1) = sample_documents(&index_scheduler, 1, 1);
    file0.persist().unwrap();
    file1.persist().unwrap();

    let to_enqueue = [
        replace_document_import_task("catto", None, 0, documents_count0),
        replace_document_import_task("doggo", Some("bone"), 1, documents_count1),
    ];

    for task in to_enqueue {
        let _ = index_scheduler.register(task, None, false).unwrap();
        index_scheduler.assert_internally_consistent();
    }
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "initial_tasks_enqueued");

    handle.advance_one_successful_batch();
    // first addition of documents should be successful
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "initial_tasks_processed");

    // Now we delete the first task
    index_scheduler
        .register(
            KindWithContent::TaskDeletion {
                query: "test_query".to_owned(),
                tasks: RoaringBitmap::from_iter([0]),
            },
            None,
            false,
        )
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "after_registering_the_task_deletion");

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "task_deletion_processed");
}

#[test]
fn task_deletion_delete_same_task_twice() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let (file0, documents_count0) = sample_documents(&index_scheduler, 0, 0);
    let (file1, documents_count1) = sample_documents(&index_scheduler, 1, 1);
    file0.persist().unwrap();
    file1.persist().unwrap();

    let to_enqueue = [
        replace_document_import_task("catto", None, 0, documents_count0),
        replace_document_import_task("doggo", Some("bone"), 1, documents_count1),
    ];

    for task in to_enqueue {
        let _ = index_scheduler.register(task, None, false).unwrap();
        index_scheduler.assert_internally_consistent();
    }
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "initial_tasks_enqueued");

    handle.advance_one_successful_batch();
    // first addition of documents should be successful
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "initial_tasks_processed");

    // Now we delete the first task multiple times in a row
    for _ in 0..2 {
        index_scheduler
            .register(
                KindWithContent::TaskDeletion {
                    query: "test_query".to_owned(),
                    tasks: RoaringBitmap::from_iter([0]),
                },
                None,
                false,
            )
            .unwrap();
        index_scheduler.assert_internally_consistent();
    }
    handle.advance_one_successful_batch();

    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "task_deletion_processed");
}

#[test]
fn document_addition_and_index_deletion() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let content = r#"
        {
            "id": 1,
            "doggo": "bob"
        }"#;

    index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggos"), primary_key: None },
            None,
            false,
        )
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_first_task");

    let (uuid, mut file) = index_scheduler.queue.create_update_file_with_uuid(0).unwrap();
    let documents_count = read_json(content.as_bytes(), &mut file).unwrap();
    file.persist().unwrap();
    index_scheduler
        .register(
            KindWithContent::DocumentAdditionOrUpdate {
                index_uid: S("doggos"),
                primary_key: Some(S("id")),
                method: ReplaceDocuments,
                content_file: uuid,
                documents_count,
                allow_index_creation: true,
            },
            None,
            false,
        )
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_second_task");

    index_scheduler
        .register(KindWithContent::IndexDeletion { index_uid: S("doggos") }, None, false)
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_third_task");

    handle.advance_one_successful_batch(); // The index creation.
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "before_index_creation");
    handle.advance_one_successful_batch(); // // after the execution of the two tasks in a single batch.
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "both_task_succeeded");
}

#[test]
fn do_not_batch_task_of_different_indexes() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
    let index_names = ["doggos", "cattos", "girafos"];

    for name in index_names {
        index_scheduler
            .register(
                KindWithContent::IndexCreation { index_uid: name.to_string(), primary_key: None },
                None,
                false,
            )
            .unwrap();
        index_scheduler.assert_internally_consistent();
    }

    for name in index_names {
        index_scheduler
            .register(KindWithContent::DocumentClear { index_uid: name.to_string() }, None, false)
            .unwrap();
        index_scheduler.assert_internally_consistent();
    }

    for _ in 0..(index_names.len() * 2) {
        handle.advance_one_successful_batch();
        index_scheduler.assert_internally_consistent();
    }

    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "all_tasks_processed");
}

#[test]
fn swap_indexes() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let to_enqueue = [
        index_creation_task("a", Some("id")), // Wrap in Some()
        index_creation_task("b", Some("id")), // Wrap in Some()
        index_creation_task("c", Some("id")), // Wrap in Some()
        index_creation_task("d", Some("id")), // Wrap in Some()
    ];

    for task in to_enqueue {
        let _ = index_scheduler.register(task, None, false).unwrap();
        index_scheduler.assert_internally_consistent();
    }

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "create_a");
    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "create_b");
    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "create_c");
    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "create_d");

    index_scheduler
        .register(
            KindWithContent::IndexSwap {
                swaps: vec![
                    IndexSwap { indexes: ("a".to_owned(), "b".to_owned()) },
                    IndexSwap { indexes: ("c".to_owned(), "d".to_owned()) },
                ],
            },
            None,
            false,
        )
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "first_swap_registered");
    index_scheduler
        .register(
            KindWithContent::IndexSwap {
                swaps: vec![IndexSwap { indexes: ("a".to_owned(), "c".to_owned()) }],
            },
            None,
            false,
        )
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "two_swaps_registered");

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "first_swap_processed");

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "second_swap_processed");

    index_scheduler.register(KindWithContent::IndexSwap { swaps: vec![] }, None, false).unwrap();
    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "third_empty_swap_processed");
}

#[test]
fn swap_indexes_errors() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let to_enqueue = [
        index_creation_task("a", Some("id")), // Wrap in Some()
        index_creation_task("b", Some("id")), // Wrap in Some()
        index_creation_task("c", Some("id")), // Wrap in Some()
        index_creation_task("d", Some("id")), // Wrap in Some()
    ];

    for task in to_enqueue {
        let _ = index_scheduler.register(task, None, false).unwrap();
        index_scheduler.assert_internally_consistent();
    }
    handle.advance_n_successful_batches(4);
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "after_the_index_creation");

    let first_snap = snapshot_index_scheduler(&index_scheduler);
    snapshot!(first_snap, name: "initial_tasks_processed");

    let err = index_scheduler
        .register(
            KindWithContent::IndexSwap {
                swaps: vec![
                    IndexSwap { indexes: ("a".to_owned(), "b".to_owned()) },
                    IndexSwap { indexes: ("b".to_owned(), "a".to_owned()) },
                ],
            },
            None,
            false,
        )
        .unwrap_err();
    snapshot!(format!("{err}"), @"Indexes must be declared only once during a swap. `a`, `b` were specified several times.");

    let second_snap = snapshot_index_scheduler(&index_scheduler);
    assert_eq!(first_snap, second_snap);

    // Index `e` does not exist, but we don't check its existence yet
    index_scheduler
        .register(
            KindWithContent::IndexSwap {
                swaps: vec![
                    IndexSwap { indexes: ("a".to_owned(), "b".to_owned()) },
                    IndexSwap { indexes: ("c".to_owned(), "e".to_owned()) },
                    IndexSwap { indexes: ("d".to_owned(), "f".to_owned()) },
                ],
            },
            None,
            false,
        )
        .unwrap();
    handle.advance_one_failed_batch();
    // Now the first swap should have an error message saying `e` and `f` do not exist
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "first_swap_failed");
}

#[test]
fn document_addition_and_index_deletion_on_unexisting_index() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let content = r#"
        {
            "id": 1,
            "doggo": "bob"
        }"#;

    let (uuid, mut file) = index_scheduler.queue.create_update_file_with_uuid(0).unwrap();
    let documents_count = read_json(content.as_bytes(), &mut file).unwrap();
    file.persist().unwrap();
    index_scheduler
        .register(
            KindWithContent::DocumentAdditionOrUpdate {
                index_uid: S("doggos"),
                primary_key: Some(S("id")),
                method: ReplaceDocuments,
                content_file: uuid,
                documents_count,
                allow_index_creation: true,
            },
            None,
            false,
        )
        .unwrap();
    index_scheduler
        .register(KindWithContent::IndexDeletion { index_uid: S("doggos") }, None, false)
        .unwrap();

    snapshot!(snapshot_index_scheduler(&index_scheduler));

    handle.advance_n_successful_batches(1);

    snapshot!(snapshot_index_scheduler(&index_scheduler));
}

#[test]
fn cancel_enqueued_task() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let (file0, documents_count0) = sample_documents(&index_scheduler, 0, 0);
    file0.persist().unwrap();

    let to_enqueue = [
        replace_document_import_task("catto", None, 0, documents_count0),
        KindWithContent::TaskCancelation {
            query: "test_query".to_owned(),
            tasks: RoaringBitmap::from_iter([0]),
        },
    ];
    for task in to_enqueue {
        let _ = index_scheduler.register(task, None, false).unwrap();
        index_scheduler.assert_internally_consistent();
    }

    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "initial_tasks_enqueued");
    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "cancel_processed");
}

#[test]
fn cancel_succeeded_task() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let (file0, documents_count0) = sample_documents(&index_scheduler, 0, 0);
    file0.persist().unwrap();

    let _ = index_scheduler
        .register(replace_document_import_task("catto", None, 0, documents_count0), None, false)
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_first_task");

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "initial_task_processed");

    index_scheduler
        .register(
            KindWithContent::TaskCancelation {
                query: "test_query".to_owned(),
                tasks: RoaringBitmap::from_iter([0]),
            },
            None,
            false,
        )
        .unwrap();

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "cancel_processed");
}

#[test]
fn cancel_processing_task() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let (file0, documents_count0) = sample_documents(&index_scheduler, 0, 0);
    file0.persist().unwrap();

    let _ = index_scheduler
        .register(replace_document_import_task("catto", None, 0, documents_count0), None, false)
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "registered_the_first_task");

    handle.advance_till([Start, BatchCreated, InsideProcessBatch]);
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "initial_task_processing");

    index_scheduler
        .register(
            KindWithContent::TaskCancelation {
                query: "test_query".to_owned(),
                tasks: RoaringBitmap::from_iter([0]),
            },
            None,
            false,
        )
        .unwrap();

    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "cancel_task_registered");
    // Now we check that we can reach the AbortedIndexation error handling
    handle.advance_till([AbortedIndexation]);
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "aborted_indexation");

    // handle.advance_till([Start, BatchCreated, BeforeProcessing, AfterProcessing]);
    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "cancel_processed");
}

#[test]
fn cancel_mix_of_tasks() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let (file0, documents_count0) = sample_documents(&index_scheduler, 0, 0);
    file0.persist().unwrap();
    let (file1, documents_count1) = sample_documents(&index_scheduler, 1, 1);
    file1.persist().unwrap();
    let (file2, documents_count2) = sample_documents(&index_scheduler, 2, 2);
    file2.persist().unwrap();

    let to_enqueue = [
        replace_document_import_task("catto", None, 0, documents_count0),
        replace_document_import_task("beavero", None, 1, documents_count1),
        replace_document_import_task("wolfo", None, 2, documents_count2),
    ];
    for task in to_enqueue {
        let _ = index_scheduler.register(task, None, false).unwrap();
        index_scheduler.assert_internally_consistent();
    }
    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "first_task_processed");

    handle.advance_till([Start, BatchCreated, InsideProcessBatch]);
    index_scheduler
        .register(
            KindWithContent::TaskCancelation {
                query: "test_query".to_owned(),
                tasks: RoaringBitmap::from_iter([0, 1, 2]),
            },
            None,
            false,
        )
        .unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "processing_second_task_cancel_enqueued");

    handle.advance_till([AbortedIndexation]);
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "aborted_indexation");

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "cancel_processed");
}

#[test]
fn test_settings_update() {
    use meilisearch_types::settings::{Settings, Unchecked};
    use milli::update::Setting;

    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let mut new_settings: Box<Settings<Unchecked>> = Box::default();
    let mut embedders = BTreeMap::default();
    let embedding_settings = milli::vector::settings::EmbeddingSettings {
        source: Setting::Set(milli::vector::settings::EmbedderSource::Rest),
        api_key: Setting::Set(S("My super secret")),
        url: Setting::Set(S("http://localhost:7777")),
        dimensions: Setting::Set(4),
        request: Setting::Set(serde_json::json!("{{text}}")),
        response: Setting::Set(serde_json::json!("{{embedding}}")),
        ..Default::default()
    };
    embedders
        .insert(S("default"), SettingEmbeddingSettings { inner: Setting::Set(embedding_settings) });
    new_settings.embedders = Setting::Set(embedders);

    index_scheduler
        .register(
            KindWithContent::SettingsUpdate {
                index_uid: S("doggos"),
                new_settings,
                is_deletion: false,
                allow_index_creation: true,
            },
            None,
            false,
        )
        .unwrap();
    index_scheduler.assert_internally_consistent();

    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "after_registering_settings_task");

    {
        let rtxn = index_scheduler.read_txn().unwrap();
        let task = index_scheduler.queue.tasks.get_task(&rtxn, 0).unwrap().unwrap();
        let task = meilisearch_types::task_view::TaskView::from_task(&task);
        insta::assert_json_snapshot!(task.details);
    }

    handle.advance_n_successful_batches(1);
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "settings_update_processed");

    {
        let rtxn = index_scheduler.read_txn().unwrap();
        let task = index_scheduler.queue.tasks.get_task(&rtxn, 0).unwrap().unwrap();
        let task = meilisearch_types::task_view::TaskView::from_task(&task);
        insta::assert_json_snapshot!(task.details);
    }

    // has everything being pushed successfully in milli?
    let index = index_scheduler.index("doggos").unwrap();
    let rtxn = index.read_txn().unwrap();

    let configs = index.embedding_configs(&rtxn).unwrap();
    let IndexEmbeddingConfig { name, config, user_provided } = configs.first().unwrap();
    insta::assert_snapshot!(name, @"default");
    insta::assert_debug_snapshot!(user_provided, @"RoaringBitmap<[]>");
    insta::assert_json_snapshot!(config.embedder_options);
}

#[test]
fn simple_new() {
    crate::IndexScheduler::test(true, vec![]);
}

#[test]
fn basic_get_stats() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let kind = index_creation_task("catto", Some("mouse")); // Wrap in Some()
    let _task = index_scheduler.register(kind, None, false).unwrap();
    let kind = index_creation_task("doggo", Some("sheep")); // Wrap in Some()
    let _task = index_scheduler.register(kind, None, false).unwrap();
    let kind = index_creation_task("whalo", Some("fish")); // Wrap in Some()
    let _task = index_scheduler.register(kind, None, false).unwrap();

    snapshot!(json_string!(index_scheduler.get_stats().unwrap()), @r#"
    {
      "indexes": {
        "catto": 1,
        "doggo": 1,
        "whalo": 1
      },
      "statuses": {
        "canceled": 0,
        "enqueued": 3,
        "failed": 0,
        "processing": 0,
        "succeeded": 0
      },
      "types": {
        "documentAdditionOrUpdate": 0,
        "documentDeletion": 0,
        "documentEdition": 0,
        "dumpCreation": 0,
        "indexCreation": 3,
        "indexDeletion": 0,
        "indexSwap": 0,
        "indexUpdate": 0,
        "settingsUpdate": 0,
        "snapshotCreation": 0,
        "taskCancelation": 0,
        "taskDeletion": 0,
        "upgradeDatabase": 0
      }
    }
    "#);

    handle.advance_till([Start, BatchCreated]);
    snapshot!(json_string!(index_scheduler.get_stats().unwrap()), @r#"
    {
      "indexes": {
        "catto": 1,
        "doggo": 1,
        "whalo": 1
      },
      "statuses": {
        "canceled": 0,
        "enqueued": 2,
        "failed": 0,
        "processing": 1,
        "succeeded": 0
      },
      "types": {
        "documentAdditionOrUpdate": 0,
        "documentDeletion": 0,
        "documentEdition": 0,
        "dumpCreation": 0,
        "indexCreation": 3,
        "indexDeletion": 0,
        "indexSwap": 0,
        "indexUpdate": 0,
        "settingsUpdate": 0,
        "snapshotCreation": 0,
        "taskCancelation": 0,
        "taskDeletion": 0,
        "upgradeDatabase": 0
      }
    }
    "#);

    handle.advance_till([
        InsideProcessBatch,
        InsideProcessBatch,
        ProcessBatchSucceeded,
        AfterProcessing,
        Start,
        BatchCreated,
    ]);
    snapshot!(json_string!(index_scheduler.get_stats().unwrap()), @r#"
    {
      "indexes": {
        "catto": 1,
        "doggo": 1,
        "whalo": 1
      },
      "statuses": {
        "canceled": 0,
        "enqueued": 1,
        "failed": 0,
        "processing": 1,
        "succeeded": 1
      },
      "types": {
        "documentAdditionOrUpdate": 0,
        "documentDeletion": 0,
        "documentEdition": 0,
        "dumpCreation": 0,
        "indexCreation": 3,
        "indexDeletion": 0,
        "indexSwap": 0,
        "indexUpdate": 0,
        "settingsUpdate": 0,
        "snapshotCreation": 0,
        "taskCancelation": 0,
        "taskDeletion": 0,
        "upgradeDatabase": 0
      }
    }
    "#);

    // now we make one more batch, the started_at field of the new tasks will be past `second_start_time`
    handle.advance_till([
        InsideProcessBatch,
        InsideProcessBatch,
        ProcessBatchSucceeded,
        AfterProcessing,
        Start,
        BatchCreated,
    ]);
    snapshot!(json_string!(index_scheduler.get_stats().unwrap()), @r#"
    {
      "indexes": {
        "catto": 1,
        "doggo": 1,
        "whalo": 1
      },
      "statuses": {
        "canceled": 0,
        "enqueued": 0,
        "failed": 0,
        "processing": 1,
        "succeeded": 2
      },
      "types": {
        "documentAdditionOrUpdate": 0,
        "documentDeletion": 0,
        "documentEdition": 0,
        "dumpCreation": 0,
        "indexCreation": 3,
        "indexDeletion": 0,
        "indexSwap": 0,
        "indexUpdate": 0,
        "settingsUpdate": 0,
        "snapshotCreation": 0,
        "taskCancelation": 0,
        "taskDeletion": 0,
        "upgradeDatabase": 0
      }
    }
    "#);
}

#[test]
fn cancel_processing_dump() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let dump_creation = KindWithContent::DumpCreation { keys: Vec::new(), instance_uid: None };
    let dump_cancellation = KindWithContent::TaskCancelation {
        query: "cancel dump".to_owned(),
        tasks: RoaringBitmap::from_iter([0]),
    };
    let _ = index_scheduler.register(dump_creation, None, false).unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "after_dump_register");
    handle.advance_till([Start, BatchCreated, InsideProcessBatch]);

    let _ = index_scheduler.register(dump_cancellation, None, false).unwrap();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "cancel_registered");

    snapshot!(format!("{:?}", handle.advance()), @"AbortedIndexation");

    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "cancel_processed");
}

#[test]
fn create_and_list_index() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);

    let index_creation =
        KindWithContent::IndexCreation { index_uid: S("kefir"), primary_key: None };
    let _ = index_scheduler.register(index_creation, None, false).unwrap();
    handle.advance_till([Start, BatchCreated, InsideProcessBatch]);
    // The index creation has not been started, the index should not exists

    let err = index_scheduler.index("kefir").map(|_| ()).unwrap_err();
    snapshot!(err, @"Index `kefir` not found.");
    let empty = index_scheduler.get_paginated_indexes_stats(&AuthFilter::default(), 0, 20).unwrap();
    snapshot!(format!("{empty:?}"), @"(0, [])");

    // After advancing just once the index should've been created, the wtxn has been released and commited
    // but the indexUpdate task has not been processed yet
    handle.advance_till([InsideProcessBatch]);

    index_scheduler.index("kefir").unwrap();
    let list = index_scheduler.get_paginated_indexes_stats(&AuthFilter::default(), 0, 20).unwrap();
    snapshot!(json_string!(list, { "[1][0][1].created_at" => "[date]", "[1][0][1].updated_at" => "[date]", "[1][0][1].used_database_size" => "[bytes]", "[1][0][1].database_size" => "[bytes]" }), @r###"
    [
      1,
      [
        [
          "kefir",
          {
            "documents_database_stats": {
              "numberOfEntries": 0,
              "totalKeySize": 0,
              "totalValueSize": 0
            },
            "database_size": "[bytes]",
            "number_of_embeddings": 0,
            "number_of_embedded_documents": 0,
            "used_database_size": "[bytes]",
            "primary_key": null,
            "field_distribution": {},
            "created_at": "[date]",
            "updated_at": "[date]"
          }
        ]
      ]
    ]
    "###);
}

// [meilisearchfj] Tests for Single Index Snapshot Import integration with the scheduler
#[cfg(test)] // Add cfg(test) annotation
mod msfj_sis_scheduler_import_tests {
    use super::*; // Keep import from parent
    // Move necessary imports inside the module
    // Use crate::test_utils path
    // Removed unused import: handle_tasks, TempIndex
    use meilisearch_types::milli::vector::settings::{EmbedderSource, EmbeddingSettings};
    // Removed unused import: SettingEmbeddingSettings
    use meilisearch_types::tasks::KindWithContent;
    use tempfile::tempdir;
    use std::collections::BTreeMap;
    use big_s::S;
    use std::path::PathBuf;
    use std::io::Write;
    use crate::{IndexScheduler, fj_snapshot_utils};
    use crate::test_utils::index_creation_task;
    use meilisearch_types::tasks::{Details, Status};
    use milli::FilterableAttributesRule;
    use milli::update::Setting;
    // Removed StatusCode import, will compare numeric values

    // Helper to create a valid snapshot for import tests
    // Moved inside the module
    fn create_test_snapshot(
        index_scheduler: &IndexScheduler,
        source_index_uid: &str,
        target_snapshot_name: &str,
    ) -> PathBuf {
        // 1. Create source index and add data/settings
        let task = index_creation_task(source_index_uid, Some("id"));
        let task_id = index_scheduler.register(task, None, false).unwrap().uid;
        handle.advance_one_successful_batch(); // Use handle to process the task

        let index = index_scheduler.index(source_index_uid).unwrap();
        let mut wtxn = index.write_txn().unwrap();
        let mut settings = milli::update::Settings::new(
            &mut wtxn,
            &index,
            index_scheduler.indexer_config(),
        );
        // Construct Vec<FilterableAttributesRule> directly
        settings.set_filterable_fields(vec![FilterableAttributesRule::Field("name".to_string())]);
        settings.execute(|_| {}, || false).unwrap();
        wtxn.commit().unwrap();

        // 2. Create the snapshot using the internal utility function
        let snapshot_dir = index_scheduler.fj_snapshots_path();
        std::fs::create_dir_all(snapshot_dir).unwrap();
        let snapshot_path = snapshot_dir.join(target_snapshot_name);

        let index_rtxn = index.read_txn().unwrap();
        let metadata = fj_snapshot_utils::read_metadata_inner(source_index_uid, &index, &index_rtxn).unwrap();
        drop(index_rtxn); // Drop txn before copying

        // Use the correct function name
        fj_snapshot_utils::create_index_snapshot(
            source_index_uid, // Pass index_uid as first argument
            &index,
            metadata,
            &snapshot_path,
        )
        .unwrap();

        snapshot_path
    }

    #[test]
    fn test_import_snapshot_happy_path() {
        let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        let source_index = "source_index_import_happy";
        let target_index = "target_index_import_happy";
        let snapshot_filename = format!("{}-test.snapshot.tar.gz", source_index);

        let snapshot_path =
            create_test_snapshot(&index_scheduler, source_index, &snapshot_filename);

        // Register the import task
        let import_task = KindWithContent::SingleIndexSnapshotImport {
            source_snapshot_path: snapshot_path.to_str().unwrap().to_string(),
            target_index_uid: target_index.to_string(),
        };
        let task_id = index_scheduler.register(import_task, None, false).unwrap().uid;

        // Process the task
        handle.advance_one_successful_batch();

        // Assertions
        let rtxn = index_scheduler.read_txn().unwrap();
        // Use correct path to get_task
        let task = index_scheduler.queue.tasks.get_task(&rtxn, task_id).unwrap().unwrap();
        assert_eq!(task.status, Status::Succeeded);
        assert!(task.error.is_none());
        match task.details {
            Some(Details::SingleIndexSnapshotImport { source_snapshot_uid, target_index_uid }) => {
                assert_eq!(source_snapshot_uid, format!("{}-test", source_index)); // Check UID extraction
                assert_eq!(target_index_uid, target_index);
            }
            _ => panic!("Incorrect task details: {:?}", task.details),
        }

        // Remove incorrect &rtxn argument
        assert!(index_scheduler.index_exists(target_index).unwrap());
        let imported_index = index_scheduler.index(target_index).unwrap();
        let index_rtxn = imported_index.read_txn().unwrap();
        // Correctly extract field names from FilterableAttributesRule enum
        let filterable: std::collections::HashSet<String> = imported_index
            .filterable_attributes_rules(&index_rtxn)
            .unwrap()
            .into_iter()
            .filter_map(|r| match r {
                FilterableAttributesRule::Field(name) => Some(name),
                _ => None,
            })
            .collect();
        assert!(filterable.contains("name"));
    }

     #[test]
    fn test_import_snapshot_with_embedders() {
        let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        let source_index = "source_index_import_embed";
        let target_index = "target_index_import_embed";
        let snapshot_filename = format!("{}-test.snapshot.tar.gz", source_index);

        // 1. Create source index and add settings including embedders
        let task = index_creation_task(source_index, Some("id"));
        let task_id = index_scheduler.register(task, None, false).unwrap().uid;
        handle.advance_one_successful_batch(); // Use handle to process the task

        let index = index_scheduler.index(source_index).unwrap();
        let mut wtxn = index.write_txn().unwrap();
        let mut settings = milli::update::Settings::new(
            &mut wtxn,
            &index,
            index_scheduler.indexer_config(),
        );
        let mut embedders = BTreeMap::default();
        embedders.insert(S("default"), Setting::Set(EmbeddingSettings {
            source: Setting::Set(EmbedderSource::UserProvided),
            dimensions: Setting::Set(1),
            ..Default::default()
        }));
        settings.set_embedder_settings(embedders);
        settings.execute(|_| {}, || false).unwrap();
        wtxn.commit().unwrap();

        // 2. Create the snapshot
        let snapshot_path =
            create_test_snapshot(&index_scheduler, source_index, &snapshot_filename);

        // 3. Register the import task
        let import_task = KindWithContent::SingleIndexSnapshotImport {
            source_snapshot_path: snapshot_path.to_str().unwrap().to_string(),
            target_index_uid: target_index.to_string(),
        };
        let task_id = index_scheduler.register(import_task, None, false).unwrap().uid;

        // 4. Process the task
        handle.advance_one_successful_batch();

        // 5. Assertions
        let rtxn = index_scheduler.read_txn().unwrap();
        // Use correct path to get_task
        let task = index_scheduler.queue.tasks.get_task(&rtxn, task_id).unwrap().unwrap();
        assert_eq!(task.status, Status::Succeeded);
        assert!(task.error.is_none());

        // Remove incorrect &rtxn argument
        assert!(index_scheduler.index_exists(target_index).unwrap());
        let imported_index = index_scheduler.index(target_index).unwrap();
        let index_rtxn = imported_index.read_txn().unwrap();
        let imported_embedders = imported_index.embedding_configs(&index_rtxn).unwrap();
        assert_eq!(imported_embedders.len(), 1);
        assert!(imported_embedders.iter().any(|c| c.name == "default"));
        let config = imported_embedders.iter().find(|c| c.name == "default").unwrap();
        // Check dimensions by matching the UserProvided variant
        match &config.config.embedder_options {
            meilisearch_types::milli::vector::EmbedderOptions::UserProvided(options) => {
                assert_eq!(options.dimensions, 1);
            }
            other => panic!("Expected UserProvided embedder options, found {:?}", other),
        }
        // Note: We can't directly check the 'source' field on the retrieved EmbedderOptions enum.
        // Matching the UserProvided variant implies the source was correctly set during configuration.
    }

    #[test]
    fn test_import_snapshot_target_exists() {
        let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        let source_index = "source_index_import_exists";
        let target_index = "target_index_import_exists"; // Same name for source and target
        let snapshot_filename = format!("{}-test.snapshot.tar.gz", source_index);

        let snapshot_path =
            create_test_snapshot(&index_scheduler, source_index, &snapshot_filename);

        // Create the target index beforehand
        let creation_task = index_creation_task(target_index, Some("id"));
        let _creation_task_id = index_scheduler.register(creation_task, None, false).unwrap().uid;
        handle.advance_one_successful_batch(); // Use handle to process the task


        // Register the import task
        let import_task = KindWithContent::SingleIndexSnapshotImport {
            source_snapshot_path: snapshot_path.to_str().unwrap().to_string(),
            target_index_uid: target_index.to_string(),
        };
        let task_id = index_scheduler.register(import_task, None, false).unwrap().uid;

        // Process the task - should fail
        handle.advance_one_failed_batch();

        // Assertions
        let rtxn = index_scheduler.read_txn().unwrap();
        // Use correct path to get_task
        let task = index_scheduler.queue.tasks.get_task(&rtxn, task_id).unwrap().unwrap();
        assert_eq!(task.status, Status::Failed);
        assert!(task.error.is_some());
        // Compare numeric status code
        let status_code = task.error.as_ref().unwrap().code.as_u16();
        assert_eq!(status_code, 409); // CONFLICT
        match task.details {
            Some(Details::SingleIndexSnapshotImport { .. }) => {} // Expected structure
            _ => panic!("Incorrect task details for failed import: {:?}", task.details),
        }
    }

     #[test]
    fn test_import_snapshot_invalid_path() {
        let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        let target_index = "target_index_invalid_path";
        let invalid_path = "/tmp/nonexistent/snapshot.tar.gz"; // Path outside snapshots dir

        // Register the import task with invalid path
        let import_task = KindWithContent::SingleIndexSnapshotImport {
            source_snapshot_path: invalid_path.to_string(),
            target_index_uid: target_index.to_string(),
        };
        let task_id = index_scheduler.register(import_task, None, false).unwrap().uid;

        // Process the task - should fail
        handle.advance_one_failed_batch();

        // Assertions
        let rtxn = index_scheduler.read_txn().unwrap();
        // Use correct path to get_task
        let task = index_scheduler.queue.tasks.get_task(&rtxn, task_id).unwrap().unwrap();
        assert_eq!(task.status, Status::Failed);
        assert!(task.error.is_some());
        // Compare numeric status code
        let status_code = task.error.as_ref().unwrap().code.as_u16();
        // InvalidSnapshotPath (400), SnapshotImportFailed (500)
        assert!(matches!(status_code, 400 | 500));
    }

    #[test]
    fn test_import_snapshot_invalid_format() {
        let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        let target_index = "target_index_invalid_format";
        let snapshot_dir = index_scheduler.fj_snapshots_path();
        std::fs::create_dir_all(snapshot_dir).unwrap();
        let invalid_snapshot_path = snapshot_dir.join("invalid_format.snapshot.tar.gz");

        // Create an empty file as an invalid snapshot
        std::fs::File::create(&invalid_snapshot_path).unwrap(); // Use full path since import removed

        // Register the import task
        let import_task = KindWithContent::SingleIndexSnapshotImport {
            source_snapshot_path: invalid_snapshot_path.to_str().unwrap().to_string(),
            target_index_uid: target_index.to_string(),
        };
        let task_id = index_scheduler.register(import_task, None, false).unwrap().uid;

        // Process the task - should fail
        handle.advance_one_failed_batch();

        // Assertions
        let rtxn = index_scheduler.read_txn().unwrap();
        // Use correct path to get_task
        let task = index_scheduler.queue.tasks.get_task(&rtxn, task_id).unwrap().unwrap();
        assert_eq!(task.status, Status::Failed);
        assert!(task.error.is_some());
        // Compare numeric status code
        let status_code = task.error.as_ref().unwrap().code.as_u16();
         // SnapshotImportFailed (500)
        assert_eq!(status_code, 500);
    }

     #[test]
    fn test_import_snapshot_version_mismatch() {
        let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        let source_index = "source_index_version_mismatch";
        let target_index = "target_index_version_mismatch";
        let snapshot_filename = format!("{}-test.snapshot.tar.gz", source_index);

        // Create a valid snapshot first
        let snapshot_path =
            create_test_snapshot(&index_scheduler, source_index, &snapshot_filename);

        // Modify the metadata.json within the snapshot to have a different version
        let temp_extract_dir = tempdir().unwrap();
        let snapshot_file = std::fs::File::open(&snapshot_path).unwrap(); // Use full path
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(snapshot_file));
        archive.unpack(temp_extract_dir.path()).unwrap();

        let metadata_path = temp_extract_dir.path().join("metadata.json");
        let mut metadata: serde_json::Value = serde_json::from_reader(std::fs::File::open(&metadata_path).unwrap()).unwrap(); // Use full path
        metadata["meilisearchVersion"] = serde_json::Value::String("0.99.0".to_string()); // Incompatible version
        let mut metadata_file = std::fs::File::create(&metadata_path).unwrap(); // Use full path
        serde_json::to_writer_pretty(&mut metadata_file, &metadata).unwrap();
        metadata_file.flush().unwrap(); // Ensure data is written before re-packing

        // Re-pack the snapshot
        let new_snapshot_file = std::fs::File::create(&snapshot_path).unwrap(); // Use full path
        let enc = flate2::write::GzEncoder::new(new_snapshot_file, flate2::Compression::default());
        let mut tar_builder = tar::Builder::new(enc);
        tar_builder.append_dir_all(".", temp_extract_dir.path()).unwrap();
        tar_builder.finish().unwrap();
        // Ensure the underlying file is flushed and closed
        let gz_encoder = tar_builder.into_inner().unwrap();
        gz_encoder.finish().unwrap();


        // Register the import task
        let import_task = KindWithContent::SingleIndexSnapshotImport {
            source_snapshot_path: snapshot_path.to_str().unwrap().to_string(),
            target_index_uid: target_index.to_string(),
        };
        let task_id = index_scheduler.register(import_task, None, false).unwrap().uid;

        // Process the task - should fail
        handle.advance_one_failed_batch();

        // Assertions
        let rtxn = index_scheduler.read_txn().unwrap();
        // Use correct path to get_task
        let task = index_scheduler.queue.tasks.get_task(&rtxn, task_id).unwrap().unwrap();
        assert_eq!(task.status, Status::Failed);
        assert!(task.error.is_some());
        // Compare numeric status code
        let status_code = task.error.as_ref().unwrap().code.as_u16();
        assert_eq!(status_code, 400); // SnapshotVersionMismatch (BAD_REQUEST)
    }
}

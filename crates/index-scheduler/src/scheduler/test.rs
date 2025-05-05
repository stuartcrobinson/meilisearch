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
    async fn create_test_snapshot( // Make the function async
        index_scheduler: &IndexScheduler,
        // handle: &mut crate::test_utils::IndexSchedulerHandle, // Removed unused handle parameter
        source_index_uid: &str,
        // Remove unused target_snapshot_name: &str,
    ) -> PathBuf {
        // Initialize tracing subscriber for this test if not already done
        // This helps ensure logs are printed during test execution.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer() // Use test writer to work with cargo test capture
            .try_init(); // Use try_init to avoid panic if already initialized

        // Assume the index `source_index_uid` already exists and is prepared by the caller.
        // Get the handle to the existing index.
        let index = index_scheduler.index(source_index_uid).unwrap_or_else(|_| {
            panic!("Source index '{}' not found before calling create_test_snapshot", source_index_uid);
        });

        // Create the snapshot using the internal utility function
        let snapshot_dir = index_scheduler.fj_snapshots_path();
        // Verify snapshot directory exists and is writable
        if !snapshot_dir.exists() {
            std::fs::create_dir_all(&snapshot_dir).unwrap_or_else(|e| {
                panic!("Failed to create snapshot directory {:?}: {}", snapshot_dir, e);
            });
        }
        // Basic writability check (create and delete a temp file)
        let test_write_path = snapshot_dir.join(".write_test");
        std::fs::File::create(&test_write_path).and_then(|_| std::fs::remove_file(&test_write_path))
            .unwrap_or_else(|e| {
                panic!("Snapshot directory {:?} is not writable: {}", snapshot_dir, e);
            });

        // snapshot_dir is the directory where the snapshot will be created.
        // snapshot_path will be the *actual* path returned by the creation function.

        let index_rtxn = index.read_txn().unwrap();
        // Handle potential error from reading metadata
        let metadata = fj_snapshot_utils::read_metadata_inner(source_index_uid, &index, &index_rtxn)
            .unwrap_or_else(|e| {
                panic!("Failed to read metadata for index '{}' before snapshot creation: {}", source_index_uid, e);
            });
        drop(index_rtxn); // Drop txn before copying

        // Use the correct function name and capture the *returned path*
        let snapshot_path = fj_snapshot_utils::create_index_snapshot(
            source_index_uid, // Pass index_uid as first argument
            &index,
            metadata,
            &snapshot_dir, // Pass the directory path
        )
        .unwrap_or_else(|e| {
             // Panic with detailed error if creation failed, showing the intended directory
            panic!("fj_snapshot_utils::create_index_snapshot failed for directory {:?}: {}", snapshot_dir, e);
        });

        // Now snapshot_path holds the actual path to the created file.
        // Perform checks on this correct path.

        // The assertion should now pass as it checks the correct path
        assert!(snapshot_path.is_file(), "[create_test_snapshot] Snapshot file missing immediately after creation call (checking actual path): {:?}", snapshot_path);

        // Return the actual path of the created snapshot
        snapshot_path
    }

    #[actix_rt::test] // Mark test as async
    async fn test_import_snapshot_happy_path() { // Make test async
        let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        let source_index = "source_index_import_happy";
        let target_index = "target_index_import_happy";

        // 1. Create and prepare the source index
        let creation_task = index_creation_task(source_index, Some("id"));
        let _creation_task_id = index_scheduler.register(creation_task, None, false).unwrap().uid;
        handle.advance_one_successful_batch(); // Process index creation
        // Apply any necessary settings here if needed for the happy path test
        // (Currently create_test_snapshot applied filterable: name)
        let index = index_scheduler.index(source_index).unwrap();
        let mut wtxn = index.write_txn().unwrap();
        let mut settings = milli::update::Settings::new(
            &mut wtxn,
            &index,
            index_scheduler.indexer_config(),
        );
        settings.set_filterable_fields(vec![FilterableAttributesRule::Field("name".to_string())]);
        settings.execute(|_| {}, || false).unwrap();
        wtxn.commit().unwrap();

        // 2. Create the snapshot of the prepared index
        let snapshot_path =
            create_test_snapshot(&index_scheduler, /* &mut handle, */ source_index).await; // Removed handle

        // 3. Register the import task
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
                // Extract the expected UID part from the filename stem
                let filename_stem = snapshot_path.file_stem().unwrap().to_str().unwrap();
                let expected_uid_part = filename_stem.split_once('-').map(|(_, uid)| uid).unwrap_or(filename_stem); // Get part after first '-' or full stem

                assert_eq!(source_snapshot_uid, expected_uid_part, "Snapshot UID mismatch in task details");
                assert_eq!(target_index_uid, target_index, "Target index UID mismatch in task details");
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

     #[actix_rt::test] // Mark test as async
    async fn test_import_snapshot_with_embedders() { // Make test async
        let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        let source_index = "source_index_import_embed";
        let target_index = "target_index_import_embed";
        // let snapshot_filename = format!("{}-test.snapshot.tar.gz", source_index); // Remove unused variable

        // 1. Create source index and add settings including embedders
        // REMOVED: Redundant index creation - create_test_snapshot handles this.
        // let task = index_creation_task(source_index, Some("id"));
        // let _task_id = index_scheduler.register(task, None, false).unwrap().uid; // Prefix unused task_id
        // handle.advance_one_successful_batch(); // Use handle to process the task

        // Get the index handle *after* create_test_snapshot has created it.
        // Note: create_test_snapshot needs to be called *before* this.
        // We'll adjust the order below.

        // Apply settings *before* snapshotting (within create_test_snapshot or similar helper)
        // This logic needs to be part of the snapshot creation setup, not the import test itself.
        // Let's assume create_test_snapshot is modified or a new helper is used
        // to handle setting embedders *before* snapshotting.
        // For now, we'll remove the direct setting application here.

        // Re-introduce getting the source index handle and wtxn *before* snapshotting
        // Note: create_test_snapshot will handle the initial index creation if needed.
        // We need to ensure the index exists *before* trying to apply settings.
        // Let's create the index first, then apply settings, then snapshot.

        // 1. Ensure source index exists (can use index_creation_task + advance)
        let creation_task = index_creation_task(source_index, Some("id"));
        let _creation_task_id = index_scheduler.register(creation_task, None, false).unwrap().uid;
        handle.advance_one_successful_batch(); // Process index creation

        // 2. Get handle and apply settings to the source index
        let index = index_scheduler.index(source_index).unwrap();
        let mut wtxn = index.write_txn().unwrap();
        let mut settings = milli::update::Settings::new(
            &mut wtxn, // Now wtxn is defined
            &index,  // Now index is defined
            index_scheduler.indexer_config(), // index_scheduler is available
        );
        let mut embedders = BTreeMap::default();
        embedders.insert(S("default"), Setting::Set(EmbeddingSettings {
            source: Setting::Set(EmbedderSource::UserProvided),
            dimensions: Setting::Set(1),
            ..Default::default()
        }));
        settings.set_embedder_settings(embedders); // Apply the embedder settings
        settings.execute(|_| {}, || false).unwrap(); // Execute settings update
        wtxn.commit().unwrap(); // Commit the settings transaction

        // 3. Create the snapshot (now that settings are applied)
        // create_test_snapshot will use the existing index with applied settings.
        let snapshot_path =
            create_test_snapshot(&index_scheduler, /* &mut handle, */ source_index).await; // Removed handle

        // 4. Register the import task
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

    #[actix_rt::test] // Mark test as async
    async fn test_import_snapshot_target_exists() { // Make test async
        let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        let source_index = "source_index_import_exists";
        let target_index = "target_index_import_exists";

        // 1. Create and prepare the source index
        let source_creation_task = index_creation_task(source_index, Some("id"));
        let _source_creation_task_id = index_scheduler.register(source_creation_task, None, false).unwrap().uid;
        handle.advance_one_successful_batch(); // Process source index creation
        // Apply settings matching what create_test_snapshot used to do
        let index = index_scheduler.index(source_index).unwrap();
        let mut wtxn = index.write_txn().unwrap();
        let mut settings = milli::update::Settings::new(
            &mut wtxn,
            &index,
            index_scheduler.indexer_config(),
        );
        settings.set_filterable_fields(vec![FilterableAttributesRule::Field("name".to_string())]);
        settings.execute(|_| {}, || false).unwrap();
        wtxn.commit().unwrap();

        // 2. Create the snapshot of the prepared source index
        let snapshot_path =
            create_test_snapshot(&index_scheduler, /* &mut handle, */ source_index).await; // Removed handle

        // 3. Create the target index beforehand (this is the point of the test)
        let creation_task = index_creation_task(target_index, Some("id"));
        let _creation_task_id = index_scheduler.register(creation_task, None, false).unwrap().uid;
        handle.advance_one_successful_batch(); // Use handle to process the task


        // Register the import task
        let import_task = KindWithContent::SingleIndexSnapshotImport {
            source_snapshot_path: snapshot_path.to_str().unwrap().to_string(),
            target_index_uid: target_index.to_string(),
        };
        let task_id = index_scheduler.register(import_task, None, false).unwrap().uid;

        // Process the task
        handle.advance_one_failed_batch(); // Expect processing to fail

        // Assertions
        let rtxn = index_scheduler.read_txn().unwrap();
        // Use correct path to get_task
        let task = index_scheduler.queue.tasks.get_task(&rtxn, task_id).unwrap().unwrap();
        assert_eq!(task.status, Status::Failed);
        assert!(task.error.is_some(), "Task should have failed with an error");
        let response_error = task.error.as_ref().unwrap();

        // Verify the error message contains expected text for target index exists.
        assert!(
            response_error.message.contains("Target index") &&
            response_error.message.contains(target_index) && // Check the specific index is mentioned
            response_error.message.contains("already exists during snapshot import"),
            "Error message mismatch. Expected 'Target index ... already exists ...', got: {}",
            response_error.message
        );
        // StatusCode is not persisted.

        match task.details {
            Some(Details::SingleIndexSnapshotImport { .. }) => {} // Check structure is still correct
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

        // Process the task
        handle.advance_one_failed_batch(); // Expect processing to fail

        // Assertions
        let rtxn = index_scheduler.read_txn().unwrap();
        // Use correct path to get_task
        let task = index_scheduler.queue.tasks.get_task(&rtxn, task_id).unwrap().unwrap();
        assert_eq!(task.status, Status::Failed);
        assert!(task.error.is_some(), "Task should have failed with an error");
        let response_error = task.error.as_ref().unwrap();

        // Verify the error message contains expected text for invalid path.
        assert!(
            response_error.message.contains("Invalid snapshot path") &&
            response_error.message.contains(invalid_path) && // Check the specific path is mentioned
            response_error.message.contains("must be within the configured snapshots directory"),
            "Error message mismatch. Expected 'Invalid snapshot path ...', got: {}",
            response_error.message
        );
        // StatusCode is not persisted.
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

        // Process the task
        handle.advance_one_failed_batch(); // Expect processing to fail

        // Assertions
        let rtxn = index_scheduler.read_txn().unwrap();
        // Use correct path to get_task
        let task = index_scheduler.queue.tasks.get_task(&rtxn, task_id).unwrap().unwrap();
        assert_eq!(task.status, Status::Failed);
        assert!(task.error.is_some(), "Task should have failed with an error");
        let response_error = task.error.as_ref().unwrap();

        // Verify the error message contains expected text for invalid format/unpack error.
        // The exact message might vary depending on the underlying tar/gz error.
        // Check for a reasonable substring.
        assert!(
            response_error.message.contains("failed to iterate over archive") || // Common tar error
            response_error.message.contains("Invalid snapshot format") || // Our specific error
            response_error.message.contains("Snapshot import failed"), // General import error
            "Error message mismatch. Expected format/unpack error, got: {}",
            response_error.message
        );
        // StatusCode is not persisted.
    }

     #[actix_rt::test] // Mark test as async
    async fn test_import_snapshot_version_mismatch() { // Make test async
        let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        let source_index = "source_index_version_mismatch";
        let target_index = "target_index_version_mismatch";

        // 1. Create and prepare the source index
        let source_creation_task = index_creation_task(source_index, Some("id"));
        let _source_creation_task_id = index_scheduler.register(source_creation_task, None, false).unwrap().uid;
        handle.advance_one_successful_batch(); // Process source index creation
        // Apply settings matching what create_test_snapshot used to do
        let index = index_scheduler.index(source_index).unwrap();
        let mut wtxn = index.write_txn().unwrap();
        let mut settings = milli::update::Settings::new(
            &mut wtxn,
            &index,
            index_scheduler.indexer_config(),
        );
        settings.set_filterable_fields(vec![FilterableAttributesRule::Field("name".to_string())]);
        settings.execute(|_| {}, || false).unwrap();
        wtxn.commit().unwrap();

        // 2. Create the snapshot of the prepared source index
        let snapshot_path =
            create_test_snapshot(&index_scheduler, /* &mut handle, */ source_index).await; // Removed handle

        // 3. Check snapshot exists *after* creation
        assert!(snapshot_path.is_file(), "[test_import_snapshot_version_mismatch] Snapshot file missing after creation: {:?}", snapshot_path);

        // Modify the metadata.json within the snapshot to have a different version
        let temp_extract_dir = tempdir().unwrap();

        // Unpack the original snapshot
        {
            let snapshot_file = std::fs::File::open(&snapshot_path).unwrap();
            let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(snapshot_file));
            // Add explicit error mapping for unpack
            archive.unpack(temp_extract_dir.path()).map_err(|e| {
                format!("Failed to unpack snapshot '{}': {}", snapshot_path.display(), e)
            }).unwrap();
            // snapshot_file is dropped here
        } // End of scope for snapshot_file

        // This is the correct place for metadata_path definition
        let metadata_path = temp_extract_dir.path().join("metadata.json");
        // Read and modify metadata
        let mut metadata: serde_json::Value = serde_json::from_reader(std::fs::File::open(&metadata_path).unwrap()).unwrap(); // Use full path
        metadata["meilisearchVersion"] = serde_json::Value::String("0.99.0".to_string()); // Incompatible version
        // Write modified metadata back
        let mut metadata_file = std::fs::File::create(&metadata_path).unwrap(); // Use full path
        serde_json::to_writer_pretty(&mut metadata_file, &metadata).unwrap();
        metadata_file.flush().unwrap();
        drop(metadata_file); // Explicitly close metadata file before repacking

        // Re-pack the snapshot
        { // Scope for repacking file handles
            let new_snapshot_file = std::fs::File::create(&snapshot_path).unwrap();
            let enc = flate2::write::GzEncoder::new(new_snapshot_file, flate2::Compression::default());
            let mut tar_builder = tar::Builder::new(enc);
        // Iterate and add entries individually relative to the temp dir
        for entry in walkdir::WalkDir::new(temp_extract_dir.path()).min_depth(1) {
            let entry = entry.unwrap();
            let path = entry.path();
            let name = path.strip_prefix(temp_extract_dir.path()).unwrap();
            if path.is_file() {
                tar_builder.append_path_with_name(path, name).unwrap();
            } else if path.is_dir() {
                tar_builder.append_dir(name, path).unwrap();
            }
        }
            tar_builder.finish().unwrap();
            // Ensure the underlying file is flushed and closed
            let gz_encoder = tar_builder.into_inner().unwrap();
            gz_encoder.finish().unwrap();
            // new_snapshot_file and enc are dropped here
        }


        // Register the import task
        let import_task = KindWithContent::SingleIndexSnapshotImport {
            source_snapshot_path: snapshot_path.to_str().unwrap().to_string(),
            target_index_uid: target_index.to_string(),
        };
        let task_id = index_scheduler.register(import_task, None, false).unwrap().uid;

        // Process the task
        handle.advance_one_failed_batch(); // Expect processing to fail

        // Assertions
        let rtxn = index_scheduler.read_txn().unwrap();
        // Use correct path to get_task
        let task = index_scheduler.queue.tasks.get_task(&rtxn, task_id).unwrap().unwrap();
        assert_eq!(task.status, Status::Failed);
        assert!(task.error.is_some(), "Task should have failed with an error");
        let response_error = task.error.as_ref().unwrap();

        // Verify the error message contains expected text for version mismatch.
        // This checks the persisted information.
        assert!(
            response_error.message.contains("Snapshot version mismatch"),
            "Error message mismatch. Expected 'Snapshot version mismatch', got: {}",
            response_error.message
        );
        // Extract expected versions from the error message itself for robustness
        let expected_snapshot_version = "0.99.0"; // As set in the test
        let expected_instance_version = env!("CARGO_PKG_VERSION"); // Get current version dynamically
        assert!(
            response_error.message.contains(&format!("snapshot version is `{}`", expected_snapshot_version)),
            "Error message mismatch. Expected snapshot version '{}', got: {}",
            expected_snapshot_version, response_error.message
        );
        assert!(
            response_error.message.contains(&format!("instance version is `{}`", expected_instance_version)),
            "Error message mismatch. Expected instance version '{}', got: {}",
            expected_instance_version, response_error.message
        );

        // The StatusCode (response_error.code) is not persisted due to #[serde(skip)].
        // Asserting on it after deserialization will likely fail as it defaults to 200 OK.
        // Checking the message content is a more reliable way to verify the correct error was stored.
    }

    #[actix_rt::test]
    async fn test_import_snapshot_with_all_settings() {
        // Corrected and added imports
        use meilisearch_types::locales::LocalizedAttributesRuleView; // Correct path
        // DIAGNOSE: Check available modules under settings
        // dbg!(meilisearch_types::settings::*); // This won't compile directly, let's check the type itself
        // Let's try importing the parent and see what the compiler suggests for facet
        // use meilisearch_types::settings; // Keep this commented unless needed

        // Corrected facet import path - Use facet_values_sort::FacetValuesSort
        // Removed unused: FacetSearchSettings
        use meilisearch_types::facet_values_sort::FacetValuesSort; // Use correct type instead of OrderByType
        // Removed unused: PrefixSearchSettings, FacetingSettings, MinWordSizeForTypos, PaginationSettings, TypoToleranceSettings
        use milli::index::PrefixSearch; // Correct path
        use milli::proximity::ProximityPrecision; // Correct path
        // Corrected LocalizedAttributesRule import path again
        use milli::LocalizedAttributesRule;
        use milli::order_by_map::OrderByMap; // Added import
        use milli::OrderBy; // Added import
        // Removed unused AttributePatterns import
        // Corrected Language import path - Assuming it's re-exported under tokenizer
        use milli::tokenizer::Language;
        use std::collections::{BTreeMap, BTreeSet, HashSet};
        // Removed unused FromStr import

        let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        let source_index = "source_index_all_settings";
        let target_index = "target_index_all_settings";

        // 1. Create and prepare the source index with various non-default settings
        let creation_task = index_creation_task(source_index, Some("id"));
        let _creation_task_id = index_scheduler.register(creation_task, None, false).unwrap().uid;
        handle.advance_one_successful_batch(); // Process index creation

        let index = index_scheduler.index(source_index).unwrap();
        let mut wtxn = index.write_txn().unwrap();
        let mut settings = milli::update::Settings::new(
            &mut wtxn,
            &index,
            index_scheduler.indexer_config(),
        );

        // Apply non-default settings
        settings.set_autorize_typos(false);
        settings.set_min_word_len_one_typo(6);
        settings.set_min_word_len_two_typos(10);
        settings.set_exact_words(BTreeSet::from(["exact".to_string()]));
        settings.set_exact_attributes(HashSet::from(["exact_attr".to_string()]));

        settings.set_max_values_per_facet(50);
        let mut sort_facet_values_by = BTreeMap::new();
        // DIAGNOSE: Check available variants/associated items for milli::OrderBy
        // dbg!(milli::OrderBy::*); // This won't compile, let's check the type itself
        // Let's try using a known variant if available, or check definition
        // sort_facet_values_by.insert("size".to_string(), dbg!(milli::OrderBy::Asc)); // Example check
        // Correct OrderBy variant usage (use Count to match assertion)
        sort_facet_values_by.insert("size".to_string(), OrderBy::Count);
        // Convert BTreeMap using FromIterator
        settings.set_sort_facet_values_by(sort_facet_values_by.into_iter().collect::<OrderByMap>()); // Use FromIterator

        settings.set_pagination_max_total_hits(500);

        settings.set_proximity_precision(ProximityPrecision::ByWord);

        settings.set_localized_attributes_rules(vec![LocalizedAttributesRule {
            attribute_patterns: vec!["title#fr".to_string()].into(), // Use .into()
            locales: vec![Language::from_code("fra").unwrap()], // Use 3-letter code "fra"
        }]);

        settings.set_separator_tokens(BTreeSet::from(["&".to_string()]));
        settings.set_non_separator_tokens(BTreeSet::from(["#".to_string()]));
        settings.set_dictionary(BTreeSet::from(["wordA".to_string(), "wordB".to_string()]));
        settings.set_search_cutoff(100);
        // Corrected PrefixSearch construction (use IndexingTime variant)
        let prefix_search_value = PrefixSearch::IndexingTime;
        settings.set_prefix_search(prefix_search_value); // Use imported milli::index::PrefixSearch
        // Removed incorrect set_facet_search call

        // Keep embedders simple as tested elsewhere
        let mut embedders = BTreeMap::default();
        embedders.insert(S("default"), Setting::Set(EmbeddingSettings {
            source: Setting::Set(EmbedderSource::UserProvided),
            dimensions: Setting::Set(1),
            ..Default::default()
        }));
        settings.set_embedder_settings(embedders);

        settings.execute(|_| {}, || false).unwrap();
        wtxn.commit().unwrap();

        // 2. Create the snapshot of the prepared index
        let snapshot_path =
            create_test_snapshot(&index_scheduler, /* &mut handle, */ source_index).await;

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
        let task = index_scheduler.queue.tasks.get_task(&rtxn, task_id).unwrap().unwrap();
        assert_eq!(task.status, Status::Succeeded, "Import task failed: {:?}", task.error);
        assert!(task.error.is_none());

        assert!(index_scheduler.index_exists(target_index).unwrap());
        // Re-add explicit type annotation to diagnose assignment
        let imported_index: milli::Index = index_scheduler.index(target_index).unwrap();
        // DIAGNOSE: Check the type of imported_index - REMOVED due to E0277
        // dbg!(&imported_index);
        let index_rtxn = imported_index.read_txn().unwrap();

        // Verify Typo Tolerance (using individual getters)
        // Corrected assertions: getters return T, not Option<T>
        assert_eq!(imported_index.authorize_typos(&index_rtxn).unwrap(), false);
        assert_eq!(imported_index.min_word_len_one_typo(&index_rtxn).unwrap(), 6);
        assert_eq!(imported_index.min_word_len_two_typos(&index_rtxn).unwrap(), 10);
        // Convert Option<&fst::Set> to Option<BTreeSet<String>> for comparison
        let actual_exact_words: Option<BTreeSet<String>> = imported_index.exact_words(&index_rtxn).unwrap().map(|fst_set| {
            fst_set.stream().into_strs().unwrap().into_iter().collect() // Use into_strs()
        });
        assert_eq!(actual_exact_words, Some(BTreeSet::from(["exact".to_string()])));
        // Convert Vec<&str> to HashSet<String> for comparison
        let actual_exact_attributes: HashSet<String> = imported_index.exact_attributes(&index_rtxn).unwrap().into_iter().map(String::from).collect();
        assert_eq!(Some(actual_exact_attributes), Some(HashSet::from(["exact_attr".to_string()])));


        // Verify Faceting (using individual getters)
        assert_eq!(imported_index.max_values_per_facet(&index_rtxn).unwrap(), Some(50));
        // Define expected using FacetValuesSort, including the default "*" entry
        let expected_sort_by: BTreeMap<String, FacetValuesSort> =
            BTreeMap::from([
                ("*".to_string(), FacetValuesSort::Alpha), // Add default entry
                ("size".to_string(), FacetValuesSort::Count)
            ]);
        // Convert milli::OrderByMap to BTreeMap<String, meilisearch_types::FacetValuesSort> for comparison
        let actual_sort_by: BTreeMap<String, FacetValuesSort> = imported_index.sort_facet_values_by(&index_rtxn).unwrap().into_iter().map(|(k, v)| (k, v.into())).collect();
        assert_eq!(actual_sort_by, expected_sort_by);


        // Verify Pagination (using individual getter)
        assert_eq!(imported_index.pagination_max_total_hits(&index_rtxn).unwrap(), Some(500));

        // Verify Proximity Precision
        let proximity = imported_index.proximity_precision(&index_rtxn).unwrap();
        // Corrected proximity assertion (wrap in Some)
        assert_eq!(proximity, Some(ProximityPrecision::ByWord));

        // Verify Localized Attributes
        let localized = imported_index.localized_attributes_rules(&index_rtxn).unwrap();
        // Corrected construction of expected_localized
        let expected_localized_view = vec![LocalizedAttributesRuleView {
            // Use .into() for AttributePatterns as suggested by compiler
            attribute_patterns: vec!["title#fr".to_string()].into(),
            // Corrected Language construction (use Language::from_code) and convert to Locale
            // Explicitly assign locale vector before struct literal
            locales: vec![ Language::from_code("fra").unwrap().into() ], // Use 3-letter code "fra" and .into()
        }]; // Removed extraneous closing parenthesis
        // Corrected assertion for localized attributes
        let expected_localized_milli: Vec<LocalizedAttributesRule> = expected_localized_view.into_iter().map(|v| v.into()).collect();
        assert_eq!(localized, Some(expected_localized_milli));


        // Verify Tokenization Settings (Handle Option<&BTreeSet>)
        let expected_separators = BTreeSet::from(["&".to_string()]);
        // Compare Option<BTreeSet> by cloning the Option<&BTreeSet>
        assert_eq!(imported_index.separator_tokens(&index_rtxn).unwrap(), Some(&expected_separators).cloned());

        let expected_non_separators = BTreeSet::from(["#".to_string()]);
        assert_eq!(imported_index.non_separator_tokens(&index_rtxn).unwrap(), Some(&expected_non_separators).cloned());

        let expected_dictionary = BTreeSet::from(["wordA".to_string(), "wordB".to_string()]);
        assert_eq!(imported_index.dictionary(&index_rtxn).unwrap(), Some(&expected_dictionary).cloned());


        // Verify Search Cutoff
        assert_eq!(imported_index.search_cutoff(&index_rtxn).unwrap(), Some(100));

        // Verify Prefix Search (Compare milli::index::PrefixSearch)
        let prefix_search = imported_index.prefix_search(&index_rtxn).unwrap();
        // Corrected expected_prefix_search construction (use IndexingTime variant)
        let expected_prefix_search = PrefixSearch::IndexingTime; // Use milli::index::PrefixSearch::IndexingTime
        assert_eq!(prefix_search, Some(expected_prefix_search)); // Getter returns Option

        // Removed Facet Search assertion as it's not set/retrieved this way

        // Verify Embedders (basic check)
        let imported_embedders = imported_index.embedding_configs(&index_rtxn).unwrap();
        assert_eq!(imported_embedders.len(), 1);
        assert!(imported_embedders.iter().any(|c| c.name == "default"));
    }
}

// [meilisearchfj] End-to-end tests for Single Index Snapshot create/import via scheduler
#[cfg(test)]
mod msfj_sis_scheduler_e2e_tests {
    use super::*; // Bring parent module's imports into scope
    use crate::test_utils::{index_creation_task, replace_document_import_task, sample_documents};
    use crate::IndexScheduler;
    use big_s::S;
    use meilisearch_types::batches::Batch; // For progress trace verification
    use meilisearch_types::facet_values_sort::FacetValuesSort;
    use meilisearch_types::locales::LocalizedAttributesRuleView;
    use meilisearch_types::milli::vector::settings::{EmbedderSource, EmbeddingSettings};
    use meilisearch_types::settings::{Settings, Unchecked};
    use meilisearch_types::tasks::{Details, KindWithContent, Status};
    use milli::index::PrefixSearch;
    use milli::proximity::ProximityPrecision;
    use milli::update::Setting;
    use milli::{FilterableAttributesRule, LocalizedAttributesRule, OrderBy, OrderByMap};
    use std::collections::{BTreeMap, BTreeSet, HashSet};
    use std::path::PathBuf;

    #[actix_rt::test]
    async fn test_e2e_snapshot_create_import_verify() {
        // === 1. Setup ===
        let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
        let source_index_uid = S("source_e2e");
        let target_index_uid = S("target_e2e");

        // === 2. Prepare Source Index ===

        // Create index
        let task = index_creation_task(&source_index_uid, Some("id"));
        let _task_id = index_scheduler.register(task, None, false).unwrap().uid;
        handle.advance_one_successful_batch();

        // Add documents
        let (file, documents_count) = sample_documents(&index_scheduler, 0, 10); // 10 documents
        file.persist().unwrap();
        let task = replace_document_import_task(&source_index_uid, Some("id"), 0, documents_count);
        let _task_id = index_scheduler.register(task, None, false).unwrap().uid;
        handle.advance_one_successful_batch();

        // Apply diverse settings
        let mut settings = Settings::<Unchecked>::default();
        settings.displayed_attributes = Setting::Set(vec![S("id"), S("name")]);
        settings.searchable_attributes = Setting::Set(vec![S("name"), S("description")]);
        settings.filterable_attributes =
            Setting::Set(vec![FilterableAttributesRule::Field(S("category"))]);
        settings.sortable_attributes = Setting::Set(vec![S("price")].into_iter().collect()); // Use BTreeSet
        settings.ranking_rules = Setting::Set(vec![
            milli::RankingRule::Typo,
            milli::RankingRule::Words,
            milli::RankingRule::Proximity,
        ]);
        settings.stop_words = Setting::Set(BTreeSet::from([S("the"), S("a")]));
        settings.synonyms = Setting::Set(BTreeMap::from([(S("cat"), vec![S("feline")])]));
        settings.distinct_attribute = Setting::Set(Some(S("sku")));
        // Typo Tolerance
        settings.typo_tolerance = Setting::Set(meilisearch_types::settings::TypoToleranceSettings {
            enabled: Setting::Set(false),
            min_word_size_for_typos: Setting::Set(
                meilisearch_types::settings::MinWordSizeForTypos {
                    one_typo: Setting::Set(6),
                    two_typos: Setting::Set(10),
                },
            ),
            disable_on_words: Setting::Set(BTreeSet::from([S("exactword")])),
            disable_on_attributes: Setting::Set(HashSet::from([S("exactattr")])),
        });
        // Faceting
        settings.faceting = Setting::Set(meilisearch_types::settings::FacetingSettings {
            max_values_per_facet: Setting::Set(50),
            sort_facet_values_by: Setting::Set(BTreeMap::from([(
                S("size"),
                FacetValuesSort::Count,
            )])),
            // facet_search: Setting::NotSet, // Assuming default or handled elsewhere
        });
        // Pagination
        settings.pagination = Setting::Set(meilisearch_types::settings::PaginationSettings {
            max_total_hits: Setting::Set(500),
        });
        // Proximity Precision
        settings.proximity_precision = Setting::Set(ProximityPrecision::ByWord);
        // Embedders
        let mut embedders = BTreeMap::default();
        embedders.insert(
            S("default"),
            Setting::Set(EmbeddingSettings {
                source: Setting::Set(EmbedderSource::UserProvided),
                dimensions: Setting::Set(1),
                ..Default::default()
            }),
        );
        settings.embedders = Setting::Set(embedders);
        // Localized Attributes
        settings.localized_attributes = Setting::Set(vec![LocalizedAttributesRuleView {
            attribute_patterns: vec![S("title#fr")].into(),
            locales: vec![milli::tokenizer::Language::from_code("fra").unwrap().into()],
        }]);
        // Tokenization
        settings.separator_tokens = Setting::Set(BTreeSet::from([S("&")]));
        settings.non_separator_tokens = Setting::Set(BTreeSet::from([S("#")]));
        settings.dictionary = Setting::Set(BTreeSet::from([S("wordA"), S("wordB")]));
        // Search Cutoff
        settings.search_cutoff_ms = Setting::Set(100);
        // Prefix Search
        settings.prefix_search = Setting::Set(PrefixSearch::IndexingTime);
        // Facet Search (Assuming it exists and needs setting)
        // settings.facet_search = Setting::Set(meilisearch_types::settings::FacetSearchSettings {
        //     enabled: Setting::Set(true),
        //     max_candidates: Setting::Set(10),
        // });

        let task = KindWithContent::SettingsUpdate {
            index_uid: source_index_uid.clone(),
            new_settings: Box::new(settings),
            is_deletion: false,
            allow_index_creation: false, // Index already exists
        };
        let _task_id = index_scheduler.register(task, None, false).unwrap().uid;
        handle.advance_one_successful_batch();

        // === 3. Create Snapshot ===
        let creation_task_payload =
            KindWithContent::SingleIndexSnapshotCreation { index_uid: source_index_uid.clone() };
        let creation_task_id =
            index_scheduler.register(creation_task_payload, None, false).unwrap().uid;

        handle.advance_one_successful_batch(); // Process snapshot creation

        let snapshot_path: PathBuf; // Declare snapshot_path here
        {
            // Scope for read transaction
            let rtxn = index_scheduler.read_txn().unwrap();
            let creation_task =
                index_scheduler.queue.tasks.get_task(&rtxn, creation_task_id).unwrap().unwrap();
            assert_eq!(
                creation_task.status,
                Status::Succeeded,
                "Snapshot creation failed: {:?}",
                creation_task.error
            );
            assert!(creation_task.error.is_none());

            let snapshot_uid = match creation_task.details {
                Some(Details::SingleIndexSnapshotCreation { snapshot_uid: Some(uid) }) => uid,
                _ => panic!("Snapshot UID not found in creation task details"),
            };

            // Construct path based on convention: {index_uid}-{snapshot_uid}.snapshot.tar.gz
            let filename = format!("{}-{}.snapshot.tar.gz", source_index_uid, snapshot_uid);
            snapshot_path = index_scheduler.fj_snapshots_path().join(filename); // Assign to outer scope variable
            assert!(snapshot_path.is_file(), "Snapshot file not found at expected path: {:?}", snapshot_path);

            // Verify creation progress trace
            let batch_id = creation_task.batch_uid.expect("Creation task should have a batch UID");
            let batch: Batch =
                index_scheduler.queue.batches.get_batch(&rtxn, batch_id).unwrap().expect("Creation batch not found");
            let progress_steps: Vec<String> =
                batch.stats.progress_trace.iter().map(|(name, _)| name.clone()).collect();
            assert_eq!(
                progress_steps,
                vec![
                    "processing tasks",
                    "reading metadata",
                    "copying index data",
                    "packaging snapshot",
                    "writing tasks to disk"
                ],
                "Progress trace mismatch for snapshot creation"
            );
        } // Read transaction dropped here

        // === 4. Import Snapshot ===
        let import_task_payload = KindWithContent::SingleIndexSnapshotImport {
            source_snapshot_path: snapshot_path.to_str().unwrap().to_string(),
            target_index_uid: target_index_uid.clone(),
        };
        let import_task_id =
            index_scheduler.register(import_task_payload, None, false).unwrap().uid;

        handle.advance_one_successful_batch(); // Process snapshot import

        {
            // Scope for read transaction
            let rtxn = index_scheduler.read_txn().unwrap();
            let import_task =
                index_scheduler.queue.tasks.get_task(&rtxn, import_task_id).unwrap().unwrap();
            assert_eq!(
                import_task.status,
                Status::Succeeded,
                "Snapshot import failed: {:?}",
                import_task.error
            );
            assert!(import_task.error.is_none());

            // Assert details
            match import_task.details {
                Some(Details::SingleIndexSnapshotImport {
                    source_snapshot_uid: details_source_uid,
                    target_index_uid: details_target_uid,
                }) => {
                    let expected_source_uid = snapshot_path
                        .file_stem()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .split_once('-')
                        .map(|(_, uid)| uid)
                        .unwrap_or("");
                    assert_eq!(details_source_uid, expected_source_uid);
                    assert_eq!(details_target_uid, target_index_uid);
                }
                _ => panic!("Incorrect details for import task: {:?}", import_task.details),
            }

            // Verify import progress trace
            let batch_id = import_task.batch_uid.expect("Import task should have a batch UID");
            let batch: Batch =
                index_scheduler.queue.batches.get_batch(&rtxn, batch_id).unwrap().expect("Import batch not found");
            let progress_steps: Vec<String> =
                batch.stats.progress_trace.iter().map(|(name, _)| name.clone()).collect();
            assert_eq!(
                progress_steps,
                vec![
                    "processing tasks",
                    "validating snapshot",
                    "unpacking snapshot",
                    "applying settings",
                    "writing tasks to disk"
                ],
                "Progress trace mismatch for snapshot import"
            );
        } // Read transaction dropped here

        // === 5. Verify Indexes ===
        let source_index = index_scheduler.index(&source_index_uid).unwrap();
        let target_index = index_scheduler.index(&target_index_uid).unwrap();

        let source_rtxn = source_index.read_txn().unwrap();
        let target_rtxn = target_index.read_txn().unwrap();

        // Verify document count
        assert_eq!(
            source_index.number_of_documents(&source_rtxn).unwrap(),
            target_index.number_of_documents(&target_rtxn).unwrap(),
            "Document count mismatch"
        );

        // Verify all settings (add more assertions as needed)
        assert_eq!(
            source_index.displayed_attributes(&source_rtxn).unwrap(),
            target_index.displayed_attributes(&target_rtxn).unwrap(),
            "Displayed attributes mismatch"
        );
        assert_eq!(
            source_index.searchable_attributes(&source_rtxn).unwrap(),
            target_index.searchable_attributes(&target_rtxn).unwrap(),
            "Searchable attributes mismatch"
        );
        assert_eq!(
            source_index.filterable_attributes_rules(&source_rtxn).unwrap(),
            target_index.filterable_attributes_rules(&target_rtxn).unwrap(),
            "Filterable attributes mismatch"
        );
        assert_eq!(
            source_index.sortable_attributes(&source_rtxn).unwrap(),
            target_index.sortable_attributes(&target_rtxn).unwrap(),
            "Sortable attributes mismatch"
        );
        assert_eq!(
            source_index.ranking_rules(&source_rtxn).unwrap(),
            target_index.ranking_rules(&target_rtxn).unwrap(),
            "Ranking rules mismatch"
        );
        assert_eq!(
            source_index.stop_words(&source_rtxn).unwrap(),
            target_index.stop_words(&target_rtxn).unwrap(),
            "Stop words mismatch"
        );
        assert_eq!(
            source_index.synonyms(&source_rtxn).unwrap(),
            target_index.synonyms(&target_rtxn).unwrap(),
            "Synonyms mismatch"
        );
        assert_eq!(
            source_index.distinct_attribute(&source_rtxn).unwrap(),
            target_index.distinct_attribute(&target_rtxn).unwrap(),
            "Distinct attribute mismatch"
        );
        // Typo Tolerance
        assert_eq!(
            source_index.authorize_typos(&source_rtxn).unwrap(),
            target_index.authorize_typos(&target_rtxn).unwrap(),
            "Typo tolerance enabled mismatch"
        );
        assert_eq!(
            source_index.min_word_len_one_typo(&source_rtxn).unwrap(),
            target_index.min_word_len_one_typo(&target_rtxn).unwrap(),
            "Min word len one typo mismatch"
        );
        assert_eq!(
            source_index.min_word_len_two_typos(&source_rtxn).unwrap(),
            target_index.min_word_len_two_typos(&target_rtxn).unwrap(),
            "Min word len two typos mismatch"
        );
        assert_eq!(
            source_index.exact_words(&source_rtxn).unwrap(),
            target_index.exact_words(&target_rtxn).unwrap(),
            "Exact words mismatch"
        );
         assert_eq!(
            source_index.exact_attributes(&source_rtxn).unwrap().into_iter().collect::<HashSet<_>>(), // Convert Vec<&str> to HashSet<String>
            target_index.exact_attributes(&target_rtxn).unwrap().into_iter().collect::<HashSet<_>>(), // Convert Vec<&str> to HashSet<String>
            "Exact attributes mismatch"
        );
        // Faceting
        assert_eq!(
            source_index.max_values_per_facet(&source_rtxn).unwrap(),
            target_index.max_values_per_facet(&target_rtxn).unwrap(),
            "Max values per facet mismatch"
        );
        assert_eq!(
            source_index.sort_facet_values_by(&source_rtxn).unwrap(),
            target_index.sort_facet_values_by(&target_rtxn).unwrap(),
            "Sort facet values by mismatch"
        );
        // Pagination
        assert_eq!(
            source_index.pagination_max_total_hits(&source_rtxn).unwrap(),
            target_index.pagination_max_total_hits(&target_rtxn).unwrap(),
            "Pagination max total hits mismatch"
        );
        // Proximity Precision
        assert_eq!(
            source_index.proximity_precision(&source_rtxn).unwrap(),
            target_index.proximity_precision(&target_rtxn).unwrap(),
            "Proximity precision mismatch"
        );
        // Embedders
        assert_eq!(
            source_index.embedding_configs(&source_rtxn).unwrap(),
            target_index.embedding_configs(&target_rtxn).unwrap(),
            "Embedder configs mismatch"
        );
        // Localized Attributes
        assert_eq!(
            source_index.localized_attributes_rules(&source_rtxn).unwrap(),
            target_index.localized_attributes_rules(&target_rtxn).unwrap(),
            "Localized attributes mismatch"
        );
        // Tokenization
        assert_eq!(
            source_index.separator_tokens(&source_rtxn).unwrap(),
            target_index.separator_tokens(&target_rtxn).unwrap(),
            "Separator tokens mismatch"
        );
        assert_eq!(
            source_index.non_separator_tokens(&source_rtxn).unwrap(),
            target_index.non_separator_tokens(&target_rtxn).unwrap(),
            "Non-separator tokens mismatch"
        );
        assert_eq!(
            source_index.dictionary(&source_rtxn).unwrap(),
            target_index.dictionary(&target_rtxn).unwrap(),
            "Dictionary mismatch"
        );
        // Search Cutoff
        assert_eq!(
            source_index.search_cutoff(&source_rtxn).unwrap(),
            target_index.search_cutoff(&target_rtxn).unwrap(),
            "Search cutoff mismatch"
        );
        // Prefix Search
        assert_eq!(
            source_index.prefix_search(&source_rtxn).unwrap(),
            target_index.prefix_search(&target_rtxn).unwrap(),
            "Prefix search mismatch"
        );
        // Facet Search (if applicable)
        // assert_eq!(
        //     source_index.facet_search(&source_rtxn).unwrap(),
        //     target_index.facet_search(&target_rtxn).unwrap(),
        //     "Facet search mismatch"
        // );
    }
}

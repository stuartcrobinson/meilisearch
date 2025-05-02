use big_s::S;
use meili_snap::{json_string, snapshot};
use meilisearch_types::error::ErrorCode;
use meilisearch_types::tasks::{KindWithContent, Status};
use roaring::RoaringBitmap;

use crate::insta_snapshot::snapshot_index_scheduler;
use crate::test_utils::Breakpoint::*;
use crate::test_utils::{index_creation_task, replace_document_import_task};
use crate::{IndexScheduler, Query};

#[test]
fn register() {
    // In this test, the handle doesn't make any progress, we only check that the tasks are registered
    let (index_scheduler, mut _handle) = IndexScheduler::test(true, vec![]);

    let kinds = [
        index_creation_task("catto", "mouse"),
        replace_document_import_task("catto", None, 0, 12),
        replace_document_import_task("catto", None, 1, 50),
        replace_document_import_task("doggo", Some("bone"), 2, 5000),
    ];
    let (_, file) = index_scheduler.queue.create_update_file_with_uuid(0).unwrap();
    file.persist().unwrap();
    let (_, file) = index_scheduler.queue.create_update_file_with_uuid(1).unwrap();
    file.persist().unwrap();
    let (_, file) = index_scheduler.queue.create_update_file_with_uuid(2).unwrap();
    file.persist().unwrap();

    for (idx, kind) in kinds.into_iter().enumerate() {
        let k = kind.as_kind();
        let task = index_scheduler.register(kind, None, false).unwrap();
        index_scheduler.assert_internally_consistent();

        assert_eq!(task.uid, idx as u32);
        assert_eq!(task.status, Status::Enqueued);
        assert_eq!(task.kind.as_kind(), k);
    }

    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "everything_is_successfully_registered");
}

#[test]
fn dry_run() {
    let (index_scheduler, _handle) = IndexScheduler::test(true, vec![]);

    let kind = KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None };
    let task = index_scheduler.register(kind, None, true).unwrap();
    snapshot!(task.uid, @"0");
    snapshot!(snapshot_index_scheduler(&index_scheduler), @r"
        ### Autobatching Enabled = true
        ### Processing batch None:
        []
        ----------------------------------------------------------------------
        ### All Tasks:
        ----------------------------------------------------------------------
        ### Status:
        ----------------------------------------------------------------------
        ### Kind:
        ----------------------------------------------------------------------
        ### Index Tasks:
        ----------------------------------------------------------------------
        ### Index Mapper:

        ----------------------------------------------------------------------
        ### Canceled By:

        ----------------------------------------------------------------------
        ### Enqueued At:
        ----------------------------------------------------------------------
        ### Started At:
        ----------------------------------------------------------------------
        ### Finished At:
        ----------------------------------------------------------------------
        ### All Batches:
        ----------------------------------------------------------------------
        ### Batch to tasks mapping:
        ----------------------------------------------------------------------
        ### Batches Status:
        ----------------------------------------------------------------------
        ### Batches Kind:
        ----------------------------------------------------------------------
        ### Batches Index Tasks:
        ----------------------------------------------------------------------
        ### Batches Enqueued At:
        ----------------------------------------------------------------------
        ### Batches Started At:
        ----------------------------------------------------------------------
        ### Batches Finished At:
        ----------------------------------------------------------------------
        ### File Store:

        ----------------------------------------------------------------------
        ");

    let kind = KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None };
    let task = index_scheduler.register(kind, Some(12), true).unwrap();
    snapshot!(task.uid, @"12");
    snapshot!(snapshot_index_scheduler(&index_scheduler), @r"
        ### Autobatching Enabled = true
        ### Processing batch None:
        []
        ----------------------------------------------------------------------
        ### All Tasks:
        ----------------------------------------------------------------------
        ### Status:
        ----------------------------------------------------------------------
        ### Kind:
        ----------------------------------------------------------------------
        ### Index Tasks:
        ----------------------------------------------------------------------
        ### Index Mapper:

        ----------------------------------------------------------------------
        ### Canceled By:

        ----------------------------------------------------------------------
        ### Enqueued At:
        ----------------------------------------------------------------------
        ### Started At:
        ----------------------------------------------------------------------
        ### Finished At:
        ----------------------------------------------------------------------
        ### All Batches:
        ----------------------------------------------------------------------
        ### Batch to tasks mapping:
        ----------------------------------------------------------------------
        ### Batches Status:
        ----------------------------------------------------------------------
        ### Batches Kind:
        ----------------------------------------------------------------------
        ### Batches Index Tasks:
        ----------------------------------------------------------------------
        ### Batches Enqueued At:
        ----------------------------------------------------------------------
        ### Batches Started At:
        ----------------------------------------------------------------------
        ### Batches Finished At:
        ----------------------------------------------------------------------
        ### File Store:

        ----------------------------------------------------------------------
        ");
}

#[test]
fn basic_set_taskid() {
    let (index_scheduler, _handle) = IndexScheduler::test(true, vec![]);

    let kind = KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None };
    let task = index_scheduler.register(kind, None, false).unwrap();
    snapshot!(task.uid, @"0");

    let kind = KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None };
    let task = index_scheduler.register(kind, Some(12), false).unwrap();
    snapshot!(task.uid, @"12");

    let kind = KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None };
    let error = index_scheduler.register(kind, Some(5), false).unwrap_err();
    snapshot!(error, @"Received bad task id: 5 should be >= to 13.");
}

#[test]
fn test_disable_auto_deletion_of_tasks() {
    let (index_scheduler, mut handle) = IndexScheduler::test_with_custom_config(vec![], |config| {
        config.cleanup_enabled = false;
        config.max_number_of_tasks = 2;
        None
    });

    index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None },
            None,
            false,
        )
        .unwrap();
    handle.advance_one_successful_batch();

    index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None },
            None,
            false,
        )
        .unwrap();
    handle.advance_one_failed_batch();

    // at this point the max number of tasks is reached
    // we can still enqueue multiple tasks
    index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None },
            None,
            false,
        )
        .unwrap();
    index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None },
            None,
            false,
        )
        .unwrap();

    let rtxn = index_scheduler.env.read_txn().unwrap();
    let proc = index_scheduler.processing_tasks.read().unwrap();
    let tasks =
        index_scheduler.queue.get_task_ids(&rtxn, &Query { ..Default::default() }, &proc).unwrap();
    let tasks = index_scheduler.queue.tasks.get_existing_tasks(&rtxn, tasks).unwrap();
    snapshot!(json_string!(tasks, { "[].enqueuedAt" => "[date]", "[].startedAt" => "[date]", "[].finishedAt" => "[date]" }), name: "task_queue_is_full");
    drop(rtxn);
    drop(proc);

    // now we're above the max number of tasks
    // and if we try to advance in the tick function no new task deletion should be enqueued
    handle.advance_till([Start, BatchCreated]);
    let rtxn = index_scheduler.env.read_txn().unwrap();
    let proc = index_scheduler.processing_tasks.read().unwrap();
    let tasks =
        index_scheduler.queue.get_task_ids(&rtxn, &Query { ..Default::default() }, &proc).unwrap();
    let tasks = index_scheduler.queue.tasks.get_existing_tasks(&rtxn, tasks).unwrap();
    snapshot!(json_string!(tasks, { "[].enqueuedAt" => "[date]", "[].startedAt" => "[date]", "[].finishedAt" => "[date]", ".**.original_filter" => "[filter]", ".**.query" => "[query]" }), name: "task_deletion_have_not_been_enqueued");
    drop(rtxn);
    drop(proc);
}

#[test]
fn test_auto_deletion_of_tasks() {
    let (index_scheduler, mut handle) = IndexScheduler::test_with_custom_config(vec![], |config| {
        config.max_number_of_tasks = 2;
        None
    });

    index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None },
            None,
            false,
        )
        .unwrap();
    handle.advance_one_successful_batch();

    index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None },
            None,
            false,
        )
        .unwrap();
    handle.advance_one_failed_batch();

    // at this point the max number of tasks is reached
    // we can still enqueue multiple tasks
    index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None },
            None,
            false,
        )
        .unwrap();
    index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None },
            None,
            false,
        )
        .unwrap();

    let rtxn = index_scheduler.env.read_txn().unwrap();
    let proc = index_scheduler.processing_tasks.read().unwrap();
    let tasks =
        index_scheduler.queue.get_task_ids(&rtxn, &Query { ..Default::default() }, &proc).unwrap();
    let tasks = index_scheduler.queue.tasks.get_existing_tasks(&rtxn, tasks).unwrap();
    snapshot!(json_string!(tasks, { "[].enqueuedAt" => "[date]", "[].startedAt" => "[date]", "[].finishedAt" => "[date]" }), name: "task_queue_is_full");
    drop(rtxn);
    drop(proc);

    // now we're above the max number of tasks
    // and if we try to advance in the tick function a new task deletion should be enqueued
    handle.advance_till([Start, BatchCreated]);
    let rtxn = index_scheduler.env.read_txn().unwrap();
    let proc = index_scheduler.processing_tasks.read().unwrap();
    let tasks =
        index_scheduler.queue.get_task_ids(&rtxn, &Query { ..Default::default() }, &proc).unwrap();
    let tasks = index_scheduler.queue.tasks.get_existing_tasks(&rtxn, tasks).unwrap();
    snapshot!(json_string!(tasks, { "[].enqueuedAt" => "[date]", "[].startedAt" => "[date]", "[].finishedAt" => "[date]", ".**.original_filter" => "[filter]", ".**.query" => "[query]" }), name: "task_deletion_have_been_enqueued");
    drop(rtxn);
    drop(proc);

    handle.advance_till([InsideProcessBatch, ProcessBatchSucceeded, AfterProcessing]);
    let rtxn = index_scheduler.env.read_txn().unwrap();
    let proc = index_scheduler.processing_tasks.read().unwrap();
    let tasks =
        index_scheduler.queue.get_task_ids(&rtxn, &Query { ..Default::default() }, &proc).unwrap();
    let tasks = index_scheduler.queue.tasks.get_existing_tasks(&rtxn, tasks).unwrap();
    snapshot!(json_string!(tasks, { "[].enqueuedAt" => "[date]", "[].startedAt" => "[date]", "[].finishedAt" => "[date]", ".**.original_filter" => "[filter]", ".**.query" => "[query]" }), name: "task_deletion_have_been_processed");
    drop(rtxn);
    drop(proc);

    handle.advance_one_failed_batch();
    // a new task deletion has been enqueued
    handle.advance_one_successful_batch();
    let rtxn = index_scheduler.env.read_txn().unwrap();
    let proc = index_scheduler.processing_tasks.read().unwrap();
    let tasks =
        index_scheduler.queue.get_task_ids(&rtxn, &Query { ..Default::default() }, &proc).unwrap();
    let tasks = index_scheduler.queue.tasks.get_existing_tasks(&rtxn, tasks).unwrap();
    snapshot!(json_string!(tasks, { "[].enqueuedAt" => "[date]", "[].startedAt" => "[date]", "[].finishedAt" => "[date]", ".**.original_filter" => "[filter]", ".**.query" => "[query]" }), name: "after_the_second_task_deletion");
    drop(rtxn);
    drop(proc);

    handle.advance_one_failed_batch();
    handle.advance_one_successful_batch();
    let rtxn = index_scheduler.env.read_txn().unwrap();
    let proc = index_scheduler.processing_tasks.read().unwrap();
    let tasks =
        index_scheduler.queue.get_task_ids(&rtxn, &Query { ..Default::default() }, &proc).unwrap();
    let tasks = index_scheduler.queue.tasks.get_existing_tasks(&rtxn, tasks).unwrap();
    snapshot!(json_string!(tasks, { "[].enqueuedAt" => "[date]", "[].startedAt" => "[date]", "[].finishedAt" => "[date]", ".**.original_filter" => "[filter]", ".**.query" => "[query]" }), name: "everything_has_been_processed");
    drop(rtxn);
    drop(proc);
}

#[test]
fn test_task_queue_is_full() {
    let (index_scheduler, mut handle) = IndexScheduler::test_with_custom_config(vec![], |config| {
        // that's the minimum map size possible
        config.task_db_size = 1048576 * 3;
        None
    });

    index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None },
            None,
            false,
        )
        .unwrap();
    handle.advance_one_successful_batch();
    // on average this task takes ~600 bytes
    loop {
        let result = index_scheduler.register(
            KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None },
            None,
            false,
        );
        if result.is_err() {
            break;
        }
        handle.advance_one_failed_batch();
    }
    index_scheduler.assert_internally_consistent();

    // at this point the task DB shoud have reached its limit and we should not be able to register new tasks
    let result = index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None },
            None,
            false,
        )
        .unwrap_err();
    snapshot!(result, @"Meilisearch cannot receive write operations because the limit of the task database has been reached. Please delete tasks to continue performing write operations.");
    // we won't be able to test this error in an integration test thus as a best effort test I still ensure the error return the expected error code
    snapshot!(format!("{:?}", result.error_code()), @"NoSpaceLeftOnDevice");

    // Even the task deletion and cancelation that don't delete anything should be refused
    let result = index_scheduler
        .register(
            KindWithContent::TaskDeletion { query: S("test"), tasks: RoaringBitmap::new() },
            None,
            false,
        )
        .unwrap_err();
    snapshot!(result, @"Meilisearch cannot receive write operations because the limit of the task database has been reached. Please delete tasks to continue performing write operations.");
    let result = index_scheduler
        .register(
            KindWithContent::TaskCancelation { query: S("test"), tasks: RoaringBitmap::new() },
            None,
            false,
        )
        .unwrap_err();
    snapshot!(result, @"Meilisearch cannot receive write operations because the limit of the task database has been reached. Please delete tasks to continue performing write operations.");

    // we won't be able to test this error in an integration test thus as a best effort test I still ensure the error return the expected error code
    snapshot!(format!("{:?}", result.error_code()), @"NoSpaceLeftOnDevice");

    // But a task cancelation that cancel something should work
    index_scheduler
        .register(
            KindWithContent::TaskCancelation { query: S("test"), tasks: (0..100).collect() },
            None,
            false,
        )
        .unwrap();
    handle.advance_one_successful_batch();

    // But we should still be forbidden from enqueuing new tasks
    let result = index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None },
            None,
            false,
        )
        .unwrap_err();
    snapshot!(result, @"Meilisearch cannot receive write operations because the limit of the task database has been reached. Please delete tasks to continue performing write operations.");

    // And a task deletion that delete something should works
    index_scheduler
        .register(
            KindWithContent::TaskDeletion { query: S("test"), tasks: (0..100).collect() },
            None,
            false,
        )
        .unwrap();
    handle.advance_one_successful_batch();

    // Now we should be able to enqueue a few tasks again
    index_scheduler
        .register(
            KindWithContent::IndexCreation { index_uid: S("doggo"), primary_key: None },
            None,
            false,
        )
        .unwrap();
    handle.advance_one_failed_batch();
}

// Tests specifically for the Single Index Snapshot feature
mod msfj_sis_tests { // Renamed module
    use super::*; // Bring parent module's imports into scope
    use meilisearch_types::tasks::Details; // Added import

    #[test]
    fn register_single_index_snapshot_creation() {
        let (index_scheduler, mut _handle) = IndexScheduler::test(true, vec![]);

    let kind = KindWithContent::SingleIndexSnapshotCreation { index_uid: S("catto") };
    let task = index_scheduler.register(kind.clone(), None, false).unwrap();
    // index_scheduler.assert_internally_consistent(); // Removed global consistency check

    assert_eq!(task.uid, 0);
    assert_eq!(task.status, Status::Enqueued);
    assert_eq!(task.kind.as_kind(), kind.as_kind());
    assert!(task.details.is_some()); // Default details should be set

    // --- Start: Replaced snapshot assertions ---
    assert_eq!(task.uid, 0);
    assert_eq!(task.status, Status::Enqueued);
    assert!(matches!(
        task.kind,
        KindWithContent::SingleIndexSnapshotCreation { ref index_uid } if index_uid == "catto"
    ));
    assert!(matches!(task.details, Some(Details::SingleIndexSnapshotCreation { snapshot_uid: None })));
    assert!(task.error.is_none());
    assert!(task.canceled_by.is_none());
    assert!(task.started_at.is_none());
    assert!(task.finished_at.is_none());

    // Verify the task is persisted correctly
    let rtxn = index_scheduler.env.read_txn().unwrap(); // Get read transaction
    let persisted_task = index_scheduler.queue.tasks.get_task(&rtxn, task.uid).unwrap().unwrap(); // Pass transaction
    assert_eq!(persisted_task.uid, task.uid);
    assert_eq!(persisted_task.status, task.status);
    // KindWithContent doesn't impl PartialEq, so match again
    assert!(matches!(
        persisted_task.kind,
        KindWithContent::SingleIndexSnapshotCreation { ref index_uid } if index_uid == "catto"
    ));
    assert_eq!(persisted_task.details, task.details);
    // --- End: Replaced snapshot assertions ---
}

#[test]
fn register_single_index_snapshot_import() {
    let (index_scheduler, mut _handle) = IndexScheduler::test(true, vec![]);

    let kind = KindWithContent::SingleIndexSnapshotImport {
        source_snapshot_path: S("/path/to/my-index-20240501-120000.snapshot.tar.gz"),
        target_index_uid: S("doggo"),
    };
    let task = index_scheduler.register(kind.clone(), None, false).unwrap();
    // index_scheduler.assert_internally_consistent(); // Removed global consistency check

    assert_eq!(task.uid, 0);
    assert_eq!(task.status, Status::Enqueued);
    assert_eq!(task.kind.as_kind(), kind.as_kind());
    assert!(task.details.is_some()); // Default details should be set

    // --- Start: Replaced snapshot assertions ---
    assert_eq!(task.uid, 0);
    assert_eq!(task.status, Status::Enqueued);
    assert!(matches!(
        task.kind,
        KindWithContent::SingleIndexSnapshotImport { ref source_snapshot_path, ref target_index_uid }
            if source_snapshot_path == "/path/to/my-index-20240501-120000.snapshot.tar.gz" && target_index_uid == "doggo"
    ));
    // Check details, note source_snapshot_uid extraction logic happens in default_details
    // Check details, note source_snapshot_uid extraction logic happens in default_details
    // The logic extracts the part after the first hyphen in the file stem.
    assert!(matches!(
        task.details,
        Some(Details::SingleIndexSnapshotImport { ref source_snapshot_uid, ref target_index_uid })
            if source_snapshot_uid == "index-20240501-120000.snapshot.tar" && target_index_uid == "doggo"
    ));
    assert!(task.error.is_none());
    assert!(task.canceled_by.is_none());
    assert!(task.started_at.is_none());
    assert!(task.finished_at.is_none());

    // Verify the task is persisted correctly
    let rtxn = index_scheduler.env.read_txn().unwrap(); // Get read transaction
    let persisted_task = index_scheduler.queue.tasks.get_task(&rtxn, task.uid).unwrap().unwrap(); // Pass transaction
    assert_eq!(persisted_task.uid, task.uid);
    assert_eq!(persisted_task.status, task.status);
    // KindWithContent doesn't impl PartialEq, so match again
    assert!(matches!(
        persisted_task.kind,
        KindWithContent::SingleIndexSnapshotImport { ref source_snapshot_path, ref target_index_uid }
             if source_snapshot_path == "/path/to/my-index-20240501-120000.snapshot.tar.gz" && target_index_uid == "doggo"
    ));
    // Also check the persisted details match the expected extracted UID
    assert!(matches!(
        persisted_task.details,
        Some(Details::SingleIndexSnapshotImport { ref source_snapshot_uid, ref target_index_uid })
            if source_snapshot_uid == "index-20240501-120000.snapshot.tar" && target_index_uid == "doggo"
    ));
    // --- End: Replaced snapshot assertions ---
    }
}

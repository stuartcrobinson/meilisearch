use big_s::S;
use meili_snap::snapshot;
use meilisearch_types::tasks::{KindWithContent, Status};
use tempfile;

use crate::insta_snapshot::snapshot_index_scheduler;
use crate::test_utils::Breakpoint;
use crate::IndexScheduler;

#[test]
fn test_single_index_snapshot_creation() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
    
    // First create an index to snapshot
    index_scheduler
        .register(
            KindWithContent::IndexCreation { 
                index_uid: S("test-index"), 
                primary_key: Some(S("id")) 
            },
            None,
            false,
        )
        .unwrap();
    
    // Add some content to the index
    let content = r#"
        {
            "id": 1,
            "title": "Test document"
        }"#;
        
    let (uuid, mut file) = index_scheduler.queue.create_update_file_with_uuid(0).unwrap();
    let documents_count = crate::test_utils::read_json(content.as_bytes(), &mut file).unwrap();
    file.persist().unwrap();
    
    index_scheduler
        .register(
            KindWithContent::DocumentAdditionOrUpdate {
                index_uid: S("test-index"),
                primary_key: Some(S("id")),
                method: meilisearch_types::milli::update::IndexDocumentsMethod::ReplaceDocuments,
                content_file: uuid,
                documents_count,
                allow_index_creation: true,
            },
            None,
            false,
        )
        .unwrap();
        
    // Process index creation and document addition
    handle.advance_n_successful_batches(2);
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "after_index_setup");
    
    // Now register the snapshot creation task
    index_scheduler
        .register(
            KindWithContent::SingleIndexSnapshotCreation { 
                index_uid: S("test-index"),
                snapshot_path: S("test-index-snapshot.tar.gz")
            },
            None,
            false,
        )
        .unwrap();
    
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "after_snapshot_task_registration");
    
    // Process the snapshot task
    handle.advance_till([
        Breakpoint::Start,
        Breakpoint::BatchCreated,
        Breakpoint::ProcessBatchSucceeded,
        Breakpoint::AfterProcessing,
    ]);
    
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "after_snapshot_creation");
    
    // Verify the task has succeeded
    let rtxn = index_scheduler.env.read_txn().unwrap();
    let tasks = index_scheduler.get_tasks(&rtxn).unwrap();
    let snapshot_task = tasks.iter().find(|t| 
        matches!(t.kind, KindWithContent::SingleIndexSnapshotCreation { .. })
    ).unwrap();
    
    snapshot!(snapshot_task.status, @"Succeeded");
}

#[test]
fn test_single_index_snapshot_import() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
    
    // Create a source index, populate it, and snapshot it first
    index_scheduler
        .register(
            KindWithContent::IndexCreation { 
                index_uid: S("source-index"), 
                primary_key: Some(S("id")) 
            },
            None,
            false,
        )
        .unwrap();
    
    // Add content to source index
    let content = r#"
        {
            "id": 1,
            "title": "Test document"
        }"#;
        
    let (uuid, mut file) = index_scheduler.queue.create_update_file_with_uuid(0).unwrap();
    let documents_count = crate::test_utils::read_json(content.as_bytes(), &mut file).unwrap();
    file.persist().unwrap();
    
    index_scheduler
        .register(
            KindWithContent::DocumentAdditionOrUpdate {
                index_uid: S("source-index"),
                primary_key: Some(S("id")),
                method: meilisearch_types::milli::update::IndexDocumentsMethod::ReplaceDocuments,
                content_file: uuid,
                documents_count,
                allow_index_creation: false,
            },
            None,
            false,
        )
        .unwrap();
        
    // Process index creation and document addition
    handle.advance_n_successful_batches(2);
    
    // Create snapshot of source index
    index_scheduler
        .register(
            KindWithContent::SingleIndexSnapshotCreation { 
                index_uid: S("source-index"),
                snapshot_path: S("source-index-snapshot.tar.gz") 
            },
            None,
            false,
        )
        .unwrap();
    
    handle.advance_n_successful_batches(1);
    
    // Now import the snapshot to a new index
    index_scheduler
        .register(
            KindWithContent::SingleIndexSnapshotImport { 
                index_uid: S("temp-uid"),
                source_path: S("source-index-snapshot.tar.gz"),
                target_index_uid: Some(S("target-index"))
            },
            None,
            false,
        )
        .unwrap();
    
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "after_import_task_registration");
    
    // Process the import task
    handle.advance_till([
        Breakpoint::Start,
        Breakpoint::BatchCreated,
        Breakpoint::ProcessBatchSucceeded,
        Breakpoint::AfterProcessing,
    ]);
    
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "after_snapshot_import");
    
    // Verify both indexes exist and contain the same document
    let source_index = index_scheduler.index("source-index").unwrap();
    let target_index = index_scheduler.index("target-index").unwrap();
    
    let source_rtxn = source_index.read_txn().unwrap();
    let target_rtxn = target_index.read_txn().unwrap();
    
    let source_docs_count = source_index.number_of_documents(&source_rtxn).unwrap();
    let target_docs_count = target_index.number_of_documents(&target_rtxn).unwrap();
    
    snapshot!(format!("Source docs: {}, Target docs: {}", source_docs_count, target_docs_count), 
                @"Source docs: 1, Target docs: 1");
}

#[test]
fn test_snapshot_task_priorities() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
    
    // Create an index
    index_scheduler
        .register(
            KindWithContent::IndexCreation { 
                index_uid: S("test-index"), 
                primary_key: Some(S("id")) 
            },
            None,
            false,
        )
        .unwrap();
    
    handle.advance_one_successful_batch();
    
    // Register a document addition and snapshot task
    let content = r#"{"id": 1, "title": "Test document"}"#;
    let (uuid, mut file) = index_scheduler.queue.create_update_file_with_uuid(0).unwrap();
    let documents_count = crate::test_utils::read_json(content.as_bytes(), &mut file).unwrap();
    file.persist().unwrap();
    
    index_scheduler
        .register(
            KindWithContent::DocumentAdditionOrUpdate {
                index_uid: S("test-index"),
                primary_key: Some(S("id")),
                method: meilisearch_types::milli::update::IndexDocumentsMethod::ReplaceDocuments,
                content_file: uuid,
                documents_count,
                allow_index_creation: false,
            },
            None,
            false,
        )
        .unwrap();
    
    index_scheduler
        .register(
            KindWithContent::SingleIndexSnapshotCreation { 
                index_uid: S("test-index"),
                snapshot_path: S("test-index-snapshot.tar.gz")
            },
            None,
            false,
        )
        .unwrap();
    
    // Process one batch - snapshot should be processed first due to higher priority
    handle.advance_one_successful_batch();
    
    let rtxn = index_scheduler.env.read_txn().unwrap();
    let tasks = index_scheduler.get_tasks(&rtxn).unwrap();
    
    // Find snapshot and document tasks
    let snapshot_task = tasks.iter().find(|t| 
        matches!(t.kind, KindWithContent::SingleIndexSnapshotCreation { .. })
    ).unwrap();
    
    let doc_task = tasks.iter().find(|t| 
        matches!(t.kind, KindWithContent::DocumentAdditionOrUpdate { .. })
    ).unwrap();
    
    // Snapshot task should be completed, document task should still be enqueued
    snapshot!(snapshot_task.status, @"Succeeded");
    snapshot!(doc_task.status, @"Enqueued");
    
    // Process the remaining task
    handle.advance_one_successful_batch();
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "after_all_tasks_processed");
}

#[test]
fn test_snapshot_concurrent_operations() {
    let (index_scheduler, mut handle) = IndexScheduler::test(true, vec![]);
    
    // Create an index
    index_scheduler
        .register(
            KindWithContent::IndexCreation { 
                index_uid: S("test-index"), 
                primary_key: Some(S("id")) 
            },
            None,
            false,
        )
        .unwrap();
    
    handle.advance_one_successful_batch();
    
    // Start a snapshot task
    index_scheduler
        .register(
            KindWithContent::SingleIndexSnapshotCreation { 
                index_uid: S("test-index"),
                snapshot_path: S("test-index-snapshot.tar.gz")
            },
            None,
            false,
        )
        .unwrap();
    
    // Register document operations to the same index
    for i in 0..3 {
        let content = format!(r#"{{"id": {}, "title": "Test document {}"}}"#, i, i);
        let (uuid, mut file) = index_scheduler.queue.create_update_file_with_uuid(i).unwrap();
        let documents_count = crate::test_utils::read_json(content.as_bytes(), &mut file).unwrap();
        file.persist().unwrap();
        
        index_scheduler
            .register(
                KindWithContent::DocumentAdditionOrUpdate {
                    index_uid: S("test-index"),
                    primary_key: Some(S("id")),
                    method: meilisearch_types::milli::update::IndexDocumentsMethod::ReplaceDocuments,
                    content_file: uuid,
                    documents_count,
                    allow_index_creation: false,
                },
                None,
                false,
            )
            .unwrap();
    }
    
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "after_registering_concurrent_tasks");
    
    // Process all batches
    handle.advance_n_successful_batches(4);
    
    snapshot!(snapshot_index_scheduler(&index_scheduler), name: "after_processing_all_tasks");
    
    // Verify final state
    let index = index_scheduler.index("test-index").unwrap();
    let rtxn = index.read_txn().unwrap();
    let docs_count = index.number_of_documents(&rtxn).unwrap();
    
    snapshot!(format!("Final document count: {}", docs_count), @"Final document count: 3");
}
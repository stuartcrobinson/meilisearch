use meilisearch_types::tasks::{Details, Kind, KindWithContent, Status, Task};
use time::OffsetDateTime;

#[test]
fn test_single_index_snapshot_creation_kind() {
    let kind_content = KindWithContent::SingleIndexSnapshotCreation { index_uid: "test".to_string() };
    assert_eq!(kind_content.as_kind(), Kind::SingleIndexSnapshotCreation);
    assert_eq!(kind_content.indexes(), &["test"]);

    let details = kind_content.default_details();
    assert!(matches!(details, Some(Details::SingleIndexSnapshotCreation { snapshot_uid: None })));

    let finished_details = kind_content.default_finished_details();
    assert!(matches!(
        finished_details,
        Some(Details::SingleIndexSnapshotCreation { snapshot_uid: None })
    )); // Finished details are same as default for this type
}

#[test]
fn test_single_index_snapshot_import_kind() {
    let kind_content = KindWithContent::SingleIndexSnapshotImport {
        source_snapshot_path: "/snapshots/test-12345.snapshot.tar.gz".to_string(),
        target_index_uid: "imported_test".to_string(),
    };
    assert_eq!(kind_content.as_kind(), Kind::SingleIndexSnapshotImport);
    // Import task itself doesn't list an index in `indexes()`, but `index_uid()` points to the target.
    assert!(kind_content.indexes().is_empty());

    let details = kind_content.default_details();
    // default_details uses file_stem, which includes intermediate extensions
    match details {
        Some(Details::SingleIndexSnapshotImport { source_snapshot_uid, target_index_uid }) => {
            assert_eq!(source_snapshot_uid, "12345.snapshot.tar"); // Expect UID with extensions
            assert_eq!(target_index_uid, "imported_test");
        }
        _ => panic!("Expected Details::SingleIndexSnapshotImport"),
    }

    let finished_details = kind_content.default_finished_details();
    // default_finished_details uses file_stem, which includes intermediate extensions
    match finished_details {
        Some(Details::SingleIndexSnapshotImport { source_snapshot_uid, target_index_uid }) => {
            assert_eq!(source_snapshot_uid, "12345.snapshot.tar"); // Expect UID with extensions
            assert_eq!(target_index_uid, "imported_test");
        }
        _ => panic!("Expected Details::SingleIndexSnapshotImport for finished details"),
    }
}

#[test]
fn test_single_index_snapshot_import_kind_no_uid_in_path() {
    let kind_content = KindWithContent::SingleIndexSnapshotImport {
        source_snapshot_path: "/snapshots/test.snapshot.tar.gz".to_string(),
        target_index_uid: "imported_test".to_string(),
    };
    let details = kind_content.default_details();
    // When no UID is found after '-', the filename stem (with extensions) is used.
    match details {
        Some(Details::SingleIndexSnapshotImport { source_snapshot_uid, target_index_uid }) => {
            assert_eq!(source_snapshot_uid, "test.snapshot.tar"); // Expect stem with extensions
            assert_eq!(target_index_uid, "imported_test");
        }
        _ => panic!("Expected Details::SingleIndexSnapshotImport"),
    }
}

#[test]
fn test_task_serialization_deserialization_single_index_snapshot_creation() {
    let task = Task {
        uid: 1,
        batch_uid: None,
        enqueued_at: OffsetDateTime::now_utc(),
        started_at: None,
        finished_at: None,
        error: None,
        canceled_by: None,
        details: Some(Details::SingleIndexSnapshotCreation { snapshot_uid: Some("snap-123".to_string()) }),
        status: Status::Succeeded,
        kind: KindWithContent::SingleIndexSnapshotCreation { index_uid: "movies".to_string() },
    };

    let serialized = serde_json::to_string(&task).unwrap();
    let deserialized: Task = serde_json::from_str(&serialized).unwrap();

    assert_eq!(task, deserialized);
    assert_eq!(deserialized.kind.as_kind(), Kind::SingleIndexSnapshotCreation);
    assert_eq!(deserialized.index_uid(), Some("movies"));
    assert!(matches!(
        deserialized.details,
        Some(Details::SingleIndexSnapshotCreation { snapshot_uid: Some(ref uid) }) if uid == "snap-123"
    ));
}

#[test]
fn test_task_serialization_deserialization_single_index_snapshot_import() {
    let task = Task {
        uid: 2,
        batch_uid: None,
        enqueued_at: OffsetDateTime::now_utc(),
        started_at: Some(OffsetDateTime::now_utc()),
        finished_at: Some(OffsetDateTime::now_utc()),
        error: None,
        canceled_by: None,
        details: Some(Details::SingleIndexSnapshotImport {
            source_snapshot_uid: "snap-abc".to_string(),
            target_index_uid: "new_movies".to_string(),
        }),
        status: Status::Succeeded,
        kind: KindWithContent::SingleIndexSnapshotImport {
            source_snapshot_path: "/path/to/movies-snap-abc.snapshot.tar.gz".to_string(),
            target_index_uid: "new_movies".to_string(),
        },
    };

    let serialized = serde_json::to_string(&task).unwrap();
    let deserialized: Task = serde_json::from_str(&serialized).unwrap();

    assert_eq!(task, deserialized);
    assert_eq!(deserialized.kind.as_kind(), Kind::SingleIndexSnapshotImport);
    // The index_uid() for an import task should be the target index uid.
    assert_eq!(deserialized.index_uid(), Some("new_movies"));
    assert!(matches!(
        deserialized.details,
        Some(Details::SingleIndexSnapshotImport { ref source_snapshot_uid, ref target_index_uid })
            if source_snapshot_uid == "snap-abc" && target_index_uid == "new_movies"
    ));
}

// Example test for Kind enum parsing (add similar for Status if needed)
#[test]
fn test_kind_from_str() {
    assert_eq!(
        "singleIndexSnapshotCreation".parse::<Kind>().unwrap(),
        Kind::SingleIndexSnapshotCreation
    );
    assert_eq!(
        "singleIndexSnapshotImport".parse::<Kind>().unwrap(),
        Kind::SingleIndexSnapshotImport
    );
    assert!("invalidKind".parse::<Kind>().is_err());
}

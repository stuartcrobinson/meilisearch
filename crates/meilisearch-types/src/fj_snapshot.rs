use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(rename_all = "camelCase")]
pub struct FjSingleIndexSnapshotImportPayload {
    /// The filename of the snapshot to import (e.g., "my_index-snapshot_uid.snapshot.tar.gz").
    /// This file must be located in the configured snapshots directory.
    #[schema(example = "movies-20240101-120000.snapshot.tar.gz")]
    pub source_snapshot_filename: String,
    /// The desired unique identifier for the index after import.
    #[schema(example = "imported_movies")]
    pub target_index_uid: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_fj_single_index_snapshot_import_payload() {
        let json = r#"
        {
            "sourceSnapshotFilename": "my_snapshot.snapshot.tar.gz",
            "targetIndexUid": "new_index_uid"
        }
        "#;
        let payload: FjSingleIndexSnapshotImportPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.source_snapshot_filename, "my_snapshot.snapshot.tar.gz");
        assert_eq!(payload.target_index_uid, "new_index_uid");
    }

    #[test]
    fn test_deserialize_fj_single_index_snapshot_import_payload_camel_case() {
        // Ensure serde(rename_all = "camelCase") is working
        let json_camel_case = r#"
        {
            "sourceSnapshotFilename": "another_snapshot.tar.gz",
            "targetIndexUid": "target_uid_test"
        }
        "#;
        let payload: FjSingleIndexSnapshotImportPayload =
            serde_json::from_str(json_camel_case).unwrap();
        assert_eq!(payload.source_snapshot_filename, "another_snapshot.tar.gz");
        assert_eq!(payload.target_index_uid, "target_uid_test");
    }
}

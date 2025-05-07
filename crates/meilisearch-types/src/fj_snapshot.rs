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

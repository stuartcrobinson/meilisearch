use meilisearch_types::settings::{Settings, Unchecked};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Represents the metadata stored alongside the index data in a single-index snapshot.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotMetadata {
    /// The version of Meilisearch that created the snapshot.
    pub meilisearch_version: String,
    /// The primary key of the index.
    pub primary_key: Option<String>,
    /// All index settings.
    // TODO: Consider if we need a more specific settings structure later.
    pub settings: Settings<Unchecked>,
    /// The creation timestamp of the original index.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// The last update timestamp of the original index.
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

use core::fmt;
use std::collections::HashSet;
use std::fmt::{Display, Write};
use std::str::FromStr;

use enum_iterator::Sequence;
use milli::update::IndexDocumentsMethod;
use milli::Object;
use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize, Serializer};
use time::{Duration, OffsetDateTime};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::batches::BatchId;
use crate::error::ResponseError;
use crate::keys::Key;
use crate::settings::{Settings, Unchecked};
use crate::{versioning, InstanceUid};

pub type TaskId = u32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub uid: TaskId,
    pub batch_uid: Option<BatchId>,

    #[serde(with = "time::serde::rfc3339")]
    pub enqueued_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,

    pub error: Option<ResponseError>,
    pub canceled_by: Option<TaskId>,
    pub details: Option<Details>,

    pub status: Status,
    pub kind: KindWithContent,
}

impl Task {
    pub fn index_uid(&self) -> Option<&str> {
        use KindWithContent::*;

        match &self.kind {
            DumpCreation { .. }
            | SnapshotCreation
            | TaskCancelation { .. }
            | TaskDeletion { .. }
            | UpgradeDatabase { .. }
            | IndexSwap { .. } => None,
            DocumentAdditionOrUpdate { index_uid, .. }
            | DocumentEdition { index_uid, .. }
            | DocumentDeletion { index_uid, .. }
            | DocumentDeletionByFilter { index_uid, .. }
            | DocumentClear { index_uid }
            | SettingsUpdate { index_uid, .. }
            | IndexCreation { index_uid, .. }
            | IndexUpdate { index_uid, .. }
            | IndexDeletion { index_uid }
            | SingleIndexSnapshotCreation { index_uid, .. }
            | SingleIndexSnapshotImport { index_uid, .. } => Some(index_uid),
        }
    }

    /// Return the list of indexes updated by this tasks.
    pub fn indexes(&self) -> Vec<&str> {
        self.kind.indexes()
    }

    /// Return the content-uuid if there is one
    pub fn content_uuid(&self) -> Option<Uuid> {
        match self.kind {
            KindWithContent::DocumentAdditionOrUpdate { content_file, .. } => Some(content_file),
            KindWithContent::DocumentEdition { .. }
            | KindWithContent::DocumentDeletion { .. }
            | KindWithContent::DocumentDeletionByFilter { .. }
            | KindWithContent::DocumentClear { .. }
            | KindWithContent::SettingsUpdate { .. }
            | KindWithContent::IndexDeletion { .. }
            | KindWithContent::IndexCreation { .. }
            | KindWithContent::IndexUpdate { .. }
            | KindWithContent::IndexSwap { .. }
            | KindWithContent::TaskCancelation { .. }
            | KindWithContent::TaskDeletion { .. }
            | KindWithContent::DumpCreation { .. }
            | KindWithContent::SnapshotCreation
            | KindWithContent::SingleIndexSnapshotCreation { .. }
            | KindWithContent::SingleIndexSnapshotImport { .. }
            | KindWithContent::UpgradeDatabase { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KindWithContent {
    DocumentAdditionOrUpdate {
        index_uid: String,
        primary_key: Option<String>,
        method: IndexDocumentsMethod,
        content_file: Uuid,
        documents_count: u64,
        allow_index_creation: bool,
    },
    DocumentDeletion {
        index_uid: String,
        documents_ids: Vec<String>,
    },
    DocumentDeletionByFilter {
        index_uid: String,
        filter_expr: serde_json::Value,
    },
    DocumentEdition {
        index_uid: String,
        filter_expr: Option<serde_json::Value>,
        context: Option<milli::Object>,
        function: String,
    },
    DocumentClear {
        index_uid: String,
    },
    SettingsUpdate {
        index_uid: String,
        new_settings: Box<Settings<Unchecked>>,
        is_deletion: bool,
        allow_index_creation: bool,
    },
    IndexDeletion {
        index_uid: String,
    },
    IndexCreation {
        index_uid: String,
        primary_key: Option<String>,
    },
    IndexUpdate {
        index_uid: String,
        primary_key: Option<String>,
    },
    IndexSwap {
        swaps: Vec<IndexSwap>,
    },
    TaskCancelation {
        query: String,
        tasks: RoaringBitmap,
    },
    TaskDeletion {
        query: String,
        tasks: RoaringBitmap,
    },
    DumpCreation {
        keys: Vec<Key>,
        instance_uid: Option<InstanceUid>,
    },
    SnapshotCreation,
    SingleIndexSnapshotCreation {
        index_uid: String,
        snapshot_path: String,
    },
    SingleIndexSnapshotImport {
        index_uid: String,
        source_path: String,
        target_index_uid: Option<String>,
    },
    UpgradeDatabase {
        from: (u32, u32, u32),
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IndexSwap {
    pub indexes: (String, String),
}

impl KindWithContent {
    pub fn as_kind(&self) -> Kind {
        match self {
            KindWithContent::DocumentAdditionOrUpdate { .. } => Kind::DocumentAdditionOrUpdate,
            KindWithContent::DocumentEdition { .. } => Kind::DocumentEdition,
            KindWithContent::DocumentDeletion { .. } => Kind::DocumentDeletion,
            KindWithContent::DocumentDeletionByFilter { .. } => Kind::DocumentDeletion,
            KindWithContent::DocumentClear { .. } => Kind::DocumentDeletion,
            KindWithContent::SettingsUpdate { .. } => Kind::SettingsUpdate,
            KindWithContent::IndexCreation { .. } => Kind::IndexCreation,
            KindWithContent::IndexDeletion { .. } => Kind::IndexDeletion,
            KindWithContent::IndexUpdate { .. } => Kind::IndexUpdate,
            KindWithContent::IndexSwap { .. } => Kind::IndexSwap,
            KindWithContent::TaskCancelation { .. } => Kind::TaskCancelation,
            KindWithContent::TaskDeletion { .. } => Kind::TaskDeletion,
            KindWithContent::DumpCreation { .. } => Kind::DumpCreation,
            KindWithContent::SnapshotCreation => Kind::SnapshotCreation,
            KindWithContent::SingleIndexSnapshotCreation { .. } => Kind::SingleIndexSnapshotCreation,
            KindWithContent::SingleIndexSnapshotImport { .. } => Kind::SingleIndexSnapshotImport,
            KindWithContent::UpgradeDatabase { .. } => Kind::UpgradeDatabase,
        }
    }

    pub fn indexes(&self) -> Vec<&str> {
        use KindWithContent::*;

        match self {
            DumpCreation { .. }
            | SnapshotCreation
            | TaskCancelation { .. }
            | TaskDeletion { .. }
            | UpgradeDatabase { .. } => vec![],
            DocumentAdditionOrUpdate { index_uid, .. }
            | DocumentEdition { index_uid, .. }
            | DocumentDeletion { index_uid, .. }
            | DocumentDeletionByFilter { index_uid, .. }
            | DocumentClear { index_uid }
            | SettingsUpdate { index_uid, .. }
            | IndexCreation { index_uid, .. }
            | IndexUpdate { index_uid, .. }
            | IndexDeletion { index_uid }
            | SingleIndexSnapshotCreation { index_uid, .. } => vec![index_uid],
            SingleIndexSnapshotImport { index_uid, target_index_uid, .. } => {
                if let Some(target) = target_index_uid {
                    if target != index_uid {
                        vec![index_uid, target]
                    } else {
                        vec![index_uid]
                    }
                } else {
                    vec![index_uid]
                }
            },
            IndexSwap { swaps } => {
                let mut indexes = HashSet::<&str>::default();
                for swap in swaps {
                    indexes.insert(swap.indexes.0.as_str());
                    indexes.insert(swap.indexes.1.as_str());
                }
                indexes.into_iter().collect()
            }
        }
    }

    /// Returns the default `Details` that correspond to this `KindWithContent`,
    /// `None` if it cannot be generated.
    pub fn default_details(&self) -> Option<Details> {
        match self {
            KindWithContent::DocumentAdditionOrUpdate { documents_count, .. } => {
                Some(Details::DocumentAdditionOrUpdate {
                    received_documents: *documents_count,
                    indexed_documents: None,
                })
            }
            KindWithContent::DocumentEdition { index_uid: _, filter_expr, context, function } => {
                Some(Details::DocumentEdition {
                    deleted_documents: None,
                    edited_documents: None,
                    original_filter: filter_expr.as_ref().map(|v| v.to_string()),
                    context: context.clone(),
                    function: function.clone(),
                })
            }
            KindWithContent::DocumentDeletion { index_uid: _, documents_ids } => {
                Some(Details::DocumentDeletion {
                    provided_ids: documents_ids.len(),
                    deleted_documents: None,
                })
            }
            KindWithContent::DocumentDeletionByFilter { index_uid: _, filter_expr } => {
                Some(Details::DocumentDeletionByFilter {
                    original_filter: filter_expr.to_string(),
                    deleted_documents: None,
                })
            }
            KindWithContent::DocumentClear { .. } | KindWithContent::IndexDeletion { .. } => {
                Some(Details::ClearAll { deleted_documents: None })
            }
            KindWithContent::SettingsUpdate { new_settings, .. } => {
                Some(Details::SettingsUpdate { settings: new_settings.clone() })
            }
            KindWithContent::IndexCreation { primary_key, .. }
            | KindWithContent::IndexUpdate { primary_key, .. } => {
                Some(Details::IndexInfo { primary_key: primary_key.clone() })
            }
            KindWithContent::IndexSwap { swaps } => {
                Some(Details::IndexSwap { swaps: swaps.clone() })
            }
            KindWithContent::TaskCancelation { query, tasks } => Some(Details::TaskCancelation {
                matched_tasks: tasks.len(),
                canceled_tasks: None,
                original_filter: query.clone(),
            }),
            KindWithContent::TaskDeletion { query, tasks } => Some(Details::TaskDeletion {
                matched_tasks: tasks.len(),
                deleted_tasks: None,
                original_filter: query.clone(),
            }),
            KindWithContent::DumpCreation { .. } => Some(Details::Dump { dump_uid: None }),
            KindWithContent::SnapshotCreation => None,
            KindWithContent::SingleIndexSnapshotCreation { index_uid, snapshot_path } => {
                Some(Details::SingleIndexSnapshotCreation {
                    index_uid: index_uid.clone(),
                    snapshot_path: snapshot_path.clone(),
                })
            },
            KindWithContent::SingleIndexSnapshotImport { source_path, index_uid, .. } => {
                Some(Details::SingleIndexSnapshotImport {
                    source_path: source_path.clone(),
                    index_uid: index_uid.clone(),
                    imported_documents: None, // Will be populated when import completes
                })
            },
            KindWithContent::UpgradeDatabase { from } => Some(Details::UpgradeDatabase {
                from: (from.0, from.1, from.2),
                to: (
                    versioning::VERSION_MAJOR.parse().unwrap(),
                    versioning::VERSION_MINOR.parse().unwrap(),
                    versioning::VERSION_PATCH.parse().unwrap(),
                ),
            }),
        }
    }

    pub fn default_finished_details(&self) -> Option<Details> {
        match self {
            KindWithContent::DocumentAdditionOrUpdate { documents_count, .. } => {
                Some(Details::DocumentAdditionOrUpdate {
                    received_documents: *documents_count,
                    indexed_documents: Some(0),
                })
            }
            KindWithContent::DocumentEdition { index_uid: _, filter_expr, context, function } => {
                Some(Details::DocumentEdition {
                    deleted_documents: Some(0),
                    edited_documents: Some(0),
                    original_filter: filter_expr.as_ref().map(|v| v.to_string()),
                    context: context.clone(),
                    function: function.clone(),
                })
            }
            KindWithContent::DocumentDeletion { index_uid: _, documents_ids } => {
                Some(Details::DocumentDeletion {
                    provided_ids: documents_ids.len(),
                    deleted_documents: Some(0),
                })
            }
            KindWithContent::DocumentDeletionByFilter { index_uid: _, filter_expr } => {
                Some(Details::DocumentDeletionByFilter {
                    original_filter: filter_expr.to_string(),
                    deleted_documents: Some(0),
                })
            }
            KindWithContent::DocumentClear { .. } => {
                Some(Details::ClearAll { deleted_documents: None })
            }
            KindWithContent::SettingsUpdate { new_settings, .. } => {
                Some(Details::SettingsUpdate { settings: new_settings.clone() })
            }
            KindWithContent::IndexDeletion { .. } => None,
            KindWithContent::IndexCreation { primary_key, .. }
            | KindWithContent::IndexUpdate { primary_key, .. } => {
                Some(Details::IndexInfo { primary_key: primary_key.clone() })
            }
            KindWithContent::IndexSwap { .. } => {
                todo!()
            }
            KindWithContent::TaskCancelation { query, tasks } => Some(Details::TaskCancelation {
                matched_tasks: tasks.len(),
                canceled_tasks: Some(0),
                original_filter: query.clone(),
            }),
            KindWithContent::TaskDeletion { query, tasks } => Some(Details::TaskDeletion {
                matched_tasks: tasks.len(),
                deleted_tasks: Some(0),
                original_filter: query.clone(),
            }),
            KindWithContent::DumpCreation { .. } => Some(Details::Dump { dump_uid: None }),
            KindWithContent::SnapshotCreation => None,
            KindWithContent::SingleIndexSnapshotCreation { index_uid, snapshot_path } => {
                Some(Details::SingleIndexSnapshotCreation {
                    index_uid: index_uid.clone(),
                    snapshot_path: snapshot_path.clone(), // Use the provided path
                })
            },
            KindWithContent::SingleIndexSnapshotImport { source_path, index_uid, .. } => {
                Some(Details::SingleIndexSnapshotImport {
                    source_path: source_path.clone(),
                    index_uid: index_uid.clone(),
                    imported_documents: Some(0), // Default to 0 documents imported
                })
            },
            KindWithContent::UpgradeDatabase { from } => Some(Details::UpgradeDatabase {
                from: *from,
                to: (
                    versioning::VERSION_MAJOR.parse().unwrap(),
                    versioning::VERSION_MINOR.parse().unwrap(),
                    versioning::VERSION_PATCH.parse().unwrap(),
                ),
            }),
        }
    }
}

impl From<&KindWithContent> for Option<Details> {
    fn from(kind: &KindWithContent) -> Self {
        match kind {
            KindWithContent::DocumentAdditionOrUpdate { documents_count, .. } => {
                Some(Details::DocumentAdditionOrUpdate {
                    received_documents: *documents_count,
                    indexed_documents: None,
                })
            }
            KindWithContent::DocumentEdition { .. } => None,
            KindWithContent::DocumentDeletion { .. } => None,
            KindWithContent::DocumentDeletionByFilter { .. } => None,
            KindWithContent::DocumentClear { .. } => None,
            KindWithContent::SettingsUpdate { new_settings, .. } => {
                Some(Details::SettingsUpdate { settings: new_settings.clone() })
            }
            KindWithContent::IndexDeletion { .. } => None,
            KindWithContent::IndexCreation { primary_key, .. } => {
                Some(Details::IndexInfo { primary_key: primary_key.clone() })
            }
            KindWithContent::IndexUpdate { primary_key, .. } => {
                Some(Details::IndexInfo { primary_key: primary_key.clone() })
            }
            KindWithContent::IndexSwap { .. } => None,
            KindWithContent::TaskCancelation { query, tasks } => Some(Details::TaskCancelation {
                matched_tasks: tasks.len(),
                canceled_tasks: None,
                original_filter: query.clone(),
            }),
            KindWithContent::TaskDeletion { query, tasks } => Some(Details::TaskDeletion {
                matched_tasks: tasks.len(),
                deleted_tasks: None,
                original_filter: query.clone(),
            }),
            KindWithContent::DumpCreation { .. } => Some(Details::Dump { dump_uid: None }),
            KindWithContent::SnapshotCreation => None,
            KindWithContent::SingleIndexSnapshotCreation { index_uid, snapshot_path } => {
                Some(Details::SingleIndexSnapshotCreation {
                    index_uid: index_uid.clone(),
                    snapshot_path: snapshot_path.clone(),
                })
            },
            KindWithContent::SingleIndexSnapshotImport { source_path, index_uid, .. } => {
                Some(Details::SingleIndexSnapshotImport {
                    source_path: source_path.clone(),
                    index_uid: index_uid.clone(),
                    imported_documents: None,
                })
            },
            KindWithContent::UpgradeDatabase { from } => Some(Details::UpgradeDatabase {
                from: *from,
                to: (
                    versioning::VERSION_MAJOR.parse().unwrap(),
                    versioning::VERSION_MINOR.parse().unwrap(),
                    versioning::VERSION_PATCH.parse().unwrap(),
                ),
            }),
        }
    }
}

/// The status of a task.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Sequence,
    PartialOrd,
    Ord,
    ToSchema,
)]
#[schema(example = json!(Status::Processing))]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Enqueued,
    Processing,
    Succeeded,
    Failed,
    Canceled,
}

impl Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Status::Enqueued => write!(f, "enqueued"),
            Status::Processing => write!(f, "processing"),
            Status::Succeeded => write!(f, "succeeded"),
            Status::Failed => write!(f, "failed"),
            Status::Canceled => write!(f, "canceled"),
        }
    }
}

impl FromStr for Status {
    type Err = ParseTaskStatusError;

    fn from_str(status: &str) -> Result<Self, Self::Err> {
        if status.eq_ignore_ascii_case("enqueued") {
            Ok(Status::Enqueued)
        } else if status.eq_ignore_ascii_case("processing") {
            Ok(Status::Processing)
        } else if status.eq_ignore_ascii_case("succeeded") {
            Ok(Status::Succeeded)
        } else if status.eq_ignore_ascii_case("failed") {
            Ok(Status::Failed)
        } else if status.eq_ignore_ascii_case("canceled") {
            Ok(Status::Canceled)
        } else {
            Err(ParseTaskStatusError(status.to_owned()))
        }
    }
}

#[derive(Debug)]
pub struct ParseTaskStatusError(pub String);
impl fmt::Display for ParseTaskStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a valid task status. Available statuses are {}.",
            self.0,
            enum_iterator::all::<Status>()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<String>>()
                .join(", ")
        )
    }
}
impl std::error::Error for ParseTaskStatusError {}

/// The type of the task.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Sequence,
    PartialOrd,
    Ord,
    ToSchema,
)]
#[serde(rename_all = "camelCase")]
#[schema(rename_all = "camelCase", example = json!(enum_iterator::all::<Kind>().collect::<Vec<_>>()))]
pub enum Kind {
    DocumentAdditionOrUpdate,
    DocumentEdition,
    DocumentDeletion,
    SettingsUpdate,
    IndexCreation,
    IndexDeletion,
    IndexUpdate,
    IndexSwap,
    TaskCancelation,
    TaskDeletion,
    DumpCreation,
    SnapshotCreation,
    /// Create a snapshot of a single index
    SingleIndexSnapshotCreation,
    /// Import a snapshot into a single index
    SingleIndexSnapshotImport,
    UpgradeDatabase,
}

impl Kind {
    pub fn related_to_one_index(&self) -> bool {
        match self {
            Kind::DocumentAdditionOrUpdate
            | Kind::DocumentEdition
            | Kind::DocumentDeletion
            | Kind::SettingsUpdate
            | Kind::IndexCreation
            | Kind::IndexDeletion
            | Kind::IndexUpdate
            | Kind::SingleIndexSnapshotCreation
            | Kind::SingleIndexSnapshotImport => true,
            Kind::IndexSwap
            | Kind::TaskCancelation
            | Kind::TaskDeletion
            | Kind::DumpCreation
            | Kind::UpgradeDatabase
            | Kind::SnapshotCreation => false,
        }
    }
}
impl Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Kind::DocumentAdditionOrUpdate => write!(f, "documentAdditionOrUpdate"),
            Kind::DocumentEdition => write!(f, "documentEdition"),
            Kind::DocumentDeletion => write!(f, "documentDeletion"),
            Kind::SettingsUpdate => write!(f, "settingsUpdate"),
            Kind::IndexCreation => write!(f, "indexCreation"),
            Kind::IndexDeletion => write!(f, "indexDeletion"),
            Kind::IndexUpdate => write!(f, "indexUpdate"),
            Kind::IndexSwap => write!(f, "indexSwap"),
            Kind::TaskCancelation => write!(f, "taskCancelation"),
            Kind::TaskDeletion => write!(f, "taskDeletion"),
            Kind::DumpCreation => write!(f, "dumpCreation"),
            Kind::SnapshotCreation => write!(f, "snapshotCreation"),
            Kind::SingleIndexSnapshotCreation => write!(f, "singleIndexSnapshotCreation"),
            Kind::SingleIndexSnapshotImport => write!(f, "singleIndexSnapshotImport"),
            Kind::UpgradeDatabase => write!(f, "upgradeDatabase"),
        }
    }
}
impl FromStr for Kind {
    type Err = ParseTaskKindError;

    fn from_str(kind: &str) -> Result<Self, Self::Err> {
        if kind.eq_ignore_ascii_case("indexCreation") {
            Ok(Kind::IndexCreation)
        } else if kind.eq_ignore_ascii_case("indexUpdate") {
            Ok(Kind::IndexUpdate)
        } else if kind.eq_ignore_ascii_case("indexSwap") {
            Ok(Kind::IndexSwap)
        } else if kind.eq_ignore_ascii_case("indexDeletion") {
            Ok(Kind::IndexDeletion)
        } else if kind.eq_ignore_ascii_case("documentAdditionOrUpdate") {
            Ok(Kind::DocumentAdditionOrUpdate)
        } else if kind.eq_ignore_ascii_case("documentEdition") {
            Ok(Kind::DocumentEdition)
        } else if kind.eq_ignore_ascii_case("documentDeletion") {
            Ok(Kind::DocumentDeletion)
        } else if kind.eq_ignore_ascii_case("settingsUpdate") {
            Ok(Kind::SettingsUpdate)
        } else if kind.eq_ignore_ascii_case("taskCancelation") {
            Ok(Kind::TaskCancelation)
        } else if kind.eq_ignore_ascii_case("taskDeletion") {
            Ok(Kind::TaskDeletion)
        } else if kind.eq_ignore_ascii_case("dumpCreation") {
            Ok(Kind::DumpCreation)
        } else if kind.eq_ignore_ascii_case("snapshotCreation") {
            Ok(Kind::SnapshotCreation)
        } else if kind.eq_ignore_ascii_case("singleIndexSnapshotCreation") {
            Ok(Kind::SingleIndexSnapshotCreation)
        } else if kind.eq_ignore_ascii_case("singleIndexSnapshotImport") {
            Ok(Kind::SingleIndexSnapshotImport)
        } else if kind.eq_ignore_ascii_case("upgradeDatabase") {
            Ok(Kind::UpgradeDatabase)
        } else {
            Err(ParseTaskKindError(kind.to_owned()))
        }
    }
}

#[derive(Debug)]
pub struct ParseTaskKindError(pub String);
impl fmt::Display for ParseTaskKindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a valid task type. Available types are {}.",
            self.0,
            enum_iterator::all::<Kind>()
                .map(|k| format!(
                    "`{}`",
                    // by default serde is going to insert `"` around the value.
                    serde_json::to_string(&k).unwrap().trim_matches('"')
                ))
                .collect::<Vec<String>>()
                .join(", ")
        )
    }
}
impl std::error::Error for ParseTaskKindError {}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum Details {
    DocumentAdditionOrUpdate {
        received_documents: u64,
        indexed_documents: Option<u64>,
    },
    SettingsUpdate {
        settings: Box<Settings<Unchecked>>,
    },
    IndexInfo {
        primary_key: Option<String>,
    },
    DocumentDeletion {
        provided_ids: usize,
        deleted_documents: Option<u64>,
    },
    DocumentDeletionByFilter {
        original_filter: String,
        deleted_documents: Option<u64>,
    },
    DocumentEdition {
        deleted_documents: Option<u64>,
        edited_documents: Option<u64>,
        original_filter: Option<String>,
        context: Option<Object>,
        function: String,
    },
    ClearAll {
        deleted_documents: Option<u64>,
    },
    TaskCancelation {
        matched_tasks: u64,
        canceled_tasks: Option<u64>,
        original_filter: String,
    },
    TaskDeletion {
        matched_tasks: u64,
        deleted_tasks: Option<u64>,
        original_filter: String,
    },
    Dump {
        dump_uid: Option<String>,
    },
    IndexSwap {
        swaps: Vec<IndexSwap>,
    },
    UpgradeDatabase {
        from: (u32, u32, u32),
        to: (u32, u32, u32),
    },
    SingleIndexSnapshotCreation {
        /// The unique identifier of the index being snapshotted
        #[serde(rename = "indexUid")]
        index_uid: String,
        /// The path where the snapshot was stored 
        #[serde(rename = "snapshotPath")]
        snapshot_path: String,
    },
    SingleIndexSnapshotImport {
        /// The path of the snapshot file to import
        #[serde(rename = "sourcePath")]
        source_path: String,
        /// The unique identifier for the index to create from the snapshot
        #[serde(rename = "indexUid")]
        index_uid: String,
        /// Number of documents that were successfully imported (populated when task completes)
        #[serde(rename = "importedDocuments")]
        imported_documents: Option<u64>,
    },
}

impl Details {
    pub fn to_failed(&self) -> Self {
        let mut details = self.clone();
        match &mut details {
            Self::DocumentAdditionOrUpdate { indexed_documents, .. } => {
                *indexed_documents = Some(0)
            }
            Self::DocumentEdition { edited_documents, .. } => *edited_documents = Some(0),
            Self::DocumentDeletion { deleted_documents, .. } => *deleted_documents = Some(0),
            Self::DocumentDeletionByFilter { deleted_documents, .. } => {
                *deleted_documents = Some(0)
            }
            Self::ClearAll { deleted_documents } => *deleted_documents = Some(0),
            Self::TaskCancelation { canceled_tasks, .. } => *canceled_tasks = Some(0),
            Self::TaskDeletion { deleted_tasks, .. } => *deleted_tasks = Some(0),
            Self::SingleIndexSnapshotImport { imported_documents, .. } => {
                *imported_documents = Some(0)
            },
            Self::SettingsUpdate { .. }
            | Self::IndexInfo { .. }
            | Self::Dump { .. }
            | Self::UpgradeDatabase { .. }
            | Self::IndexSwap { .. }
            | Self::SingleIndexSnapshotCreation { .. } => (),
        }

        details
    }
}

/// Serialize a `time::Duration` as a best effort ISO 8601 while waiting for
/// https://github.com/time-rs/time/issues/378.
/// This code is a port of the old code of time that was removed in 0.2.
pub fn serialize_duration<S: Serializer>(
    duration: &Option<Duration>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match duration {
        Some(duration) => {
            // technically speaking, negative duration is not valid ISO 8601
            if duration.is_negative() {
                return serializer.serialize_none();
            }

            const SECS_PER_DAY: i64 = Duration::DAY.whole_seconds();
            let secs = duration.whole_seconds();
            let days = secs / SECS_PER_DAY;
            let secs = secs - days * SECS_PER_DAY;
            let hasdate = days != 0;
            let nanos = duration.subsec_nanoseconds();
            let hastime = (secs != 0 || nanos != 0) || !hasdate;

            // all the following unwrap can't fail
            let mut res = String::new();
            write!(&mut res, "P").unwrap();

            if hasdate {
                write!(&mut res, "{}D", days).unwrap();
            }

            const NANOS_PER_MILLI: i32 = Duration::MILLISECOND.subsec_nanoseconds();
            const NANOS_PER_MICRO: i32 = Duration::MICROSECOND.subsec_nanoseconds();

            if hastime {
                if nanos == 0 {
                    write!(&mut res, "T{}S", secs).unwrap();
                } else if nanos % NANOS_PER_MILLI == 0 {
                    write!(&mut res, "T{}.{:03}S", secs, nanos / NANOS_PER_MILLI).unwrap();
                } else if nanos % NANOS_PER_MICRO == 0 {
                    write!(&mut res, "T{}.{:06}S", secs, nanos / NANOS_PER_MICRO).unwrap();
                } else {
                    write!(&mut res, "T{}.{:09}S", secs, nanos).unwrap();
                }
            }

            serializer.serialize_str(&res)
        }
        None => serializer.serialize_none(),
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{Details, Kind, KindWithContent};
    use crate::heed::types::SerdeJson;
    use crate::heed::{BytesDecode, BytesEncode};

    #[test]
    fn bad_deser() {
        let details = Details::TaskDeletion {
            matched_tasks: 1,
            deleted_tasks: None,
            original_filter: "hello".to_owned(),
        };
        let serialised = SerdeJson::<Details>::bytes_encode(&details).unwrap();
        let deserialised = SerdeJson::<Details>::bytes_decode(&serialised).unwrap();
        meili_snap::snapshot!(format!("{:?}", details), @r###"TaskDeletion { matched_tasks: 1, deleted_tasks: None, original_filter: "hello" }"###);
        meili_snap::snapshot!(format!("{:?}", deserialised), @r###"TaskDeletion { matched_tasks: 1, deleted_tasks: None, original_filter: "hello" }"###);
    }

    #[test]
    fn all_kind_can_be_from_str() {
        for kind in enum_iterator::all::<Kind>() {
            let s = kind.to_string();
            let k = Kind::from_str(&s).map_err(|e| format!("Could not from_str {s}: {e}")).unwrap();
            assert_eq!(kind, k, "{kind}.to_string() returned {s} which was parsed as {k}");
        }
    }

    mod single_index_snapshot_tests {
        use super::*;

        // Helper function for creating test instances
        fn create_snapshot_import(target: Option<&str>) -> KindWithContent {
            KindWithContent::SingleIndexSnapshotImport { 
                index_uid: "test-index".to_string(),
                source_path: "/path/to/snapshot.idx.snapshot".to_string(),
                target_index_uid: target.map(String::from)
            }
        }

        #[test]
        fn test_kind_basic_functionality() {
            // Test string conversion and classification for Kind variants
            let test_cases = [
                (Kind::SingleIndexSnapshotCreation, "singleIndexSnapshotCreation"),
                (Kind::SingleIndexSnapshotImport, "singleIndexSnapshotImport"),
            ];
            
            for (kind, expected_str) in test_cases {
                // Test display formatting and parsing
                assert_eq!(kind.to_string(), expected_str);
                assert_eq!(Kind::from_str(expected_str).unwrap(), kind);
                
                // Test classification - critical for task scheduling
                assert!(kind.related_to_one_index(), 
                    "Single index snapshot tasks should be classified as relating to one index");
            }
        }

        #[test]
        fn test_affected_indexes() {
            // Test which indexes are affected by each task type - critical for task scheduling
            
            // Creation task affects only the specified index
            let creation = KindWithContent::SingleIndexSnapshotCreation { 
                index_uid: "test-index".to_string(),
                snapshot_path: "/path/to/snapshot.tar.gz".to_string()
            };
            assert_eq!(creation.indexes(), vec!["test-index"]);
            
            // Import task without target affects only the specified index
            let import = create_snapshot_import(None);
            assert_eq!(import.indexes(), vec!["test-index"]);
            
            // Import task with target affects both source and target indexes
            let import_with_target = create_snapshot_import(Some("new-index"));
            let affected_indexes = import_with_target.indexes();
            assert_eq!(affected_indexes.len(), 2);
            assert!(affected_indexes.contains(&"test-index"));
            assert!(affected_indexes.contains(&"new-index"));
        }

        #[test]
        fn test_serialization_and_deserialization() {
            // Test JSON serialization/deserialization - critical for API communication
            
            // Create test instances
            let creation = KindWithContent::SingleIndexSnapshotCreation { 
                index_uid: "test-index".to_string(),
                snapshot_path: "/path/to/snapshot.tar.gz".to_string()
            };
            
            // Import with target
            let import_with_target = create_snapshot_import(Some("new-index"));
            
            // Import without target
            let import_no_target = create_snapshot_import(None);
            
            // Test round-trip serialization for creation task
            let json = serde_json::to_string(&creation).unwrap();
            let deserialized: KindWithContent = serde_json::from_str(&json).unwrap();
            match deserialized {
                KindWithContent::SingleIndexSnapshotCreation { index_uid, .. } => {
                    assert_eq!(index_uid, "test-index");
                }
                _ => panic!("Deserialized to wrong task variant")
            }
            
            // Test round-trip serialization for import task with target
            let json = serde_json::to_string(&import_with_target).unwrap();
            let deserialized: KindWithContent = serde_json::from_str(&json).unwrap();
            match deserialized {
                KindWithContent::SingleIndexSnapshotImport { index_uid, source_path, target_index_uid } => {
                    assert_eq!(index_uid, "test-index");
                    assert_eq!(source_path, "/path/to/snapshot.idx.snapshot");
                    assert_eq!(target_index_uid, Some("new-index".to_string()));
                }
                _ => panic!("Deserialized to wrong task variant")
            }
            
            // Test round-trip serialization for import task without target
            let json = serde_json::to_string(&import_no_target).unwrap();
            let deserialized: KindWithContent = serde_json::from_str(&json).unwrap();
            match deserialized {
                KindWithContent::SingleIndexSnapshotImport { index_uid, source_path, target_index_uid } => {
                    assert_eq!(index_uid, "test-index");
                    assert_eq!(source_path, "/path/to/snapshot.idx.snapshot");
                    assert_eq!(target_index_uid, None);
                }
                _ => panic!("Deserialized to wrong task variant")
            }
        }
    }

    #[cfg(test)]
    mod single_index_snapshot_task_details_test {
        use super::*;
        use serde_json::json;

        #[test]
        fn test_single_index_snapshot_details_serialization() {
            // Define test cases with (details_instance, expected_json_value)
            let test_cases = vec![
                // Creation task with required snapshot path
                (
                    Details::SingleIndexSnapshotCreation {
                        index_uid: "products".to_string(),
                        snapshot_path: "/path/to/snapshot.tar.gz".to_string(),
                    },
                    json!({"indexUid": "products", "snapshotPath": "/path/to/snapshot.tar.gz"}),
                ),
                // Import task without document count (still in progress)
                (
                    Details::SingleIndexSnapshotImport {
                        source_path: "/path/to/source.tar.gz".to_string(),
                        index_uid: "products".to_string(),
                        imported_documents: None,
                    },
                    json!({"sourcePath": "/path/to/source.tar.gz", "indexUid": "products", "importedDocuments": null}),
                ),
                // Import task with document count (completed)
                (
                    Details::SingleIndexSnapshotImport {
                        source_path: "/path/to/source.tar.gz".to_string(),
                        index_uid: "products".to_string(),
                        imported_documents: Some(1000),
                    },
                    json!({"sourcePath": "/path/to/source.tar.gz", "indexUid": "products", "importedDocuments": 1000}),
                ),
            ];

            // Test both serialization and deserialization
            for (details, expected_json) in test_cases {
                // Test serialization produces expected JSON
                let json_str = serde_json::to_string(&details).unwrap();
                assert_eq!(serde_json::from_str::<serde_json::Value>(&json_str).unwrap(), expected_json);
                
                // Test round-trip deserialization preserves data
                let round_trip: Details = serde_json::from_str(&json_str).unwrap();
                assert_eq!(serde_json::to_string(&round_trip).unwrap(), json_str);
            }
        }

        #[test]
        fn test_task_methods_for_new_task_types() {
            // ---- Test KindWithContent to Details conversion methods ----
            
            // Setup test task instances
            let creation_task = KindWithContent::SingleIndexSnapshotCreation { 
                index_uid: "products".to_string(),
                snapshot_path: "/path/to/snapshot.tar.gz".to_string(),
            };
            
            let import_task = KindWithContent::SingleIndexSnapshotImport { 
                source_path: "/path/to/source.tar.gz".to_string(),
                index_uid: "products".to_string(),
                target_index_uid: None
            };

            // Test default_details() generates correct initial state
            assert_snapshot_creation_details(
                creation_task.default_details().unwrap(),
                "products", 
                "/path/to/snapshot.tar.gz"
            );
            
            assert_snapshot_import_details(
                import_task.default_details().unwrap(),
                "/path/to/source.tar.gz",
                "products", 
                None
            );

            // Test default_finished_details() preserves required paths
            assert_snapshot_creation_details(
                creation_task.default_finished_details().unwrap(),
                "products", 
                "/path/to/snapshot.tar.gz"
            );
            
            // Test default_finished_details() sets success markers
            assert_snapshot_import_details(
                import_task.default_finished_details().unwrap(),
                "/path/to/source.tar.gz",
                "products", 
                Some(0) // Should initialize count to 0 for finished state
            );
            
            // Test From<&KindWithContent> for Option<Details>
            let option_details: Option<Details> = (&creation_task).into();
            assert_snapshot_creation_details(
                option_details.unwrap(),
                "products", 
                "/path/to/snapshot.tar.gz"
            );
        }
        
        #[test]
        fn test_to_failed_for_new_details_types() {
            // Test to_failed() produces appropriate failure state
            
            // For import tasks, imported_documents should be reset to 0
            let import_details = Details::SingleIndexSnapshotImport {
                source_path: "/path/to/source.tar.gz".to_string(),
                index_uid: "products".to_string(),
                imported_documents: Some(1000), // Assume partial success
            };
            
            // After failure, count should be reset to 0
            assert_snapshot_import_details(
                import_details.to_failed(),
                "/path/to/source.tar.gz",
                "products", 
                Some(0)
            );
            
            // For creation tasks, snapshot_path should be preserved
            let creation_details = Details::SingleIndexSnapshotCreation {
                index_uid: "products".to_string(),
                snapshot_path: "/path/to/snapshot.tar.gz".to_string(),
            };
            
            // Path should remain unchanged for failed snapshot creation
            assert_snapshot_creation_details(
                creation_details.to_failed(),
                "products", 
                "/path/to/snapshot.tar.gz"
            );
        }
        
        // Helper functions for more concise assertions
        
        fn assert_snapshot_creation_details(
            details: Details,
            expected_uid: &str,
            expected_path: &str
        ) {
            if let Details::SingleIndexSnapshotCreation { index_uid, snapshot_path } = details {
                assert_eq!(index_uid, expected_uid);
                assert_eq!(snapshot_path, expected_path);
            } else {
                panic!("Expected SingleIndexSnapshotCreation variant");
            }
        }
        
        fn assert_snapshot_import_details(
            details: Details,
            expected_src: &str,
            expected_uid: &str,
            expected_docs: Option<u64>
        ) {
            if let Details::SingleIndexSnapshotImport { source_path, index_uid, imported_documents } = details {
                assert_eq!(source_path, expected_src);
                assert_eq!(index_uid, expected_uid);
                assert_eq!(imported_documents, expected_docs);
            } else {
                panic!("Expected SingleIndexSnapshotImport variant");
            }
        }
    }
}

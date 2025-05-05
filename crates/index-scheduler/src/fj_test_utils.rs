//! Fork-specific test utilities for index-scheduler tests.

use std::path::Path;

use meilisearch_types::tasks::{KindWithContent, Task};
use uuid::Uuid;

use crate::test_utils::IndexSchedulerHandle; // Removed read_json import
use crate::Result;

// Define the extension trait
pub(crate) trait FjIndexSchedulerHandleExt {
    /// Registers a task using the internal scheduler instance.
    /// This is async because it calls an async helper.
    async fn fj_register_task(&mut self, task: KindWithContent) -> Result<Task>;

    /// Gets task information using the internal scheduler instance.
    /// This is async to match the pattern used in tests calling handle.get_task.
    async fn fj_get_task(&self, task_id: u32) -> Result<Task>;

    /// Creates an update file with the given JSON content using the internal file store.
    /// Returns the UUID of the file and the number of documents.
    fn fj_create_update_file(&self, content: serde_json::Value) -> Result<(Uuid, u64)>;

    /// Returns the configured path for snapshots from the internal scheduler instance.
    fn fj_snapshots_path(&self) -> &Path;
}

// Implement the trait for the original handle
impl FjIndexSchedulerHandleExt for IndexSchedulerHandle {
    async fn fj_register_task(&mut self, task: KindWithContent) -> Result<Task> { // Added async
        // Use the public register method from IndexSchedulerHandle
        // Note: register_task returns task_id, but we need the Task.
        // We'll register and then immediately fetch the task.
        // This assumes register is synchronous enough for get_task to find it.
        let task_id = self.register_task(task).await?; // Use the async register_task helper
        // Fetch the task using the public get_task helper
        self.get_task(task_id)?
            .ok_or_else(|| crate::Error::TaskNotFound(task_id)) // Handle case where task isn't found immediately
    }

    // Updated implementation: async, uses transaction, returns Result<Task>
    async fn fj_get_task(&self, task_id: u32) -> Result<Task> {
        // Use the public get_task method from IndexSchedulerHandle
        self.get_task(task_id)?
            .ok_or_else(|| crate::Error::TaskNotFound(task_id)) // Return error if None
    }


    fn fj_create_update_file(&self, content: serde_json::Value) -> Result<(Uuid, u64)> {
        // Use the public create_update_file method from IndexSchedulerHandle
        self.create_update_file(content)
    }

    fn fj_snapshots_path(&self) -> &Path {
        // Use the public snapshots_path method from IndexSchedulerHandle
        self.snapshots_path()
    }
}

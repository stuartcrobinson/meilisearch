# Single-Index Snapshot Feature Design Document (Revised)

(Revised with Gemini's review and my new requirement for including version number and optional index name change during import)

## 1. Overall Purpose and Goal

The primary goal is to implement functionality within Meilisearch to create and import snapshots for a single index, rather than the entire instance. This enables rapid horizontal scaling of specific Meilisearch indexes, facilitating efficient replication across different machines (e.g., cloud VMs for a managed service). Users should also be able to specify a new name for the index during the import process.

A key consideration during implementation is to minimize potential merge conflicts with the main open-source Meilisearch repository, as this feature will initially reside in a separate fork (`meilisearchfj`) that needs regular updates from upstream. This is achieved by preferring the addition of new, isolated code paths over modifying existing snapshot logic.

## 2. Relevant Files for Implementation

The following files have been identified as relevant to this feature and are considered part of the working context:

*   `crates/index-scheduler/src/error.rs`
*   `crates/index-scheduler/src/index_mapper/index_map.rs`
*   `crates/index-scheduler/src/index_mapper/mod.rs`
*   `crates/index-scheduler/src/lib.rs`
*   `crates/index-scheduler/src/processing.rs`
*   `crates/index-scheduler/src/queue/mod.rs`
*   `crates/index-scheduler/src/scheduler/mod.rs`
*   `crates/index-scheduler/src/scheduler/process_batch.rs`
*   `crates/index-scheduler/src/upgrade/mod.rs`
*   `crates/index-scheduler/src/versioning.rs`
*   `crates/meilisearch-types/src/tasks.rs`
*   `crates/meilisearch/src/routes/mod.rs`
*   `crates/meilisearch/src/routes/snapshot.rs`
*   `crates/milli/src/index.rs`

## 3. High-Level Implementation Guide (Backend Focus)

This guide outlines the steps for implementing the core backend functionality, suitable for a test-driven development (TDD) approach, deferring API implementation.

### Guiding Principles:

*   **Isolation**: Add new code paths (enums, functions, modules) rather than modifying existing full snapshot logic.
*   **Incremental Development & TDD**: Each step should result in testable backend functionality. Tests will initially involve manually constructing and registering tasks.
*   **Clarity**: Define task payloads, snapshot format, and processing logic clearly.

### Implementation Steps:

### 1. Define New Task Kinds and Payloads

*   **File**: `crates/meilisearch-types/src/tasks.rs`
*   **Action**:
    *   Add `SingleIndexSnapshotCreation` and `SingleIndexSnapshotImport` variants to the `Kind` enum.
    *   Add corresponding variants to the `KindWithContent` enum.
        *   `SingleIndexSnapshotCreation` payload: Needs `index_uid: String`.
        *   `SingleIndexSnapshotImport` payload: Needs `source_snapshot_path: String` (path accessible by the Meilisearch instance, likely within `snapshots_path`) and `target_index_uid: String` (the desired final name for the imported index).
    *   Add corresponding variants to the `Details` enum (e.g., `SingleIndexSnapshotCreation { snapshot_uid: Option<String> }`, `SingleIndexSnapshotImport { source_snapshot_uid: String, target_index_uid: String }`). Using a generated, unique snapshot UID is recommended.
    *   Update necessary `impl` blocks (`as_kind`, `indexes`, `default_details`).
*   **Testing (TDD)**: Write unit tests verifying the new enum variants exist, have the correct payloads, and `Task::index_uid()`/`default_details()` behave as expected. Test serialization/deserialization.

### 2. Implement Task Registration Logic

*   **File**: `crates/index-scheduler/src/queue.rs`
*   **Action**:
    *   Ensure `Queue::register` correctly handles the new `KindWithContent` variants when called internally (e.g., from tests).
    *   Verify that `filter_out_references_to_newer_tasks` and `check_index_swap_validity` don't negatively interact.
*   **Testing (TDD)**: Write integration tests (within the `index-scheduler` crate) that manually construct `KindWithContent` for the new types, call `queue.register`, and verify that tasks with the correct kind, payload, and initial status (`enqueued`) are persisted in the task database.

### 3. Implement Core Snapshot Creation Logic

*   **Files**: Potentially new helper module/functions (e.g., `crates/index-scheduler/src/snapshot_utils.rs` or similar).
*   **Action**:
    *   **Define Format**: Specify the snapshot format. Recommendation: A gzipped tarball (`.snapshot.tar.gz`) containing:
        *   `data.mdb`: The LMDB data file for the index.
        *   `metadata.json`: A JSON file containing essential index settings and metadata.
    *   **Define Metadata**: Specify the exact content of `metadata.json`. It *must* include core information and all relevant index settings, grouped conceptually as follows (implementation requires the specific fields):
        *   **Core Info**: `meilisearchVersion` (String, e.g., "1.7.0"), `primaryKey` (String or null).
        *   **Attribute Settings**: `displayedAttributes`, `searchableAttributes` (user-defined), `filterableAttributes`, `sortableAttributes`.
        *   **Search Tuning**: `rankingRules`, `stopWords`, `synonyms`, `typoTolerance`.
        *   **Other Settings**: `pagination`, `faceting`, `embedders`.
        *   **Timestamps**: Consider including index creation/update timestamps if needed for `Index::new_with_creation_dates`.
    *   **Implement Core Function**: Create a new, isolated function (e.g., `create_index_snapshot(index: &Index, snapshots_path: &Path) -> Result<String>`).
        *   Inside this function (assumes caller ensures `index` state consistency, e.g., via scheduler lock):
            *   **Read Settings**: Acquire an `RoTxn` on the target `Index` and read all necessary settings. Release the `RoTxn`.
            *   **Copy Data**: Call `Index::copy_to_path(...)` to copy `data.mdb` to a temporary location.
            *   **Package**: Generate `metadata.json` (including current Meilisearch version), create the tarball with the copied `data.mdb` and `metadata.json`.
            *   **Store**: Generate a unique snapshot identifier (`snapshot_uid`, e.g., based on timestamp/UUID). Create the snapshot filename incorporating this UID (e.g., `{index_uid}-{snapshot_uid}.snapshot.tar.gz`). Move the final snapshot tarball to the configured `snapshots_path`.
            *   Return the generated `snapshot_uid`.
*   **Testing (TDD)**: Write unit/integration tests calling the `create_index_snapshot` function directly with a test index handle and path. Verify the snapshot file (`.snapshot.tar.gz`) is created correctly in the specified path with the expected naming convention. Unpack and verify `metadata.json` (including version) and `data.mdb`. Check that the correct `snapshot_uid` is returned. Test error handling (e.g., I/O errors).

### 4. Integrate Snapshot Creation into Scheduler

*   **Files**: `crates/index-scheduler/src/scheduler.rs`, `crates/index-scheduler/src/scheduler/process_batch.rs`.
*   **Action**:
    *   Modify the main task processing logic (likely `IndexScheduler::process_batch` or a called function like `process_index_operation`) to recognize `KindWithContent::SingleIndexSnapshotCreation`.
    *   Create a new, isolated function (e.g., `process_single_index_snapshot_creation`) within the scheduler module to handle this task type.
    *   Inside this function:
        *   Retrieve `index_uid`.
        *   Use `IndexMapper` to get the `Index` handle.
        *   **Ensure Exclusive Scheduling**: Mark the index as "currently updating" via `IndexMapper::set_currently_updating_index` to block other write tasks on this index.
        *   **Call Core Logic**: Call the `create_index_snapshot` function (from Step 3) with the index handle and `snapshots_path`.
        *   **Finalize Task**: Based on the `Result` from the core logic: update the task status to `Succeeded` and store the returned `snapshot_uid` in `Details` on success, or update the task status to `Failed` and store the error on failure.
        *   **Release Lock**: Release the "currently updating" status via `IndexMapper::set_currently_updating_index(None)`.
*   **Testing (TDD)**: Write integration tests: Manually enqueue a `SingleIndexSnapshotCreation` task. Run the scheduler's `tick()` method (or relevant parts). Verify the task's final status (`Succeeded`/`Failed`) and `details.snapshot_uid` (on success). The snapshot file integrity is already tested in Step 3. Test error handling (e.g., index not found).

### 5. Implement Core Snapshot Import Logic (`IndexMapper` Method)

*   **Files**: `crates/index-scheduler/src/index_mapper/mod.rs`, `crates/index-scheduler/src/index_mapper/index_map.rs`.
*   **Action**:
    *   **Add Dedicated `IndexMapper` Method**: Implement a new method `IndexMapper::import_index_from_snapshot(target_index_uid: &str, snapshot_path: &Path) -> Result<(Index, ParsedMetadata)>`. (Define a helper struct `ParsedMetadata` to hold the deserialized content of `metadata.json`, e.g., `struct ParsedMetadata { version: String, settings: Settings<Unchecked>, ... }`). This method will *not* reuse `IndexMapper::create_index`.
    *   **Inside `IndexMapper::import_index_from_snapshot`**:
        *   **Validate Request & Path**: Perform security check on `snapshot_path` (must be within `snapshots_path`). Check if `target_index_uid` already exists using `self.index_exists`; fail if it does.
        *   **Unpack & Validate Snapshot**: Untar snapshot to a temporary directory (Recommendation: create this temp directory *within* the main Meilisearch data path, e.g., `data.ms/tmp_snapshot_import_{uuid}/`, to ensure same filesystem). Verify `data.mdb` and `metadata.json`. Parse `metadata.json`. Perform **Version Check**: Compare `major` and `minor` version from metadata against the current instance version. Fail with `SnapshotVersionMismatch` if they don't match (allow patch differences). Store parsed metadata.
        *   **Prepare Index Directory & Data**: Generate a new internal UUID. Create `indexes/{new_uuid}/`. Move unpacked `data.mdb` from the temporary directory into this new directory.
        *   **Register, Open, and Map Index**:
            *   Call `milli::Index::new_with_creation_dates(..., creation: false)` on the prepared directory using dates from metadata if available.
            *   Acquire `RwTxn` for the main scheduler env (`self.env`).
            *   Acquire write lock on `self.index_map`.
            *   Update `self.index_mapping` (`target_index_uid` -> `new_uuid`).
            *   Insert the opened `Index` into the LRU map: `let outcome = self.index_map.available.insert(new_uuid, index.clone());`.
            *   Handle potential eviction: `if let InsertionOutcome::Evicted(evicted_uuid, evicted_index) = outcome { self.index_map.close(evicted_uuid, evicted_index, self.enable_mdb_writemap, 0); }`.
            *   Release `self.index_map` write lock.
            *   Commit `RwTxn`.
        *   **Cleanup**: Ensure the temporary unpack directory is reliably cleaned up (e.g., using `defer` or RAII pattern if applicable, or explicit cleanup in error paths) after unpacking.
        *   Return the opened `Index` and the parsed metadata.
*   **Testing (TDD)**: Write integration tests calling the `IndexMapper::import_index_from_snapshot` method directly with a prepared snapshot file and target UID. Verify the index directory is created, `data.mdb` is present, the mapping exists in `index_mapping`, the `Index` object is returned, and the `IndexMap` contains the new index. **Specifically test the scenario where importing causes an LRU eviction to ensure `IndexMap::close` is handled correctly.** Test errors (invalid path, target exists, bad format, version mismatch, I/O). Verify temporary directory cleanup on success and failure.

### 6. Integrate Snapshot Import into Scheduler

*   **Files**: `crates/index-scheduler/src/scheduler.rs`, `crates/index-scheduler/src/scheduler/process_batch.rs`.
*   **Action**:
    *   Modify the task processing logic to recognize `KindWithContent::SingleIndexSnapshotImport`.
    *   Create a new, isolated function (e.g., `process_single_index_snapshot_import`) within the scheduler module.
    *   **Inside `process_single_index_snapshot_import`**:
        *   Retrieve `source_snapshot_path` and `target_index_uid` from the task payload.
        *   **Call Core Logic**: Call `IndexMapper::import_index_from_snapshot` (from Step 5).
        *   **Apply Settings**: On success from the core logic, use the returned `Index` handle and parsed metadata (`ParsedMetadata.settings`) to apply the settings to the newly imported index. This typically involves creating an `update::Settings` builder, populating it from the parsed settings, and executing it within a write transaction on the imported index.
        *   **Finalize Task**: Based on the `Result` from *both* the core logic and the settings application: update the task status to `Succeeded` only if both succeeded. If either fails, update the status to `Failed` and store the relevant error in `Details`.
*   **Testing (TDD)**: Write integration tests: Place a valid snapshot file in `snapshots_path`. Enqueue an `SingleIndexSnapshotImport` task. Run `tick()`. Verify the task's final status. Check that the new index exists (via `IndexMapper` or API) and has the correct settings applied (check settings API or direct `milli::Index` methods). Test error handling during the settings application phase (ensure task fails correctly).

### 7. Add Progress Reporting (Optional Backend Part)

*   **File**: `crates/index-scheduler/src/processing.rs`
*   **Action**:
    *   Define new enum variants for progress steps (e.g., `ValidatingSnapshot`, `CopyingIndexData`, `PackagingSnapshot`, `UnpackingSnapshot`, `ApplyingSettings`).
    *   Update the processing functions (from steps 4 & 6) to report progress via the `Progress` object.
*   **Testing (TDD)**: Enhance tests from steps 4 & 6 to check the `details` field of completed tasks for expected progress steps.

## 4. Error Handling Guide

Follow these guidelines for handling errors related to the new snapshot tasks:

1.  **Leverage Existing Enums**: Primarily use the existing `index_scheduler::Error` enum. Add new, specific variants only when necessary. Potential additions:
    *   `SnapshotCreationFailed { index_uid: String, source: Box<dyn std::error::Error + Send + Sync> }`
    *   `InvalidSnapshotFormat { path: PathBuf }`
    *   `SnapshotImportFailed { target_index_uid: String, source: Box<dyn std::error::Error + Send + Sync> }`
    *   `SnapshotImportTargetIndexExists { target_index_uid: String }`
    *   `SnapshotVersionMismatch { path: PathBuf, snapshot_version: String, current_version: String }`
    *   `InvalidSnapshotPath { path: PathBuf }` (for security check failure)

2.  **Wrap Underlying Errors**: When catching errors from `heed`, `std::io`, `milli`, `tar`, `flate2`, `serde_json`, etc., wrap them using `From` implementations or map them into appropriate existing or new variants within `index_scheduler::Error` (like `HeedTransaction`, `IoError`, `Milli`, or the new snapshot-specific ones).

3.  **Task Error Reporting**: In the snapshot creation/import processing functions, catch any `Result::Err(e)`. Convert the caught `index_scheduler::Error` into a `meilisearch_types::error::ResponseError`. Store this `ResponseError` in the `Task::error` field and set the `Task::status` to `Failed`. Update `Task::details` appropriately (e.g., `details.to_failed()`).

4.  **Use `?` for Propagation**: Use the `?` operator within the processing logic to propagate errors cleanly up to the point where they are caught and recorded in the task.

5.  **Minimize New Error Types**: Avoid creating entirely new Error enums unless the snapshot logic becomes significantly complex and warrants its own error domain. Stick to adding variants to `index_scheduler::Error` to reduce boilerplate and potential conflicts.

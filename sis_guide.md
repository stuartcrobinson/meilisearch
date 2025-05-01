# Single-Index Snapshot Feature Design Document (Revised)

(Revised with Gemini's review and my new requirement for including version number and optional index name change during import)

## 1. Overall Purpose and Goal

The primary goal is to implement functionality within Meilisearch to create and import snapshots for a single index, rather than the entire instance. This enables rapid horizontal scaling of specific Meilisearch indexes, facilitating efficient replication across different machines (e.g., cloud VMs for a managed service). Users should also be able to specify a new name for the index during the import process.

A key consideration during implementation is to minimize potential merge conflicts with the main open-source Meilisearch repository, as this feature will initially reside in a separate fork (`meilisearchfj`) that needs regular updates from upstream. This is achieved by preferring the addition of new, isolated code paths over modifying existing snapshot logic.

## 2. Relevant Files for Implementation

The following files have been identified as relevant to this feature and are considered part of the working context:

*   `crates/index-scheduler/src/index_mapper/index_map.rs`
*   `crates/index-scheduler/src/lib.rs`
*   `crates/meilisearch-types/src/tasks.rs`
*   `crates/meilisearch/src/routes/mod.rs`
*   `crates/meilisearch/src/routes/snapshot.rs`
*   `crates/milli/src/index.rs`
*   `crates/index-scheduler/src/scheduler/mod.rs`
*   `crates/index-scheduler/src/index_mapper/mod.rs`
*   `crates/index-scheduler/src/processing.rs`
*   `crates/index-scheduler/src/queue/mod.rs`

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

### 3. Define Snapshot Format and Implement Creation Logic

*   **Files**: `crates/index-scheduler/src/scheduler.rs`, potentially new helper modules/functions within `index-scheduler`.
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
    *   **Implement Creation**:
        *   Modify the main task processing logic in `scheduler.rs` (likely `IndexScheduler::process_batch` or a called function) to recognize `KindWithContent::SingleIndexSnapshotCreation`.
        *   Create a new, isolated function (e.g., `process_single_index_snapshot_creation`) to handle this task type.
        *   Inside this function:
            *   Retrieve `index_uid`.
            *   Use `IndexMapper` to get the index's internal UUID and path.
            *   **Ensure Exclusive Scheduling**: Mark the index as "currently updating" via `IndexMapper::set_currently_updating_index` to block other write tasks on this index.
            *   **Read Settings**: Acquire an `RoTxn` on the target `Index` and read all necessary settings. Release the `RoTxn`.
            *   **Copy Data**: Call `Index::copy_to_path(...)` to copy `data.mdb` (this uses its own internal `RoTxn`). The data state will be consistent with the settings read previously due to the scheduler-level exclusion.
            *   **Package**: Generate `metadata.json` (including current Meilisearch version), create the tarball with the copied `data.mdb` and `metadata.json`.
            *   **Store**: Generate a unique snapshot identifier (`snapshot_uid`, e.g., based on timestamp/UUID). Create the snapshot filename incorporating this UID (e.g., `{index_uid}-{snapshot_uid}.snapshot.tar.gz`). Move the final snapshot tarball to the configured `snapshots_path`.
            *   **Finalize Task**: Update the task status (`Succeeded`/`Failed`) and `Details` (using the generated `snapshot_uid`).
            *   **Release Lock**: Release the "currently updating" status via `IndexMapper::set_currently_updating_index(None)`.
*   **Testing (TDD)**: Write integration tests: Manually enqueue a `SingleIndexSnapshotCreation` task. Run the scheduler's `tick()` method (or relevant parts). Verify the snapshot file (`.snapshot.tar.gz`) is created correctly in `snapshots_path` with the expected naming convention. Unpack and verify `metadata.json` (including version) and `data.mdb`. Check task status and `details.snapshot_uid`. Test error handling.

### 4. Implement Single Index Snapshot Import Logic

*   **Files**: `crates/index-scheduler/src/scheduler.rs`, `crates/index-scheduler/src/index_mapper/mod.rs`, `crates/index-scheduler/src/index_mapper/index_map.rs`, potentially new helpers.
*   **Action**:
    *   Modify the task processing logic to recognize `KindWithContent::SingleIndexSnapshotImport`.
    *   Create a new, isolated function (e.g., `process_single_index_snapshot_import`).
    *   **Add Dedicated `IndexMapper` Method**: Implement a new method like `IndexMapper::import_index_from_snapshot(target_index_uid: &str, snapshot_path: &Path) -> Result<Index>` to encapsulate the core import logic. This method will *not* reuse `IndexMapper::create_index`.
    *   **Inside `process_single_index_snapshot_import`**:
        *   Retrieve `source_snapshot_path` and `target_index_uid`.
        *   Call the new `IndexMapper::import_index_from_snapshot` method.
        *   Handle the result, update task status (`Succeeded`/`Failed`) and Details.
    *   **Inside `IndexMapper::import_index_from_snapshot`**:
        *   **4a. Validate Request & Path**: Perform security check on `source_snapshot_path` (must be within `snapshots_path`). Check if `target_index_uid` already exists using `self.index_exists`; fail if it does.
        *   **4b. Unpack & Validate Snapshot**: Untar snapshot (e.g., to temp dir). Verify `data.mdb` and `metadata.json`. Parse `metadata.json`. Perform **Version Check**: Compare `major` and `minor` version from metadata against the current instance version. Fail with `SnapshotVersionMismatch` if they don't match (allow patch differences).
        *   **4c. Prepare Index Directory & Data**: Generate a new internal UUID. Create `indexes/{new_uuid}/`. Move unpacked `data.mdb` into this directory.
        *   **4d. Register, Open, and Map Index**:
            *   Call `milli::Index::new_with_creation_dates(..., creation: false)` on the prepared directory using dates from metadata if available.
            *   Acquire `RwTxn` for the main scheduler env (`self.env`).
            *   Acquire write lock on `self.index_map`.
            *   Update `self.index_mapping` (`target_index_uid` -> `new_uuid`).
            *   Insert the opened `Index` into the LRU map: `let outcome = self.index_map.available.insert(new_uuid, index.clone());`.
            *   Handle potential eviction: `if let InsertionOutcome::Evicted(evicted_uuid, evicted_index) = outcome { self.index_map.close(evicted_uuid, evicted_index, self.enable_mdb_writemap, 0); }`.
            *   Release `self.index_map` write lock.
            *   Commit `RwTxn`.
            *   Return the opened `Index`.
        *   **4e. Apply Settings**: (This should happen *after* the index is successfully registered and opened, likely back in `process_single_index_snapshot_import`). Use the parsed metadata to configure the newly opened index via `update::Settings`.
        *   **4f. Cleanup**: Clean up temporary unpack directory.
*   **Testing (TDD)**: Write integration tests: Place a valid snapshot file in `snapshots_path`. Enqueue an `SingleIndexSnapshotImport` task. Run `tick()`. Verify the new index exists via `IndexMapper`, on disk, has correct data (search test) and settings (API/direct check). Check task status. **Specifically test the scenario where importing causes an LRU eviction to ensure `IndexMap::close` is handled correctly.** Test errors (invalid path, target exists, bad format, version mismatch, I/O).

### 5. Add Progress Reporting (Optional Backend Part)

*   **File**: `crates/index-scheduler/src/processing.rs`
*   **Action**:
    *   Define new enum variants for progress steps (e.g., `ValidatingSnapshot`, `CopyingIndexData`, `PackagingSnapshot`, `UnpackingSnapshot`, `ApplyingSettings`).
    *   Update the processing functions (from steps 3 & 4) to report progress via the `Progress` object.
*   **Testing (TDD)**: Enhance tests from steps 3 & 4 to check the `details` field of completed tasks for expected progress steps.

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
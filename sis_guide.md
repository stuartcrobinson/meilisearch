# Single-Index Snapshot Feature Design Document (Revised)

(Revised with Gemini's review and my new requirement for including version number and optional index name change during import)

## A. Overall Purpose and Goal

The primary goal is to implement functionality within Meilisearch to create and import snapshots for a single index, rather than the entire instance. This enables rapid horizontal scaling of specific Meilisearch indexes, facilitating efficient replication across different machines (e.g., cloud VMs for a managed service). Users should also be able to specify a new name for the index during the import process.

A key consideration during implementation is to minimize potential merge conflicts with the main open-source Meilisearch repository, as this feature will initially reside in a separate fork (`meilisearchfj`) that needs regular updates from upstream. This is achieved by preferring the addition of new, isolated code paths over modifying existing snapshot logic.

## B. Relevant Files for Implementation

The following files have been identified as relevant to this feature and are considered part of the working context:

```
/add \
crates/index-scheduler/src/error.rs \
crates/index-scheduler/src/index_mapper/index_map.rs \
crates/index-scheduler/src/index_mapper/mod.rs \
crates/index-scheduler/src/lib.rs \
crates/index-scheduler/src/processing.rs \
crates/index-scheduler/src/queue/mod.rs \
crates/index-scheduler/src/scheduler/mod.rs \
crates/index-scheduler/src/scheduler/process_batch.rs \
crates/index-scheduler/src/upgrade/mod.rs \
crates/index-scheduler/src/versioning.rs \
crates/meilisearch-types/src/tasks.rs \
crates/meilisearch/src/routes/mod.rs \
crates/meilisearch/src/routes/snapshot.rs \
crates/milli/src/index.rs
```

## C. High-Level Implementation Guide (Backend Focus)

This guide outlines the steps for implementing the core backend functionality, suitable for a test-driven development (TDD) approach, deferring API implementation.

### Guiding Principles:

*   **User Control**: This guide outlines the steps. For each step, first ensure you understand the required actions. Then, explicitly instruct the AI assistant to generate the necessary code changes using SEARCH/REPLACE blocks. Review and apply the changes before asking the assistant to proceed with the next step.
*   **Isolation**: Add new code paths (enums, functions, modules) rather than modifying existing full snapshot logic.
*   **Incremental Development & TDD**: Each step should result in testable backend functionality. Tests will initially involve manually constructing and registering tasks.
*   **Test Simplicity**: Strive for simplicity and isolation in tests, particularly when testing core logic. Whenever possible, set up test state using direct, lower-level APIs (like `IndexMapper` or `milli::update` methods) instead of complex mechanisms such as the full task queue (`handle.register_task`, `handle.advance_one_successful_batch`). Direct setup leads to clearer, more focused, less coupled, and more maintainable tests that are less prone to breaking from unrelated changes in higher-level systems.
*   **Clarity**: Define task payloads, snapshot format, and processing logic clearly.
*   **Fork Naming Conventions**: To easily distinguish custom code for this fork (`meilisearchfj`) from upstream Meilisearch code (aiding maintainability and merging), use the following naming conventions:
    *   **New Files**: Prefix all *new* files created specifically for this fork with `fj_` (e.g., `fj_snapshot_utils.rs`).
    *   **New Tests**: Prefix custom test modules or filenames with `msfj_` (general fork tests) or `msfj_sis_` (Single Index Snapshot specific tests) (e.g., `mod msfj_sis_tests { ... }` or `tests/msfj_sis_tasks.rs`).
    *   **Running Tests**: Use `cargo test` with a filter argument matching the prefix:
        *   Run *all* custom tests: `cargo test msfj_`
        *   Run *only* Single Index Snapshot tests: `cargo test msfj_sis_`
    *   **New Code in Existing Files**: **CRITICAL:** Prefix *all* new functions, structs, enums, or significant code blocks added to *existing* Meilisearch files with `fj_` (e.g., `fn fj_my_helper_function(...)`, `struct FjMyStruct { ... }`). This applies even when adding a new method to an existing `impl` block. This is crucial for differentiating fork-specific additions within upstream files and preventing merge conflicts. **Do not forget this prefix.**

### Implementation Steps:

### 1. Define New Task Kinds and Payloads

*   **File**: `crates/meilisearch-types/src/tasks.rs`
*   **Action**:
    *   Add `SingleIndexSnapshotCreation` and `SingleIndexSnapshotImport` variants to the `Kind` enum.
    *   Add corresponding variants to the `KindWithContent` enum.
        *   `SingleIndexSnapshotCreation` payload: Needs `index_uid: String`.
        *   `SingleIndexSnapshotImport` payload: Needs `source_snapshot_path: String` (path accessible by the Meilisearch instance, likely within `snapshots_path`) and `target_index_uid: String` (the desired final name for the imported index).
    *   Add corresponding variants to the `Details` enum:
        *   `SingleIndexSnapshotCreation { snapshot_uid: Option<String> }` (Stores the unique identifier generated during creation, used in the snapshot filename).
        *   `SingleIndexSnapshotImport { source_snapshot_uid: String, target_index_uid: String }` (`source_snapshot_uid` refers to the UID embedded in the source snapshot's filename).
        *   Using a generated, unique snapshot UID (e.g., timestamp-based or UUID) is recommended.
    *   Update necessary `impl` blocks (`as_kind`, `indexes`, `default_details`).
*   **Testing (TDD)**: Write unit tests verifying the new enum variants exist, have the correct payloads, and `Task::index_uid()`/`default_details()` behave as expected. Test serialization/deserialization.
*   **Step Completion Check**: Add the `cargo test` command here to run only the tests implemented for this step.

### 2. Implement Task Registration Logic

*   **File**: `crates/index-scheduler/src/queue.rs`
*   **Action**:
    *   Ensure `Queue::register` correctly handles the new `KindWithContent` variants when called internally (e.g., from tests).
    *   Verify that `filter_out_references_to_newer_tasks` and `check_index_swap_validity` don't negatively interact.
*   **Testing (TDD)**: Write integration tests (within the `index-scheduler` crate) that manually construct `KindWithContent` for the new types, call `queue.register`, and verify that tasks with the correct kind, payload, and initial status (`enqueued`) are persisted in the task database.
*   **Step Completion Check**: Add the `cargo test` command here to run only the tests implemented for this step.

### 3. Implement Core Snapshot Creation Logic

```
/add \
crates/milli/src/index.rs \
crates/index-scheduler/src/error.rs \
crates/index-scheduler/src/lib.rs
```

*   **Files**: Potentially new helper module/functions (e.g., `crates/index-scheduler/src/snapshot_utils.rs` or similar).
*   **Action**:
    *   **Define Format**: Specify the snapshot format. Recommendation: A gzipped tarball (`.snapshot.tar.gz`) containing:
        *   `data.mdb`: The LMDB data file for the index.
        *   `metadata.json`: A JSON file containing essential index settings and metadata.
    *   **Define Metadata**: Specify the exact content of `metadata.json`. It *must* include core information and all relevant index settings, grouped conceptually as follows (implementation requires the specific fields):
        *   **Core Info**: `meilisearchVersion` (String, e.g., "1.7.0"), `primaryKey` (String or null).
        *   **Attribute Settings**: `displayedAttributes`, `searchableAttributes` (user-defined), `filterableAttributes`, `sortableAttributes`, `distinctAttribute`.
        *   **Search Tuning**: `rankingRules`, `stopWords`, `synonyms`, `typoTolerance`.
        *   **Other Settings**: `pagination`, `faceting`, `embedders`.
        *   **Timestamps**: Include `createdAt` and `updatedAt` timestamps (read from the source index).
    *   **Implement Core Function**: Create a new, isolated function (e.g., `create_index_snapshot(index: &Index, snapshots_path: &Path) -> Result<String>`).
        *   Inside this function (assumes caller ensures `index` state consistency, e.g., via scheduler lock):
            *   **Read Settings & Timestamps**: Acquire an `RoTxn` on the target `Index` and read all necessary settings, including `created_at` and `updated_at`. Release the `RoTxn`.
            *   **Copy Data**: Call `Index::copy_to_path(...)` to copy `data.mdb` to a temporary location.
            *   **Package**: Generate `metadata.json` (including current Meilisearch version), create the tarball with the copied `data.mdb` and `metadata.json`.
            *   **Store**: Generate a unique snapshot identifier (`snapshot_uid`, e.g., based on timestamp/UUID). Create the snapshot filename incorporating this UID (e.g., `{index_uid}-{snapshot_uid}.snapshot.tar.gz`). Move the final snapshot tarball to the configured `snapshots_path`.
            *   Return the generated `snapshot_uid`.
*   **Testing (TDD)**: Write unit/integration tests calling the `create_index_snapshot` function directly with a test index handle and path. Verify the snapshot file (`.snapshot.tar.gz`) is created correctly in the specified path with the expected naming convention. Unpack and verify `metadata.json` (including version) and `data.mdb`. Check that the correct `snapshot_uid` is returned. Test error handling (e.g., I/O errors).
*   **Step Completion Check**: Add the `cargo test` command here to run only the tests implemented for this step.
    ```
    cargo test -p index-scheduler -- tests::msfj_sis_snapshot_creation::msfj_sis_snapshot_creation_tests
    ```

### 3b. Complete Metadata Retrieval and Testing (Deferred)

*   **Files**: `crates/index-scheduler/src/fj_snapshot_utils.rs`, `crates/index-scheduler/src/tests/msfj_sis_snapshot_creation.rs`.
*   **Action**:
    *   **Implement Missing Settings Retrieval**: In `fj_snapshot_utils.rs::read_metadata`, replace the `Setting::NotSet` placeholders for `typoTolerance`, `pagination`, `faceting`, `embedders`, and `localizedAttributes`. Implement the logic to read the actual values for these settings from the `milli::Index` using an `RoTxn`. Ensure correct type conversions to the `meilisearch_types::settings` equivalents where necessary (similar to how `proximity_precision` or `ranking_rules` are handled).
    *   **Update Tests**: In `msfj_sis_snapshot_creation.rs`, enhance the existing tests or add new ones to specifically verify that the `typoTolerance`, `pagination`, `faceting`, `embedders`, and `localizedAttributes` fields in the unpacked `metadata.json` contain the correct values corresponding to the settings applied to the test index before snapshotting. This involves:
        *   Setting non-default values for these settings on the test index.
        *   Creating the snapshot.
        *   Unpacking the snapshot.
        *   Deserializing `metadata.json`.
        *   Asserting that the corresponding fields in the deserialized `SnapshotMetadata` match the expected values.
*   **Goal**: Ensure the `metadata.json` within the snapshot accurately and completely reflects *all* configurable settings of the source index as defined in the original Step 3 metadata specification.
*   **Step Completion Check**: Add the `cargo test` command here to run only the tests implemented for this step.
    ```
    cargo test -p index-scheduler -- tests::msfj_sis_snapshot_creation::msfj_sis_snapshot_creation_tests
    ```

### 4. Integrate Snapshot Creation into Scheduler

```
/add \
crates/index-scheduler/src/scheduler/process_batch.rs \
crates/index-scheduler/src/scheduler/mod.rs \
crates/index-scheduler/src/index_mapper/mod.rs \
crates/meilisearch-types/src/tasks.rs \
crates/index-scheduler/src/lib.rs
```

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
*   **Testing (TDD)**: Write integration tests: Manually enqueue a `SingleIndexSnapshotCreation` task. Run the scheduler's `tick()` method (or relevant parts). Verify the scheduler correctly calls the core `create_index_snapshot` function and handles its `Result` to update the task's final status (`Succeeded`/`Failed`) and `details.snapshot_uid` (on success). The snapshot file integrity itself is already tested in Step 3. Test error handling (e.g., index not found).
*   **Step Completion Check**: Add the `cargo test` command here to run only the tests implemented for this step.
```
cargo test -p index-scheduler -- tests::msfj_sis_scheduler_integration
```

### 5. Implement Core Snapshot Import Logic (`IndexMapper` Method)

```
/add \
crates/index-scheduler/src/index_mapper/mod.rs \
crates/index-scheduler/src/index_mapper/index_map.rs \
crates/milli/src/index.rs \
crates/index-scheduler/src/error.rs \
crates/meilisearch-types/src/tasks.rs \
crates/index-scheduler/src/lib.rs
```
*   **Files**: `crates/index-scheduler/src/index_mapper/mod.rs`, `crates/index-scheduler/src/index_mapper/index_map.rs`.
*   **Action**:
    *   **Add Dedicated `IndexMapper` Method**: Implement a new method `IndexMapper::import_index_from_snapshot(target_index_uid: &str, snapshot_path: &Path) -> Result<(Index, ParsedMetadata)>`. (Define a helper struct `ParsedMetadata` to hold the deserialized content of `metadata.json`, e.g., `struct ParsedMetadata { version: String, settings: Settings<Unchecked>, ... }`). This method will *not* reuse `IndexMapper::create_index`.
    *   **Inside `IndexMapper::import_index_from_snapshot`**:
        *   **Validate Request & Path**: Perform security check on `snapshot_path` (must be within `snapshots_path`). Check if `target_index_uid` already exists using `self.index_exists`; fail if it does.
        *   **Unpack & Validate Snapshot**: Untar snapshot to a temporary directory (Recommendation: create this temp directory *within* the main Meilisearch data path, e.g., `data.ms/tmp_snapshot_import_{uuid}/`, to ensure same filesystem). Verify `data.mdb` and `metadata.json`. Parse `metadata.json`. Perform **Version Check**: Compare `major` and `minor` version from metadata against the current instance version. Fail with `SnapshotVersionMismatch` if they don't match (allow patch differences). Store parsed metadata.
        *   **Prepare Index Directory & Data**: Generate a new internal UUID. Create `indexes/{new_uuid}/`. Move unpacked `data.mdb` from the temporary directory into this new directory.
        *   **Register, Open, and Map Index**:
            *   Call `milli::Index::new_with_creation_dates(..., creation: false)` on the prepared directory, passing the `createdAt` and `updatedAt` timestamps read from the snapshot's metadata.
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
*   **Step Completion Check**: Add the `cargo test` command here to run only the tests implemented for this step.
```
cargo test -p index-scheduler -- tests::msfj_sis_index_mapper_import
```

### 6. Integrate Snapshot Import into Scheduler

```
/add \
crates/index-scheduler/src/scheduler/process_batch.rs \
crates/index-scheduler/src/scheduler/mod.rs \
crates/index-scheduler/src/index_mapper/mod.rs \
crates/meilisearch-types/src/tasks.rs \
crates/index-scheduler/src/lib.rs \
crates/milli/src/index.rs \
crates/index-scheduler/src/error.rs
```

*   **Files**: `crates/index-scheduler/src/scheduler.rs`, `crates/index-scheduler/src/scheduler/process_batch.rs`.
*   **Action**:
    *   Modify the task processing logic to recognize `KindWithContent::SingleIndexSnapshotImport`.
    *   Create a new, isolated function (e.g., `process_single_index_snapshot_import`) within the scheduler module.
    *   **Inside `process_single_index_snapshot_import`**:
        *   Retrieve `source_snapshot_path` and `target_index_uid` from the task payload.
        *   **Call Core Logic**: Call `IndexMapper::import_index_from_snapshot` (from Step 5).
        *   **Apply Settings**: On success from the core logic, use the returned `Index` handle and parsed metadata (`ParsedMetadata.settings`) to apply the settings to the newly imported index. This typically involves creating an `update::Settings` builder, populating it from the parsed settings, and executing it within a write transaction on the imported index.
        *   **Finalize Task**: Based on the `Result` from *both* the core logic and the settings application: update the task status to `Succeeded` only if both succeeded. If either fails, update the status to `Failed` and store the relevant error in `Details`.
*   **Testing (TDD)**: Write integration tests: Place a valid snapshot file in `snapshots_path`. Enqueue an `SingleIndexSnapshotImport` task. Run `tick()`. Verify the scheduler correctly calls the core `IndexMapper::import_index_from_snapshot` function, handles its `Result`, attempts settings application on success, and updates the task's final status based on the outcome of both steps. Check that the new index exists (via `IndexMapper` or API) and has the correct settings applied (check settings API or direct `milli::Index` methods). Test error handling during the settings application phase (ensure task fails correctly).
*   **Step Completion Check**: Add the `cargo test` command here to run only the tests implemented for this step.
```
cargo test -p index-scheduler -- scheduler::test::msfj_sis_scheduler_import_tests
```

### 7. Add Progress Reporting (Optional Backend Part)

```
/add \
crates/index-scheduler/src/processing.rs \
crates/index-scheduler/src/scheduler/mod.rs \
crates/index-scheduler/src/scheduler/process_batch.rs
```

*   **File**: `crates/index-scheduler/src/processing.rs`
*   **Action**:
    *   Define new enum variants for progress steps (e.g., `ValidatingSnapshot`, `CopyingIndexData`, `PackagingSnapshot`, `UnpackingSnapshot`, `ApplyingSettings`).
    *   Update the processing functions (from steps 4 & 6) to report progress via the `Progress` object.
*   **Testing (TDD)**: Enhance tests from steps 4 & 6 to check the `details` field of completed tasks for expected progress steps.
*   **Step Completion Check**: Add the `cargo test` command here to run only the tests implemented for this step.

### 8. End to end tests 
```
cargo test -p index-scheduler -- msfj_sis_scheduler_e2e_tests::test_e2e_snapshot_create_import_verify
```

## D. Error Handling Guide

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

## E. Development Workflow Notes

### Code Changes:

Do not generate any SEARCH/REPLACE blocks or suggest code modifications for any step until explicitly asked by the user to do so for that specific step.


### Troubleshooting:

When encountering persistent compiler errors (like type mismatches, unresolved paths, or missing methods), avoid excessive trial-and-error. Instead, **prioritize understanding the involved types and interfaces.** Ask the AI assistant to show the definitions of relevant structs, enums, traits, and functions. Examine their fields (public vs. private), methods, and implemented traits. Additionally, request examples of similar code usage elsewhere in the Meilisearch codebase. This direct inspection often reveals the root cause (e.g., incorrect type usage, privacy issues, missing trait implementations, or incorrect method calls) much faster than repeated code change attempts.

When the AI LLM coder (like aider) is trying to debug and fix errors, it should feel encouraged to ask to look at any other files that might be helpful.

Debugging:

 1 Look at Definitions Sooner: When facing persistent type errors (E0223, E0308, E0433, E0599) or ambiguity, don't spend too long trying path        
   variations. Ask to see the definition of the relevant struct/enum/trait and the function/method being called much earlier. This would have quickly
   revealed that EmbeddingSettings was a struct and that milli::OrderBy was the wrong enum.                                                          
 2 Verify Re-exports: When using paths like crate::some_module::Type, be mindful that some_module might be a re-export. If errors persist, ask to see
   the lib.rs or mod.rs file where the re-export occurs (pub use ...) and potentially the lib.rs of the original crate to confirm exactly which type 
   is being re-exported, especially if names might collide (like OrderBy).                                                                           
 3 Trust Specific Error Codes (Sometimes): While E0599 was misleading due to the underlying E0223, the E0308 (mismatched types) and E0433 (failed to 
   resolve path) errors were quite accurate once the major ambiguity was gone. Pay close attention to the expected vs. found types in E0308.         
 4 Look for Examples: Yes, being more aggressive about asking for examples of similar code elsewhere in the Meilisearch codebase would likely have   
   shown the correct way to construct EmbeddingSettings or set the sort_facet_values_by option much faster. Existing tests or core logic often       
   provide the best patterns.                                                                                                                        
 5 Clean Builds: While it didn't solve the core issue here, running cargo clean periodically during complex debugging can rule out stale build       
   artifacts causing strange behavior.                                                                                                               

When debugging persistent test failures, avoid sequential trial-and-error fixes. Instead, adopt a broader diagnostic approach. Formulate multiple hypotheses for the root cause (e.g., path issues, permissions, resource lifecycles, library interactions). Instrument the code around the failure point with detailed logging, assertions, and contextual error messages (`map_err`) to pinpoint the exact failure location and state. Verify assumptions, like directory existence or file accessibility, just before the failing operation. This systematic approach helps identify the true cause, such as premature temporary file cleanup or incorrect path handling, more efficiently than isolated fixes.

The key is to reduce assumptions and verify types and paths by looking directly at the relevant source code definitions and re-exports when compiler 
errors become stubborn.                                                                                                                              


TODO:  add a requirement that for each implementation step, we take a few steps to review everything newly written for for accuracy and reasonablenes and good software practices, and that it follows the rest of this sis_guide.md . and then double check that any before writing the tests and moving on

NOTE:  when the AI agent interacts with the human, it should avoid pleasantries whenever possible.  no need for thank you. we want the conversation to be efficient and concise.


## E. Development Workflow Notes

### Code Changes:

Do not generate any SEARCH/REPLACE blocks or suggest code modifications for any step until explicitly asked by the user to do so for that specific step.


### Troubleshooting:

When encountering persistent compiler errors (like type mismatches, unresolved paths, or missing methods), avoid excessive trial-and-error. Instead, **prioritize understanding the involved types and interfaces.** Ask the AI assistant to show the definitions of relevant structs, enums, traits, and functions. Examine their fields (public vs. private), methods, and implemented traits. Additionally, request examples of similar code usage elsewhere in the Meilisearch codebase. This direct inspection often reveals the root cause (e.g., incorrect type usage, privacy issues, missing trait implementations, or incorrect method calls) much faster than repeated code change attempts.

When the AI LLM coder (like aider) is trying to debug and fix errors, it should feel encouraged to ask to look at any other files that might be helpful.

Debugging:

 1 Look at Definitions Sooner: When facing persistent type errors (E0223, E0308, E0433, E0599) or ambiguity, don't spend too long trying path        
   variations. Ask to see the definition of the relevant struct/enum/trait and the function/method being called much earlier. This would have quickly
   revealed that EmbeddingSettings was a struct and that milli::OrderBy was the wrong enum.                                                          
 2 Verify Re-exports: When using paths like crate::some_module::Type, be mindful that some_module might be a re-export. If errors persist, ask to see
   the lib.rs or mod.rs file where the re-export occurs (pub use ...) and potentially the lib.rs of the original crate to confirm exactly which type 
   is being re-exported, especially if names might collide (like OrderBy).                                                                           
 3 Trust Specific Error Codes (Sometimes): While E0599 was misleading due to the underlying E0223, the E0308 (mismatched types) and E0433 (failed to 
   resolve path) errors were quite accurate once the major ambiguity was gone. Pay close attention to the expected vs. found types in E0308.         
 4 Look for Examples: Yes, being more aggressive about asking for examples of similar code elsewhere in the Meilisearch codebase would likely have   
   shown the correct way to construct EmbeddingSettings or set the sort_facet_values_by option much faster. Existing tests or core logic often       
   provide the best patterns.                                                                                                                        
 5 Clean Builds: While it didn't solve the core issue here, running cargo clean periodically during complex debugging can rule out stale build       
   artifacts causing strange behavior.                                                                                                               

When debugging persistent test failures, avoid sequential trial-and-error fixes. Instead, adopt a broader diagnostic approach. Formulate multiple hypotheses for the root cause (e.g., path issues, permissions, resource lifecycles, library interactions). Instrument the code around the failure point with detailed logging, assertions, and contextual error messages (`map_err`) to pinpoint the exact failure location and state. Verify assumptions, like directory existence or file accessibility, just before the failing operation. This systematic approach helps identify the true cause, such as premature temporary file cleanup or incorrect path handling, more efficiently than isolated fixes.

The key is to reduce assumptions and verify types and paths by looking directly at the relevant source code definitions and re-exports when compiler 
errors become stubborn.                                                                                                                              


TODO:  add a requirement that for each implementation step, we take a few steps to review everything newly written for for accuracy and reasonablenes and good software practices, and that it follows the rest of this sis_guide.md . and then double check that any before writing the tests and moving on

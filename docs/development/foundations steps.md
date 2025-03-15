NOTE: we are droppign 1F because we cant test scheduling without actual snapshots and i dont wanna deal with mocking for no reason.  also, 1E did not include any tests for scheduler


# Phase 1 Refined - Foundation Steps (6 Steps)

## Step 1A: Basic Task Type Definitions
**Implementation Details:**
- Add to `meilisearch-types/src/tasks.rs`:
  - New variants `SingleIndexSnapshotCreation` and `SingleIndexSnapshotImport` to `Kind` enum
  - Corresponding variants to `KindWithContent` enum with required fields
  - Update `Display` and `FromStr` trait implementations
  - Implement JSON serialization with proper camelCase naming
- Add comment documentation explaining each task type's purpose

**Testing Notes:**
- Test serialization to verify correct JSON format (camelCase)
- Test deserialization from JSON
- Test `FromStr` implementation with various inputs
- Test equality comparisons and conversions between `Kind` and `KindWithContent`

## Step 1B: Task Details Schema
**Implementation Details:**
- Add to `meilisearch-types/src/tasks.rs`:
  - New variants to `Details` enum:
    - `SingleIndexSnapshotCreation { index_uid: String, snapshot_path: Option<String> }`
    - `SingleIndexSnapshotImport { source_path: String, index_uid: String, imported_documents: Option<u64> }`
  - Update `default_details()` method to generate appropriate details for new task types
  - Update `default_finished_details()` to set success markers for imported documents
  - Ensure proper JSON serialization for all fields

**Testing Notes:**
- Test default details generation for new task types
- Verify serialization/deserialization with various field values
- Test conversion between task KindWithContent and corresponding Details

## Step 1C: Error Types
**Implementation Details:**
- Add to `index-scheduler/src/error.rs`:
  - `SingleIndexSnapshotCreationError` with various specific failure modes
  - `SingleIndexSnapshotImportError` with import-specific failures
  - `SnapshotVersionMismatch` with fields for expected vs. actual version
  - `SnapshotFileNotFound` with path field
  - Update the `Error` enum to include these new error variants
  - Implement `Display` and error conversions for all new types

**Testing Notes:**
- Test error construction and message formatting
- Verify error chains propagate correctly
- Test conversion from lower-level errors (I/O, compression)
- Test error response serialization to ensure proper client error reporting

## Step 1D: Progress Tracking
**Implementation Details:**
- Add to `index-scheduler/src/processing.rs`:
  - Use `make_enum_progress!` macro to define:
    - `SingleIndexSnapshotCreationProgress` with steps like `StartingSnapshot`, `CopyingIndexData`, `CreatingArchive`
    - `SingleIndexSnapshotImportProgress` with steps like `ExtractingSnapshot`, `ValidatingData`, `CreatingIndex`
  - Implement default first/last step getters
  - Set up proper display formatting for progress steps

**Testing Notes:**
- Test progress step ordering
- Verify progress reporting integration with the scheduler's update system
- Test display formatting of progress steps for logs and API

## Step 1E: Task Validation
**Implementation Details:**
- Modify `index-scheduler/src/scheduler.rs`:
  - Update `register()` method to validate new task types
  - Add index existence validation for SingleIndexSnapshotCreation
  - Verify snapshot path exists for SingleIndexSnapshotImport
  - Implement content file generation for task storage
  - Update batching logic to properly handle new tasks

**Testing Notes:**
- Test registration with valid parameters 
- Test error cases with non-existent indexes
- Test error cases with invalid snapshot paths
- Verify task content (UUIDs, parameters) is properly stored

## Step 1F: Scheduler Priority and Dependencies
**Implementation Details:**
- Modify `index-scheduler/src/scheduler/mod.rs`:
  - Update `should_process_first()` to prioritize single index snapshots
  - Modify `should_be_processed()` to identify tasks affecting the same index
  - Implement index-specific dependency tracking
  - Add logic to prevent concurrent index modifications during snapshot operations
  - Update task query logic to provide proper filtering

**Testing Notes:**
- Test task ordering when multiple different priority tasks exist
- Verify conflicting tasks (same index operations) are properly held
- Test that snapshot tasks block other operations on the same index
- Verify snapshot import tasks properly handle target index conflicts

Each step builds incrementally with focused tests that verify its functionality in isolation, creating a solid foundation for implementing the actual snapshot processes.
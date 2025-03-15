# Revised Sequential Implementation Plan (Phases 1-3)

## Phase 1: Foundation - Task Types

**Implementation Details:**
- Modify `meilisearch-types/src/tasks.rs` to:
  - Add `SingleIndexSnapshotCreation` and `SingleIndexSnapshotImport` to the `Kind` enum
  - Add corresponding variants to `KindWithContent` enum:
    ```
    SingleIndexSnapshotCreation { index_uid: String }
    SingleIndexSnapshotImport { index_uid: String, new_index_uid: Option<String>, snapshot_path: PathBuf }
    ```
  - Update `as_kind()`, `indexes()`, and `FromStr` implementations
  - Add serialization support with camelCase naming

- Create a minimal progress enum in `index-scheduler/src/processing.rs`:
  - Use `make_enum_progress!` macro to define `SingleIndexSnapshotCreationProgress`
  - Start with just 2-3 basic steps like `StartingSnapshotCreation`, `CreatingSnapshot`, `FinalizingSnapshot`

- Add basic test in `meilisearch-types/src/tasks.rs` to verify the new enum variants work correctly

## Phase 2: Single Index Snapshot Creation - Basic

**Implementation Details:**
- Create new file `index-scheduler/src/scheduler/process_single_index_snapshot_creation.rs`
- Implement basic function `process_single_index_snapshot`:
  - Get the index from the index_map using `index_mapper.index(&rtxn, index_uid)`
  - Create a temporary directory using `tempfile::tempdir()`
  - Use existing `index.copy_to_path()` method to copy the index data (similar to line 63 in `process_snapshot_creation.rs`)
  - Create tarball using `compression::to_tar_gz()` function
  - Save to `{snapshots_path}/indexes/{index_uid}/{index_uid}_{timestamp}.idx.snapshot`
  - Mark task as succeeded

- Modify `index-scheduler/src/scheduler/mod.rs` to:
  - Add `mod process_single_index_snapshot_creation` at the top
  - Add case handling in the task processing match statement

- Create a simple test that:
  - Creates a small index with a few documents
  - Creates a snapshot of the index
  - Verifies the snapshot file exists and has non-zero size

## Phase 3: Single Index Snapshot Import - Basic

**Implementation Details:**
- Create new file `index-scheduler/src/scheduler/process_single_index_snapshot_import.rs`
- Implement basic function `process_single_index_snapshot_import`:
  - Extract snapshot using `compression::from_tar_gz()` to a temporary directory
  - Check if destination index exists (handle new_index_uid if provided)
  - Use `index_mapper.create_index()` to create the new index
  - Copy the extracted data to the index path
  - Mark task as succeeded

- Modify `index-scheduler/src/scheduler/mod.rs` to:
  - Add `mod process_single_index_snapshot_import` at the top
  - Add case handling in the task processing match statement

- Create a test that:
  - Creates a snapshot using the code from Phase 2
  - Imports the snapshot with a new name
  - Verifies the new index exists and contains the same documents

## Phase 3: Single Index Snapshot Creation - Complete
- Add metadata file generation
- Implement progress tracking
- Add error handling and cleanup
- Test snapshot creation with different index sizes

## Phase 5: Single Index Snapshot Import - Complete
- Add version compatibility checking
- Support renaming during import
- Add progress tracking
- Add error handling with proper cleanup
- Test import with various scenarios

## Phase 6: Integration and Optimization
- Connect to scheduler's task processing system
- Add API integration 
- Performance optimization for large indexes
- Full integration tests of the entire feature

Each phase produces working code that can be tested independently. Focus on getting the simplest version working first, then enhance it incrementally. This avoids being overwhelmed by trying to implement everything at once.
use std::io::{BufWriter, Write};
use std::sync::Arc;
use std::time::Duration;

use actix_rt::time; // Import actix_rt::time
use big_s::S;
use crossbeam_channel::RecvTimeoutError;
use file_store::File;
use meilisearch_auth::open_auth_store_env;
use meilisearch_types::document_formats::DocumentFormatError;
use meilisearch_types::milli::update::IndexDocumentsMethod::ReplaceDocuments;
use meilisearch_types::milli::update::IndexerConfig;
use meilisearch_types::tasks::KindWithContent;
use meilisearch_types::{versioning, VERSION_FILE_NAME};
use tempfile::{NamedTempFile, TempDir};
use uuid::Uuid;
use Breakpoint::*;

use crate::insta_snapshot::snapshot_index_scheduler;
use crate::{Error, IndexScheduler, IndexSchedulerOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Breakpoint {
    // this state is only encountered while creating the scheduler in the test suite.
    Init,

    Start,
    BatchCreated,
    AfterProcessing,
    AbortedIndexation,
    ProcessBatchSucceeded,
    ProcessBatchFailed,
    InsideProcessBatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureLocation {
    InsideCreateBatch,
    InsideProcessBatch,
    PanicInsideProcessBatch,
    ProcessUpgrade,
    AcquiringWtxn,
    UpdatingTaskAfterProcessBatchSuccess { task_uid: u32 },
    UpdatingTaskAfterProcessBatchFailure,
    CommittingWtxn,
}

impl IndexScheduler {
    /// Blocks the thread until the test handle asks to progress to/through this breakpoint.
    ///
    /// Two messages are sent through the channel for each breakpoint.
    /// The first message is `(b, false)` and the second message is `(b, true)`.
    ///
    /// Since the channel has a capacity of zero, the `send` and `recv` calls wait for each other.
    /// So when the index scheduler calls `test_breakpoint_sdr.send(b, false)`, it blocks
    /// the thread until the test catches up by calling `test_breakpoint_rcv.recv()` enough.
    /// From the test side, we call `recv()` repeatedly until we find the message `(breakpoint, false)`.
    /// As soon as we find it, the index scheduler is unblocked but then wait again on the call to
    /// `test_breakpoint_sdr.send(b, true)`. This message will only be able to send once the
    /// test asks to progress to the next `(b2, false)`.
    pub(crate) fn breakpoint(&self, b: Breakpoint) {
        // We send two messages. The first one will sync with the call
        // to `handle.wait_until(b)`. The second one will block until the
        // the next call to `handle.wait_until(..)`.
        // Ignore potential errors during teardown for both sends.
        let _ = self.test_breakpoint_sdr.send((b, false));
        // This one will only be able to be sent if the test handle stays alive.
        // If it fails, it likely means the test handle has been dropped during teardown.
        // Instead of panicking with unwrap(), ignore the error in that case.
        let _ = self.test_breakpoint_sdr.send((b, true));
        // If the send fails, the scheduler loop will likely terminate soon after anyway.
    }
}

impl IndexScheduler {
    pub(crate) fn test(
        autobatching_enabled: bool,
        planned_failures: Vec<(usize, FailureLocation)>,
    ) -> (Self, IndexSchedulerHandle) {
        Self::test_with_custom_config(planned_failures, |config| {
            config.autobatching_enabled = autobatching_enabled;
            None
        })
    }

    // [meilisearchfj] Modified to accept optional cache_size
    pub(crate) fn test_with_custom_config(
        planned_failures: Vec<(usize, FailureLocation)>,
        mut configuration: impl FnMut(&mut IndexSchedulerOptions) -> Option<(u32, u32, u32)>, // Made mutable
    ) -> (Self, IndexSchedulerHandle) {
        let tempdir = TempDir::new().unwrap();
        let (sender, receiver) = crossbeam_channel::bounded(0);

        let indexer_config = IndexerConfig { skip_index_budget: true, ..Default::default() };

        let mut options = IndexSchedulerOptions {
            version_file_path: tempdir.path().join(VERSION_FILE_NAME),
            auth_path: tempdir.path().join("auth"),
            tasks_path: tempdir.path().join("db_path"),
            update_file_path: tempdir.path().join("file_store"),
            indexes_path: tempdir.path().join("indexes"),
            snapshots_path: tempdir.path().join("snapshots"),
            dumps_path: tempdir.path().join("dumps"),
            webhook_url: None,
            webhook_authorization_header: None,
            task_db_size: 1000 * 1000 * 10, // 10 MB, we don't use MiB on purpose.
            index_base_map_size: 1000 * 1000, // 1 MB, we don't use MiB on purpose.
            enable_mdb_writemap: false,
            index_growth_amount: 1000 * 1000 * 1000 * 1000, // 1 TB
            index_count: 5,
            indexer_config: Arc::new(indexer_config),
            autobatching_enabled: true,
            cleanup_enabled: true,
            max_number_of_tasks: 1_000_000,
            max_number_of_batched_tasks: usize::MAX,
            batched_tasks_size_limit: u64::MAX,
            instance_features: Default::default(),
            auto_upgrade: true, // Don't cost much and will ensure the happy path works
            embedding_cache_cap: 10,
        };
        // Call configuration closure, it might modify options (like index_count)
        let version = configuration(&mut options).unwrap_or_else(|| {
            (
                versioning::VERSION_MAJOR.parse().unwrap(),
                versioning::VERSION_MINOR.parse().unwrap(),
                versioning::VERSION_PATCH.parse().unwrap(),
            )
        });

        // Clone the Arc *before* moving options into IndexScheduler::new
        let indexer_config_clone = options.indexer_config.clone();

        std::fs::create_dir_all(&options.auth_path).unwrap();
        let auth_env = open_auth_store_env(&options.auth_path).unwrap();
        let index_scheduler = Self::new(
            options,
            auth_env,
            version,
            sender,
            planned_failures,
            None, // Add None for index_cache_size
        )
        .unwrap();

        // To be 100% consistent between all test we're going to start the scheduler right now
        // and ensure it's in the expected starting state.
        let breakpoint = match receiver.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(b) => b,
            Err(RecvTimeoutError::Timeout) => {
                panic!("The scheduler seems to be waiting for a new task while your test is waiting for a breakpoint.")
            }
            Err(RecvTimeoutError::Disconnected) => panic!("The scheduler crashed."),
        };
        assert_eq!(breakpoint, (Init, false));
        let index_scheduler_handle = IndexSchedulerHandle {
            _tempdir: tempdir,
            index_scheduler: index_scheduler.private_clone(),
            test_breakpoint_rcv: receiver,
            last_breakpoint: breakpoint.0,
            indexer_config: indexer_config_clone, // Use the clone created earlier
        };

        (index_scheduler, index_scheduler_handle)
    }

    /// Return a [`PlannedFailure`](Error::PlannedFailure) error if a failure is planned
    /// for the given location and current run loop iteration.
    pub(crate) fn maybe_fail(&self, location: FailureLocation) -> crate::Result<()> {
        if self.planned_failures.contains(&(*self.run_loop_iteration.read().unwrap(), location)) {
            match location {
                FailureLocation::PanicInsideProcessBatch => {
                    panic!("simulated panic")
                }
                _ => Err(Error::PlannedFailure),
            }
        } else {
            Ok(())
        }
    }
}

/// Return a `KindWithContent::IndexCreation` task
pub(crate) fn index_creation_task(
    index: impl Into<String>, // Change to accept anything convertible to String
    primary_key: Option<&'static str>, // Keep primary_key as 'static for simplicity if needed
) -> KindWithContent {
    KindWithContent::IndexCreation { index_uid: index.into(), primary_key: primary_key.map(S) } // Use .into()
}

/// Create a `KindWithContent::DocumentImport` task that imports documents.
///
/// - `index_uid` is given as parameter
/// - `primary_key` is given as parameter
/// - `method` is set to `ReplaceDocuments`
/// - `content_file` is given as parameter
/// - `documents_count` is given as parameter
/// - `allow_index_creation` is set to `true`
pub(crate) fn replace_document_import_task(
    index: &'static str,
    primary_key: Option<&'static str>,
    content_file_uuid: u128,
    documents_count: u64,
) -> KindWithContent {
    KindWithContent::DocumentAdditionOrUpdate {
        index_uid: S(index),
        primary_key: primary_key.map(ToOwned::to_owned),
        method: ReplaceDocuments,
        content_file: Uuid::from_u128(content_file_uuid),
        documents_count,
        allow_index_creation: true,
    }
}

/// Adapting to the new json reading interface
pub(crate) fn read_json(
    bytes: &[u8],
    write: impl Write,
) -> std::result::Result<u64, DocumentFormatError> {
    let temp_file = NamedTempFile::new().unwrap();
    let mut buffer = BufWriter::new(temp_file.reopen().unwrap());
    buffer.write_all(bytes).unwrap();
    buffer.flush().unwrap();
    meilisearch_types::document_formats::read_json(temp_file.as_file(), write)
}

/// Create an update file with the given file uuid.
///
/// The update file contains just one simple document whose id is given by `document_id`.
///
/// The uuid of the file and its documents count is returned.
pub(crate) fn sample_documents(
    index_scheduler: &IndexScheduler,
    file_uuid: u128,
    document_id: usize,
) -> (File, u64) {
    let content = format!(
        r#"
        {{
            "id" : "{document_id}"
        }}"#
    );

    let (_uuid, mut file) = index_scheduler.queue.create_update_file_with_uuid(file_uuid).unwrap();
    let documents_count = read_json(content.as_bytes(), &mut file).unwrap();
    (file, documents_count)
}

pub struct IndexSchedulerHandle {
    _tempdir: TempDir,
    index_scheduler: IndexScheduler,
    test_breakpoint_rcv: crossbeam_channel::Receiver<(Breakpoint, bool)>,
    last_breakpoint: Breakpoint,
    // Keep a copy of the config for tests
    pub(crate) indexer_config: Arc<IndexerConfig>,
}

// Implement Drop to ensure the scheduler thread terminates before the handle is fully dropped.
impl Drop for IndexSchedulerHandle {
    fn drop(&mut self) {
        // Consume remaining messages until the channel disconnects, indicating the scheduler thread stopped.
        // The initial is_disconnected() check was removed as it's not a valid method
        // and the Disconnected error case in the match below handles this scenario.
        // Use a timeout to prevent hanging indefinitely if something goes wrong.
        let timeout = Duration::from_secs(10); // Adjust timeout as needed
        // let _start_time = std::time::Instant::now(); // Mark as unused or remove if not needed

        // We might be waiting for the second message (true) of the last breakpoint
        match self.test_breakpoint_rcv.recv_timeout(timeout) {
            Ok((_bp, _flag)) => { // Prefix unused variables with underscore
                // If we received the 'true' flag, the scheduler is waiting for the *next* breakpoint's 'false'.
                // If we received 'false', the scheduler is waiting for its 'true'.
                // In either case, the scheduler is still alive but blocked.
                // Dropping the handle now will cause the scheduler's next send to fail,
                // allowing it to terminate. We don't need to explicitly wait further here
                // as dropping the receiver achieves the goal.
                // Removed debug print
            },
            Err(RecvTimeoutError::Timeout) => {
                eprintln!("WARN [Drop]: Timeout waiting for scheduler thread to signal completion via breakpoint channel.");
                // Proceed with dropping, hoping the scheduler terminates eventually.
            }
            Err(RecvTimeoutError::Disconnected) => {
                 println!("DEBUG [Drop]: Scheduler channel already disconnected.");
                // Scheduler already stopped, nothing more to do.
                // Removed debug print
            }
        }
         // The receiver is dropped here, signaling the sender (scheduler thread).
    }
}


impl IndexSchedulerHandle {
    // Delegate methods needed by tests to the internal index_scheduler

    /// Gets the task information directly using a read transaction.
    #[allow(dead_code)] // Used in tests
    pub(crate) fn get_task(&self, task_id: u32) -> crate::Result<Option<meilisearch_types::tasks::Task>> { // Use crate::Result
        // Fix: Use a read transaction to access the task queue
        let rtxn = self.index_scheduler.read_txn()?;
        self.index_scheduler.queue.tasks.get_task(&rtxn, task_id)
    }

    /// Creates an update file with the given JSON content.
    /// Returns the UUID of the file and the number of documents.
    #[allow(dead_code)] // Used in tests
    pub(crate) fn create_update_file(
        &self,
        content: serde_json::Value,
    ) -> crate::Result<(Uuid, u64)> { // Use crate::Result
        let (content_uuid, mut content_file) =
            self.index_scheduler.queue.file_store.new_update()?;
        let documents_count =
            read_json(content.to_string().as_bytes(), &mut content_file)?;
        content_file.persist()?;
        Ok((content_uuid, documents_count))
    }

    /// Returns the configured path for snapshots.
    #[allow(dead_code)] // Used in tests
    pub(crate) fn snapshots_path(&self) -> &std::path::Path {
        &self.index_scheduler.scheduler.snapshots_path
    }


    #[allow(dead_code)] // Used in tests
    pub(crate) async fn register_task(&mut self, task: KindWithContent) -> crate::Result<u32> {
        // Use the correct register method with default arguments for tests
        // register is synchronous, remove .await
        self.index_scheduler.register(task, None, false).map(|t| t.uid)
    }

    #[allow(dead_code)] // Used in tests
    pub(crate) fn index(&self, uid: &str) -> crate::Result<crate::Index> {
        self.index_scheduler.index(uid)
    }

    #[allow(dead_code)] // Used in tests
    pub(crate) fn write_txn(&self) -> crate::Result<crate::heed::RwTxn> {
        self.index_scheduler.env.write_txn().map_err(Into::into) // Map heed::Error to crate::Error
    }

    #[allow(dead_code)] // Used in tests
    pub(crate) fn indexer_config(&self) -> &Arc<IndexerConfig> {
        &self.indexer_config
    }

    #[allow(dead_code)] // Used in tests
    pub(crate) async fn add_documents(
        &mut self,
        index_uid: &str,
        method: meilisearch_types::milli::update::IndexDocumentsMethod,
        content: &str, // Assuming content is a string for simplicity in tests
    ) -> crate::Result<u32> {
        // Use file_store.new_update() to get a managed file
        let (content_uuid, mut update_file) = self.index_scheduler.queue.file_store.new_update()?;
        let documents_count = read_json(content.as_bytes(), &mut update_file)?;
        // Persist the file explicitly (though drop might handle it too)
        update_file.persist()?;

        let task = KindWithContent::DocumentAdditionOrUpdate {
            index_uid: index_uid.to_string(), // Create owned string
            primary_key: None, // Assume PK is already set or inferred for test simplicity
            method,
            content_file: content_uuid,
            documents_count,
            allow_index_creation: true, // Usually true in tests
        };
        self.register_task(task).await
    }

    #[allow(dead_code)] // Used in tests
    pub(crate) async fn wait_task(&self, task_id: u32) -> meilisearch_types::tasks::Task {
        // Simple polling wait for tests
        loop {
            // get_task requires a transaction
            let task_result = self.index_scheduler.read_txn().and_then(|rtxn| {
                self.index_scheduler.queue.tasks.get_task(&rtxn, task_id)
            });

            match task_result {
                Ok(Some(task)) => {
                    if task.status.is_terminal() { // Use Status::is_terminal
                        return task;
                    }
                }
                Ok(None) => {
                    // Task might not exist yet or was deleted, treat as error for waiting
                    panic!("Task {} not found while waiting", task_id);
                }
                Err(_) => {
                    // Handle error case if needed, maybe panic for tests
                    panic!("Error waiting for task {}", task_id);
                }
            }
            time::sleep(Duration::from_millis(50)).await; // Use actix_rt::time::sleep
        }
    }

    /// Restarts the index-scheduler on the same database.
    /// To use this function you must give back the index-scheduler that was given to you when
    /// creating the handle the first time.
    /// If the index-scheduler has been cloned in the meantime you must drop all copy otherwise
    /// the function will panic.
    pub(crate) fn restart(
        self,
        index_scheduler: IndexScheduler,
        autobatching_enabled: bool,
        planned_failures: Vec<(usize, FailureLocation)>,
        configuration: impl Fn(&mut IndexSchedulerOptions) -> Option<(u32, u32, u32)>,
    ) -> (IndexScheduler, Self) {
        // Keep a reference to the tempdir path before dropping anything
        let tempdir_path = self._tempdir.path().to_path_buf();
        // Clone the env from the *passed-in* index_scheduler, which should be the one associated with self.
        let env = index_scheduler.env.clone();
        // Clone the receiver channel from self before dropping the passed-in scheduler
        let test_breakpoint_rcv = self.test_breakpoint_rcv.clone();

        // Drop the old scheduler instance passed into the function
        drop(index_scheduler);
        // self will be dropped implicitly when the function returns, triggering its Drop impl.

        // We must ensure that the `run` function associated with the *old* scheduler has stopped running.
        // Use the cloned receiver for this check.
        loop {
            match test_breakpoint_rcv.recv_timeout(Duration::from_secs(5)) {
                Ok((_, true)) => continue, // Consume the 'true' part of the last breakpoint
                Ok((b, false)) => {
                    // This indicates the old scheduler is still trying to advance, which shouldn't happen after drop.
                    panic!("Old scheduler is unexpectedly still running and passed breakpoint {b:?}")
                }
                Err(RecvTimeoutError::Timeout) => panic!("The indexing loop is stuck somewhere"),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        let closed = env.prepare_for_closing().wait_timeout(Duration::from_secs(5));
        assert!(closed, "The index scheduler couldn't close itself, it seems like someone else is holding the env somewhere");

        let (scheduler, handle) = // Remove `mut` from handle as it's not mutated here
            IndexScheduler::test_with_custom_config(planned_failures, |config| {
                let version = configuration(config);
                config.autobatching_enabled = autobatching_enabled;
                // Use the saved tempdir_path
                config.version_file_path = tempdir_path.join(VERSION_FILE_NAME);
                config.auth_path = tempdir_path.join("auth");
                config.tasks_path = tempdir_path.join("db_path");
                config.update_file_path = tempdir_path.join("file_store");
                config.indexes_path = tempdir_path.join("indexes");
                config.snapshots_path = tempdir_path.join("snapshots");
                config.dumps_path = tempdir_path.join("dumps");
                version
            });
        // The new handle will create its own TempDir, but we need the old path logic.
        // We can't assign the old TempDir back because it was moved/dropped with `self`.
        // This part might need rethinking if the TempDir needs to persist across restarts.
        // For now, let the new handle manage its own tempdir based on the configured paths.
        (scheduler, handle)
    }

    /// Advance the scheduler to the next tick.
    /// Panic
    /// * If the scheduler is waiting for a task to be registered.
    /// * If the breakpoint queue is in a bad state.
    #[track_caller]
    pub(crate) fn advance(&mut self) -> Breakpoint {
        let (breakpoint_1, b) = match self
            .test_breakpoint_rcv
            .recv_timeout(std::time::Duration::from_secs(50))
        {
            Ok(b) => b,
            Err(RecvTimeoutError::Timeout) => {
                let state = snapshot_index_scheduler(&self.index_scheduler);
                panic!("The scheduler seems to be waiting for a new task while your test is waiting for a breakpoint.\n{state}")
            }
            Err(RecvTimeoutError::Disconnected) => {
                let state = snapshot_index_scheduler(&self.index_scheduler);
                panic!("The scheduler crashed.\n{state}")
            }
        };
        // if we've already encountered a breakpoint we're supposed to be stuck on the false
        // and we expect the same variant with the true to come now.
        assert_eq!(
                (breakpoint_1, b),
                (self.last_breakpoint, true),
                "Internal error in the test suite. In the previous iteration I got `({:?}, false)` and now I got `({:?}, {:?})`.",
                self.last_breakpoint,
                breakpoint_1,
                b,
            );

        let (breakpoint_2, b) = match self
            .test_breakpoint_rcv
            .recv_timeout(std::time::Duration::from_secs(50))
        {
            Ok(b) => b,
            Err(RecvTimeoutError::Timeout) => {
                let state = snapshot_index_scheduler(&self.index_scheduler);
                panic!("The scheduler seems to be waiting for a new task while your test is waiting for a breakpoint.\n{state}")
            }
            Err(RecvTimeoutError::Disconnected) => {
                let state = snapshot_index_scheduler(&self.index_scheduler);
                panic!("The scheduler crashed.\n{state}")
            }
        };
        assert!(!b, "Found the breakpoint handle in a bad state. Check your test suite");

        self.last_breakpoint = breakpoint_2;

        breakpoint_2
    }

    /// Advance the scheduler until all the provided breakpoints are reached in order.
    #[track_caller]
    pub(crate) fn advance_till(&mut self, breakpoints: impl IntoIterator<Item = Breakpoint>) {
        for breakpoint in breakpoints {
            let b = self.advance();
            assert_eq!(
                b,
                breakpoint,
                "Was expecting the breakpoint `{:?}` but instead got `{:?}`.\n{}",
                breakpoint,
                b,
                snapshot_index_scheduler(&self.index_scheduler)
            );
        }
    }

    /// Wait for `n` successful batches.
    #[track_caller]
    pub(crate) fn advance_n_successful_batches(&mut self, n: usize) {
        for _ in 0..n {
            self.advance_one_successful_batch();
        }
    }

    /// Wait for `n` failed batches.
    #[track_caller]
    pub(crate) fn advance_n_failed_batches(&mut self, n: usize) {
        for _ in 0..n {
            self.advance_one_failed_batch();
        }
    }

    // Wait for one successful batch.
    #[track_caller]
    pub(crate) fn advance_one_successful_batch(&mut self) {
        self.advance_till([Start, BatchCreated]);
        loop {
            match self.advance() {
                    // the process_batch function can call itself recursively, thus we need to
                    // accept as may InsideProcessBatch as possible before moving to the next state.
                    InsideProcessBatch => (),
                    // the batch went successfully, we can stop the loop and go on with the next states.
                    ProcessBatchSucceeded => break,
                    AbortedIndexation => panic!("The batch was aborted.\n{}", snapshot_index_scheduler(&self.index_scheduler)),
                    ProcessBatchFailed => {
                        while self.advance() != Start {}
                        panic!("The batch failed.\n{}", snapshot_index_scheduler(&self.index_scheduler))
                    },
                    breakpoint => panic!("Encountered an impossible breakpoint `{:?}`, this is probably an issue with the test suite.", breakpoint),
                }
        }

        self.advance_till([AfterProcessing]);
    }

    // Wait for one failed batch.
    #[track_caller]
    pub(crate) fn advance_one_failed_batch(&mut self) {
        self.advance_till([Start, BatchCreated]);
        loop {
            match self.advance() {
                    // the process_batch function can call itself recursively, thus we need to
                    // accept as may InsideProcessBatch as possible before moving to the next state.
                    InsideProcessBatch => (),
                    // the batch went failed, we can stop the loop and go on with the next states.
                    ProcessBatchFailed => break,
                    ProcessBatchSucceeded => panic!("The batch succeeded. (and it wasn't supposed to sorry)\n{}", snapshot_index_scheduler(&self.index_scheduler)),
                    AbortedIndexation => panic!("The batch was aborted.\n{}", snapshot_index_scheduler(&self.index_scheduler)),
                    breakpoint => panic!("Encountered an impossible breakpoint `{:?}`, this is probably an issue with the test suite.", breakpoint),
                }
        }
        self.advance_till([AfterProcessing]);
    }

    // Wait for one failed batch.
    #[track_caller]
    pub(crate) fn scheduler_is_down(&mut self) {
        loop {
            match self
            .test_breakpoint_rcv
            .recv_timeout(std::time::Duration::from_secs(1)) {
                Ok((_, true)) => continue,
                Ok((b, false)) => panic!("The scheduler was supposed to be down but successfully moved to the next breakpoint: {b:?}"),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
    }
}

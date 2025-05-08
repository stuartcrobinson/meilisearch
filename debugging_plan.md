# Debugging Plan: `test_fj_create_index_snapshot_success` 500 Error

## 1. Problem Overview

The test `routes::fj_snapshot::msfj_sis_api_handler_tests::test_fj_create_index_snapshot_success` is failing.
It expects an HTTP 200 OK status but receives an HTTP 500 Internal Server Error.
The panic message `assertion left == right failed: Response: ... left: 500 right: 200` indicates the test assertion itself is what's panicking because the response status is not 200. The 500 error originates from within the Actix request handling.

## 2. Initial Diagnosis and Hypothesis

An HTTP 500 error in Actix typically means one of the following occurred within the request handler or middleware:
*   A panic.
*   An `Err` was returned from a function using the `?` operator, and this error was not converted into a specific HTTP error response by the handler code, leading Actix to default to a 500.
*   An extractor (like `Data<T>`, `web::Path<T>`, `GuardedData`) failed.

**Primary Hypothesis: `Data<AuthController>` Mismatch in Test Setup**

The `GuardedData` extractor is used in the `fj_create_index_snapshot` handler:
```rust
index_scheduler: GuardedData<ActionPolicy<{ actions::SNAPSHOTS_CREATE }>, Data<S>>,
```
Looking at the implementation of `GuardedData` (in `crates/meilisearch/src/extractors/authentication/mod.rs`), it internally attempts to extract `Data<P::Auth>` where `P` is the policy (`ActionPolicy` in this case). The associated type `P::Auth` for `ActionPolicy` is `meilisearch_auth::AuthController`.

```rust
// Relevant part from GuardedData::from_request
let auth_controller_fut = Data::<P::Auth>::from_request(req, payload_copy);
```

So, `GuardedData` expects `Data<AuthController>` to be available in the Actix application data.

However, the test `test_fj_create_index_snapshot_success` sets up the Actix application with:
```rust
let mock_auth_controller = MockAuthController;
let app_data_auth = web::Data::new(mock_auth_controller);
// ...
App::new().app_data(app_data_auth.clone()) // Provides Data<MockAuthController>
```
Actix's `Data<T>::from_request` extractor requires an exact type match for `T`. When `GuardedData` tries to extract `Data<AuthController>`, it will not find it because the application data contains `Data<MockAuthController>`. This failure in the `Data` extractor will result in an error that Actix converts to an HTTP 500 response.

This mismatch is the most probable cause of the 500 error.

## 3. Solution Strategy

To resolve this, the test environment needs to provide `Data<AuthController>` as expected by `GuardedData`.

1.  **Use Real `AuthController`**: Instead of `MockAuthController`, instantiate a real `meilisearch_auth::AuthController`.
2.  **Configure for "No Master Key"**:
    *   The `default_opt_with_replication()` function in the test already creates an `Opt` with `master_key: None`.
    *   Initialize the `AuthController` with a temporary `heed::Env` (for its database) and this `None` master key.
    *   When no master key is set, `AuthController::is_api_key_set()` should return `false`.
    *   This will cause `ActionPolicy::check` to grant access, as per its logic:
        ```rust
        // crates/meilisearch/src/extractors/authentication/policies.rs
        if auth.is_api_key_set() {
            // ... logic for checking API key ...
        } else {
            // No master key set, all allowed
            Ok(())
        }
        ```
3.  **Minimal Changes**: This approach modifies only the test setup, adhering to the principle of minimizing changes to the core Meilisearch FOSS code.

## 4. Step-by-Step Guide to Fix the Test

Modify `crates/meilisearch/src/routes/fj_snapshot.rs` within the `msfj_sis_api_handler_tests` module:

1.  **Remove `MockAuthController`**: Delete the `MockAuthController` struct definition.
2.  **Import `AuthController` and `EnvOpenOptions`**:
    ```rust
    use meilisearch_auth::AuthController;
    use meilisearch_types::heed::{EnvOpenOptions, WithoutTls}; // Ensure WithoutTls is imported if not already
    ```
3.  **Update Test Setup in `test_fj_create_index_snapshot_success`**:
    *   Replace `MockAuthController` instantiation with `AuthController` instantiation.
    *   The `Opt` already has a temporary `db_path`. Use this for the `AuthController`'s environment.

    ```rust
    // Before:
    // let mock_auth_controller = MockAuthController;
    // let app_data_auth = web::Data::new(mock_auth_controller);

    // After:
    let opt_for_auth = default_opt_with_replication(); // Or use the existing `app_opt.get_ref()`
    let mut env_options = EnvOpenOptions::<WithoutTls>::new(); // Specify type if ambiguous
    env_options.map_size(100 * 1024 * 1024); // 100MB, example size
    // Ensure opt_for_auth.db_path is unique if multiple tests run in parallel and need distinct dbs
    let auth_env = env_options.open(opt_for_auth.db_path.join("auth_db_test_create")).unwrap(); // Use a sub-path or ensure unique db_path
    let auth_controller = AuthController::new(auth_env, &opt_for_auth.master_key).unwrap();
    let app_data_auth = web::Data::new(auth_controller);
    ```
    *Note: Ensure the `db_path` for the `AuthController`'s `heed::Env` is distinct for this test or managed in a way that doesn't conflict with other tests or the `IndexScheduler`'s environment if they were to use the same path.* Using a subdirectory like `auth_db_test_create` under the main temp `db_path` is a common strategy.

4.  **Run the Test**: Execute `cargo test -p meilisearch -- --nocapture routes::fj_snapshot::msfj_sis_api_handler_tests::test_fj_create_index_snapshot_success` again.

## 5. Address Unused Imports (Cleanup)

The compiler output showed several unused imports in `crates/meilisearch/src/routes/fj_snapshot.rs` within the test module:
*   `meilisearch_types::error::ErrorCode`
*   `meilisearch_types::error::deserr_codes`
*   `serde_json::Value`
*   `std::fs::{self, File}` (specifically `self` and `File` if `fs` is used for `create_dir_all` for example)
*   `std::io::Write`
*   `std::path::PathBuf`

These should be removed if they are indeed not used after the fix, to keep the code clean.

## 6. Further Debugging (If Issue Persists)

If the 500 error remains after applying the fix:
1.  **Enable Backtrace**: Run the test with `RUST_BACKTRACE=1` (e.g., `RUST_BACKTRACE=1 cargo test ...`) to get a detailed stack trace if the 500 is due to a panic.
2.  **Log/Debug Statements**: Add `dbg!(...)` or `println!(...)` statements inside `fj_create_index_snapshot` to trace execution:
    *   At the beginning of the handler.
    *   Before and after the call to `index_scheduler.register_fj_task(...).await`.
    *   Inspect the `task` object returned by `register_fj_task`.
    *   Before `Ok(Json(task.into()))`.
3.  **Simplify Handler Return**: Temporarily change `fj_create_index_snapshot` to return a very simple `Ok(Json("test_string"))` or `Ok(Json(serde_json::json!({"message": "test"})))`. If this works, the problem lies in the `task` object processing or serialization (`task.into()` or `SummarizedTaskView` itself). If it still fails, the issue is earlier, likely still in the setup or guard/extractor phase.
4.  **Inspect `SummarizedTaskView::from_task`**: Review the `From<Task> for SummarizedTaskView` implementation (or `SummarizedTaskView::from_task` method) for any potential panics, though it appears straightforward. The mock task provided by `MockIndexScheduler` seems valid.

## 7. Addressing Compiler Error E0599 for `EnvOpenOptions`

After applying the fix for the 500 error, a new compiler error emerged:
```
error[E0599]: no function or associated item named `new` found for struct `EnvOpenOptions<WithoutTls>` in the current scope
  --> crates/meilisearch/src/routes/fj_snapshot.rs:185:61
   |
185 |         let mut env_options = EnvOpenOptions::<WithoutTls>::new();
   |                                                              ^^^ function or associated item not found in `EnvOpenOptions<WithoutTls>`
   |
   = note: the function or associated item was found for
           - `EnvOpenOptions`
```

### 7.1. Diagnosis of E0599

The error message indicates that `EnvOpenOptions::<WithoutTls>::new()` is not a valid way to call the constructor. The note `the function or associated item was found for - EnvOpenOptions` strongly suggests that the `new` method exists directly on the `EnvOpenOptions` struct, likely without requiring the `WithoutTls` generic parameter to be specified *during the call to `new()`*.

The `WithoutTls` type parameter is typically associated with the `heed::Env` that is eventually opened, or it's a default type parameter for `EnvOpenOptions` that doesn't need to be (and in this case, cannot be) specified with turbofish syntax on the `new()` method itself.

### 7.2. Solution Strategy for E0599

1.  **Modify `EnvOpenOptions` Instantiation**:
    *   Change the line:
        ```rust
        let mut env_options = EnvOpenOptions::<WithoutTls>::new();
        ```
        to:
        ```rust
        let mut env_options: EnvOpenOptions<WithoutTls> = EnvOpenOptions::new();
        // or, if type inference works (it should, given the later use):
        // let mut env_options = EnvOpenOptions::new();
        ```
    *   The explicit type annotation `EnvOpenOptions<WithoutTls>` on the variable `env_options` is correct and important for ensuring the right kind of environment is configured. However, the `::new()` call itself should be on the base `EnvOpenOptions` type.

2.  **Verify in Other Code**: Briefly check other parts of the Meilisearch codebase (especially tests) where `EnvOpenOptions` is used to confirm this instantiation pattern.

### 7.3. Step-by-Step Guide to Fix E0599

Modify `crates/meilisearch/src/routes/fj_snapshot.rs` within the `msfj_sis_api_handler_tests` module:

1.  **Correct `EnvOpenOptions` instantiation in `test_fj_create_index_snapshot_success`**:
    *   Locate the line: `let mut env_options = EnvOpenOptions::<WithoutTls>::new();`
    *   Change it to: `let mut env_options: EnvOpenOptions<WithoutTls> = EnvOpenOptions::new();`
    *   This change should also be applied to any other tests in this file that might have copied this pattern (e.g., commented-out tests, if they are to be re-enabled later).

2.  **Run the Test**: Execute `cargo test -p meilisearch -- --nocapture routes::fj_snapshot::msfj_sis_api_handler_tests::test_fj_create_index_snapshot_success` again to confirm the E0599 error is resolved and to see if the original test now passes or if new issues arise.

## 8. Addressing Compiler Error E0308 for `EnvOpenOptions` (Mismatched Types)

After fixing E0599, a new compiler error E0308 (mismatched types) appeared:
```
error[E0308]: mismatched types
  --> crates/meilisearch/src/routes/fj_snapshot.rs:185:59
   |
185 |         let mut env_options: EnvOpenOptions<WithoutTls> = EnvOpenOptions::new();
   |                                                            ^^^^^^^^^^^^^^^^^^^^^ expected `EnvOpenOptions<WithoutTls>`, found `EnvOpenOptions<WithTls>`
   |
   = note: expected struct `EnvOpenOptions<WithoutTls>`
              found struct `EnvOpenOptions<WithTls>`
```

### 8.1. Definitive Instantiation of `EnvOpenOptions<WithoutTls>` for `AuthController`

The compiler error `E0308: mismatched types` (expected `Env<WithoutTls>`, found `Env<WithTls>`) when calling `AuthController::new(auth_env, ...)` confirms that `AuthController` requires its `auth_env` to be specifically `heed::Env<WithoutTls>`. This, in turn, means the `env_options` used to open this environment must be `heed::EnvOpenOptions<WithoutTls>`.

The example code from `crates/milli/src/index.rs` (specifically in `TempIndex::new_with_map_size`) provides the correct pattern:
```rust
let options = EnvOpenOptions::new(); // This is EnvOpenOptions<DefaultType> or EnvOpenOptions<WithTls>
let mut options = options.read_txn_without_tls(); // This converts it to EnvOpenOptions<WithoutTls>
// ...
// Index::new then takes options: heed::EnvOpenOptions<WithoutTls>
```
The method `read_txn_without_tls()` on an `EnvOpenOptions` instance (which defaults to `EnvOpenOptions<DefaultType>` or `EnvOpenOptions<WithTls>`) converts it to `EnvOpenOptions<WithoutTls>`. This is the correct approach.

### 8.2. Solution Strategy

1.  **Modify `EnvOpenOptions` Instantiation**:
    *   Start by creating a default `EnvOpenOptions::new()`.
    *   Call `.read_txn_without_tls()` on this instance.
    *   The variable holding the result should be explicitly typed as `EnvOpenOptions<WithoutTls>`.
    *   The line should become:
        ```rust
        let env_options_default = EnvOpenOptions::new();
        let mut env_options: EnvOpenOptions<WithoutTls> = env_options_default.read_txn_without_tls();
        ```
    *   The import `use meilisearch_types::heed::WithoutTls;` is necessary for the type annotation.

### 8.3. Step-by-Step Guide to Fix `EnvOpenOptions` Instantiation

Modify `crates/meilisearch/src/routes/fj_snapshot.rs` within the `msfj_sis_api_handler_tests` module:

1.  **Correct `EnvOpenOptions` instantiation in `test_fj_create_index_snapshot_success`**:
    *   Locate the line where `env_options` is initialized.
    *   Change it to:
        ```rust
        let env_options_default = EnvOpenOptions::new();
        let mut env_options: EnvOpenOptions<WithoutTls> = env_options_default.read_txn_without_tls();
        ```
    *   Ensure `meilisearch_types::heed::WithoutTls` is imported and used in the type annotation.

2.  **Run the Test**: Execute `cargo test -p meilisearch -- --nocapture routes::fj_snapshot::msfj_sis_api_handler_tests::test_fj_create_index_snapshot_success` again. This should resolve the compiler errors related to `EnvOpenOptions` instantiation.

## 9. Addressing Compiler Error E0133 for `EnvOpenOptions::open` (Unsafe Function Call)

After correcting the `EnvOpenOptions<WithoutTls>` instantiation, a new compiler error E0133 appeared:
```
error[E0133]: call to unsafe function `EnvOpenOptions::<T>::open` is unsafe and requires unsafe function or block
  --> crates/meilisearch/src/routes/fj_snapshot.rs:191:24
   |
191 |         let auth_env = env_options.open(auth_db_path).unwrap();
   |                          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ call to unsafe function
```

### 9.1. Diagnosis of E0133

The error `E0133` indicates that the `EnvOpenOptions::open` method is an `unsafe` function. This is a characteristic of the `heed` library, as opening an LMDB environment has preconditions that the library cannot always guarantee at compile time (e.g., path validity, ensuring the environment isn't concurrently opened in a conflicting way by another part of the same process without `MDB_NOSUBDIR`). Therefore, the caller must assert these conditions are met by using an `unsafe` block.

Examples from `crates/milli/src/index.rs` and `crates/meilitool/src/main.rs` confirm this pattern:
```rust
// From milli:
// let env = unsafe { options.open(path) }?;

// From meilitool:
// let env = unsafe { EnvOpenOptions::new().read_txn_without_tls().max_dbs(100).open(&path) }
```

### 9.2. Solution Strategy for E0133

1.  **Wrap `open` Call in `unsafe` Block**:
    *   The call to `env_options.open(auth_db_path).unwrap()` must be wrapped in an `unsafe` block.
    *   The line should become:
        ```rust
        let auth_env = unsafe { env_options.open(auth_db_path).unwrap() };
        ```

### 9.3. Step-by-Step Guide to Fix E0133

Modify `crates/meilisearch/src/routes/fj_snapshot.rs` within the `msfj_sis_api_handler_tests` module:

1.  **Wrap `env_options.open()` call in `unsafe` block in `test_fj_create_index_snapshot_success`**:
    *   Locate the line: `let auth_env = env_options.open(auth_db_path).unwrap();`
    *   Change it to:
        ```rust
        let auth_env = unsafe { env_options.open(auth_db_path).unwrap() };
        ```

2.  **Run the Test**: Execute `cargo test -p meilisearch -- --nocapture routes::fj_snapshot::msfj_sis_api_handler_tests::test_fj_create_index_snapshot_success` again. This should resolve the E0133 compiler error. If the test compiles, it will either pass or fail based on the original 500 error logic.

## 10. Addressing `Mdb(DbsFull)` Error in `test_fj_create_index_snapshot_success`

After resolving previous compiler errors, the test `test_fj_create_index_snapshot_success` failed with a runtime panic:
```
thread 'routes::fj_snapshot::msfj_sis_api_handler_tests::test_fj_create_index_snapshot_success' panicked at crates/meilisearch/src/routes/fj_snapshot.rs:192:87:
called `Result::unwrap()` on an `Err` value: Internal(Mdb(DbsFull))
```

### 10.1. Diagnosis of `Mdb(DbsFull)`

*   **Error**: `Mdb(DbsFull)` is an error from LMDB indicating that an attempt was made to create a new named database, but the maximum number of named databases allowed for the environment has already been reached.
*   **Location**: The panic occurred at `crates/meilisearch/src/routes/fj_snapshot.rs:192`, which is the line where `AuthController::new(...)` is called:
    ```rust
    let auth_controller = AuthController::new(auth_env, &opt_for_auth.master_key).unwrap(); // Line 192
    ```
    This means the `AuthController::new` function failed, returning an `Err` that contained the `Mdb(DbsFull)` error.
*   **Cause**: The `AuthController` creates several named databases within its LMDB environment. If the `EnvOpenOptions` used to open `auth_env` does not specify a `max_dbs` value, or specifies one that is too small, this error will occur when `AuthController` tries to create its internal databases. The default `max_dbs` for `heed` is often very small (e.g., 0 or 1, effectively allowing only the main unnamed database unless configured).

### 10.2. Solution Strategy for `Mdb(DbsFull)`

1.  **Increase `max_dbs`**: Modify the `EnvOpenOptions` for the `auth_env` in the test setup to include a call to `.max_dbs(N)`, where `N` is a number sufficient for `AuthController`. A value like 10 should be safe, as `AuthController` typically creates a few databases (e.g., for keys, owners, etc.).
    *   The relevant code in the test is:
        ```rust
        let env_options_default = EnvOpenOptions::new();
        let mut env_options: EnvOpenOptions<WithoutTls> = env_options_default.read_txn_without_tls();
        env_options.map_size(100 * 1024 * 1024); // 100MB
        // ...
        let auth_env = unsafe { env_options.open(auth_db_path).unwrap() };
        ```
    *   The `max_dbs` setting should be added to `env_options` before calling `.open()`.

### 10.3. Step-by-Step Guide to Fix `Mdb(DbsFull)`

Modify `crates/meilisearch/src/routes/fj_snapshot.rs` within the `msfj_sis_api_handler_tests` module:

1.  **Set `max_dbs` for `auth_env` in `test_fj_create_index_snapshot_success`**:
    *   Locate the initialization of `env_options`.
    *   Add `.max_dbs(10)` (or a similar reasonable number) to the chain of `EnvOpenOptions` configurations.
        ```rust
        let env_options_default = EnvOpenOptions::new();
        let mut env_options: EnvOpenOptions<WithoutTls> = env_options_default.read_txn_without_tls();
        env_options.map_size(100 * 1024 * 1024); // 100MB
        env_options.max_dbs(10); // Add this line
        // ...
        let auth_env = unsafe { env_options.open(auth_db_path).unwrap() };
        ```

2.  **Run the Test**: Execute `cargo test -p meilisearch -- --nocapture routes::fj_snapshot::msfj_sis_api_handler_tests::test_fj_create_index_snapshot_success` again. This should resolve the `Mdb(DbsFull)` error.

## 11. Diagnosing 500 Error with Simplified Handler (`Ok(Json("test"))`)

After fixing the `Mdb(DbsFull)` error and simplifying the `fj_create_index_snapshot` handler to return `Ok(Json("test"))` (commit `fbe93f7`), the test `test_fj_create_index_snapshot_success` still fails with an HTTP 500 error. This indicates the error occurs *before* the handler's main logic, likely within Actix's request processing, specifically during extractor resolution.

**Current Handler Signature (Simplified Body):**
```rust
pub async fn fj_create_index_snapshot<S: FjSnapshotScheduler>(
    _index_scheduler: GuardedData<ActionPolicy<{ actions::SNAPSHOTS_CREATE }>, Data<S>>,
    _index_uid: web::Path<String>,
    _req: HttpRequest,
    _opt: web::Data<Opt>,
) -> Result<Json<&'static str>, ResponseError> { // Changed return type for diagnostic
    Ok(Json("test")) // Simplified diagnostic return
}
```

**Test Setup Provides:**
*   `Data<Arc<MockIndexScheduler>>` (for `Data<S>` where `S` is `Arc<MockIndexScheduler>`)
*   `Data<AuthController>` (for `GuardedData`'s internal authentication policy check)
*   `Data<Opt>`

**Hypothesis:** The `GuardedData<ActionPolicy<{ actions::SNAPSHOTS_CREATE }>, Data<S>>` extractor is failing to resolve, leading to the 500 error. This could be due to an issue with the policy check itself, or how `GuardedData` wraps the `Data<S>` extractor, despite the necessary `Data` types seemingly being available in the application.

**Diagnostic Strategy:**
To isolate whether `GuardedData` is the cause, temporarily remove it from the handler signature and replace it with the inner `Data<S>` directly.

1.  **Modify Handler Signature**:
    Change `fj_create_index_snapshot` to take `_index_scheduler: Data<S>` instead of `GuardedData<ActionPolicy<{ actions::SNAPSHOTS_CREATE }>, Data<S>>`.
    The new signature would be:
    ```rust
    pub async fn fj_create_index_snapshot<S: FjSnapshotScheduler>(
        _index_scheduler: Data<S>, // Changed from GuardedData
        _index_uid: web::Path<String>,
        _req: HttpRequest,
        _opt: web::Data<Opt>,
    ) -> Result<Json<&'static str>, ResponseError> {
        Ok(Json("test"))
    }
    ```
    This change is purely for diagnostics. The `GuardedData` extractor is essential for security and will need to be reinstated and fixed if it's identified as the problem.

2.  **Run the Test**:
    Execute `cargo test -p meilisearch -- --nocapture routes::fj_snapshot::msfj_sis_api_handler_tests::test_fj_create_index_snapshot_success`.
    *   If the test now passes (returns HTTP 200 OK with the body "test"), it strongly implicates `GuardedData` as the source of the 500 error. Further investigation would then focus on why `GuardedData` is failing (e.g., an unexpected failure in the `ActionPolicy` check, or an issue with how `GuardedData` handles the generic `Data<S>`).
    *   If the test still fails with an HTTP 500 error, the problem likely lies with one of the other extractors (`Data<S>` itself, `web::Path<String>`, `web::Data<Opt>`) or a more fundamental Actix configuration issue for this specific route.

## 12. Diagnosing 500 Error (text/plain) with `Data<S>` Extractor

After removing `GuardedData` (commit `72ddd84`), the test `test_fj_create_index_snapshot_success` still fails with an HTTP 500 error. Notably, the response content type changed from `application/json` to `text/plain; charset=utf-8`. This often indicates a panic within an extractor or an error that Actix cannot serialize into its standard JSON `ResponseError` format.

**Current Handler Signature (Simplified Body, No GuardedData):**
```rust
pub async fn fj_create_index_snapshot<S: FjSnapshotScheduler>(
    _index_scheduler: Data<S>, // Using Data<S> directly
    _index_uid: web::Path<String>,
    _req: HttpRequest,
    _opt: web::Data<Opt>,
) -> Result<Json<&'static str>, ResponseError> {
    Ok(Json("test"))
}
```
In the test, `S` is `Arc<MockIndexScheduler>`, and the route is configured with `fj_create_index_snapshot::<Arc<MockIndexScheduler>>`.

**Hypothesis:** The issue might be with how Actix resolves the generic `Data<S>` extractor, or with `Data<Arc<MockIndexScheduler>>` itself, or one of the other extractors (`web::Path`, `Data<Opt>`). The `text/plain` error points towards a lower-level failure.

**Diagnostic Strategy:**
To eliminate the generic `S` from the handler's immediate signature, we'll change `_index_scheduler: Data<S>` to the concrete type used in the test: `_index_scheduler: Data<Arc<MockIndexScheduler>>`.

1.  **Modify Handler Signature to be Concrete**:
    Change `fj_create_index_snapshot` to take `_index_scheduler: Data<Arc<MockIndexScheduler>>`.
    The new signature would be:
    ```rust
    pub async fn fj_create_index_snapshot( // Removed generic <S: FjSnapshotScheduler>
        _index_scheduler: Data<Arc<MockIndexScheduler>>, // Now concrete
        _index_uid: web::Path<String>,
        _req: HttpRequest,
        _opt: web::Data<Opt>,
    ) -> Result<Json<&'static str>, ResponseError> {
        Ok(Json("test"))
    }
    ```
    The corresponding route definition in the test already uses `fj_create_index_snapshot::<Arc<MockIndexScheduler>>`. We will need to remove the turbofish from the route definition in the test as the function will no longer be generic.

2.  **Run the Test**:
    Execute `cargo test -p meilisearch -- --nocapture routes::fj_snapshot::msfj_sis_api_handler_tests::test_fj_create_index_snapshot_success`.
    *   If the test now passes, it suggests an issue with Actix's handling of generic `Data<S>` extractors in this specific setup.
    *   If it still fails, the problem is likely with `Data<Arc<MockIndexScheduler>>` itself (e.g., `MockIndexScheduler` not being properly `Send`/`Sync` in a way Actix expects, despite appearances), or with `web::Path` or `Data<Opt>` failing in a way that causes a panic.

## 13. Diagnosing 500 Error with Concrete `GuardedData` (Inspired by `swap_indexes.rs`)

The test `test_fj_create_index_snapshot_success` continues to fail with an HTTP 500 error (text/plain response) even when the handler uses a concrete `Data<Arc<MockIndexScheduler>>` and a simplified body. This suggests the failure might be in how `Data<Arc<MockIndexScheduler>>` is extracted, or another extractor.

The `crates/meilisearch/src/routes/swap_indexes.rs` file provides an example of a working handler:
```rust
pub async fn swap_indexes(
    index_scheduler: GuardedData<ActionPolicy<{ actions::INDEXES_SWAP }>, Data<IndexScheduler>>,
    // ... other extractors
) -> Result<HttpResponse, ResponseError>
```
This uses `GuardedData` with a concrete `Data<IndexScheduler>`.

Our previous steps involved:
1.  `GuardedData<..., Data<S>>` (generic `S`): HTTP 500 (JSON). This suggests `GuardedData` was catching an error.
2.  `Data<S>` (generic `S`, no `GuardedData`): HTTP 500 (text/plain).
3.  `Data<Arc<MockIndexScheduler>>` (concrete mock, no `GuardedData`): HTTP 500 (text/plain).

**Hypothesis:** The issue might be related to how `GuardedData` interacts with generic type parameters for its inner `Data` extractor, or there's a fundamental issue with one of the extractors that `GuardedData` was previously masking by returning a `ResponseError`.

**Diagnostic Strategy:**
Reinstate `GuardedData` but make its inner `Data` extractor use the concrete `Arc<MockIndexScheduler>` type from the test setup. This aligns more closely with the `swap_indexes.rs` pattern of `GuardedData` wrapping a concrete `Data` type.

1.  **Modify Handler Signature**:
    Change `fj_create_index_snapshot` to:
    ```rust
    pub async fn fj_create_index_snapshot(
        _index_scheduler: GuardedData<ActionPolicy<{ actions::SNAPSHOTS_CREATE }>, Data<Arc<MockIndexScheduler>>>, // Concrete mock in GuardedData
        _index_uid: web::Path<String>,
        _req: HttpRequest,
        _opt: web::Data<Opt>,
    ) -> Result<Json<&'static str>, ResponseError> { // Still simplified body
        Ok(Json("test"))
    }
    ```
    The function is no longer generic over `S`. The route definition in the test already calls the non-generic version.

2.  **Run the Test**:
    Execute `cargo test -p meilisearch -- --nocapture routes::fj_snapshot::msfj_sis_api_handler_tests::test_fj_create_index_snapshot_success`.
    *   If the test now passes (returns HTTP 200 OK), it would indicate that `GuardedData` requires a concrete inner `Data<T>` type or that the generic `S` was causing resolution issues for `GuardedData`.
    *   If it fails with an HTTP 500 (JSON response), it means `GuardedData` itself is failing (e.g., policy check, or its internal extraction of `Data<AuthController>` or `Data<Arc<MockIndexScheduler>>`). This would be progress, as a JSON error is more structured.
    *   If it fails with an HTTP 500 (text/plain response), this would be unexpected as `GuardedData` should convert internal errors to `ResponseError` (JSON).

## 14. Correcting E0412 for `MockIndexScheduler` and Testing `GuardedData`

The attempt in Step 13 to make the `fj_create_index_snapshot` handler's signature use `Data<Arc<MockIndexScheduler>>` directly (commit `99d46b7`) resulted in a compiler error:
```
error[E0412]: cannot find type `MockIndexScheduler` in this scope
  --> crates/meilisearch/src/routes/fj_snapshot.rs:58:89
   |
58 | | ...ATE }>, Data<Arc<MockIndexScheduler>>>, // DIAGNOSTIC: Concrete Gu ...
   | | ^^^^^^^^^^^^^^^^^^ help: a struct with a similar name exists: `IndexScheduler`
   |
note: struct `crate::routes::fj_snapshot::msfj_sis_api_handler_tests::MockIndexScheduler` exists but is inaccessible
```

### 14.1. Diagnosis of E0412

The error `E0412` occurs because `MockIndexScheduler` is defined within the `#[cfg(test)] mod msfj_sis_api_handler_tests { ... }` module. This makes it a test-only type, inaccessible to the `fj_create_index_snapshot` function, which is part of the main (non-test) codebase. A non-test function cannot have a test-only type in its signature.

### 14.2. Solution Strategy for E0412 and `GuardedData` Testing

To correctly test `GuardedData` with the `Arc<MockIndexScheduler>` type provided by the test setup, while respecting Rust's scoping rules:

1.  **Make Handler Generic Again**: The `fj_create_index_snapshot` handler must be generic over `S: FjSnapshotScheduler`.
2.  **Use Generic `Data<S>` in `GuardedData`**: The `_index_scheduler` parameter in the handler should be `GuardedData<ActionPolicy<{ actions::SNAPSHOTS_CREATE }>, Data<S>>`.
3.  **Specify Concrete Type in Test Route**: The test setup will register the route using `fj_create_index_snapshot::<Arc<MockIndexScheduler>>`.

This approach allows Actix to resolve `S` to `Arc<MockIndexScheduler>` specifically for the test instance. Consequently, `GuardedData` will attempt to extract `Data<Arc<MockIndexScheduler>>` (the concrete mock type) during the test, achieving the diagnostic goal of Step 13 without violating type visibility.

### 14.3. Step-by-Step Guide

1.  **Modify Handler Signature in `crates/meilisearch/src/routes/fj_snapshot.rs`**:
    *   Change `fj_create_index_snapshot` to be generic over `S: FjSnapshotScheduler`.
    *   Update its `_index_scheduler` parameter to use `Data<S>`.
    ```rust
    pub async fn fj_create_index_snapshot<S: FjSnapshotScheduler>( // Make generic again
        _index_scheduler: GuardedData<ActionPolicy<{ actions::SNAPSHOTS_CREATE }>, Data<S>>, // Use generic S
        _index_uid: web::Path<String>,
        _req: HttpRequest,
        _opt: web::Data<Opt>,
    ) -> Result<Json<&'static str>, ResponseError> { // Still simplified body
        Ok(Json("test"))
    }
    ```

2.  **Update Route Registration in Test (`test_fj_create_index_snapshot_success`)**:
    *   Modify the route registration to use the turbofish syntax to specify `Arc<MockIndexScheduler>` for `S`.
    ```rust
    // In test_fj_create_index_snapshot_success:
    .service(
        web::resource("/indexes/{index_uid}/snapshots")
            .route(web::post().to(fj_create_index_snapshot::<Arc<MockIndexScheduler>>)), // Add turbofish
    ),
    ```

3.  **Run the Test**:
    Execute `cargo test -p meilisearch -- --nocapture routes::fj_snapshot::msfj_sis_api_handler_tests::test_fj_create_index_snapshot_success`.
    *   This should resolve the `E0412` compiler error.
    *   The test will then reveal whether `GuardedData<ActionPolicy<{ actions::SNAPSHOTS_CREATE }>, Data<Arc<MockIndexScheduler>>>` extracts successfully (leading to a 200 OK with "test" body) or fails (leading to an HTTP 500 error, hopefully JSON formatted by `GuardedData` or `ResponseError`).

## 15. Verifying `GuardedData` with Standard `ActionPolicy`

After previous refactoring (e.g., commit `30ff0f4` and subsequent removal of `ActionGuard`), the test `test_fj_create_index_snapshot_success` should be using `GuardedData` with a standard `ActionPolicy`. The `ActionPolicy`'s `Guard` associated type is expected to be `()`.

**Handler Signature (as of commit `30ff0f4`):**
```rust
pub async fn fj_create_index_snapshot<S: FjSnapshotScheduler>(
    _index_scheduler: GuardedData<ActionPolicy<{ actions::SNAPSHOTS_CREATE }>, Data<S>>,
    // ...
) -> Result<Json<&'static str>, ResponseError> { // Simplified body
    Ok(Json("test"))
}
```
In the test, `S` is `Arc<MockIndexScheduler>`.

**`GuardedData` Internals (Standard Usage):**
The `GuardedData<P, T>` extractor, when `P::Guard` is `()`, typically:
1.  Extracts `Data<P::Auth>` (here, `Data<AuthController>`). This is provided in the test.
2.  Extracts `T` via `T::from_request()` (here, `T` is `Data<S>`, so it extracts `Data<Arc<MockIndexScheduler>>`). This is provided in the test.
3.  Attempts to extract `Data<P::Guard>` which is `Data<()>` for the policy check. `Data<()>` is generally available by default in Actix and does not need to be explicitly added to `app_data`.
4.  Performs the policy check `P::check(auth, guard_data, data_for_policy_check)`.

**Hypothesis:**
If the test still fails with an HTTP 500 JSON error, it implies an issue within the `GuardedData` extractor or the `ActionPolicy::check` method, even with the standard `Guard = ()` setup. The error could stem from:
*   The `ActionPolicy::check` logic itself (though with no master key, it should default to `Ok(())`).
*   An unexpected failure during the extraction of `Data<AuthController>` or `Data<Arc<MockIndexScheduler>>` that `GuardedData` is catching.
*   A more subtle interaction within `GuardedData::from_request`.

**Diagnostic Strategy:**
The test setup should now be correct for standard `ActionPolicy` usage.
1.  **Confirm Test Setup**: Ensure `test_fj_create_index_snapshot_success` in `crates/meilisearch/src/routes/fj_snapshot.rs` no longer attempts to add any `ActionGuard` related data. It should only provide `Data<Arc<MockIndexScheduler>>`, `Data<Opt>`, and `Data<AuthController>`.
2.  **Run the Test**: Execute `cargo test -p meilisearch -- --nocapture routes::fj_snapshot::msfj_sis_api_handler_tests::test_fj_create_index_snapshot_success`.
    *   If the test passes (HTTP 200 OK with "test" body), the removal of `ActionGuard` and reliance on the standard `ActionPolicy` (with `Guard = ()`) was the correct fix.
    *   If it still fails with an HTTP 500 JSON error, the problem is deeper within the interaction of `GuardedData`, `ActionPolicy`, or the provided `Data` types, even under standard conditions. Further debugging would involve stepping into `GuardedData::from_request` or `ActionPolicy::check`.
    *   If it fails with an HTTP 500 text/plain error, it indicates a panic that `GuardedData` is not catching, possibly during the extraction of `Data<S>` or `Data<P::Auth>` before the policy check.
```

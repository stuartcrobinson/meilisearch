# Debugging Tips Log (Learnings from index-scheduler tests)

**Mission**: This document logs specific error patterns (compiler errors, test failures) encountered *while debugging the `meilisearchfj` codebase*, particularly the Single Index Snapshot feature, and the concrete solutions that resolved them in this context. Avoid general debugging advice; focus on recording the exact error signature, the specific situation it occurred in, and the precise fix applied.

## Rust Compiler Errors

*   **E0308: Mismatched Types**
    *   **Instance 1**: `expected &str, found String` when passing `S("...")` to function expecting `&str`. **Fix**: Borrow the string: `&S("...")`.
    *   **Instance 2**: `expected RankingRuleView, found Criterion` when setting `ranking_rules`. **Fix**: Convert using `.into()`: `Criterion::Typo.into()`.
    *   **Instance 3**: `expected BTreeSet<String>, found HashSet<String>` when setting `disable_on_attributes`. **Fix**: Use `BTreeSet::from(...)`.
    *   **Instance 4**: `expected Option<Map<...>>, found HashMap<Vec<...>, ...>` when comparing synonyms. **Fix**: Realized `milli::Index::synonyms` returns `HashMap`, removed incorrect `fst::Map` conversion logic and compared `HashMap`s directly.

*   **E0432: Unresolved Import**
    *   **Instance 1**: `TypoToleranceSettings`, `MinWordSizeForTypos`. **Fix**: Used aliases from dump reader: `TypoSettings`, `MinWordSizeTyposSetting`. Required looking at `crates/dump/src/reader/v6/mod.rs`.
    *   **Instance 2**: `milli::update::RankingRule` or `milli::update::settings::RankingRule`. **Fix**: Realized `milli::update::settings` is private (E0603) and the public type is `milli::Criterion`. Changed import and usage.

*   **E0597 / E0716: Borrowed value does not live long enough / Temporary value dropped while borrowed**
    *   **Instance**: Passing `&S("...")` to a function expecting `&'static str` (like `replace_document_import_task`). **Fix**: Pass the string literal `"..."` directly, as it has a `'static` lifetime. The `S()` macro creates an owned `String` whose borrow is temporary.

*   **E0599: No method/associated item found**
    *   **Instance 1**: `WildcardSetting::Set`. **Fix**: `WildcardSetting` is a struct wrapping `Setting`. Correct usage is `Setting::Set(...).into()`. Required looking at `meilisearch-types/src/settings.rs`.
    *   **Instance 2**: `.stream()` on `BTreeSet<String>`. **Fix**: The variable held a `BTreeSet`, not an `fst::Set`. The `.map(|fst_set| fst_set.stream()...)` logic was incorrect for comparing tokenization settings. Correct approach: compare `Option<BTreeSet<String>>` directly using `.map(|s| s.clone())`.
    *   **Instance 3**: `.cloned()` on `Option<BTreeSet<String>>`. **Fix**: `.cloned()` is for iterators. Use `.map(|s| s.clone())` to convert `Option<&BTreeSet<String>>` to `Option<BTreeSet<String>>`.
    *   **Instance 4**: `.stream()` or `.as_fst()` on `fst::Map<AlwaysMatch>`. **Fix**: The `convert_synonyms` helper assumed the wrong `fst::Map` type parameter. Correct fix was to remove the helper entirely as `milli::Index::synonyms` returns `HashMap`.

*   **E0603: Module is private**
    *   **Instance**: `milli::update::settings`. **Fix**: Found the public equivalent `milli::Criterion` by checking `milli/src/lib.rs` and `milli/src/update/mod.rs`.

## Test Failures

*   **Symptom**: `assert_eq!` fails comparing `PromptData`.
    *   **Cause**: `PromptData` struct (from `milli/src/prompt/mod.rs`) does not implement `PartialEq`.
    *   **Fix**: Compare the fields (`template`, `max_bytes`) individually in the assertion.

*   **Snapshot UID Mismatches in Tests**
    *   **Symptom**: `assert_eq!` fails comparing snapshot UIDs in import tests (e.g., `test_import_snapshot_happy_path`, `test_e2e_snapshot_create_import_verify`).
    *   **Cause**: Inconsistent extraction/construction of the UID between the task processing logic (`process_batch.rs`) and the test assertions (`scheduler/test.rs`). The processing logic might store the full UUID, while the test expects a partial one or includes extra parts like `.snapshot.tar`.
    *   **Fix**: Ensure both the processing logic and the test assertion extract/construct the *same* representation of the UID (e.g., only the full UUID string) from the snapshot filename/path before comparison. Use string splitting (`split_once`, `split('.')`) carefully on the filename stem.

*   **Snapshot File Path Assertion Failures**
    *   **Symptom**: `assert!(snapshot_filepath.exists(), ...)` fails in snapshot creation tests (e.g., `test_create_index_snapshot_success`).
    *   **Cause**: If the snapshot creation function's return type changes (e.g., from `String` UID to `PathBuf`), tests might incorrectly reconstruct the expected path using the old logic, leading to assertions on malformed paths.
    *   **Fix**: Update tests to use the returned value (e.g., the `PathBuf`) directly in assertions and subsequent file operations, adapting to the new function signature.

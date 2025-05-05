# Coding Recommendations (Learnings from index-scheduler tests)

**Mission**: This document captures specific, novel coding patterns, type usages, and best practices learned *directly* from debugging sessions within the `meilisearchfj` codebase. Avoid adding general Rust or Meilisearch knowledge; focus only on actionable insights gained from solving concrete problems encountered during development in this specific context.

*   **`'static` Lifetimes**: Functions requiring `&'static str` (like `replace_document_import_task`) cannot accept borrows (`&`) of owned `String`s created at runtime (e.g., via `S()` macro). The temporary borrow doesn't live long enough. Pass string literals (`"..."`) directly in these cases.
*   **`String` vs `&str`**: When a function expects `&str` but receives a `String` (often from `S()`), borrow it (`&my_string`) as suggested by E0308.
*   **Cloning `Option<&T>`**: To get an `Option<T>` from `Option<&T>` (where `T: Clone`), use `.map(|t_ref| t_ref.clone())`. The `.cloned()` method is for iterators. (Encountered with `Option<&BTreeSet<String>>`).
*   **`WildcardSetting` Construction**: `WildcardSetting` wraps `Setting<Vec<String>>`. Construct it using `Setting::Set(vec![...]).into()`, not `WildcardSetting::Set(...)`.
*   **Ranking Rules**: Use `milli::Criterion` enum variants (e.g., `Criterion::Typo`) when building `Settings`. Convert to `RankingRuleView` using `.into()` where needed (e.g., for the `ranking_rules` field).
*   **Collection Types**: Use the specific collection type expected (e.g., `BTreeSet` for `TypoSettings::disable_on_attributes`, not `HashSet`).
*   **Snapshot UID Consistency**: When dealing with snapshot filenames (`{index_uid}-{uuid}.snapshot.tar.gz`), ensure consistent handling of the `uuid` part. Extract the full UUID when storing it (e.g., in task details) and use the full UUID when reconstructing filenames or comparing values. Avoid storing/comparing partial UUIDs or including extra filename parts unless necessary.
*   **Function Return Value Changes**: When a function's return type changes (e.g., from `String` to `PathBuf`), update all call sites to handle the new type correctly, especially in tests that construct paths or perform assertions based on the returned value.

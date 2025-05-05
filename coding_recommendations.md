# Coding Recommendations

This document captures general coding practices and recommendations derived from debugging sessions to improve future development productivity.

## General Principles

*   **Prioritize Understanding**: When encountering persistent compiler errors (like type mismatches, unresolved paths, or missing methods), avoid excessive trial-and-error. Instead, prioritize understanding the involved types and interfaces. Ask the AI assistant to show the definitions of relevant structs, enums, traits, and functions. Examine their fields (public vs. private), methods, and implemented traits.
*   **Look for Examples**: Request examples of similar code usage elsewhere in the Meilisearch codebase. Existing tests or core logic often provide the best patterns for using types and functions correctly.
*   **Verify Re-exports**: When using paths like `crate::some_module::Type`, be mindful that `some_module` might be a re-export. If errors persist, ask to see the `lib.rs` or `mod.rs` file where the re-export occurs (`pub use ...`) and potentially the `lib.rs` of the original crate to confirm exactly which type is being re-exported, especially if names might collide.
*   **Clean Builds**: Running `cargo clean` periodically during complex debugging can rule out stale build artifacts causing strange behavior, although it's often not the root cause of type or logic errors.
*   **Systematic Debugging**: When debugging persistent test failures, avoid sequential trial-and-error fixes. Formulate multiple hypotheses, instrument the code with logging/assertions, verify assumptions (e.g., file paths, permissions), and pinpoint the exact failure location and state.

## Specific Recommendations (Rust/Meilisearch Context)

*   **Lifetimes (`'static`)**: Be cautious when functions require `'static` lifetimes (e.g., `replace_document_import_task`). String literals (`"..."`) have a `'static` lifetime. Owned `String`s created at runtime (e.g., via `S()` macro or `format!`) do *not*. Passing a reference (`&`) to an owned `String` creates a temporary borrow that is *not* `'static`. If a function truly needs `'static str`, pass a string literal directly or ensure the `String` lives for the entire program duration (which is rare and often indicates a design issue).
*   **Type Mismatches (`String` vs `&str`)**: Pay close attention to function signatures. If a function expects `&str` but receives `String`, the compiler (E0308) will often suggest borrowing (`&my_string`). Conversely, if it expects `String` but receives `&str`, you might need `.to_string()` or `.to_owned()`.
*   **Option Clones (`Option<&T>` to `Option<T>`)**: To convert an `Option<&T>` to an `Option<T>` where `T: Clone`, use `.map(|ref_t| ref_t.clone())`, not `.cloned()` (which works on iterators).
*   **Enum vs. Associated Items**: If the compiler complains about "no associated item named `X` found for struct `Y`" (E0599) when you use `Y::X(...)`, `Y` might be an enum, and `X` is likely a variant, requiring the syntax `Y::X(...)`. If `Y` is a struct and `X` is intended to be a constructor or associated function, ensure it's defined correctly (e.g., `impl Y { fn X(...) -> Self { ... } }`). If `Y` is a struct wrapping another type (like `WildcardSetting` wrapping `Setting`), you might need to use `.into()` or a constructor of the outer struct.
*   **Trait Bounds**: Errors like "the method `foo` exists for struct `Bar`, but its trait bounds were not satisfied" (E0599) mean `Bar` needs to implement a specific trait (e.g., `AsRef<[u8]>`, `Iterator`) for `foo` to be callable. Check the documentation or definition of `foo` to see the required traits.
*   **Collection Types**: Ensure you're using the correct collection type expected by a function or struct field (e.g., `BTreeSet` vs. `HashSet`). The compiler error E0308 will usually indicate the mismatch.

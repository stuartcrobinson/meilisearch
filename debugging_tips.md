# Debugging Tips Log

This document logs specific error patterns encountered during debugging sessions and the solutions that worked.

## Rust Compiler Errors

*   **Error E0308: Mismatched Types**
    *   **Symptom**: `expected <TypeA>, found <TypeB>`
    *   **Common Causes**:
        *   Passing `String` where `&str` is expected (Solution: Borrow with `&`).
        *   Passing `&str` where `String` is expected (Solution: Use `.to_string()` or `.to_owned()`).
        *   Using the wrong collection type (e.g., `HashSet` vs. `BTreeSet`).
        *   Incorrect enum variant or struct construction.
        *   Forgetting `.into()` when converting between related types (e.g., `Criterion` to `RankingRuleView`).
    *   **Debugging Strategy**: Check the function/method signature for the expected type. Verify the type of the value being passed. Look for necessary conversions (`&`, `.to_string()`, `.into()`, collection conversions).

*   **Error E0432: Unresolved Import**
    *   **Symptom**: `unresolved import `<path>` ` or `no `<Item>` in `<module>` `
    *   **Common Causes**:
        *   Incorrect path to the item.
        *   Item is not public (`pub`) or not re-exported from the module.
        *   Typo in the item or module name.
        *   Dependency not correctly specified in `Cargo.toml`.
    *   **Debugging Strategy**: Verify the exact path and item name. Check the source module (`mod.rs` or `lib.rs`) to ensure the item is public (`pub`) or re-exported (`pub use`). Check for typos. Look at `Cargo.toml`. Ask to see the definition file.

*   **Error E0597: Borrowed value does not live long enough**
    *   **Symptom**: Compiler indicates a variable is dropped while still borrowed, often related to `'static` lifetime requirements.
    *   **Common Causes**: Passing a reference (`&`) to a locally owned `String` or other temporary value into a function expecting a `&'static str`.
    *   **Debugging Strategy**: If `'static` is required, pass a string literal (`"..."`) directly. If `'static` is not strictly necessary, review the function signature or how the value is being stored/used later to see if the lifetime requirement can be relaxed. Avoid creating temporary `String`s just to borrow them for `'static` contexts.

*   **Error E0599: No method/associated item found**
    *   **Symptom**: `no method named \`foo\` found for struct \`Bar\` in the current scope` or `no associated item named \`Baz\` found for struct \`Bar\` in the current scope`.
    *   **Common Causes**:
        *   Method/item doesn't exist or has a typo.
        *   Method requires a trait that `Bar` doesn't implement (check trait bounds in the error message).
        *   Trying to call an enum variant like an associated item (`Enum::Variant` is correct, `Enum::Variant(...)` might be needed).
        *   Trying to call a method on the wrong type (e.g., calling `.stream()` on `BTreeSet` instead of an `fst::Set`).
        *   Confusing struct constructors with enum variants (e.g., `WildcardSetting::Set` vs `Setting::Set(...).into()`).
    *   **Debugging Strategy**: Verify method/item name. Check the type definition (`struct Bar` or `enum Bar`) and its `impl` blocks. Check the required traits mentioned in the error details. Look at examples of how the type/method is used elsewhere.

*   **Error E0603: Module is private**
    *   **Symptom**: `module \`foo\` is private`.
    *   **Common Causes**: Trying to import or use an item from a module not declared as `pub`. Modules are private by default in Rust. `pub(crate)` makes it visible only within the same crate.
    *   **Debugging Strategy**: Check the module definition (`mod foo;` or `mod foo { ... }`). If access is needed from outside, the module or the specific item needs to be made `pub` or re-exported publicly (`pub use`). If the item *is* re-exported, use the public path.

*   **Error E0716: Temporary value dropped while borrowed**
    *   **Symptom**: Similar to E0597, often occurs when borrowing the result of a function/macro call that creates a temporary value (like `&S("...")`).
    *   **Common Causes**: Creating a temporary owned value (e.g., `String` from `S()`) and immediately borrowing it (`&`) for a context that requires a longer lifetime (like `'static`). The temporary owned value is dropped at the end of the statement, invalidating the borrow.
    *   **Debugging Strategy**: If `'static` is needed, use a literal directly (`"..."`). If a shorter lifetime is acceptable, assign the owned value to a variable first (`let my_string = S("...");`) and then borrow the variable (`&my_string`), ensuring the variable lives long enough for the borrow.

## Test Failures

*   **Symptom**: Test fails with assertion error comparing complex structs or settings.
    *   **Common Causes**:
        *   Settings not applied correctly before snapshot/import.
        *   Incorrect retrieval method used in assertion (e.g., `displayed_fields` vs. `user_defined_displayed_fields`).
        *   Comparing different types (e.g., `milli::ProximityPrecision` vs. `ProximityPrecisionView` without conversion).
        *   Comparing collections with different ordering or types (e.g., `Option<&BTreeSet>` vs `Option<BTreeSet>`).
        *   Default values being implicitly added during retrieval but not accounted for in the expected value (e.g., default `*` entry in `sort_facet_values_by`).
    *   **Debugging Strategy**: Verify settings application logic. Use the correct getter methods matching the setting applied. Ensure types match in assertions, using `.into()` or manual conversion where needed. Handle `Option` and references correctly (e.g., using `.map(|s| s.clone())` for `Option<&T>` -> `Option<T>`). Check documentation or source for default behaviors during retrieval. Instrument with `dbg!` macro around assertions.

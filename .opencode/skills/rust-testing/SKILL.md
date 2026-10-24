---
name: rust-testing
description: Use when writing, refactoring, or reviewing Rust tests in GcodeViewer — `#[cfg(test)]` modules under `crates/core/src/` and integration tests under `crates/core/tests/`. Covers choosing between `#[test]` and `#[rstest]`, `#[fixture]` for reusable G-code constants, `indoc!` for multiline fragments, given/when/then structure, per-file test scope, and parallel-backend invariants. Model new tests on `crates/core/src/gcode/lexer.rs`.
---

# Writing Rust tests in GcodeViewer

Follow the *AAA* pattern:
* **Arrange**: Set up all conditions, variables, and objects needed for the test.
* **Act**: Run the specific method or behavior you want to test.
* **Assert**: Check if the result matches what you expected and/or if the system behaves as expected.

Sections can be omitted or merged when very trivial.

Don't force section comments, unless necessary for readability.

## Choosing a tool for the job

| Situation | Use |
| --- | --- |
| One scenario, no parameters | `#[test]` |
| Several scenarios share one body | `#[rstest]` + `#[case::name(...)]` |
| Cartesian product of inputs/backends | `#[rstest]` + `#[values(...)]` |
| A reusable value injected into a test | `#[fixture]` |
| A multiline G-code fragment | `indoc!` |

## Parametrized tests

Always provide names for cases.

Test name tells what behavior is being tested, case name tells what kind of data is used.

```rust
#[rstest]
#[case::no_number("X")]
#[case::sign_only("X-")]
#[case::duplicate_command("G1 G2")]
fn rejects_invalid_text(#[case] text: &str) {
    assert!(Words::lex(text).is_err());
}
```

When a test (and all its cases) should run on all versions of a value or a cartesian
product of value is needed, use this syntax:

```rust
#[rstest]
fn a_fitting_run_is_welded_and_counted(
    #[values(Backend::Sequential, Backend::Multithreaded)] backend: Backend,
) {
    let source = format!("{PREAMBLE}{ARC}");

    let result = backend.run(&source, &options("arcs"), 1 << 20);

    assert_eq!(result, (WELDED_ARC.to_owned(), 1));
}
```

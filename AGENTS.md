# Rocky

A CLI tool for automating repeatable work. Combine multiple tools into a single project, then run it on demand or on a schedule. Written in Rust.

## Commands

- Build: `cargo build`
- Test: `cargo nextest run` (fall back to `cargo test` if nextest isn't installed)
- Lint: `cargo clippy --all-targets -- -D warnings`
- Format: `cargo fmt` (check only: `cargo fmt --check`)

## Rust Rules

- No `unwrap()` or `expect()` outside tests. Return `Result` and propagate with `?`.
- Errors: `thiserror` for library code, `anyhow` in `main` and binaries.
- No `unsafe` code. Enforce with `#![forbid(unsafe_code)]` at the crate root.
- Ask before adding a new dependency.
- Keep modules small, with one responsibility per file.

## Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If you're uncertain, ask.
- If a request has multiple interpretations, present them instead of picking one silently.
- If a simpler approach exists, say so. Push back when it's warranted.
- If something is unclear, stop, name what's confusing, and ask.

## Simplicity First

**Write the minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No flexibility or configurability that wasn't requested.
- No defensive checks for states that can't occur. When the compiler can't prove a state is impossible, propagate an error with `?` rather than panicking.
- If 200 lines could be 50, rewrite it.

The test: would a senior engineer call this overcomplicated? If so, simplify.

## Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match the existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it; don't delete it.

When your changes leave things unused:
- Remove imports, variables, and functions that *your* changes orphaned.
- Leave pre-existing dead code alone unless asked.

The test: every changed line should trace directly to the request.

## Goal-Driven Execution

**Define success criteria. Loop until verified.**

Turn tasks into verifiable goals:
- "Add validation" → write tests for invalid inputs, then make them pass.
- "Fix the bug" → write a test that reproduces it, then make it pass.
- "Refactor X" → confirm tests pass before and after.

For multi-step tasks, state a brief plan first:

```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you work independently; weak ones ("make it work") need constant clarification.

**Definition of done:** `cargo fmt`, `cargo clippy` (as above), and the test suite all pass. Don't report a task as complete until they do.

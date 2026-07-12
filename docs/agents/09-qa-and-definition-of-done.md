# QA And Definition Of Done

Orchestration note: use this doc as the final pass and merge gate for any phase or major change. After the subtasks finish, require the cleanup review, final code review, documentation updates, git commit with Codex as co-author, and push to `main` before calling the work done.

## Continuous Checks

Run these after every phase:

```text
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Run this scan as a guardrail:

```text
rg "unsafe|unwrap\\(|expect\\(|panic!|todo!|unimplemented!|unreachable!" src tests
```

Interpretation:

- production code in `src/` must stay clean
- tests may use assertions, but avoid hiding design problems behind panic-heavy helpers

## QA Responsibilities

- verify `Cargo.toml` has no dependencies
- verify crate roots forbid unsafe code
- verify production code has no unwrap, expect, panic, todo, unimplemented, unreachable, or unchecked indexing
- track supported, shimmed, and unsupported upstream tests
- require a minimized failing regression test before each bug fix
- require script-level tests for user-visible Lua behavior
- require GC latency tests for heap work

## Phase Exit Rule

A phase is not done because the code "looks complete". It is done only when:

- the phase-specific tests failed first
- the implementation makes those tests pass
- continuous checks pass
- no new unsupported behavior was introduced silently
- the final cleanup pass and documentation pass are complete
- the change set is committed and pushed only after the final pass succeeds

## Project Definition Of Done

The project is working only when all of the following are true:

- `cargo fmt --check` passes
- `cargo test` passes
- `cargo clippy --all-targets -- -D warnings` passes
- `Cargo.toml` has no external dependencies
- crate roots forbid unsafe code
- production code has no forbidden panic or unwrap patterns
- the parser accepts every local upstream Lua test file
- supported portions of `tests/upstream/lua-5.5.0-tests` pass
- the conformance runner emits a deterministic report for all local upstream scripts
- unsupported C-ABI or dynamic-loading items are documented with exact reasons
- GC latency tests demonstrate bounded per-step work and heap-size-independent pauses at configured safepoints
- `collectgarbage` matches Lua 5.5 observable behavior for supported modes
- `all.lua` runs as far as the documented stdlib-only and no-unsafe boundary without Rust panics

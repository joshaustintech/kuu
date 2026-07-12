# Kuu

Kuu is an in-progress clean-room implementation of Lua 5.5 written in Rust.

This repository intentionally stays small and strict:

- no Cargo dependencies
- no unsafe Rust
- stdlib-only implementation work

The long-term goal is to build a compatible Lua 5.5 runtime and tooling
surface while keeping the codebase easy to audit and reason about.

Status: active development.

## Local conformance

The Lua 5.5 manual, complete upstream Lua fixture corpus, Rust agent harness,
and C-facing fixture sources are vendored under this repository. No sibling
checkout is required. Run `./scripts/run-upstream-smoke.sh` to execute every
Lua smoke script; failures are expected while implementation proceeds. The
same gate is available as the ignored Rust test:
`cargo test --test upstream_smoke -- --ignored`.

Recent progress:

- handle-based GC foundations are in place with incremental step control and
  collector metrics for tests
- `collectgarbage` is wired into the runtime for the supported control surface

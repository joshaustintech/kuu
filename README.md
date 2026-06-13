# Kuu

Kuu is an in-progress clean-room implementation of Lua 5.5 written in Rust.

This repository intentionally stays small and strict:

- no Cargo dependencies
- no unsafe Rust
- stdlib-only implementation work

The long-term goal is to build a compatible Lua 5.5 runtime and tooling
surface while keeping the codebase easy to audit and reason about.

Status: active development.

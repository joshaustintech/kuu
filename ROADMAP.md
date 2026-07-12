# Kuu Lua 5.5 roadmap

Source of truth is vendored in this repository:

- Language and runtime specification: `docs/spec/lua-5.5/manual.html`
- Complete Lua smoke corpus: `tests/upstream/lua-5.5.0-tests/`
- Harness contract: `AGENT_HARNESS.md`

## Completion target

Implement Lua 5.5 sections 2.1-2.6 (values, environments, errors, metatables,
GC, coroutines), 3.1-3.4 (lexing, variables, statements, expressions), 6
(standard libraries), 7 (standalone interpreter), and 9 (syntax). The target
is observable behavior from `all.lua`; C ABI and dynamic loading remain
explicitly unsupported under safe, dependency-free Rust.

## Milestones

1. Lexer/parser: complete syntax, diagnostics, attributes, `global`, and all
   pure-Lua fixtures parse/tokenize.
2. Resolver/compiler: locals, upvalues, closures, varargs, control flow,
   bytecode, and protected execution.
3. VM core: values, tables, calls, operators, metamethods, errors, and
   coroutines.
4. Memory: safe handle-based GC, finalizers, `__close`, and collection modes.
5. Libraries: `basic`, `package`, `coroutine`, `string`, `utf8`, `table`,
   `math`, `io`, `os`, and `debug` surfaces used by the corpus.
6. CLI/conformance: Lua-compatible flags, stdin/REPL, deterministic smoke
   reporting, and regression-free upstream progress.

## Corpus inventory

All 34 Lua scripts are copied locally. `libs/` and `ltests/` C fixtures are
also copied for classification and future Rust-native shims. `api.lua` and C
dynamic-loading behavior are unsupported; every other script is an active
conformance target. Run `./scripts/run-upstream-smoke.sh` for the expected
failure report, or `cargo test --test upstream_smoke -- --ignored` for the
Rust smoke gate.

## Current status

Parser, compiler, VM core, GC foundations, metamethods, standard-library
helpers, and CLI are implemented in slices. Full corpus still expected to
fail. Each loop iteration must fix one failing script, preserve all existing
Rust checks, update this roadmap or harness ledger, and commit/push.

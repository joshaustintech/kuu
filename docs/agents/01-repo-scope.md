# Repo Scope

This project is a clean-room Lua 5.5 implementation in Rust.

Source of truth:

- Manual: `/Users/josh/lua-5.5/doc/manual.html`
- Tests: `/Users/josh/lua-5.5.0-tests`
- Test harness: `/Users/josh/lua-5.5.0-tests/all.lua`

Manual sections that matter most:

- `2.1` values and types
- `2.2` scopes, variables, and environments
- `2.3` error handling
- `2.4` metatables and metamethods
- `2.5` garbage collection
- `2.6` coroutines
- `3.1` lexical conventions
- `3.2` variables
- `3.3` statements
- `3.4` expressions
- `6` standard libraries
- `7` standalone interpreter behavior
- `9` complete syntax

Upstream test inventory:

- Harness: `all.lua`
- Language core: `main.lua`, `literals.lua`, `locals.lua`, `constructs.lua`, `code.lua`, `goto.lua`, `vararg.lua`, `closure.lua`, `calls.lua`
- Runtime behavior: `nextvar.lua`, `events.lua`, `sort.lua`, `tpack.lua`
- Numeric behavior: `math.lua`, `bitwise.lua`, `bwcoercion.lua`
- Libraries: `strings.lua`, `utf8.lua`, `pm.lua`, `attrib.lua`, `files.lua`, `db.lua`
- GC and stress: `gc.lua`, `gengc.lua`, `tracegc.lua`, `memerr.lua`, `big.lua`, `verybig.lua`, `heavy.lua`, `cstack.lua`
- Coroutines: `coroutine.lua`
- C-facing tests: `api.lua`, `libs/*.c`, `ltests/*`

Observed realities from the local suite:

- `_VERSION` must report `Lua 5.5`.
- `all.lua` routes `dofile` through `loadfile`, `string.dump`, and `load`.
- The suite depends on `debug`, `package`, `io`, `os`, coroutines, `__close`, finalizers, and GC mode controls.
- Lua 5.5 adds `global` declarations and attributes such as `<const>` and `<close>`.

Compatibility boundary:

- Pure Lua language and library behavior is in scope.
- A Rust-native compatibility shim for `T` is in scope where behavior can be represented safely.
- Dynamic C library loading is out of scope under stdlib-only and no-unsafe constraints.
- A real Lua C ABI is out of scope under stdlib-only and no-unsafe constraints.

Classification rules for upstream failures:

- `supported`: pure Lua or Rust-implementable runtime behavior
- `shimmed`: internal test helper behavior reproduced in Rust without C ABI
- `unsupported`: requires C ABI, platform dynamic loading, or forbidden unsafe code

When a test is unsupported, record:

- exact test file
- exact feature
- why stdlib-only or no-unsafe blocks it
- whether a Rust-native shim could cover a subset later

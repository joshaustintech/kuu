# Phases 8-10

Orchestration note: when this phase is requested, assess every subtask first, split independent work into concurrent small agents, keep dependent work sequential, and follow the default work loop in `AGENTS.md`.

## Phase 8: Low-Latency GC

Goal:
Build the safe handle heap before advanced runtime features depend on it.

Subtask 8.1 prompt:
```text
Implement a GC heap with stable safe handles, region metadata, side-table colors, load barriers, store barriers, and root registration. Add tests that all object access goes through barrier APIs.
```

Done when:

- heap objects are only accessed through heap APIs

Subtask 8.2 prompt:
```text
Implement incremental tri-color marking with a fixed work budget. Roots include stack, frames, globals, registry, threads, open upvalues, and interned strings.
```

Done when:

- one step visits at most the configured work budget
- unreachable cycles are collected

Subtask 8.3 prompt:
```text
Implement incremental region relocation with forwarding tables and load-barrier remapping. Preserve object identity through handles and table keys.
```

Done when:

- relocation preserves identity and contents
- remap work stays bounded

Subtask 8.4 prompt:
```text
Implement __gc finalization queues and VM support for to-be-closed variables with __close. Ensure close order and error propagation follow Lua rules.
```

Done when:

- reverse-order close tests pass
- close-on-error behavior passes

Subtask 8.5 prompt:
```text
Implement collectgarbage modes and parameters for Lua 5.5 observable behavior. Expose GC metrics for tests, including per-step work and pause counters.
```

Done when:

- selected `gc.lua`, `gengc.lua`, and `tracegc.lua` fixtures pass
- latency tests prove bounded step behavior

## Phase 9: Metatables And Operators

Goal:
Implement metamethod-driven runtime behavior.

Subtask 9.1 prompt:
```text
Implement metamethod dispatch for arithmetic, bitwise, unary, comparison, concat, length, and equality. Add recursion limits and typed errors.
```

Done when:

- `events.lua` and `bwcoercion.lua` operator fixtures pass

Subtask 9.2 prompt:
```text
Implement __index and __newindex with table and function forms. Include loop detection and clear runtime errors.
```

Done when:

- chained metatable fixtures pass

## Phase 10: Standard Libraries

Goal:
Implement Lua libraries with Rust std only.

Subtask 10.1 prompt:
```text
Implement base functions: assert, error, getmetatable, setmetatable, ipairs, pairs, next, pcall, xpcall, print, rawequal, rawget, rawlen, rawset, select, tonumber, tostring, type, warn, load, loadfile, dofile, collectgarbage, _G, _VERSION.
```

Subtask 10.2 prompt:
```text
Implement string and utf8 libraries with byte-oriented semantics and the pattern, formatting, and iteration behavior required by the local tests.
```

Subtask 10.3 prompt:
```text
Implement table and math libraries, including integer/float behavior, randomseed return values, and table sorting and packing semantics required by the local tests.
```

Subtask 10.4 prompt:
```text
Implement io and os libraries using Rust std only. Support file handles, stdio, open/read/write/close/lines, clock, difftime, remove, rename, setlocale, time, and tmpname.
```

Subtask 10.5 prompt:
```text
Implement package and debug behaviors needed by the local tests. package.loadlib must return a clear unsupported result under stdlib-only and no-unsafe constraints.
```

Subtask 10.6 prompt:
```text
Implement coroutine.create, resume, running, status, wrap, yield, isyieldable, and close, integrated with VM frames and to-be-closed variables.
```

Done when:

- selected library fixtures pass
- unsupported dynamic loading behavior is explicit and test-classified

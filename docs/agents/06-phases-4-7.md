# Phases 4-7

## Phase 4: Resolver

Goal:
Resolve locals, globals, upvalues, labels, gotos, and attributes before codegen.

Subtask 4.1 prompt:
```text
Implement scope resolution for Lua 5.5 local and global declaration rules, including implicit chunk-level global *, explicit global lists, global<const> *, and read-only global diagnostics.
```

Done when:

- resolver explains whether each name is local, global, or upvalue
- invalid writes fail before bytecode generation

Subtask 4.2 prompt:
```text
Resolve nested captures and classify upvalues. Add tests for shadowing, _ENV capture, and multi-level closure capture.
```

Done when:

- deterministic upvalue descriptors are produced

Subtask 4.3 prompt:
```text
Implement label and goto validation, including scope crossing restrictions and to-be-closed variable restrictions. Use goto.lua snippets as fixtures.
```

Done when:

- valid and invalid goto cases are distinguished before runtime

## Phase 5: Values And Bytecode

Goal:
Define runtime values and a safe bytecode surface.

Subtask 5.1 prompt:
```text
Implement Value with Nil, Boolean, Integer, Number, String handle, Table handle, Closure handle, NativeFunction, Thread handle, Userdata handle, and LightUserdata placeholder. Implement Lua equality and hash keys without unsafe code.
```

Done when:

- integer `42` equals number `42.0`
- value semantics tests pass

Subtask 5.2 prompt:
```text
Define bytecode instructions for loads, moves, globals, tables, arithmetic, comparisons, jumps, calls, returns, closures, varargs, for loops, concat, and close operations. Add encode/decode roundtrip tests.
```

Done when:

- instruction roundtrip tests pass
- invalid offsets return errors instead of panicking

## Phase 6: Compiler

Goal:
Compile resolved AST into executable bytecode.

Subtask 6.1 prompt:
```text
Implement compilation for literals, locals, globals, arithmetic, comparisons, logical and/or, tables, indexing, assignment, and multi-assignment. Add bytecode golden tests.
```

Done when:

- simple arithmetic and assignment fixtures compile correctly

Subtask 6.2 prompt:
```text
Compile if, while, repeat, numeric for, generic for, break, goto, labels, and return. Patch jumps through checked APIs only.
```

Done when:

- control-flow golden tests pass
- no invalid jump target is emitted silently

Subtask 6.3 prompt:
```text
Compile function declarations, local functions, global functions, nested closures, upvalues, varargs, method syntax, and tail-call markers.
```

Done when:

- closure and vararg compile tests pass

Subtask 6.4 prompt:
```text
Implement deterministic string.dump and load support for Kuu bytecode. The format may be Kuu-specific but must roundtrip compiled chunks and reject malformed input safely.
```

Done when:

- `load(string.dump(f))()` roundtrips

## Phase 7: VM Core

Goal:
Execute bytecode with explicit call frames.

Subtask 7.1 prompt:
```text
Implement the VM dispatch loop with explicit call frames, registers, stack values, Return, Load, Move, arithmetic, comparison, jumps, and Call for native functions. Add script-level tests.
```

Done when:

- `print("hello")` runs through the binary

Subtask 7.2 prompt:
```text
Implement table allocation, raw get/set, array/hash behavior, globals through _ENV, and table length basics. Use GC handles for all heap objects.
```

Done when:

- table and global fixtures pass

Subtask 7.3 prompt:
```text
Implement Lua function calls, native calls, returns, varargs, closure creation, open upvalues, closed upvalues, and tail-call frame reuse where valid.
```

Done when:

- closure and vararg integration tests pass
- deep Lua recursion uses VM frames, not Rust recursion

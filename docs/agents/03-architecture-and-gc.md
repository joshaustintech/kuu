# Architecture And GC

Execution pipeline:

```text
source bytes
  -> lexer
  -> token stream with spans
  -> parser
  -> AST
  -> resolver
  -> bytecode compiler
  -> VM
  -> low-latency GC heap
  -> observable Lua results
```

Core modules:

- `src/lib.rs`: public API and lint policy
- `src/main.rs`: CLI entrypoint
- `src/error.rs`: typed errors
- `src/span.rs`: source locations
- `src/lexer.rs`: tokenization
- `src/parser.rs`: AST parser
- `src/ast.rs`: AST nodes
- `src/resolve.rs`: locals, globals, upvalues, attributes
- `src/value.rs`: Lua values and key semantics
- `src/proto.rs`: compiled function prototypes
- `src/instruction.rs`: bytecode ISA
- `src/compiler.rs`: AST to bytecode
- `src/vm.rs`: interpreter and frames
- `src/gc.rs`: heap, handles, barriers, relocation
- `src/stdlib/`: library functions

## ZGC-Inspired Design

Reference properties to preserve:

- pauses independent of heap size
- concurrent or incremental work split into bounded steps
- relocating collection
- load-barrier based remapping

Safe Rust adaptation:

- use stable integer-like `GcHandle` values, not raw tagged pointers
- keep color, forwarding, generation, and pin state in side metadata
- never store long-lived direct Rust references to movable heap objects
- route every heap read through a load barrier
- route every heap write through a store barrier

Core GC pieces:

- region allocator by object kind and size class
- forwarding table from old handle to new handle
- relocation set chosen incrementally
- remembered sets for cross-generation or remap-sensitive edges
- root scanner for stacks, globals, registry, upvalues, threads, and interned strings
- resumable tasks for mark, remap, weak processing, finalization, and sweep

Rust-specific GC rules:

Handle access:

DO:
```rust
let table = heap.resolve_table(handle)?;
```

DON'T:
```rust
let table = &heap.tables[handle.index()];
```

Barriered writes:

DO:
```rust
heap.set_table_field(table, key, value)?;
```

DON'T:
```rust
table.entries.insert(key, value);
```

Forwarding-aware reads:

DO:
```rust
let resolved = heap.load_barrier(handle)?;
```

DON'T:
```rust
let resolved = handle;
```

## Latency Requirements

- A GC step must consume at most a configured work budget.
- A full cycle must be completable through repeated bounded steps.
- Small allocations after a large heap build must not trigger heap-proportional pauses.
- Relocation must preserve identity and observable Lua semantics.

Observable Lua surface that GC must support:

- `collectgarbage("count")`
- `collectgarbage("step")`
- `collectgarbage("stop")`
- `collectgarbage("restart")`
- `collectgarbage("isrunning")`
- `collectgarbage("incremental")`
- `collectgarbage("generational")`
- `collectgarbage("param")`

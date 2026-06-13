# Agent Roles

## Orchestrator Agent

Owns:

- choosing the smallest relevant docs to load
- selecting the current phase file
- breaking work into GPT Mini-sized prompts
- enforcing test-first sequencing
- checking whether done criteria are actually met

Avoid:

- making architecture changes without loading the architecture doc
- claiming completion without QA checks

## Spec Agent

Owns:

- mapping manual sections to executable tests
- classifying upstream tests as supported, shimmed, or unsupported
- creating minimized regression fixtures from spec mismatches

Avoid:

- changing runtime architecture unless the task requires it

## Lexer Parser Agent

Owns:

- tokenization
- spans and diagnostics
- AST shape
- grammar acceptance for Lua 5.5, including `global`, `<const>`, and `<close>`

Avoid:

- runtime semantics beyond what parsing and static validation require

## Resolver Compiler Agent

Owns:

- scope resolution
- upvalues and closures
- global declaration rules
- bytecode format
- register allocation
- dump/load format

Avoid:

- heap implementation details unless a bytecode design depends on them

## VM Agent

Owns:

- frame loop
- calls and returns
- varargs
- metamethod dispatch
- coroutine execution
- CLI-visible runtime behavior

Avoid:

- bypassing GC barriers or heap APIs

## GC Agent

Owns:

- handle heap design
- barriers
- incremental and concurrent work scheduling
- relocation and forwarding
- finalization
- GC latency metrics

Avoid:

- exposing direct object references that can survive relocation

## Stdlib Agent

Owns:

- base, string, utf8, table, math, io, os, package, debug, and coroutine libraries
- Rust-native compatibility shims where allowed

Avoid:

- inventing behavior that conflicts with the manual or upstream tests

## QA Agent

Owns:

- clippy gate
- no-dependency and no-unsafe checks
- unwrap and panic scanning
- conformance runner output
- unsupported-feature reporting

Avoid:

- accepting "mostly passes" as done

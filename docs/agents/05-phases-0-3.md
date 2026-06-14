# Phases 0-3

Orchestration note: when this phase is requested, assess every subtask first, split independent work into concurrent small agents, keep dependent work sequential, and follow the default work loop in `AGENTS.md`.

## Phase 0: Project Guardrails

Goal:
Create an empty Rust project that enforces the repo rules before language work starts.

Subtask 0.1 prompt:
```text
Create crate-level lint policy for Kuu. Forbid unsafe code. Deny clippy unwrap, expect, panic, todo, unimplemented, unreachable, and indexing/slicing. Add a minimal compile test. Do not add dependencies.
```

Done when:

- guardrail tests fail first, then pass
- `Cargo.toml` has no dependencies
- `cargo clippy --all-targets -- -D warnings` passes

Subtask 0.2 prompt:
```text
Implement src/error.rs with KError, KErrorKind, KResult, source spans, Display, and From<std::io::Error>. Write tests for formatting and conversion. Avoid unwrap, expect, panic, and indexing.
```

Done when:

- syntax and I/O error tests pass
- production code stays panic-free

## Phase 1: Test Harness And Spec Fixtures

Goal:
Make conformance measurable before implementation.

Subtask 1.1 prompt:
```text
Create a Rust integration test helper that runs the Kuu binary on a Lua script, captures stdout, stderr, and exit status, and compares expected output. Add one failing print("hello") fixture.
```

Done when:

- harness compiles
- `hello.lua` test exists and fails before VM support

Subtask 1.2 prompt:
```text
Add a conformance inventory test that lists /Users/josh/lua-5.5.0-tests/*.lua, assigns each file to a target phase, and fails if a local upstream test is unclassified.
```

Done when:

- all local upstream Lua scripts are classified
- `api.lua` and C-library checks are marked as shimmed or unsupported

## Phase 2: Lexer

Goal:
Tokenize Lua 5.5 with exact spans.

Subtask 2.1 prompt:
```text
Implement lexer token types and scanning for names, keywords, punctuation, operators, comments, and EOF. Include Lua 5.5 keywords, especially global. Write table-driven tests from manual section 3.1.
```

Done when:

- keyword and operator coverage tests pass
- line and column tracking is correct

Subtask 2.2 prompt:
```text
Implement decimal and hexadecimal integer and float tokenization. Keep token text intact and reject malformed numerals with syntax errors.
```

Done when:

- numerals used by upstream tests tokenize correctly
- malformed numerals return syntax errors

Subtask 2.3 prompt:
```text
Implement short strings, escapes, long bracket strings, and long bracket comments. Add tests for nested-looking brackets and escape failures.
```

Done when:

- every upstream Lua test file tokenizes successfully

## Phase 3: Parser And AST

Goal:
Parse the full Lua 5.5 grammar into a stable AST.

Subtask 3.1 prompt:
```text
Implement AST expression nodes and a Pratt parser for Lua 5.5 precedence and associativity. Include unary operators, varargs, functions, tables, prefix expressions, calls, and method calls.
```

Done when:

- precedence and associativity tests pass
- right-associative `^` and `..` are correct

Subtask 3.2 prompt:
```text
Implement statement parsing for assignments, calls, labels, break, goto, do, while, repeat, if, numeric for, generic for, function declarations, local declarations, global declarations, attributes, and return.
```

Done when:

- parser accepts every upstream test file
- malformed attributes produce useful syntax errors

Subtask 3.3 prompt:
```text
Add stable AST snapshot tests for arithmetic, tables, functions, closures, globals, and to-be-closed variables. Use deterministic formatting.
```

Done when:

- snapshot tests pass repeatedly with stable output

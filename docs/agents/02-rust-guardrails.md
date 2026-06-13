# Rust Guardrails

These rules apply to all production Rust code in this repo.

## Non-Negotiable Rules

- Use Rust stdlib only. `Cargo.toml` must not add dependencies.
- Forbid unsafe code in every crate root.
- Pass `cargo clippy --all-targets -- -D warnings`.
- Deny `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic`, `clippy::todo`, `clippy::unimplemented`, `clippy::unreachable`, and `clippy::indexing_slicing`.
- Use typed errors instead of panic-based control flow.
- Use explicit stacks and state machines where Lua input could drive deep recursion.

## Rule Examples

Stdlib only:

DO:
```rust
use std::collections::BTreeMap;
```

DON'T:
```rust
use hashbrown::HashMap;
```

No unsafe:

DO:
```rust
#![forbid(unsafe_code)]
```

DON'T:
```rust
unsafe { ptr.add(1).read() }
```

No unwrap or expect:

DO:
```rust
let token = tokens.get(index).ok_or_else(|| KError::eof(span))?;
```

DON'T:
```rust
let token = tokens[index];
let next = maybe_token.unwrap();
```

Typed errors, not panic:

DO:
```rust
return Err(KError::runtime("divide by zero", span));
```

DON'T:
```rust
panic!("divide by zero");
```

Checked arithmetic:

DO:
```rust
let next = pc.checked_add(offset).ok_or_else(|| KError::overflow(span))?;
```

DON'T:
```rust
let next = pc + offset;
```

Bound-safe access:

DO:
```rust
if let Some(value) = stack.get(slot) {
    use_value(value);
} else {
    return Err(KError::stack_oob(slot, span));
}
```

DON'T:
```rust
use_value(&stack[slot]);
```

Explicit interpreter stacks:

DO:
```rust
while let Some(frame) = vm.frames.last_mut() {
    step_frame(frame, heap)?;
}
```

DON'T:
```rust
fn eval_call(expr: &Expr) -> KResult<Value> {
    eval_call(next_expr)
}
```

## Preferred Rust Patterns

- `Result<T, KError>` for fallible operations
- `Option::ok_or_else` for missing values
- `slice.get` and `Vec::get_mut` for indexed access
- `checked_add`, `checked_sub`, `checked_mul` for counters and offsets
- small helper functions that isolate invariants and return errors instead of panicking
- deterministic iteration where tests depend on stable output

## Production Code Prohibitions

Do not use in `src/`:

- `unwrap`
- `expect`
- `panic!`
- `todo!`
- `unimplemented!`
- `unreachable!`
- indexing that can panic
- hidden recursion that can overflow on user-controlled depth

Tests may use assertion macros, but they should still avoid brittle panic-heavy helpers when a typed harness is practical.

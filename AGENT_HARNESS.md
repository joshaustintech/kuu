# Agent harness

Reusable task loop for Rust repositories.

## Loop contract

Each loop must:

1. Read `AGENTS.md` and the current source.
2. Pick one smallest unchecked task.
3. State scope in one sentence.
4. Edit only files required for that scope.
5. Run the smallest proof command.
6. Run `./scripts/after-task.sh`.
7. Update this file with result and next task.
8. Stop if proof fails and record the exact blocker.

## Checklist

- [ ] Confirm project-specific build and test commands.
- [ ] Add or update the smallest failing/targeted test.
- [ ] Implement one narrow slice.
- [ ] Run focused proof.
- [ ] Run the full after-task gate.
- [ ] Record proof and next task.

## Security review rule

Security leads are unproven until they identify exact code, attacker-controlled input, reachable preconditions, impact, and a reproducible proof or test. A heuristic or regex hit alone is not a finding. Review the classes in `scripts/security-watchlist.md` when a lead appears.

## Progress log

- 2026-07-12: Added zero-dependency `rust-harness` binary with `#![forbid(unsafe_code)]`. Proof: `cargo run`, `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` passed.
- 2026-07-12: Vendored harness workflow, Lua 5.5 manual, all 34 Lua smoke scripts, and `libs/`/`ltests/` fixtures. Rebased parser, lexer, and inventory tests on repo-local paths. Added ignored Rust conformance gate and `scripts/run-upstream-smoke.sh`. Proof: targeted smoke reports 34 expected failures; Rust format, test, Clippy, and security review pass. Next: fix one supported Lua smoke failure per loop iteration.
- 2026-07-12: Added Lua hexadecimal integer/float literal lowering with compiler regressions. Proof: focused compiler tests, full Rust checks, and smoke improved to 6 pass / 28 fail; `code.lua` and `heavy.lua` now pass. Next: fix one supported runtime failure.

# Kuu Agent Harness

## Trust Contract

Kuu is **not complete**. No agent may claim a phase, milestone, roadmap, or
implementation is complete unless this file contains a current, revision-bound
receipt for every applicable completion criterion. A passing focused test proves
only that behavior; it never proves a milestone or project complete.

When evidence is missing, stale, contradictory, or a required gate fails, say
`incomplete` and record the blocker. Never infer completion from implemented
slices, prior claims, commit messages, a clean diff, or an all-green focused
test run.

## Authority And Provenance Gate

Before selecting work or reporting status, record:

```text
git status --short --branch
git rev-parse HEAD
git ls-files --error-unmatch ROADMAP.md AGENT_HARNESS.md
```

- `ROADMAP.md` defines milestone scope; `AGENT_HARNESS.md` defines proof.
- If either tracked authority file is absent, branch is behind its intended
  source, or checkout provenance is unclear, stop milestone accounting and
  report `status unverified` with exact command output.
- Never alter, overwrite, or silently work around unrelated dirty files.

## Completion States

Use exactly one state per item:

- `not started`: no implementation receipt.
- `in progress`: scoped work exists; exit criteria not proven.
- `blocked`: exact failing command/fixture or missing authority recorded.
- `unsupported`: C-ABI or dynamic-loading feature only; exact test, feature,
  and safe-Rust reason recorded.
- `complete`: every listed exit criterion has a current receipt.

`complete` is forbidden while any supported upstream fixture fails, the
conformance report is absent/stale, or any required QA gate fails.

## Required Receipt

Every status update records date, `HEAD`, command, exit code, and concise
result. For implementation slices also record failing-first regression test,
files changed, and next remaining item. Do not call a command "passed" unless
this loop ran it successfully at recorded `HEAD`.

Required project-completion commands:

```text
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
rg "unsafe|unwrap\\(|expect\\(|panic!|todo!|unimplemented!|unreachable!" src tests
./scripts/run-upstream-smoke.sh
```

The upstream receipt must deterministically classify every local script as
`pass`, `fail`, or `unsupported`, identify each supported failure, and retain
the report path. `api.lua`/C dynamic loading may be unsupported only with an
exact reason; no other script becomes unsupported by convenience.

## Remaining Work Ledger

This ledger is deliberately fail-closed. Update it after evidence, never by
guessing.

| Item | State | Evidence / next proof |
| --- | --- | --- |
| Authority files and branch provenance | complete | 2026-07-19, `94e6f100f25403e9c89b13c350aa2736860256ee`: stashed local changes, rebased the local conformance commit onto `origin/main`, restored the stash, and `git push` updated `main` from `cb38b3f9` to `94e6f100`. `ROADMAP.md` and this harness are tracked. |
| Milestones 1-6 | in progress | Origin roadmap says full corpus is expected to fail. Do not mark any complete until its fixture and conformance receipts exist. |
| Phase 12 full conformance | in progress | 2026-07-19, `f4547850` plus uncommitted `src/vm.rs` and `tests/vm_phase7.rs`: a copied `calls.lua` prefix failed first with `invalid table handle`; allocation during GC sweep is now marked before sweep. Regression passes and bounded smoke improves from 26 to 25 failures: 9 pass, 21 fail, 4 timeout. `calls.lua` now reaches distinct `attempt to call a non-callable value`; minimize that next. Each smoke run left no Kuu process. |
| GC/finalizers | blocked | Lua-level table `__gc` finalizer remains unproven/failing. Add failing-first regression, then prove finalizer and `__close` order/error behavior. |
| IO library | blocked | `file:seek()` and `io.lines()` remain failing/unimplemented. Add fixture-level regressions and run affected upstream scripts. |
| QA gate | blocked | 2026-07-19, `978bce65` plus uncommitted `tests/support/mod.rs` and `tests/phase11_cli.rs`: test process harness now enforces a 10-second default timeout, kills and reaps hung Kuu children, and reports `TimedOut`; a 100 ms infinite-loop regression passes and leaves no Kuu process. `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets -- -D warnings` exit 0; post-suite process check found no Kuu process. Forbidden-pattern scan reports only crate-root `#![forbid(unsafe_code)]`. Deterministic smoke receipt exists but 26 supported scripts still fail or time out. |
| Tooling phases 13-16 | not started | `kuulsp`, MCP server, `kuufmt`, and `kuulint` have no current implementation or integration receipts. |

## Loop Procedure

1. Run authority/provenance gate. If it fails, update ledger only; do not make
   completion claims.
2. Read `ROADMAP.md`, this ledger, relevant phase doc, and current failing
   fixture.
3. Choose one supported blocked item. Write a minimized test that fails first.
4. Implement smallest safe slice; run focused proof.
5. Run all required receipt commands. A failing gate keeps the item `blocked`.
6. Update this ledger with only observed facts, including remaining failures.
7. Before any `complete` claim, independently re-read roadmap, ledger, test
   report, git status, and receipts. If one is missing, say `incomplete`.

## Prior False-Completion Diagnosis

The prior loop could stop after a local slice because its old checklist asked
for a "smallest unchecked task" and "next task" but had no authoritative
milestone matrix, no branch/provenance check, no mandatory full-conformance
receipt, and no rule making a failed QA gate veto completion. Its
`after-task.sh` gate ran formatting, Clippy, Rust tests, and security review,
but not the upstream smoke runner. This was process failure, not evidence of
completion. This harness corrects it by making completion evidence-bound and
fail-closed.

# Kuu Agent Table of Contents

This file is the table of contents for the agent playbook. Do not load every file by default. Load only the smallest set needed for the current task.

Project targets:

- Spec: `/Users/josh/lua-5.5/doc/manual.html`
- Tests: `/Users/josh/lua-5.5.0-tests`
- Harness entrypoint: `/Users/josh/lua-5.5.0-tests/all.lua`

## Load Order

Start here, then load only the files that match the task:

1. [01-repo-scope.md](/Users/josh/kuu/docs/agents/01-repo-scope.md)
2. [02-rust-guardrails.md](/Users/josh/kuu/docs/agents/02-rust-guardrails.md)
3. [03-architecture-and-gc.md](/Users/josh/kuu/docs/agents/03-architecture-and-gc.md)
4. [04-agent-roles.md](/Users/josh/kuu/docs/agents/04-agent-roles.md)
5. [05-phases-0-3.md](/Users/josh/kuu/docs/agents/05-phases-0-3.md)
6. [06-phases-4-7.md](/Users/josh/kuu/docs/agents/06-phases-4-7.md)
7. [07-phases-8-10.md](/Users/josh/kuu/docs/agents/07-phases-8-10.md)
8. [08-phases-11-12.md](/Users/josh/kuu/docs/agents/08-phases-11-12.md)
9. [09-qa-and-definition-of-done.md](/Users/josh/kuu/docs/agents/09-qa-and-definition-of-done.md)

## File Guide

### [01-repo-scope.md](/Users/josh/kuu/docs/agents/01-repo-scope.md)

Purpose:
Define the source of truth, local test inventory, and the hard compatibility boundary around C API and dynamic library tests.

Load when:
- You are deciding what the project must implement.
- You need to classify an upstream Lua 5.5 test.
- You need to explain why a test is supported, shimmed, or unsupported.

Do not load when:
- You are already implementing a scoped parser, VM, GC, or stdlib subtask and the compatibility boundary is already known.

### [02-rust-guardrails.md](/Users/josh/kuu/docs/agents/02-rust-guardrails.md)

Purpose:
Define all Rust-specific engineering constraints, including stdlib-only, no unsafe, no unwraps, no panic-based production control flow, and explicit DO/DON'T examples.

Load when:
- You are writing or reviewing Rust code.
- You are adding error handling, allocation code, stack logic, or lint policy.
- You are preparing a GPT Mini prompt that touches Rust implementation details.

Do not load when:
- You are only classifying Lua tests or editing plan text that does not contain Rust implementation guidance.

### [03-architecture-and-gc.md](/Users/josh/kuu/docs/agents/03-architecture-and-gc.md)

Purpose:
Define the execution pipeline, module layout, and the safe Rust adaptation of a ZGC-inspired low-latency collector.

Load when:
- You are designing or editing the lexer, parser, compiler, VM, value model, or GC.
- You need the handle/barrier/region rules for heap work.
- You are evaluating whether a design violates the low-latency GC requirement.

Do not load when:
- You are doing a narrow documentation-only change unrelated to runtime architecture.

### [04-agent-roles.md](/Users/josh/kuu/docs/agents/04-agent-roles.md)

Purpose:
Define what each specialized agent owns and what it should avoid.

Load when:
- You are orchestrating work across multiple agents or prompts.
- You need to decide which agent should own a failure or feature.

Do not load when:
- A single agent is already working on one tightly scoped code task.

### [05-phases-0-3.md](/Users/josh/kuu/docs/agents/05-phases-0-3.md)

Purpose:
Cover guardrails, test harness setup, lexer, and parser milestones with GPT Mini prompts and test-driven done criteria.

Load when:
- You are working on project bootstrap, harnesses, lexing, parsing, ASTs, or syntax diagnostics.

Do not load when:
- You are working only on VM execution, GC, libraries, or final conformance reporting.

### [06-phases-4-7.md](/Users/josh/kuu/docs/agents/06-phases-4-7.md)

Purpose:
Cover resolver, values, bytecode, compiler, and VM core milestones with GPT Mini prompts and test-driven done criteria.

Load when:
- You are working on scoping, upvalues, globals, bytecode, closures, calls, or core execution.

Do not load when:
- You are working only on lexer/parser setup or GC/library behavior.

### [07-phases-8-10.md](/Users/josh/kuu/docs/agents/07-phases-8-10.md)

Purpose:
Cover low-latency GC, metatables, operators, and standard library milestones with GPT Mini prompts and test-driven done criteria.

Load when:
- You are implementing heap management, `collectgarbage`, finalizers, metamethods, or standard libraries.

Do not load when:
- You are working only on early parsing/compiler milestones or CLI plumbing.

### [08-phases-11-12.md](/Users/josh/kuu/docs/agents/08-phases-11-12.md)

Purpose:
Cover standalone CLI behavior, REPL work, full-suite execution, and C-API compatibility triage.

Load when:
- You are implementing the CLI, `arg`, `-e`, `-l`, stdin execution, or conformance runner behavior.
- You are closing the gap to the upstream suite.

Do not load when:
- You are doing isolated runtime work below the CLI layer.

### [09-qa-and-definition-of-done.md](/Users/josh/kuu/docs/agents/09-qa-and-definition-of-done.md)

Purpose:
Define continuous checks, reporting, and the project-level test-driven definition of done.

Load when:
- You are running QA, preparing a merge gate, or deciding whether a phase is actually complete.
- You need the exact conformance and GC latency completion criteria.

Do not load when:
- You are still exploring an implementation idea and do not need exit criteria yet.

## Usage Rule

For any task, load:

- `01-repo-scope.md` if the supported surface is unclear
- `02-rust-guardrails.md` for any Rust implementation work
- exactly one phase file matching the current milestone
- `03-architecture-and-gc.md` only if the task touches runtime design or GC
- `09-qa-and-definition-of-done.md` when validating completion

## Git And GitHub Workflow

- Use `git` directly for staging, committing, branching, and pushing.
- Do not use `gh` for git or GitHub operations unless the user explicitly asks for it.

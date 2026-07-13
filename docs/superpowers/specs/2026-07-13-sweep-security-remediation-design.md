# Sweep and Security Remediation Design

## Context

The non-security audit work is already on `main`. A final sweep found four remaining quality gaps, while the exact `main` CI run exposed three red security gates. The user has now approved fixing both groups and landing the result directly on `main` after review and verification.

## Goals

- Make the cross-platform CI checks finish without executing Criterion benchmarks.
- Align the public sourcemap WASM documentation and tests with generated runtime contracts.
- Reject invalid deterministic benchmark bounds instead of silently producing misleading inputs.
- Close completed audit documentation and remove ignored session artifacts.
- Clear every reproducible Rust dependency and GitHub Actions security finding.
- Review untrusted CLI input boundaries and unsafe Rust invariants, then fix every issue that can be demonstrated with a regression test.
- Preserve public APIs unless an existing behavior is unsafe, incorrect, or already contradicted by generated declarations.

## Non-goals

- New product features or unrelated refactors.
- Publishing a release.
- Changing branch protection or repository security settings.
- Replacing intentional low-level Rust optimizations when their safety invariants are valid and tested.
- Claiming that an external advisory source was checked when access is unavailable.

## Design

### Cross-platform CI

The platform matrix will execute Rust libraries, binaries, integration tests, examples, and doc tests without running benchmark harnesses. Benchmark compilation remains covered by the existing dedicated job and by an all-targets check. A workflow policy test will reject `cargo test --workspace --all-targets` in the platform check job so Criterion sampling cannot return unnoticed.

### WASM contract

The sourcemap WASM README will describe out-of-range `source()` and `name()` results as `undefined`, matching wasm-bindgen declarations and runtime behavior. `ignoreList` will be documented as `Uint32Array`. Runtime tests will pin these boundary values for both `SourceMap` and `LazySourceMap` where applicable.

### Benchmark input validation

`createDeterministicLookups` will reject non-integer or negative counts and non-positive or non-integer exclusive bounds with `RangeError`. The seed remains coerced to unsigned 32-bit state because that behavior is intentional. Tests will cover valid deterministic output and each invalid input class.

### Rust dependency advisories

The lockfile will move `crossbeam-epoch` to a patched version accepted by the existing dependency graph. The implementation is complete only when both Cargo Audit and Cargo Deny report no advisory failure. Duplicate-version warnings are not security failures and will only be changed if a safe dependency update removes them without unrelated churn.

### Release workflow supply chain

Release jobs will stop installing `@napi-rs/cli` globally outside the lockfile. They will install the frozen pnpm workspace and invoke the locked CLI through pnpm. The npm self-upgrade step will be removed because Node 24 already provides an npm version compatible with trusted publishing. Workflow policy tests and zizmor will guard against reintroducing ad hoc package installation.

### Targeted code security review

The review will focus on boundaries that consume untrusted data:

- CLI URL fetching and externally referenced source maps.
- Source extraction paths, platform-specific path components, and symlink containment.
- Response and decoded input resource limits where remote input can cause unbounded allocation.
- Unsafe Rust blocks that construct strings or write through raw pointers.
- Repository secrets and generated artifact handling.

A code change requires a concrete exploit or invariant failure reproduced by a focused test. Path extraction must keep every created file under the selected output directory on supported platforms. Network behavior will retain its documented purpose, but redirects, filenames, and response handling must not bypass the intended output or resource boundary.

### JavaScript advisory coverage

The user approved external pnpm advisory analysis after being informed that it submits the dependency graph to the registry audit service. The result will be recorded as pass or as a concrete dependency update. GitHub Dependabot alerts are currently unavailable because the repository feature is disabled, so they are not an acceptance gate.

### Documentation and local hygiene

The prior implementation plan will receive an explicit completion note instead of selectively changing its historical checklist. Ignored `.superpowers/sdd` session artifacts will be removed after confirming they contain no tracked files.

## Testing strategy

Every behavior change follows red-green TDD. Independent changes may be implemented in parallel, but each receives a separate requirements and code-quality review before integration.

Focused validation includes:

- Workflow policy tests before and after CI edits.
- WASM builds plus Node runtime tests.
- Benchmark helper unit tests.
- Cargo Audit, Cargo Deny, and zizmor.
- Focused CLI repro tests for every confirmed security finding.
- A real fixture or real remote asset when a CLI fetch or extraction behavior changes.

Final validation includes formatting, Clippy with warnings denied, all non-benchmark Rust tests, all-target compilation, JavaScript hygiene and tests, package builds, and a clean diff. After fast-forwarding into local `main`, the exact pushed commit must complete all relevant GitHub Actions jobs. Any remaining failure must be explicitly identified and must not be caused by this change set.

## Landing strategy

Work occurs on `codex/fix-all-sweep-security`. Commits are signed and conventional, with focused scopes where practical. After final review, local `main` is fast-forwarded to the reviewed branch and pushed. The feature branch is removed after the remote commit and relevant CI checks are verified.

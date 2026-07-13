# Audit Findings Remediation Design

## Scope

Implement every non-security finding from the 2026-07-13 standard codebase audit. Security findings remain explicitly out of scope at the user's request.

The work covers correctness, public package contracts, release reliability, dependency determinism, CI coverage, benchmark reliability, lazy lookup performance, documentation, and the status of currently unreleased binding crates.

## Design decisions

### Parser behavior

Parser variants that cannot flatten indexed source maps must reject `sections` with the existing `ParseError::NestedIndexMap` error. They must never return a successful empty map for indexed input. The full `SourceMap::from_json` path remains the supported indexed-map parser.

### Browser package contract

`@srcmap/sourcemap-wasm/browser` will provide static ESM named exports for the generated web bindings plus a default async initializer. The initializer remains idempotent. Browser consumers must be able to import `SourceMap` exactly as documented.

The undocumented and nonexistent `pkg/fast.js` path will not be revived. Documentation will point users to the public `LazySourceMap` export instead.

### NAPI type contract

Hand-maintained declarations must reflect runtime nullability. A lightweight drift check will protect the published declarations from diverging from the generated NAPI surface again.

### Release behavior

Rust crate publication will skip only when the exact crate version is confirmed to exist on crates.io. Authentication, packaging, dependency-order, registry, and other publication errors must fail the workflow.

### JavaScript dependency graph

`pnpm-lock.yaml` is the only authoritative JavaScript lockfile. Stale per-package npm lockfiles will be removed. CI and benchmark jobs will use frozen pnpm installs.

### Test and benchmark gates

Existing correctness checks must affect process exit status. Parallel encoder tests, public lazy WASM bindings, browser exports, and symbolicate WASM bindings must run in CI. Random benchmark lookup inputs will use a fixed deterministic generator.

### Lazy lookup performance

Investigation confirmed that public lazy lookups cache every traversed line and never create the backward cache miss assumed by the prefix-rescan finding. That finding is therefore invalid, and no VLQ checkpoints are retained. Internal lookup paths borrow cached mapping slices without cloning them. The public `decode_line` owned return type stays compatible.

Performance changes must be measured against deterministic ascending, descending, repeated, and randomized real-world lookup workloads. Revert any optimization that does not improve its target without a meaningful regression elsewhere.

### Binding status

Generator NAPI, remapping NAPI, and scopes WASM remain in the Rust workspace as experimental, unpublished binding crates. Their manifests and contributor documentation will state that status explicitly so they are not mistaken for released npm packages.

### Documentation

Fresh-clone instructions will use Corepack and pnpm, list the binding builds required before JavaScript tests, remove the nonexistent pre-commit hook claim, and distinguish released packages from experimental binding crates.

## Verification contract

Every behavior change follows a red-green test cycle. The final branch must pass:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --lib --tests --examples --all-features
pnpm install --frozen-lockfile
pnpm run check
pnpm run test
cargo deny check
```

Package and workflow-specific regression tests must also pass. Performance work requires before-and-after benchmark evidence. The working tree must be clean except for the intended branch changes.

## Explicit exclusions

- All security findings from the audit.
- New roadmap features such as debug-ID extraction or streaming Node APIs.
- Publishing any currently experimental binding package.
- Unrelated refactoring of large Rust modules.

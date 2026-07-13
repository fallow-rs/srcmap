# Fix Audit Findings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix every non-security finding from the 2026-07-13 srcmap audit and prove the resulting package, CI, release, documentation, and performance contracts.

**Architecture:** Preserve existing public APIs wherever possible. Reject unsupported inputs explicitly, make package exports and declarations match runtime behavior, tighten CI and release gates, and optimize lazy lookup internals behind the existing API. Keep the current Rust workspace and pnpm monorepo structure.

**Tech Stack:** Rust 2024, Cargo, wasm-bindgen, NAPI-RS, Node.js test runner, pnpm, GitHub Actions, Criterion2 and CodSpeed.

## Global Constraints

- Do not implement or disclose any security finding from the audit.
- Do not add dependencies unless the standard library or current dependencies cannot solve the problem.
- Follow red-green TDD for behavior changes.
- Keep public Rust and JavaScript APIs backward compatible except where correcting an already-false declaration or rejecting a previously silent invalid result.
- Use 0-based lines and columns internally.
- Use arrow functions and named exports in new JavaScript helpers.
- Keep performance changes only with deterministic before-and-after evidence.
- Do not publish, push, or alter repository protection settings.
- Never use em dashes in code, comments, documentation, or commit messages.

---

### Task 1: Reject indexed maps in unsupported parser variants

**Files:**
- Modify: `crates/sourcemap/src/lib.rs`
- Modify: `packages/sourcemap-wasm/__tests__/sourcemap-wasm.test.mjs`

**Interfaces:**
- Preserve `SourceMap::from_json_no_content`, `SourceMap::from_json_lines`, and `LazySourceMap::from_json` signatures.
- Return `ParseError::NestedIndexMap` when `RawSourceMapLite.sections` or `RawSourceMap.sections` is present.

- [ ] Add Rust tests showing all three constructors currently accept an indexed map and return an empty map.
- [ ] Run the focused tests and confirm they fail because the constructors return `Ok`.
- [ ] Add the same `sections.is_some()` guard already used by `LazySourceMap::from_json_no_content` to every unsupported constructor.
- [ ] Add a WASM constructor test proving the no-content JavaScript path throws for indexed input.
- [ ] Run `cargo test -p srcmap-sourcemap indexed` and the sourcemap WASM Node test, expecting success.

### Task 2: Repair the browser entrypoint and fast-mode documentation

**Files:**
- Modify: `packages/sourcemap-wasm/browser/index.mjs`
- Create: `packages/sourcemap-wasm/__tests__/browser.test.mjs`
- Modify: `packages/sourcemap-wasm/package.json`
- Modify: `packages/sourcemap-wasm/README.md`

**Interfaces:**
- Default export remains `init(input): Promise<void>` and stays idempotent.
- Named exports include `SourceMap`, `LazySourceMap`, `resultPtr`, and `wasmMemory` from the generated web module.

- [ ] Add a Node ESM smoke test that imports the documented named exports from `browser/index.mjs`; confirm it fails because `SourceMap` is not exported.
- [ ] Replace dynamic export mutation with static imports and re-exports from `../web/srcmap_sourcemap_wasm.js`.
- [ ] Keep a module-level initialization promise and call the generated default initializer exactly once.
- [ ] Add the browser smoke test to the package and root JavaScript test commands.
- [ ] Replace the nonexistent `pkg/fast.js` documentation with direct `LazySourceMap` usage from the supported package entrypoint.
- [ ] Build the web target and run the browser smoke test, expecting success.

### Task 3: Correct and guard NAPI declaration nullability

**Files:**
- Modify: `packages/sourcemap/index.d.ts`
- Modify: `packages/sourcemap/__tests__/sourcemap.test.mjs`
- Create: `.github/scripts/check-napi-declarations.mjs`
- Create: `.github/scripts/check-napi-declarations.test.mjs`
- Modify: `package.json`

**Interfaces:**
- `source(index: number): string | null`
- `name(index: number): string | null`

- [ ] Add runtime tests for out-of-range `source()` and `name()` and confirm the declarations still disagree with observed nulls.
- [ ] Update the declarations and JSDoc return descriptions.
- [ ] Add a dependency-free drift checker that compares selected public signatures in the generated declaration at `target/napi-sourcemap.d.ts` with `packages/sourcemap/index.d.ts`.
- [ ] Add unit tests for matching and mismatching declarations.
- [ ] Add a root `check:napi-declarations` command and run it after building the NAPI package.

### Task 4: Make Rust publication failures actionable

**Files:**
- Create: `.github/scripts/publish-crate-if-needed.sh`
- Create: `.github/scripts/publish-crate-if-needed.test.mjs`
- Modify: `.github/workflows/release.yml`
- Modify: `package.json`

**Interfaces:**
- Script usage: `publish-crate-if-needed.sh <crate-name>`.
- Skip only when crates.io confirms the workspace version exists.
- Propagate every `cargo publish` failure otherwise.

- [ ] Add tests with stubbed `cargo`, `curl`, and `jq` commands for existing version, missing version with successful publish, registry lookup failure, and failed publish.
- [ ] Confirm the old workflow fails the policy test because it contains `cargo publish ... || echo`.
- [ ] Implement the script using `cargo metadata` for the exact package version and the crates.io version endpoint for existence.
- [ ] Replace every inline publish command with the helper.
- [ ] Add the helper tests to the root test command and confirm all branches pass.

### Task 5: Make pnpm locking authoritative

**Files:**
- Delete: `benchmarks/package-lock.json`
- Delete: `packages/codec/package-lock.json`
- Delete: `packages/gen-mapping/package-lock.json`
- Delete: `packages/sourcemap/package-lock.json`
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/bench.yml`
- Modify: `.github/workflows/coverage.yml`
- Modify: `CONTRIBUTING.md`

**Interfaces:**
- `pnpm-lock.yaml` is the only JavaScript lockfile.
- CI installs use `pnpm install --ignore-scripts --frozen-lockfile`.

- [ ] Add or extend a workflow policy test that rejects `--no-frozen-lockfile` and checked-in `package-lock.json` files.
- [ ] Confirm it fails against the current repository.
- [ ] Remove obsolete npm lockfiles and switch every pnpm CI install to frozen mode.
- [ ] Update contributor commands to use Corepack and pnpm.
- [ ] Run `pnpm install --frozen-lockfile` and the workflow policy test.

### Task 6: Make benchmark inputs and correctness deterministic

**Files:**
- Create: `benchmarks/workload.mjs`
- Create: `benchmarks/workload.test.mjs`
- Modify: `benchmarks/real-world.mjs`
- Modify: `benchmarks/package.json`
- Modify: `package.json`

**Interfaces:**
- Export `createDeterministicLookups(count, maxLine, maxColumn, seed)`.
- Export `setFailureExitCode(results)` or an equivalently focused correctness helper.

- [ ] Add tests proving identical seeds produce identical lookup arrays, different seeds differ, bounds hold, and any failed result sets `process.exitCode`.
- [ ] Confirm tests fail before the helpers exist.
- [ ] Move lookup generation and aggregate correctness status into the helper module.
- [ ] Use a named seed constant in `real-world.mjs` and make any WASM or NAPI mismatch fail the process after reporting all maps.
- [ ] Run the helper tests and a real-world benchmark correctness-only smoke where fixtures are available.

### Task 7: Cover the public lazy WASM binding

**Files:**
- Modify: `packages/sourcemap-wasm/__tests__/sourcemap-wasm.test.mjs`

**Interfaces:**
- Cover `LazySourceMap` constructor, `fromParts`, structured lookup, flat lookup, static-buffer lookup, batch lookup, caching, out-of-order queries, invalid JSON, and cleanup.

- [ ] Add tests that import `LazySourceMap` and compare results with eager `SourceMap` using the same fixtures.
- [ ] Confirm at least the indexed-map and backward-query cases fail before dependent fixes.
- [ ] Keep assertions behavioral and reuse existing fixture constants.
- [ ] Run the package test after Tasks 1 and 10, expecting success.

### Task 8: Execute parallel encoder tests in CI

**Files:**
- Modify: `.github/workflows/ci.yml`
- Create or modify: `.github/scripts/workflow-policy.test.mjs`

**Interfaces:**
- Linux CI must run codec and generator tests with `--features parallel`.

- [ ] Add a workflow policy assertion that both feature-gated test commands are present.
- [ ] Confirm it fails against the current workflow.
- [ ] Add a focused `Parallel features` job or Linux-only step with exact package commands.
- [ ] Run both feature-specific Cargo test commands locally and the workflow policy test.

### Task 9: Build and test symbolicate WASM before release

**Files:**
- Create: `packages/symbolicate-wasm/__tests__/symbolicate-wasm.test.mjs`
- Modify: `packages/symbolicate-wasm/package.json`
- Modify: `package.json`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Test `parseStackTrace` and `symbolicate` through the generated Node wrapper.
- Exercise successful loading, missing maps, malformed loader values, repeated-file caching, and output shape.

- [ ] Write Node boundary tests first and confirm they cannot run because CI and package scripts do not build or execute this package.
- [ ] Add package test scripts and include the tests in root `test:js` and coverage commands.
- [ ] Build node and web symbolicate targets in JS Runtime CI.
- [ ] Add a web-module export smoke test without instantiating browser-only globals.
- [ ] Run the symbolicate WASM package tests after building both targets.

### Task 10: Validate fast-lazy prefix rescans and remove lookup clones

**Files:**
- Modify: `crates/sourcemap/src/lib.rs`
- Modify: `crates/sourcemap/benches/parse.rs`
- Modify: `benchmarks/real-world.mjs`

**Interfaces:**
- Preserve public `LazySourceMap::decode_line() -> Result<Vec<Mapping>, DecodeError>`.
- Internal original-position lookup must use cached slices without cloning.
- Do not add checkpoints unless a reachable public cache-miss path is proven.

- [ ] Add behavioral tests for descending and randomized lookup order.
- [ ] Confirm that public backward queries cannot miss previously traversed cached lines, classifying the prefix-rescan finding as invalid.
- [ ] Remove the experimental checkpoint implementation and its unreachable private-cache regression test.
- [ ] Split cache population from access so internal lookup borrows the cached vector while public `decode_line` clones only for its owned return contract.
- [ ] Add deterministic ascending, descending, repeated, and randomized fast-lazy lookup benchmarks.
- [ ] Measure baseline and candidate using Criterion2. Keep only the clone-removal change after isolated warm-cache workloads improve without a meaningful eager regression.
- [ ] Run all sourcemap Rust and WASM tests.

### Task 11: Make contributor onboarding executable

**Files:**
- Modify: `CONTRIBUTING.md`

**Interfaces:**
- Fresh-clone setup uses Corepack, pnpm, explicit NAPI and WASM builds, then root checks and tests.

- [ ] Replace npm commands with the repository's pinned pnpm workflow.
- [ ] List the exact build commands required before JavaScript tests.
- [ ] Remove the claim that a pre-commit hook is installed.
- [ ] Check every documented command against `package.json` and package scripts.

### Task 12: Classify unreleased binding crates

**Files:**
- Modify: `packages/generator/Cargo.toml`
- Modify: `packages/remapping/Cargo.toml`
- Modify: `packages/scopes-wasm/Cargo.toml`
- Modify: `CONTRIBUTING.md`
- Modify: `README.md`

**Interfaces:**
- These crates remain workspace members.
- Their Cargo packages use `publish = false`.
- Documentation labels them experimental and not published as npm packages.

- [ ] Add a manifest policy test that identifies workspace binding crates without `publish = false` and without a release path.
- [ ] Confirm it fails for the three selected crates.
- [ ] Add `publish = false` and document their experimental status separately from released packages.
- [ ] Run Cargo metadata, workspace check, and documentation link checks.

### Task 13: Complete repository verification and review

**Files:**
- Modify only files required by failures found during verification.

- [ ] Run `cargo fmt --all --check`.
- [ ] Run `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- [ ] Run `cargo test --workspace --lib --tests --examples --all-features`.
- [ ] Run `pnpm install --frozen-lockfile`.
- [ ] Build all released NAPI and WASM bindings required by JavaScript tests.
- [ ] Run `pnpm run check` and `pnpm run test`.
- [ ] Run `cargo deny check`.
- [ ] Run package packing dry-runs for every released npm package and inspect included files and exports.
- [ ] Re-run the original browser import, indexed-parser, benchmark-exit, and declaration-nullability reproductions.
- [ ] Review `git diff` against this plan, confirm every audit finding maps to evidence, and remove any unrelated change.

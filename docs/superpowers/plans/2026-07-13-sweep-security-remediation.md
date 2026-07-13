# Sweep and Security Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix every reproducible quality and security issue found by the final sweep, restore all CI gates, and land the reviewed result on `main`.

**Architecture:** Keep fixes inside existing subsystem boundaries. Centralize the VLQ capacity invariant, enforce extraction containment before filesystem writes, parse and validate fetched URLs structurally, and protect workflow contracts with dependency-free policy tests. Use focused commits and review each independently testable task.

**Tech Stack:** Rust 2024, Cargo, Miri, ureq 3, url 2, wasm-bindgen, Node.js test runner, pnpm 10.33.0, GitHub Actions, Cargo Audit, Cargo Deny, actionlint, and zizmor 1.26.1.

## Global Constraints

- Every behavior change follows red-green TDD. Documentation and generated lockfile updates receive focused validation even when no behavior test can fail first.
- Preserve public APIs unless an existing behavior is unsafe, incorrect, or contradicted by generated declarations.
- Keep 0-based source-map positions unchanged.
- Do not publish a release or change branch protection.
- Add no dependency except `url = "2"`, which is required for standards-compliant URL parsing and joining.
- Use signed conventional commits and never add AI attribution.
- Never use em dashes in code, comments, documentation, commit messages, or reports.
- Long-running command output goes to `/tmp`; return only the tail or matching failures.

---

### Task 1: Make 64-bit VLQ encoding memory-safe

**Files:**
- Modify: `crates/codec/src/lib.rs`
- Modify: `crates/codec/src/vlq.rs`
- Modify: `crates/codec/src/encode.rs`
- Modify: `crates/generator/src/lib.rs`

**Interfaces:**
- Preserve `vlq_encode`, `vlq_decode`, `vlq_encode_unsigned`, and `vlq_decode_unsigned` signatures.
- Export `MAX_VLQ_BYTES: usize = 13` from `srcmap_codec::vlq` for every unchecked caller.
- `vlq_encode_unchecked` and `vlq_encode_unsigned_unchecked` require exactly `MAX_VLQ_BYTES` spare bytes.

- [ ] **Step 1: Add extreme-value regression tests**

Extend `crates/codec/src/vlq.rs` tests with signed `i64::MIN` and `i64::MAX` round trips and an unsigned `u64::MAX` round trip:

```rust
#[test]
fn signed_extremes_roundtrip() {
    for value in [i64::MIN, i64::MAX] {
        let mut encoded = Vec::new();
        vlq_encode(&mut encoded, value);
        assert!(encoded.len() <= MAX_VLQ_BYTES);
        assert_eq!(vlq_decode(&encoded, 0).unwrap(), (value, encoded.len()));
    }
}

#[test]
fn unsigned_max_roundtrips() {
    let mut encoded = Vec::new();
    vlq_encode_unsigned(&mut encoded, u64::MAX);
    assert_eq!(encoded.len(), MAX_VLQ_BYTES);
    assert_eq!(vlq_decode_unsigned(&encoded, 0).unwrap(), (u64::MAX, encoded.len()));
}
```

- [ ] **Step 2: Prove the current invariant fails**

Run:

```bash
cargo test -p srcmap-codec signed_extremes_roundtrip
cargo test -p srcmap-codec unsigned_max_roundtrips
```

Expected: decode overflow or incorrect `i64::MIN` output. If Miri is installed, also run `cargo +nightly miri test -p srcmap-codec signed_extremes_roundtrip` and expect an out-of-bounds raw-pointer write.

- [ ] **Step 3: Implement the 13-byte invariant and full-range decoding**

Use a `u128` signed intermediate so `i64::MIN` has a distinct mathematical representation:

```rust
pub const MAX_VLQ_BYTES: usize = 13;

let magnitude = value.unsigned_abs() as u128;
let mut vlq = (magnitude << 1) | u128::from(value.is_negative());
```

Accumulate signed decode digits into `u128`, reject more than 13 digits, and convert the magnitude explicitly:

```rust
let magnitude = result >> 1;
let value = if result & 1 == 0 {
    i64::try_from(magnitude).map_err(|_| DecodeError::VlqOverflow { offset: pos })?
} else if magnitude == 1_u128 << 63 {
    i64::MIN
} else {
    -i64::try_from(magnitude).map_err(|_| DecodeError::VlqOverflow { offset: pos })?
};
```

Use `u128` plus `u64::try_from` for unsigned decode. Update every safety comment, reserve call, and unchecked caller in codec and generator to use `MAX_VLQ_BYTES`. Reserve for the exact number of fields immediately before each unsafe block so a capacity-estimate overflow cannot invalidate the safety contract.

- [ ] **Step 4: Verify codec, generator, and Miri**

Run:

```bash
cargo test -p srcmap-codec
cargo test -p srcmap-generator
cargo test -p srcmap-generator --features parallel
cargo +nightly miri test -p srcmap-codec
```

Expected: all available commands pass with extreme values round-tripping and no Miri undefined behavior.

- [ ] **Step 5: Commit**

```bash
git add crates/codec/src/vlq.rs crates/codec/src/encode.rs crates/generator/src/lib.rs
git commit -S -m "fix: make full-range vlq encoding memory-safe"
```

### Task 2: Restore CI and release supply-chain gates

**Files:**
- Modify: `.github/scripts/workflow-policy.test.mjs`
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/release.yml`
- Modify: `Cargo.lock`

**Interfaces:**
- Platform CI executes `cargo test --workspace --lib --bins --tests --examples` and separately compiles all targets.
- Release NAPI commands use the locked workspace `@napi-rs/cli` through pnpm.
- Node 24's bundled npm performs trusted publishing without a global self-update.

- [ ] **Step 1: Add failing workflow policy tests**

Add assertions that the `check` job includes the narrow test command plus `cargo check --workspace --all-targets`, excludes `cargo test --workspace --all-targets`, and that `release.yml` contains no `npm install` command. Require frozen pnpm installs and `corepack pnpm exec napi` in both `build-napi` and `publish-npm`.

```js
assert.match(checkJob, /cargo test --workspace --lib --bins --tests --examples/);
assert.match(checkJob, /cargo check --workspace --all-targets/);
assert.doesNotMatch(checkJob, /cargo test --workspace --all-targets/);
assert.doesNotMatch(releaseWorkflow, /\bnpm install\b/);
```

- [ ] **Step 2: Run the policy test red**

Run `node --test .github/scripts/workflow-policy.test.mjs`.

Expected: failures identify the benchmark-running test command and the three ad hoc npm installs.

- [ ] **Step 3: Fix platform test execution**

Replace the platform test command with:

```yaml
      - name: Run tests
        run: cargo test --workspace --lib --bins --tests --examples

      - name: Compile all targets
        run: cargo check --workspace --all-targets
```

Keep doc tests, Clippy, formatting, and the dedicated benchmark compile job.

- [ ] **Step 4: Use locked release tooling**

In both release jobs, enable Corepack and install the frozen workspace:

```yaml
      - name: Enable Corepack
        run: corepack enable
      - name: Install JS dependencies
        run: corepack pnpm install --ignore-scripts --frozen-lockfile
```

Remove the npm self-update and global NAPI install steps. Use `corepack pnpm exec napi build --release --platform --target ${{ matrix.settings.target }}` in each package build and `corepack pnpm exec napi artifacts -d artifacts` in each artifact move step.

- [ ] **Step 5: Update the vulnerable Rust lock entry**

Establish the advisory failure with `cargo audit` and `cargo deny check`, then run:

```bash
cargo update -p crossbeam-epoch --precise 0.9.20
```

Confirm the parallel feature graph uses the patched version with `cargo tree -p srcmap-codec --features parallel -i crossbeam-epoch@0.9.20`.

- [ ] **Step 6: Verify policy and security tools**

Run:

```bash
node --test .github/scripts/workflow-policy.test.mjs
actionlint .github/workflows/*.yml
uvx zizmor --config .github/zizmor.yml --min-confidence medium --format plain .
cargo audit
cargo deny check
```

Expected: all commands pass and zizmor reports no ad hoc package installation.

- [ ] **Step 7: Commit**

```bash
git add .github/scripts/workflow-policy.test.mjs .github/workflows/ci.yml .github/workflows/release.yml Cargo.lock
git commit -S -m "ci: restore secure bounded validation"
```

### Task 3: Align WASM contracts and validate benchmark inputs

**Files:**
- Modify: `packages/sourcemap-wasm/__tests__/sourcemap-wasm.test.mjs`
- Modify: `packages/sourcemap-wasm/README.md`
- Modify: `benchmarks/workload.test.mjs`
- Modify: `benchmarks/workload.mjs`
- Modify: `docs/superpowers/plans/2026-07-13-fix-audit-findings.md`

**Interfaces:**
- Out-of-range WASM `source()` and `name()` return `undefined`.
- WASM `ignoreList` is a `Uint32Array`.
- `createDeterministicLookups(count, maxLine, maxColumn, seed)` throws `RangeError` for invalid count or bounds.

- [ ] **Step 1: Add WASM characterization coverage**

Add eager and lazy assertions for out-of-range source/name and typed ignore lists. The runtime assertions are expected to pass immediately because the implementation is already correct; their purpose is to pin the generated contract before correcting documentation.

```js
assert.equal(sm.source(999), undefined);
assert.equal(sm.name(999), undefined);
assert.ok(sm.ignoreList instanceof Uint32Array);
```

- [ ] **Step 2: Correct both README API tables**

For `SourceMap` and `LazySourceMap`, change `source()` and `name()` to `string | undefined`, describe out-of-range results as `undefined`, and change `ignoreList` to `Uint32Array`.

- [ ] **Step 3: Add failing benchmark validation tests**

Assert that count rejects `-1`, `1.5`, and `NaN`; each bound rejects `-1`, `0`, `1.5`, and `NaN`; and count `0` returns `[]`. Require these messages:

```text
count must be a non-negative integer
maxLine must be a positive integer
maxColumn must be a positive integer
```

- [ ] **Step 4: Run benchmark tests red**

Run `node --test --test-name-pattern="rejects invalid|accepts zero" benchmarks/workload.test.mjs`.

Expected: `Missing expected exception (RangeError)`.

- [ ] **Step 5: Implement the minimal guards**

```js
if (!Number.isInteger(count) || count < 0) {
  throw new RangeError("count must be a non-negative integer");
}
if (!Number.isInteger(maxLine) || maxLine <= 0) {
  throw new RangeError("maxLine must be a positive integer");
}
if (!Number.isInteger(maxColumn) || maxColumn <= 0) {
  throw new RangeError("maxColumn must be a positive integer");
}
```

- [ ] **Step 6: Record historical plan completion**

Add a `Completion status` section stating that the earlier audit plan landed on `main` on 2026-07-13 and that its unchecked boxes are preserved as a historical planning record.

- [ ] **Step 7: Verify and commit**

Run the sourcemap WASM build/tests and benchmark package tests, then commit:

```bash
git add packages/sourcemap-wasm/__tests__/sourcemap-wasm.test.mjs packages/sourcemap-wasm/README.md benchmarks/workload.test.mjs benchmarks/workload.mjs docs/superpowers/plans/2026-07-13-fix-audit-findings.md
git commit -S -m "fix: close sweep contract gaps"
```

### Task 4: Contain source extraction writes

**Files:**
- Modify: `crates/cli/src/main.rs`
- Modify: `crates/cli/tests/cli.rs`

**Interfaces:**
- Source extraction never overwrites an existing destination.
- Every parent and final path remains below the selected output directory.
- Absolute, drive-prefixed, UNC, and backslash paths are skipped consistently.

- [ ] **Step 1: Add focused exploit repros**

Create temporary source maps in integration tests that attempt to overwrite a sentinel file, write through a parent symlink, write through a final symlink, and use `C:/outside.ts`, `C:\\outside.ts`, `\\\\server\\share\\outside.ts`, and `..\\outside.ts`. Assert the command leaves every sentinel and outside target unchanged and reports the malicious source as skipped.

- [ ] **Step 2: Run the extraction tests red**

Run `cargo test -p srcmap-cli --test cli sources_extract_security`.

Expected: existing files are overwritten, a symlink target changes, or platform-specific absolute inputs are extracted rather than skipped.

- [ ] **Step 3: Restrict source paths to relative normal components**

Update `sanitize_source_path` to reject backslashes, prefixes, roots, and drive-like first components. Continue folding safe forward-slash `.` and `..` components so the existing webpack fixture behavior remains compatible. Assert the returned path is relative.

```rust
if source.contains('\\') {
    return Err(CliError::path_traversal("source name contains a backslash"));
}

match component {
    Component::Prefix(_) | Component::RootDir => {
        return Err(CliError::path_traversal(format!(
            "source name is absolute: {source}"
        )));
    }
    Component::ParentDir => {
        components.pop();
    }
    Component::CurDir => {}
    Component::Normal(value) => {
        if components.is_empty() && value.to_string_lossy().ends_with(':') {
            return Err(CliError::path_traversal(format!(
                "source name contains a drive prefix: {source}"
            )));
        }
        components.push(value.to_os_string());
    }
}
```

- [ ] **Step 4: Prepare parents without following symlinks**

Add a helper with this contract:

```rust
fn prepare_extraction_path(output_dir: &Path, relative: &Path) -> Result<PathBuf, CliError>
```

Canonicalize the output root, walk parent components one at a time, reject symlinks, create missing directories individually, and recheck canonical containment. Open the final file with `OpenOptions::new().write(true).create_new(true)` so existing files and final-component symlinks cannot be followed or replaced. Treat containment failures as skipped sources, not partial writes.

- [ ] **Step 5: Verify real extraction behavior**

Run focused tests, the full CLI integration suite, and extract `crates/cli/tests/fixtures/webpack.js.map` into a fresh temporary directory. Confirm all expected source files appear and no file outside that directory changes.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/main.rs crates/cli/tests/cli.rs
git commit -S -m "fix: contain extracted source writes"
```

### Task 5: Bound and validate remote fetches

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/sourcemap/Cargo.toml`
- Modify: `crates/sourcemap/src/utils.rs`
- Modify: `crates/cli/Cargo.toml`
- Modify: `crates/cli/src/main.rs`
- Modify: `crates/cli/tests/cli.rs`

**Interfaces:**
- `resolve_source_map_url(base, reference)` uses `Url::join` and preserves its `Option<String>` signature.
- Remote calls have a named global timeout and a bounded redirect count.
- Redirects and document-derived source maps stay same-origin unless `--allow-cross-origin` is explicitly supplied.
- Saved filenames are derived from one parsed URL path segment and cannot be `.` or `..`.

- [ ] **Step 1: Add RFC URL regression tests**

Extend `crates/sourcemap/src/utils.rs` tests for root-relative, scheme-relative, query-bearing, fragment-only, dot-segment, and encoded references. Use exact expected absolute URLs.

- [ ] **Step 2: Add local-server security repros**

In CLI integration tests, use `TcpListener::bind("127.0.0.1:0")` helpers to serve a bundle that redirects or declares an absolute `sourceMappingURL` to a second listener. Without the opt-in, assert the second listener receives no request. With `--allow-cross-origin`, assert a normal second-listener source map is fetched. Add a slow-response test that exceeds the configured test timeout through an injectable or test-shortened constant.

- [ ] **Step 3: Run URL and fetch tests red**

Run:

```bash
cargo test -p srcmap-sourcemap resolve_url
cargo test -p srcmap-cli --test cli fetch_security
```

Expected: RFC-relative cases resolve incorrectly, cross-origin follow-up requests occur, and the slow server does not time out deterministically.

- [ ] **Step 4: Add and use `url = "2"`**

Add the dependency at workspace level and to the sourcemap and CLI crates. Replace string URL joining with `Url::parse` and `Url::join`. Compare `(scheme, host_str, port_or_known_default)` for origin equality.

```rust
fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}
```

- [ ] **Step 5: Configure the HTTP client**

Create one ureq agent with named constants for timeout and redirect count. Disable automatic redirects, follow redirects manually up to the bound, resolve each `Location` with `Url::join`, and apply the same-origin policy before the next request. Return the final URL with the body so source-map references resolve against the actual bundle location.

```rust
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FETCH_REDIRECTS: usize = 10;

fn http_get(
    agent: &ureq::Agent,
    initial_url: &Url,
    allow_cross_origin: bool,
) -> Result<(Url, String), CliError>
```

- [ ] **Step 6: Validate filenames and expose the opt-in**

Derive filenames from `Url::path_segments().next_back()`, fall back to `bundle.js` for an empty segment, and reject `.`, `..`, slash, and backslash content. Add `--allow-cross-origin` to the `Fetch` command and its generated schema description.

- [ ] **Step 7: Verify real-world fetches and commit**

Run all focused tests, the full sourcemap and CLI suites, then fetch `https://unpkg.com/terser@5.31.0/dist/bundle.min.js`, one cross-origin local fixture using the explicit opt-in, and one same-origin local development fixture. Commit:

```bash
git add Cargo.toml Cargo.lock crates/sourcemap/Cargo.toml crates/sourcemap/src/utils.rs crates/cli/Cargo.toml crates/cli/src/main.rs crates/cli/tests/cli.rs
git commit -S -m "fix: constrain derived source map fetches"
```

### Task 6: Complete audit, cleanup, review, and landing

**Files:**
- Remove ignored local files under `.superpowers/sdd/` after confirming `git ls-files .superpowers/sdd` is empty.
- Modify only files required by concrete failures found during this task.

**Interfaces:**
- No open Rust, JavaScript, workflow, secret, or unsafe-code finding remains without an explicit disposition.

- [ ] **Step 1: Run the remaining automated audits**

Run `pnpm audit --json`, Cargo Audit, Cargo Deny, zizmor, actionlint, a tracked-secret pattern scan, and a review of every unsafe call site's capacity or UTF-8 invariant. The approved pnpm audit baseline has no advisories.

- [ ] **Step 2: Run complete local verification**

Redirect verbose output to `/tmp` and require success from:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo check --workspace --all-targets
cargo test --workspace --lib --bins --tests --examples
cargo test --workspace --doc
pnpm run fmt:js:check
pnpm run lint:js
pnpm run test:js
pnpm run typos
```

Also prove the platform test command finishes without any `Running benches/` entry.

- [ ] **Step 3: Perform task and whole-branch reviews**

Review every task against this plan, fix all Critical and Important findings, then run a final whole-branch review from merge-base `26d02cb70ad702ff4851ee4d63f835c7f1a30779`.

- [ ] **Step 4: Clean ignored session artifacts**

Confirm `.superpowers/sdd` contains no tracked file, remove the ignored task briefs, reports, review packages, and progress ledger, then confirm `git status --ignored --short .superpowers/sdd` is empty.

- [ ] **Step 5: Land and verify main**

Fast-forward local `main` to the reviewed branch, rerun the proportional post-merge gates, push `main`, and monitor the exact pushed commit until every relevant GitHub Actions job completes. Remove the feature branch after remote verification.

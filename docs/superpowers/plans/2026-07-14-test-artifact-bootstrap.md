# Test Artifact Bootstrap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add one root command that safely builds every generated NAPI and WASM artifact required by JavaScript tests in a clean checkout or worktree.

**Architecture:** Define the build contract in root package scripts, use `--no-js` for NAPI builds to preserve tracked loaders, and reuse the command in the JavaScript runtime CI job. Lock the contract with the existing workflow policy test and document it in the contributor quick start.

**Tech Stack:** pnpm 10, Node.js test runner, NAPI-RS CLI, wasm-pack, GitHub Actions.

## Global Constraints

- Add no dependencies.
- Preserve tracked NAPI JavaScript loaders.
- Do not make `pnpm test` rebuild artifacts automatically.
- Keep the existing focused package build commands available.
- Use signed conventional commits without attribution.

---

### Task 1: Define the bootstrap contract test-first

**Files:**
- Modify: `.github/scripts/workflow-policy.test.mjs`
- Modify: `package.json`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: root `package.json` scripts and the `js-runtime` workflow job.
- Produces: `build:test-artifacts`, `build:test-artifacts:napi`, and `build:test-artifacts:wasm` package scripts used by contributors and CI.

- [ ] **Step 1: Write the failing policy test**

Add `PACKAGE_JSON_URL` and extend `Generated artifact policy` with a test that checks the exact root scripts and CI step:

```js
const PACKAGE_JSON_URL = new URL("package.json", ROOT_URL);

it("uses one safe bootstrap command for JavaScript test artifacts", async () => {
  const packageJson = JSON.parse(await readFile(PACKAGE_JSON_URL, "utf8"));
  assert.equal(
    packageJson.scripts["build:test-artifacts"],
    "pnpm run build:test-artifacts:napi && pnpm run build:test-artifacts:wasm",
  );
  assert.equal(
    packageJson.scripts["build:test-artifacts:napi"],
    "pnpm --filter @srcmap/codec exec napi build --release --platform --no-js --dts ../../target/napi-codec.d.ts && pnpm --filter @srcmap/sourcemap exec napi build --release --platform --no-js --dts ../../target/napi-sourcemap.d.ts",
  );
  assert.equal(
    packageJson.scripts["build:test-artifacts:wasm"],
    "pnpm --filter @srcmap/sourcemap-wasm build:all && pnpm --filter @srcmap/generator-wasm build:all && pnpm --filter @srcmap/remapping-wasm build:all && pnpm --filter @srcmap/symbolicate-wasm build:all",
  );

  const workflow = await readFile(CI_WORKFLOW_URL, "utf8");
  const job = workflowJob(workflow, "js-runtime");
  assert.match(
    job,
    /      - name: Build JavaScript test artifacts\n        run: corepack pnpm run build:test-artifacts/,
  );
  assert.doesNotMatch(job, /corepack pnpm --filter @srcmap\/.+ build/);
});
```

- [ ] **Step 2: Run the policy test and verify red**

Run:

```bash
node --test .github/scripts/workflow-policy.test.mjs
```

Expected: FAIL because `build:test-artifacts` is not defined.

- [ ] **Step 3: Add the root package scripts**

Add these scripts before `check:napi-declarations` in `package.json`:

```json
"build:test-artifacts": "pnpm run build:test-artifacts:napi && pnpm run build:test-artifacts:wasm",
"build:test-artifacts:napi": "pnpm --filter @srcmap/codec exec napi build --release --platform --no-js --dts ../../target/napi-codec.d.ts && pnpm --filter @srcmap/sourcemap exec napi build --release --platform --no-js --dts ../../target/napi-sourcemap.d.ts",
"build:test-artifacts:wasm": "pnpm --filter @srcmap/sourcemap-wasm build:all && pnpm --filter @srcmap/generator-wasm build:all && pnpm --filter @srcmap/remapping-wasm build:all && pnpm --filter @srcmap/symbolicate-wasm build:all"
```

- [ ] **Step 4: Make CI use the root command**

Replace the `Build N-API packages` and `Build WASM packages` steps in the `js-runtime` job with:

```yaml
      - name: Build JavaScript test artifacts
        run: corepack pnpm run build:test-artifacts
      - name: Check N-API declarations
        run: node .github/scripts/check-napi-declarations.mjs
```

- [ ] **Step 5: Run the policy test and verify green**

Run:

```bash
node --test .github/scripts/workflow-policy.test.mjs
```

Expected: PASS.

### Task 2: Document and prove clean-worktree setup

**Files:**
- Modify: `CONTRIBUTING.md:15-38,77-96`

**Interfaces:**
- Consumes: `pnpm build:test-artifacts` from Task 1.
- Produces: one contributor-facing setup command and explicit JavaScript test prerequisite.

- [ ] **Step 1: Replace duplicated quick-start build commands**

Keep the Rust workspace build, then replace the package-specific NAPI and WASM command block with:

```bash
# Build all generated NAPI and WASM artifacts used by the JavaScript tests
corepack pnpm run build:test-artifacts
```

- [ ] **Step 2: Update the JavaScript testing note**

Use:

```bash
corepack pnpm run test:js       # JS/WASM tests (run build:test-artifacts first)
```

- [ ] **Step 3: Install clean-worktree dependencies**

Run:

```bash
corepack pnpm install --ignore-scripts --frozen-lockfile
```

Expected: the lockfile is unchanged and no generated binding artifacts exist before bootstrap.

- [ ] **Step 4: Execute the bootstrap command**

Run with bounded output:

```bash
corepack pnpm run build:test-artifacts > /tmp/srcmap-test-artifacts-build.log 2>&1
```

Expected: the command exits successfully, both platform NAPI binaries exist, and every listed WASM package has Node and browser outputs.

- [ ] **Step 5: Verify tracked loader hygiene**

Run:

```bash
git status --short
git diff -- packages/codec/index.js packages/sourcemap/index.js
```

Expected: only the intended policy, package, workflow, and contributor documentation files are modified; tracked loader files are unchanged.

- [ ] **Step 6: Run full verification**

Run with bounded logs:

```bash
pnpm check > /tmp/srcmap-test-artifacts-check.log 2>&1
pnpm test > /tmp/srcmap-test-artifacts-test.log 2>&1
git diff --check
```

Expected: all commands exit successfully.

- [ ] **Step 7: Review and commit**

Review `git status --short`, the complete diff, and staged scope. Commit with:

```bash
git commit -S -m "build: add test artifact bootstrap"
```

- [ ] **Step 8: Publish and verify**

Push `codex/test-artifact-bootstrap`, create a ready pull request targeting `main`, require all checks, squash merge with a conventional subject, and verify CI, coverage, benchmarks, and Release Drafter on the exact merge commit.

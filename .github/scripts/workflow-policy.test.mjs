import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { access, readdir, readFile } from "node:fs/promises";
import { describe, it } from "node:test";

const ROOT_URL = new URL("../../", import.meta.url);
const WORKFLOWS_URL = new URL("workflows/", new URL("../", import.meta.url));
const CI_WORKFLOW_URL = new URL("ci.yml", WORKFLOWS_URL);
const COVERAGE_WORKFLOW_URL = new URL("coverage.yml", WORKFLOWS_URL);
const RELEASE_WORKFLOW_URL = new URL("release.yml", WORKFLOWS_URL);
const FALLOW_CONFIG_URL = new URL(".fallowrc.json", ROOT_URL);

const trackedPackageLocks = async () => {
  const tracked = execFileSync("git", ["ls-files", "--", ":(glob)**/package-lock.json"], {
    cwd: ROOT_URL,
    encoding: "utf8",
  })
    .trim()
    .split("\n")
    .filter(Boolean);

  const existing = await Promise.all(
    tracked.map(async (path) => {
      try {
        await access(new URL(path, ROOT_URL));
        return path;
      } catch {
        return null;
      }
    }),
  );

  return existing.filter((path) => path !== null);
};

const workflowFiles = async () => {
  const entries = await readdir(WORKFLOWS_URL, { withFileTypes: true });
  return entries.filter((entry) => entry.isFile() && /\.ya?ml$/.test(entry.name));
};

const workflowJob = (workflow, jobName) => {
  const marker = `  ${jobName}:\n`;
  const start = workflow.indexOf(marker);
  assert.notEqual(start, -1, `missing ${jobName} job`);

  const bodyStart = start + marker.length;
  const remaining = workflow.slice(bodyStart);
  const nextJob = remaining.search(/^  [\w-]+:/m);
  return nextJob === -1 ? remaining : remaining.slice(0, nextJob);
};

describe("JavaScript dependency policy", () => {
  it("keeps pnpm-lock.yaml as the only tracked JavaScript lockfile", async () => {
    assert.deepEqual(await trackedPackageLocks(), []);
  });

  it("uses frozen pnpm installs in every workflow", async () => {
    for (const entry of await workflowFiles()) {
      const workflow = await readFile(new URL(entry.name, WORKFLOWS_URL), "utf8");
      assert.doesNotMatch(workflow, /--no-frozen-lockfile/, entry.name);

      const installs = workflow.split("\n").filter((line) => /\bpnpm install\b/.test(line));

      for (const install of installs) {
        assert.match(install, /--ignore-scripts\b/, `${entry.name}: ${install.trim()}`);
        assert.match(install, /--frozen-lockfile\b/, `${entry.name}: ${install.trim()}`);
      }
    }
  });
});

describe("Generated artifact policy", () => {
  it("ignores generated wasm-pack binaries during artifactless analysis", async () => {
    const config = JSON.parse(await readFile(FALLOW_CONFIG_URL, "utf8"));

    assert.ok(config.ignoreUnresolvedImports.includes("**/srcmap_*_wasm_bg.wasm"));
  });
});

describe("Rust feature coverage", () => {
  it("keeps platform tests bounded and compiles every target", async () => {
    const workflow = await readFile(CI_WORKFLOW_URL, "utf8");
    const job = workflowJob(workflow, "check");

    assert.match(job, /cargo test --workspace --lib --bins --tests --examples/);
    assert.match(job, /cargo check --workspace --all-targets/);
    assert.doesNotMatch(job, /cargo test --workspace --all-targets/);
  });

  it("tests both parallel encoders in Linux CI", async () => {
    const workflow = await readFile(CI_WORKFLOW_URL, "utf8");
    const job = workflowJob(workflow, "parallel-features");

    assert.match(job, /^    runs-on: ubuntu-latest$/m);
    assert.match(job, /^        run: cargo test -p srcmap-codec --features parallel$/m);
    assert.match(job, /^        run: cargo test -p srcmap-generator --features parallel$/m);
  });
});

describe("Release supply-chain policy", () => {
  it("uses frozen workspace tooling for NAPI build and publish jobs", async () => {
    const workflow = await readFile(RELEASE_WORKFLOW_URL, "utf8");
    const install = "corepack pnpm install --ignore-scripts --frozen-lockfile";
    const buildJob = workflowJob(workflow, "build-napi");
    const publishJob = workflowJob(workflow, "publish-npm");

    assert.doesNotMatch(workflow, /\bnpm install\b/);
    assert.ok(buildJob.includes(install), "build-napi must install the frozen workspace");
    assert.ok(publishJob.includes(install), "publish-npm must install the frozen workspace");
    assert.match(buildJob, /corepack pnpm exec napi build --release --platform --target/);
    assert.match(publishJob, /corepack pnpm exec napi artifacts -d artifacts/);
  });
});

describe("NAPI declaration coverage", () => {
  it("checks generated declarations after the NAPI build and before JavaScript tests", async () => {
    const workflow = await readFile(CI_WORKFLOW_URL, "utf8");
    const job = workflowJob(workflow, "js-runtime");
    const build = "corepack pnpm --filter @srcmap/sourcemap build";
    const declarationStep =
      "      - name: Check N-API declarations\n        run: node .github/scripts/check-napi-declarations.mjs";
    const check = "node .github/scripts/check-napi-declarations.mjs";
    const test = "corepack pnpm run test:js";

    assert.ok(job.includes(declarationStep), "missing explicit NAPI declaration check step");
    assert.ok(job.indexOf(build) < job.indexOf(check), "NAPI declarations must build before check");
    assert.ok(job.indexOf(check) < job.indexOf(test), "NAPI declarations must check before tests");
  });
});

describe("WASM package coverage", () => {
  it("builds symbolicate WASM targets before JavaScript tests", async () => {
    const workflow = await readFile(CI_WORKFLOW_URL, "utf8");
    const job = workflowJob(workflow, "js-runtime");
    const build = "corepack pnpm --filter @srcmap/symbolicate-wasm build:all";
    const test = "corepack pnpm run test:js";

    assert.ok(job.includes(build), "missing symbolicate WASM build");
    assert.ok(
      job.indexOf(build) < job.indexOf(test),
      "symbolicate WASM must build before JS tests",
    );
  });

  it("builds symbolicate WASM targets before JavaScript coverage", async () => {
    const workflow = await readFile(COVERAGE_WORKFLOW_URL, "utf8");
    const job = workflowJob(workflow, "coverage");
    const build = "corepack pnpm --filter @srcmap/symbolicate-wasm build:all";
    const coverage = "corepack pnpm run coverage:js";

    assert.ok(job.includes(build), "missing symbolicate WASM coverage build");
    assert.ok(
      job.indexOf(build) < job.indexOf(coverage),
      "symbolicate WASM must build before JS coverage",
    );
  });
});

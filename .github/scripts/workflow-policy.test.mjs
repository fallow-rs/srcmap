import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { access, readdir, readFile } from "node:fs/promises";
import { describe, it } from "node:test";

const ROOT_URL = new URL("../../", import.meta.url);
const WORKFLOWS_URL = new URL("workflows/", new URL("../", import.meta.url));
const CI_WORKFLOW_URL = new URL("ci.yml", WORKFLOWS_URL);

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

describe("Rust feature coverage", () => {
  it("tests both parallel encoders in Linux CI", async () => {
    const workflow = await readFile(CI_WORKFLOW_URL, "utf8");
    const job = workflowJob(workflow, "parallel-features");

    assert.match(job, /^    runs-on: ubuntu-latest$/m);
    assert.match(job, /^        run: cargo test -p srcmap-codec --features parallel$/m);
    assert.match(job, /^        run: cargo test -p srcmap-generator --features parallel$/m);
  });
});

describe("WASM package coverage", () => {
  it("builds symbolicate WASM targets before JavaScript tests", async () => {
    const workflow = await readFile(CI_WORKFLOW_URL, "utf8");
    const job = workflowJob(workflow, "js-runtime");
    const build = "corepack pnpm --filter @srcmap/symbolicate-wasm build:all";
    const test = "corepack pnpm run test:js";

    assert.ok(job.includes(build), "missing symbolicate WASM build");
    assert.ok(job.indexOf(build) < job.indexOf(test), "symbolicate WASM must build before JS tests");
  });
});

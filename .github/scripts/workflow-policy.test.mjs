import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { access, readdir, readFile } from "node:fs/promises";
import { describe, it } from "node:test";

const ROOT_URL = new URL("../../", import.meta.url);
const WORKFLOWS_URL = new URL("workflows/", new URL("../", import.meta.url));

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

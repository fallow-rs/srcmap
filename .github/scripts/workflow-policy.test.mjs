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
const WASM_PACK_INSTALL_ACTION = "taiki-e/install-action@43aecc8d72668fbcfe75c31400bc4f890f1c5853";
const WASM_PACK_WORKFLOWS = new Map([
  ["bench.yml", "        if: matrix.kind == 'node'\n"],
  ["ci.yml", ""],
  ["coverage.yml", ""],
  ["release.yml", ""],
]);

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

const workflowSteps = (workflow, action) => {
  const matches = [...workflow.matchAll(new RegExp(`^(\\s*)- uses: ${action}[^\\n]*$`, "gm"))];

  return matches.map((match) => {
    const start = match.index;
    const remaining = workflow.slice(start + match[0].length + 1);
    const nextStep = remaining.search(new RegExp(`^${match[1]}- `, "m"));
    return nextStep === -1
      ? workflow.slice(start)
      : workflow.slice(start, start + match[0].length + 1 + nextStep);
  });
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

describe("Checkout credential policy", () => {
  it("does not persist GitHub credentials in workflow worktrees", async () => {
    for (const entry of await workflowFiles()) {
      const workflow = await readFile(new URL(entry.name, WORKFLOWS_URL), "utf8");

      for (const step of workflowSteps(workflow, "actions/checkout@")) {
        assert.match(
          step,
          /^\s+persist-credentials: false$/m,
          `${entry.name}: checkout must disable persisted credentials`,
        );
      }
    }
  });

  it("uses a step-scoped token and explicit authenticated remote for badge pushes", async () => {
    const workflow = await readFile(COVERAGE_WORKFLOW_URL, "utf8");
    const job = workflowJob(workflow, "coverage");
    const badgeStep = job.slice(job.indexOf("      - name: Update coverage badges"));

    assert.match(badgeStep, /^        env:\n          GH_TOKEN: \$\{\{ github\.token \}\}$/m);
    assert.doesNotMatch(job, /^    env:\n(?:      .*\n)*      GH_TOKEN:/m);
    assert.doesNotMatch(badgeStep, /git push origin badges/);
    const authenticatedPushes = badgeStep.match(
      /git push "https:\/\/x-access-token:\$\{GH_TOKEN\}@github\.com\/\$\{GITHUB_REPOSITORY\}\.git" badges/g,
    );
    assert.equal(authenticatedPushes?.length, 2);
  });
});

describe("Pinned wasm-pack installation policy", () => {
  it("uses only the pinned install action for wasm-pack in every workflow", async () => {
    for (const entry of await workflowFiles()) {
      const workflow = await readFile(new URL(entry.name, WORKFLOWS_URL), "utf8");

      assert.doesNotMatch(workflow, /rustwasm\.github\.io\/wasm-pack/, entry.name);
      assert.doesNotMatch(workflow, /curl[^\n|]*\|\s*sh/, entry.name);

      const condition = WASM_PACK_WORKFLOWS.get(entry.name);
      if (condition !== undefined) {
        const install = [
          "      - name: Install wasm-pack",
          condition.trimEnd(),
          `        uses: ${WASM_PACK_INSTALL_ACTION} # v2.83.2`,
          "        with:",
          "          tool: wasm-pack@0.13.1",
        ]
          .filter(Boolean)
          .join("\n");
        assert.ok(workflow.includes(install), `${entry.name}: missing pinned wasm-pack installer`);
      }

      if (/\bwasm-pack (?:build|test)\b/.test(workflow)) {
        assert.ok(
          WASM_PACK_WORKFLOWS.has(entry.name),
          `${entry.name}: wasm-pack use is missing from the pinned installer policy`,
        );
      }
    }
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

  it("checks Cargo advisories across all features", async () => {
    const workflow = await readFile(CI_WORKFLOW_URL, "utf8");
    const job = workflowJob(workflow, "deny");
    const action = [
      "      - uses: EmbarkStudios/cargo-deny-action@bb137d7af7e4fb67e5f82a49c4fce4fad40782fe # v2",
      "        with:",
      "          arguments: --all-features",
    ].join("\n");

    assert.ok(job.includes(action), "Cargo Deny must check the all-features graph");
  });
});

describe("Release supply-chain policy", () => {
  it("contains no ad hoc npm installs", async () => {
    const workflow = await readFile(RELEASE_WORKFLOW_URL, "utf8");

    assert.doesNotMatch(workflow, /\bnpm install\b/);
  });

  it("requires provenance for every npm publish attempt", async () => {
    const workflow = await readFile(RELEASE_WORKFLOW_URL, "utf8");
    const publishes = workflow.split("\n").filter((line) => /\bnpm publish\b/.test(line));

    assert.ok(publishes.length > 0, "release workflow must publish npm packages");
    for (const publish of publishes) {
      assert.match(publish, /--provenance\b/, publish.trim());
    }
    assert.doesNotMatch(workflow, /retrying without provenance/i);
  });

  it("uses ordered frozen tooling for both NAPI package builds", async () => {
    const workflow = await readFile(RELEASE_WORKFLOW_URL, "utf8");
    const buildJob = workflowJob(workflow, "build-napi");
    const enable = "      - name: Enable Corepack\n        run: corepack enable";
    const install = [
      "      - name: Install JS dependencies",
      "        run: corepack pnpm install --ignore-scripts --frozen-lockfile",
    ].join("\n");
    const codecBuild = [
      "      - name: Build codec NAPI binary",
      "        run: |",
      "          cd packages/codec",
      "          corepack pnpm exec napi build --release --platform --target ${{ matrix.settings.target }}",
    ].join("\n");
    const sourcemapBuild = [
      "      - name: Build sourcemap NAPI binary",
      "        run: |",
      "          cd packages/sourcemap",
      "          corepack pnpm exec napi build --release --platform --target ${{ matrix.settings.target }}",
    ].join("\n");

    assert.ok(buildJob.includes(enable), "build-napi must enable Corepack");
    assert.ok(buildJob.includes(install), "build-napi must install the frozen workspace");
    assert.ok(buildJob.includes(codecBuild), "missing codec NAPI build step");
    assert.ok(buildJob.includes(sourcemapBuild), "missing sourcemap NAPI build step");
    assert.ok(buildJob.indexOf(enable) < buildJob.indexOf(install));
    assert.ok(buildJob.indexOf(install) < buildJob.indexOf(codecBuild));
    assert.ok(buildJob.indexOf(install) < buildJob.indexOf(sourcemapBuild));
  });

  it("uses ordered frozen tooling for both NAPI artifact moves", async () => {
    const workflow = await readFile(RELEASE_WORKFLOW_URL, "utf8");
    const publishJob = workflowJob(workflow, "publish-npm");
    const enable = "      - name: Enable Corepack\n        run: corepack enable";
    const install = [
      "      - name: Install JS dependencies",
      "        run: corepack pnpm install --ignore-scripts --frozen-lockfile",
    ].join("\n");
    const codecArtifacts = [
      "      - name: Move codec artifacts",
      "        run: corepack pnpm exec napi artifacts -d artifacts",
      "        working-directory: packages/codec",
    ].join("\n");
    const sourcemapArtifacts = [
      "      - name: Move sourcemap artifacts",
      "        run: corepack pnpm exec napi artifacts -d artifacts",
      "        working-directory: packages/sourcemap",
    ].join("\n");

    assert.ok(publishJob.includes(enable), "publish-npm must enable Corepack");
    assert.ok(publishJob.includes(install), "publish-npm must install the frozen workspace");
    assert.ok(publishJob.includes(codecArtifacts), "missing codec artifact move step");
    assert.ok(publishJob.includes(sourcemapArtifacts), "missing sourcemap artifact move step");
    assert.ok(publishJob.indexOf(enable) < publishJob.indexOf(install));
    assert.ok(publishJob.indexOf(install) < publishJob.indexOf(codecArtifacts));
    assert.ok(publishJob.indexOf(install) < publishJob.indexOf(sourcemapArtifacts));
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

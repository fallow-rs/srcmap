import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, it } from "node:test";

const ROOT_URL = new URL("../../", import.meta.url);
const SCRIPT_URL = new URL(".github/scripts/publish-crate-if-needed.sh", ROOT_URL);
const WORKFLOW_URL = new URL(".github/workflows/release.yml", ROOT_URL);

const writeExecutable = async (path, contents) => {
  await writeFile(path, contents);
  await chmod(path, 0o755);
};

const runPublishHelper = async ({ lookup = "existing", publishExit = 0 } = {}) => {
  const directory = await mkdtemp(join(tmpdir(), "srcmap-publish-test-"));
  const logPath = join(directory, "commands.log");

  await writeFile(logPath, "");

  await Promise.all([
    writeExecutable(
      join(directory, "cargo"),
      `#!/usr/bin/env bash
set -eu
printf 'cargo %s\\n' "$*" >> "$COMMAND_LOG"
if [[ "$1" == "metadata" ]]; then
  printf '{"packages":[{"name":"srcmap-codec","version":"1.2.3"}]}'
  exit 0
fi
exit "${publishExit}"
`,
    ),
    writeExecutable(
      join(directory, "curl"),
      `#!/usr/bin/env bash
set -eu
printf 'curl %s\\n' "$*" >> "$COMMAND_LOG"
if [[ "${lookup}" == "failure" ]]; then
  exit 7
fi
output=''
while [[ $# -gt 0 ]]; do
  if [[ "$1" == "--output" || "$1" == "-o" ]]; then
    output="$2"
    shift 2
    continue
  fi
  shift
done
if [[ "${lookup}" == "existing" ]]; then
  printf '{"version":{"num":"1.2.3"}}' > "$output"
  printf '200'
else
  printf '{"errors":[{"detail":"Not Found"}]}' > "$output"
  printf '404'
fi
`,
    ),
    writeExecutable(
      join(directory, "jq"),
      `#!/usr/bin/env bash
set -eu
printf 'jq %s\\n' "$*" >> "$COMMAND_LOG"
if [[ "$*" == *'.packages[]'* ]]; then
  printf '1.2.3\\n'
  exit 0
fi
if [[ "$*" == *'.version.num'* ]]; then
  printf '1.2.3\\n'
  exit 0
fi
exit 1
`,
    ),
  ]);

  const result = await new Promise((resolve, reject) => {
    const child = spawn("bash", [SCRIPT_URL.pathname, "srcmap-codec"], {
      cwd: new URL("../..", import.meta.url).pathname,
      env: {
        ...process.env,
        COMMAND_LOG: logPath,
        PATH: `${directory}:${process.env.PATH}`,
      },
    });
    let stderr = "";
    let stdout = "";

    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.on("error", reject);
    child.on("close", (exitCode) => resolve({ exitCode, stderr, stdout }));
  });

  const commands = await readFile(logPath, "utf8");
  await rm(directory, { force: true, recursive: true });
  return { ...result, commands };
};

describe("publish-crate-if-needed", () => {
  it("skips publishing when crates.io confirms the workspace version exists", async () => {
    const result = await runPublishHelper();

    assert.equal(result.exitCode, 0);
    assert.match(result.stdout, /already published/i);
    assert.doesNotMatch(result.commands, /cargo publish/);
  });

  it("publishes when crates.io reports that the workspace version is missing", async () => {
    const result = await runPublishHelper({ lookup: "missing" });

    assert.equal(result.exitCode, 0);
    assert.match(result.commands, /cargo publish -p srcmap-codec/);
  });

  it("stops when the crates.io lookup fails", async () => {
    const result = await runPublishHelper({ lookup: "failure" });

    assert.notEqual(result.exitCode, 0);
    assert.doesNotMatch(result.commands, /cargo publish/);
  });

  it("propagates cargo publish failures", async () => {
    const result = await runPublishHelper({ lookup: "missing", publishExit: 42 });

    assert.equal(result.exitCode, 42);
  });

  it("does not mask cargo publish failures in the release workflow", async () => {
    const workflow = await readFile(WORKFLOW_URL, "utf8");

    assert.doesNotMatch(workflow, /cargo publish[^\n]*\|\|/);
  });
});

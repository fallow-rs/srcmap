import { appendFileSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";

const rustShard = ({
  label,
  cacheKey,
  packageName,
  bench,
  features = "codspeed",
  fixtures = false,
}) => ({
  kind: "rust",
  mode: "simulation",
  label,
  cache_key: cacheKey,
  package: packageName,
  bench,
  features,
  fixtures,
});

const jsShard = ({ label, cacheKey, command, fixtures = true }) => ({
  kind: "node",
  mode: "simulation",
  label,
  cache_key: cacheKey,
  command,
  fixtures,
});

// The rayon benchmarks (`vlq_parallel`, `generate_parallel`) are not here.
// The CPU simulation counts the wait for the rayon workers as system call
// time and cannot show a parallel speedup, so bench-walltime.yml measures them
// on a CodSpeed macro runner.
const SHARDS = {
  codec: rustShard({
    label: "codec vlq",
    cacheKey: "codec-vlq",
    packageName: "srcmap-codec",
    bench: "vlq",
  }),
  sourcemap: rustShard({
    label: "sourcemap parse",
    cacheKey: "sourcemap-parse",
    packageName: "srcmap-sourcemap",
    bench: "parse",
    fixtures: true,
  }),
  generator: rustShard({
    label: "generator",
    cacheKey: "generator",
    packageName: "srcmap-generator",
    bench: "generate",
  }),
  remapping: rustShard({
    label: "remapping",
    cacheKey: "remapping",
    packageName: "srcmap-remapping",
    bench: "remap",
    fixtures: true,
  }),
  packages: jsShard({
    label: "package runtime",
    cacheKey: "package-runtime",
    command: "corepack pnpm --dir benchmarks run bench:codspeed:packages",
  }),
};

const ALL_SHARDS = Object.values(SHARDS);

const commandOutput = (command, args) => {
  try {
    return execFileSync(command, args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  } catch {
    return "";
  }
};

const changedFilesForEvent = () => {
  const eventName = process.env.GITHUB_EVENT_NAME ?? "";
  if (eventName === "workflow_dispatch") return null;

  if (eventName === "pull_request") {
    const baseRef = process.env.GITHUB_BASE_REF;
    if (!baseRef) return null;
    commandOutput("git", ["fetch", "--no-tags", "--depth=1", "origin", baseRef]);
    const diff = commandOutput("git", ["diff", "--name-only", `origin/${baseRef}...HEAD`]);
    return diff.trim() ? diff.trim().split("\n") : [];
  }

  if (eventName === "push") {
    const eventPath = process.env.GITHUB_EVENT_PATH;
    if (!eventPath) return null;
    const event = JSON.parse(readFileSync(eventPath, "utf8"));
    const before = event.before;
    const after = event.after ?? process.env.GITHUB_SHA;
    if (!before || /^0+$/.test(before) || !after) return null;
    const diff = commandOutput("git", ["diff", "--name-only", `${before}..${after}`]);
    return diff.trim() ? diff.trim().split("\n") : [];
  }

  return null;
};

const selectShards = (files) => {
  if (files === null) return ALL_SHARDS;

  const selected = new Set();

  for (const file of files) {
    if (
      file === ".github/workflows/bench.yml" ||
      file === ".github/scripts/generate-benchmark-matrix.mjs" ||
      file === "Cargo.toml" ||
      file === "Cargo.lock" ||
      file === "package.json" ||
      file === "pnpm-lock.yaml" ||
      file === "pnpm-workspace.yaml"
    ) {
      return ALL_SHARDS;
    }

    if (file.startsWith("crates/codec/")) selected.add(SHARDS.codec);
    if (file.startsWith("crates/sourcemap/")) selected.add(SHARDS.sourcemap);
    if (file.startsWith("crates/generator/")) selected.add(SHARDS.generator);
    if (file.startsWith("crates/remapping/")) selected.add(SHARDS.remapping);
    if (file === "benchmarks/download-fixtures.mjs") {
      selected.add(SHARDS.sourcemap);
      selected.add(SHARDS.remapping);
    }
    if (file.startsWith("benchmarks/") || file.startsWith("packages/")) {
      selected.add(SHARDS.packages);
    }
  }

  return selected.size === 0 ? ALL_SHARDS : [...selected];
};

const include = selectShards(changedFilesForEvent());
const json = JSON.stringify(include);

if (process.env.GITHUB_OUTPUT) {
  appendFileSync(process.env.GITHUB_OUTPUT, `include=${json}\n`);
} else {
  console.log(json);
}

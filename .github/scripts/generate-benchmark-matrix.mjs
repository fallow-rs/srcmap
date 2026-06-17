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

const SHARDS = {
  codec: rustShard({
    label: "codec vlq",
    cacheKey: "codec-vlq",
    packageName: "srcmap-codec",
    bench: "vlq",
  }),
  codecParallel: rustShard({
    label: "codec vlq parallel",
    cacheKey: "codec-vlq-parallel",
    packageName: "srcmap-codec",
    bench: "vlq",
    features: "codspeed,parallel",
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
  generatorParallel: rustShard({
    label: "generator parallel",
    cacheKey: "generator-parallel",
    packageName: "srcmap-generator",
    bench: "generate",
    features: "codspeed,parallel",
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

const allShards = () => [
  SHARDS.codec,
  SHARDS.codecParallel,
  SHARDS.sourcemap,
  SHARDS.generator,
  SHARDS.generatorParallel,
  SHARDS.remapping,
  SHARDS.packages,
];

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

const includeShard = (selected, shard) => {
  selected.set(shard.label, shard);
};

const selectShards = (files) => {
  if (files === null) return allShards();

  const selected = new Map();

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
      return allShards();
    }

    if (file.startsWith("crates/codec/")) {
      includeShard(selected, SHARDS.codec);
      includeShard(selected, SHARDS.codecParallel);
    }
    if (file.startsWith("crates/sourcemap/")) includeShard(selected, SHARDS.sourcemap);
    if (file.startsWith("crates/generator/")) {
      includeShard(selected, SHARDS.generator);
      includeShard(selected, SHARDS.generatorParallel);
    }
    if (file.startsWith("crates/remapping/")) includeShard(selected, SHARDS.remapping);
    if (file.startsWith("benchmarks/download-fixtures.mjs")) {
      includeShard(selected, SHARDS.sourcemap);
      includeShard(selected, SHARDS.remapping);
      includeShard(selected, SHARDS.packages);
    }
    if (
      file.startsWith("benchmarks/") ||
      file.startsWith("packages/") ||
      file === "benchmarks/package.json"
    ) {
      includeShard(selected, SHARDS.packages);
    }
  }

  return selected.size === 0 ? allShards() : [...selected.values()];
};

const include = selectShards(changedFilesForEvent());
const json = JSON.stringify(include);

if (process.env.GITHUB_OUTPUT) {
  appendFileSync(process.env.GITHUB_OUTPUT, `include=${json}\n`);
} else {
  console.log(json);
}

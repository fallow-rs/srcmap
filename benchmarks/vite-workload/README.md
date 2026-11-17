# Vite workload benchmark

This benchmark answers two questions about source map work in Vite:

1. Which share of a Vite run goes to source map work?
2. How fast is each remapping implementation on the maps that Vite combines?

It records a real Vite run, then replays the recorded maps.

## What the recorder measures

`instrument.mjs` adds a recorder to the built Vite of a project. The recorder wraps these functions:

| Function | What the recorder keeps |
|---|---|
| `combineSourcemaps` | Call count, time, and the input maps |
| `_getCombinedSourcemap` | The number of maps per module at the end of the plugin chain, and the chain |
| `MagicString.generateMap` and `generateDecodedMap` | Call count and time |
| `ssrTransform` | Call count and time, which includes the two rows above |
| `loadAndTransform` | Call count and time for the full transform of one module |

The recorder does nothing until `SRCMAP_WORKLOAD_OUT` is set. It writes one NDJSON file per process when the process exits. The first line is the summary. With `SRCMAP_WORKLOAD_CAPTURE=1`, the lines after it hold the recorded maps.

The recorder covers the Vite code only. It does not see the map work inside plugins or inside the native transform.

## Record a workload

Use a project that you can install and run. Do each workload two times: one time for the timing, and one time for the maps. The capture run is slower, so its timing is not valid.

```sh
node vite-workload/instrument.mjs <project-dir>

# A Vitest run. Remove the Vitest transform cache first, or the run transforms nothing.
cd <project-dir>
rm -rf node_modules/.vitest-cache
SRCMAP_WORKLOAD_OUT=/tmp/out/my-project-vitest-time pnpm vitest run
rm -rf node_modules/.vitest-cache
SRCMAP_WORKLOAD_OUT=/tmp/out/my-project-vitest SRCMAP_WORKLOAD_CAPTURE=1 pnpm vitest run

# An unbundled dev server, without a browser. Use `--env ssr` for the SSR environment.
SRCMAP_WORKLOAD_OUT=/tmp/out/my-project-dev-client-time \
  node vite-workload/crawl.mjs --root <project-dir> --env client --entry src/main.ts
SRCMAP_WORKLOAD_OUT=/tmp/out/my-project-dev-client SRCMAP_WORKLOAD_CAPTURE=1 \
  node vite-workload/crawl.mjs --root <project-dir> --env client --entry src/main.ts

node vite-workload/instrument.mjs <project-dir> --restore
```

`crawl.mjs` transforms one module at a time, so the transform time in its summary has no overlap. A Vitest run transforms modules in parallel, so compare its combine time with the wall time only.

Copy the files into `benchmarks/fixtures/vite-workload/`:

- the capture file as `<name>.ndjson`
- the timing file as `<name>.time.ndjson`

Git ignores that folder. The fixtures hold the source text of the recorded project, so do not publish fixtures of a private project.

## Replay

```sh
pnpm --dir benchmarks run bench:vite-workload
```

For each fixture the benchmark prints the recorded share, the shape of the chains, and the time per implementation:

- `@jridgewell/remapping`, pairwise: what Vite does today.
- `@jridgewell/remapping`, one call per chain: the full plugin chain in one `remapping` call.
- `@srcmap/remapping`: only when `packages/remapping-wasm` is built.

Each implementation runs in a new process. The benchmark reports CPU time, because wall time is not stable on a busy machine.

- **cold** is the first pass. A Vite run pays this cost, because it combines each module one time.
- **warm** is the best of the later passes.

The benchmark also counts the outputs that differ from the output of Vite today.

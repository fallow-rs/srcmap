import { Bench } from 'tinybench';
import { readFileSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { TraceMap, originalPositionFor } from '@jridgewell/trace-mapping';
import { SourceMapConsumer } from 'source-map-js';
import { SourceMap } from '../packages/sourcemap-wasm/pkg/srcmap_sourcemap_wasm.js';
import { SourceMap as NapiSourceMap } from '../packages/sourcemap/index.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const fixturesDir = join(__dirname, 'fixtures');

// ── Load fixtures ────────────────────────────────────────────────

const FIXTURES = [
  { name: 'Preact', file: 'preact.js.map' },
  { name: 'Chart.js', file: 'chartjs.js.map' },
  { name: 'PDF.js', file: 'pdfjs.js.map' },
];

const maps = [];

for (const fixture of FIXTURES) {
  const filePath = join(fixturesDir, fixture.file);

  if (!existsSync(filePath)) {
    console.error(`Missing: fixtures/${fixture.file}`);
    console.error('Run: npm run download-fixtures\n');
    process.exit(1);
  }

  const json = readFileSync(filePath, 'utf-8');
  const parsed = JSON.parse(json);

  const lines = (parsed.mappings.match(/;/g) || []).length + 1;
  const segments = parsed.mappings
    .split(';')
    .reduce((total, line) => total + (line ? line.split(',').length : 0), 0);

  maps.push({
    name: fixture.name,
    json,
    size: json.length,
    lines,
    segments,
    sources: parsed.sources?.length || 0,
  });
}

// ── Header ───────────────────────────────────────────────────────

console.log('=== Real-World Source Map Benchmarks ===\n');
console.log('Libraries:');
console.log('  @jridgewell/trace-mapping  — de facto standard JS source map consumer');
console.log('  source-map-js              — Mozilla source-map fork (used by Vite, PostCSS)');
console.log('  srcmap WASM                — srcmap Rust core via WebAssembly');
console.log('  srcmap NAPI                — srcmap Rust core via N-API\n');

console.log('Source maps:');
for (const m of maps) {
  const sizeStr =
    m.size > 1024 * 1024
      ? `${(m.size / 1024 / 1024).toFixed(1)} MB`
      : `${(m.size / 1024).toFixed(0)} KB`;

  console.log(
    `  ${m.name.padEnd(12)} ${sizeStr.padStart(8)}  ${String(m.segments).padStart(7)} segments  ${String(m.lines).padStart(6)} lines  ${m.sources} sources`
  );
}

// ── Correctness check ────────────────────────────────────────────

// trace-mapping normalizes source paths (resolves ./ segments),
// srcmap returns raw paths. Normalize both for fair comparison.
const normalizePath = (s) => s?.replace(/\/\.\//g, '/') ?? null;

console.log('\n--- Correctness Check ---\n');

for (const { name, json } of maps) {
  const trace = new TraceMap(json);
  const wasm = new SourceMap(json);
  const napi = new NapiSourceMap(json);
  const maxLine = wasm.lineCount;

  let wasmPass = true;
  let napiPass = true;
  let checked = 0;

  for (
    let line = 0;
    line < maxLine && checked < 200;
    line += Math.max(1, Math.floor(maxLine / 100))
  ) {
    for (let col = 0; col < 300; col += 30) {
      const expected = originalPositionFor(trace, { line: line + 1, column: col });
      const expectedNull = !expected.source;

      // WASM
      const wr = wasm.originalPositionFor(line, col);
      if (expectedNull !== (wr === null)) wasmPass = false;
      else if (!expectedNull && wr !== null) {
        if (
          normalizePath(expected.source) !== normalizePath(wr.source) ||
          expected.line !== wr.line + 1 ||
          expected.column !== wr.column
        )
          wasmPass = false;
      }

      // NAPI
      const nr = napi.originalPositionFor(line, col);
      if (expectedNull !== (nr === null)) napiPass = false;
      else if (!expectedNull && nr !== null) {
        if (
          normalizePath(expected.source) !== normalizePath(nr.source) ||
          expected.line !== nr.line + 1 ||
          expected.column !== nr.column
        )
          napiPass = false;
      }

      checked++;
    }
  }

  console.log(
    `  ${name}: WASM ${wasmPass ? 'PASS' : 'FAIL'}, NAPI ${napiPass ? 'PASS' : 'FAIL'} (${checked} lookups)`
  );
}

// ── Parse benchmarks ─────────────────────────────────────────────

console.log('\n--- Parse ---\n');

for (const { name, json, size } of maps) {
  console.log(`### ${name}\n`);

  // Fewer iterations for large maps
  const iterations = size > 1024 * 1024 ? 50 : 200;
  const bench = new Bench({ warmupIterations: 10, iterations });

  bench
    .add('trace-mapping', () => new TraceMap(json))
    .add('source-map-js', () => new SourceMapConsumer(json))
    .add('srcmap WASM', () => new SourceMap(json))
    .add('srcmap NAPI', () => new NapiSourceMap(json));

  await bench.run();

  console.table(
    bench.tasks.map((task) => ({
      Name: task.name,
      'ops/sec': Math.round(task.result.hz).toLocaleString(),
      'avg (ms)': task.result.mean.toFixed(2),
      'p99 (ms)': task.result.p99.toFixed(2),
    }))
  );
}

// ── Single lookup ────────────────────────────────────────────────

console.log('\n--- Single Lookup ---\n');

for (const { name, json } of maps) {
  console.log(`### ${name}\n`);

  const trace = new TraceMap(json);
  const smjs = new SourceMapConsumer(json);
  const wasm = new SourceMap(json);
  const napi = new NapiSourceMap(json);

  // Pick a lookup position roughly in the middle of the map
  const midLine = Math.floor(wasm.lineCount / 2);

  const bench = new Bench({ warmupIterations: 500, iterations: 5000 });

  bench
    .add('trace-mapping', () =>
      originalPositionFor(trace, { line: midLine + 1, column: 20 })
    )
    .add('source-map-js', () =>
      smjs.originalPositionFor({ line: midLine + 1, column: 20 })
    )
    .add('srcmap WASM', () => wasm.originalPositionFor(midLine, 20))
    .add('srcmap WASM (flat)', () => wasm.originalPositionFlat(midLine, 20))
    .add('srcmap NAPI', () => napi.originalPositionFor(midLine, 20));

  await bench.run();

  console.table(
    bench.tasks.map((task) => ({
      Name: task.name,
      'ops/sec': Math.round(task.result.hz).toLocaleString(),
      'avg (ns)': Math.round(task.result.mean * 1_000_000).toLocaleString(),
      'p99 (ns)': Math.round(task.result.p99 * 1_000_000).toLocaleString(),
    }))
  );
}

// ── Batch lookup (1000x) ─────────────────────────────────────────

console.log('\n--- 1000x Lookup ---\n');

for (const { name, json } of maps) {
  console.log(`### ${name}\n`);

  const trace = new TraceMap(json);
  const smjs = new SourceMapConsumer(json);
  const wasm = new SourceMap(json);
  const napi = new NapiSourceMap(json);
  const maxLine = wasm.lineCount;

  const lookups = [];
  const flatPositions = [];

  for (let i = 0; i < 1000; i++) {
    const line = Math.floor(Math.random() * maxLine);
    const column = Math.floor(Math.random() * 200);
    lookups.push({ line, column });
    flatPositions.push(line, column);
  }
  const posArray = new Int32Array(flatPositions);

  const bench = new Bench({ warmupIterations: 20, iterations: 200 });

  bench
    .add('trace-mapping', () => {
      for (const { line, column } of lookups)
        originalPositionFor(trace, { line: line + 1, column });
    })
    .add('source-map-js', () => {
      for (const { line, column } of lookups)
        smjs.originalPositionFor({ line: line + 1, column });
    })
    .add('srcmap WASM (individual)', () => {
      for (const { line, column } of lookups)
        wasm.originalPositionFor(line, column);
    })
    .add('srcmap WASM (batch)', () => {
      wasm.originalPositionsFor(posArray);
    })
    .add('srcmap NAPI (batch)', () => {
      napi.originalPositionsFor(flatPositions);
    });

  await bench.run();

  console.table(
    bench.tasks.map((task) => ({
      Name: task.name,
      'ops/sec': Math.round(task.result.hz).toLocaleString(),
      'avg (μs)': (task.result.mean * 1000).toFixed(1),
      'per lookup (ns)': Math.round(task.result.mean * 1_000_000).toLocaleString(),
    }))
  );
}

import { Bench } from 'tinybench';
import { TraceMap, originalPositionFor, generatedPositionFor } from '@jridgewell/trace-mapping';
import { SourceMap } from '../packages/sourcemap/index.js';
import { encode } from '@jridgewell/sourcemap-codec';

// ── Generate realistic source maps ────────────────────────────────

function generateSourceMap(lines, segsPerLine, numSources) {
  const sources = Array.from({ length: numSources }, (_, i) => `src/file${i}.js`);
  const names = Array.from({ length: 20 }, (_, i) => `var${i}`);
  const sourcesContent = sources.map((_, i) => `// source file ${i}\n${'const x = 1;\n'.repeat(lines)}`);

  const mappings = [];
  let src = 0, srcLine = 0, srcCol = 0, name = 0;

  for (let line = 0; line < lines; line++) {
    const lineSegs = [];
    let genCol = 0;

    for (let s = 0; s < segsPerLine; s++) {
      genCol += 2 + (s * 3) % 20;
      if (s % 7 === 0) src = (src + 1) % numSources;
      srcLine += 1;
      srcCol = (s * 5 + 1) % 30;

      if (s % 4 === 0) {
        name = (name + 1) % names.length;
        lineSegs.push([genCol, src, srcLine, srcCol, name]);
      } else {
        lineSegs.push([genCol, src, srcLine, srcCol]);
      }
    }
    mappings.push(lineSegs);
  }

  return JSON.stringify({
    version: 3,
    sources,
    sourcesContent,
    names,
    mappings: encode(mappings),
  });
}

const SMALL_JSON = generateSourceMap(50, 10, 3);
const MEDIUM_JSON = generateSourceMap(500, 20, 5);
const LARGE_JSON = generateSourceMap(2000, 50, 10);

const maps = [
  { name: 'small (50 lines, 500 segs)', json: SMALL_JSON },
  { name: 'medium (500 lines, 10K segs)', json: MEDIUM_JSON },
  { name: 'large (2000 lines, 100K segs)', json: LARGE_JSON },
];

console.log(`JSON sizes: small=${(SMALL_JSON.length/1024).toFixed(1)}KB, medium=${(MEDIUM_JSON.length/1024).toFixed(1)}KB, large=${(LARGE_JSON.length/1024).toFixed(1)}KB\n`);

// ── Correctness verification ──────────────────────────────────────

console.log('Verifying correctness...\n');
for (const { name, json } of maps) {
  const trace = new TraceMap(json);
  const srcmap = new SourceMap(json);

  let pass = true;
  let checked = 0;

  for (let line = 0; line < srcmap.lineCount && checked < 100; line += Math.max(1, Math.floor(srcmap.lineCount / 50))) {
    for (let col = 0; col < 200; col += 20) {
      const expected = originalPositionFor(trace, { line: line + 1, column: col });
      const actual = srcmap.originalPositionFor(line, col);

      const expectedNull = !expected.source;
      const actualNull = actual === null;

      if (expectedNull !== actualNull) {
        console.log(`  MISMATCH at ${line}:${col}: trace=${JSON.stringify(expected)}, srcmap=${JSON.stringify(actual)}`);
        pass = false;
      } else if (!expectedNull && !actualNull) {
        if (expected.source !== actual.source ||
            expected.line !== actual.line + 1 ||
            expected.column !== actual.column) {
          console.log(`  MISMATCH at ${line}:${col}: trace=${JSON.stringify(expected)}, srcmap=${JSON.stringify(actual)}`);
          pass = false;
        }
      }
      checked++;
    }
  }

  console.log(`${name}: ${pass ? 'PASS' : 'FAIL'} (checked ${checked} lookups)`);
}

// ── Parse benchmarks ──────────────────────────────────────────────

console.log('\n--- Parse Benchmarks ---\n');

for (const { name, json } of maps) {
  console.log(`\n### ${name}\n`);

  const bench = new Bench({ warmupIterations: 20, iterations: 200 });

  bench
    .add('trace-mapping parse', () => new TraceMap(json))
    .add('srcmap parse', () => new SourceMap(json));

  await bench.run();

  console.table(
    bench.tasks.map((task) => ({
      Name: task.name,
      'ops/sec': Math.round(task.result.hz).toLocaleString(),
      'avg (μs)': (task.result.mean * 1000).toFixed(1),
      'p99 (μs)': (task.result.p99 * 1000).toFixed(1),
    }))
  );
}

// ── Lookup benchmarks ─────────────────────────────────────────────

console.log('\n--- originalPositionFor Benchmarks ---\n');

for (const { name, json } of maps) {
  console.log(`\n### ${name}\n`);

  const trace = new TraceMap(json);
  const srcmap = new SourceMap(json);
  const maxLine = srcmap.lineCount;

  const lookups = [];
  for (let i = 0; i < 1000; i++) {
    lookups.push({
      line: Math.floor(Math.random() * maxLine),
      column: Math.floor(Math.random() * 200),
    });
  }

  const bench = new Bench({ warmupIterations: 50, iterations: 500 });

  bench
    .add('trace-mapping lookup (1000x)', () => {
      for (const { line, column } of lookups) {
        originalPositionFor(trace, { line: line + 1, column });
      }
    })
    .add('srcmap lookup (1000x)', () => {
      for (const { line, column } of lookups) {
        srcmap.originalPositionFor(line, column);
      }
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

// ── Batch lookup benchmark ─────────────────────────────────────────

console.log('\n--- Batch originalPositionFor (1000 lookups/call) ---\n');

for (const { name, json } of maps) {
  console.log(`\n### ${name}\n`);

  const trace = new TraceMap(json);
  const srcmap = new SourceMap(json);
  const maxLine = srcmap.lineCount;

  const lookups = [];
  const flatPositions = [];
  for (let i = 0; i < 1000; i++) {
    const line = Math.floor(Math.random() * maxLine);
    const column = Math.floor(Math.random() * 200);
    lookups.push({ line, column });
    flatPositions.push(line, column);
  }

  const bench = new Bench({ warmupIterations: 50, iterations: 500 });

  bench
    .add('trace-mapping 1000x individual', () => {
      for (const { line, column } of lookups) {
        originalPositionFor(trace, { line: line + 1, column });
      }
    })
    .add('srcmap batch 1000x', () => {
      srcmap.originalPositionsFor(flatPositions);
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

// ── Single lookup benchmark ────────────────────────────────────────

console.log('\n--- Single originalPositionFor ---\n');

{
  const json = MEDIUM_JSON;
  const trace = new TraceMap(json);
  const srcmap = new SourceMap(json);

  const bench = new Bench({ warmupIterations: 1000, iterations: 5000 });

  bench
    .add('trace-mapping single lookup', () => {
      originalPositionFor(trace, { line: 251, column: 30 });
    })
    .add('srcmap single lookup', () => {
      srcmap.originalPositionFor(250, 30);
    });

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

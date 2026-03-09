import { Bench } from 'tinybench';
import { decode as jridgewellDecode, encode as jridgewellEncode } from '@jridgewell/sourcemap-codec';
import { decode as srcmapDecode, encode as srcmapEncode } from '../packages/codec/index.js';

// Test mappings of increasing complexity
const SMALL_MAP = 'AAAA;AACA,GAAG;AACA,IAAI,EAAE';
const MEDIUM_MAP = Array.from({ length: 100 }, () => 'AAAA,GAAG,EAAE,IAAI,KAAK').join(';');
const LARGE_MAP = Array.from({ length: 1000 }, () =>
  Array.from({ length: 50 }, () => 'AAAA').join(',')
).join(';');

const maps = [
  { name: 'small (3 lines)', mappings: SMALL_MAP },
  { name: 'medium (100 lines, 500 segments)', mappings: MEDIUM_MAP },
  { name: 'large (1000 lines, 50K segments)', mappings: LARGE_MAP },
];

// Verify correctness first
console.log('Verifying correctness...\n');
for (const { name, mappings } of maps) {
  const jResult = jridgewellDecode(mappings);
  const sResult = srcmapDecode(mappings);

  const jEncoded = jridgewellEncode(jResult);
  const sEncoded = srcmapEncode(sResult);

  const decodeMatch = JSON.stringify(jResult) === JSON.stringify(sResult);
  const encodeMatch = jEncoded === sEncoded;

  console.log(`${name}:`);
  console.log(`  decode match: ${decodeMatch ? 'PASS' : 'FAIL'}`);
  console.log(`  encode match: ${encodeMatch ? 'PASS' : 'FAIL'}`);

  if (!decodeMatch) {
    console.log('  jridgewell:', JSON.stringify(jResult).slice(0, 200));
    console.log('  srcmap:    ', JSON.stringify(sResult).slice(0, 200));
  }
}

console.log('\n--- Benchmarks ---\n');

for (const { name, mappings } of maps) {
  console.log(`\n### ${name}\n`);

  // Pre-decode for encode benchmarks
  const decoded = jridgewellDecode(mappings);

  const bench = new Bench({ warmupIterations: 100, iterations: 1000 });

  bench
    .add('jridgewell decode', () => jridgewellDecode(mappings))
    .add('srcmap decode', () => srcmapDecode(mappings))
    .add('jridgewell encode', () => jridgewellEncode(decoded))
    .add('srcmap encode', () => srcmapEncode(decoded));

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

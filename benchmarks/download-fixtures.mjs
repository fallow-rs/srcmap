import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const fixturesDir = join(__dirname, 'fixtures');

// Real-world source maps from popular open source projects.
// Each entry tries multiple URLs (fallbacks) since package layouts vary by version.
const FIXTURES = [
  {
    name: 'Preact',
    file: 'preact.js.map',
    urls: [
      'https://unpkg.com/preact@10.23.2/dist/preact.module.js.map',
      'https://unpkg.com/preact@10.23.2/dist/preact.js.map',
    ],
  },
  {
    name: 'Chart.js',
    file: 'chartjs.js.map',
    urls: [
      'https://unpkg.com/chart.js@4.4.3/dist/chart.js.map',
      'https://unpkg.com/chart.js@4.4.3/dist/chart.umd.js.map',
    ],
  },
  {
    name: 'PDF.js',
    file: 'pdfjs.js.map',
    urls: [
      'https://unpkg.com/pdfjs-dist@4.4.168/build/pdf.worker.mjs.map',
      'https://unpkg.com/pdfjs-dist@4.4.168/build/pdf.mjs.map',
    ],
  },
];

if (!existsSync(fixturesDir)) {
  mkdirSync(fixturesDir, { recursive: true });
}

console.log('Downloading real-world source maps...\n');

let allOk = true;

for (const fixture of FIXTURES) {
  const dest = join(fixturesDir, fixture.file);

  if (existsSync(dest)) {
    console.log(`  ${fixture.name}: already exists, skipping`);
    continue;
  }

  let downloaded = false;

  for (const url of fixture.urls) {
    try {
      const res = await fetch(url);
      if (!res.ok) continue;

      const text = await res.text();
      const parsed = JSON.parse(text);

      if (!parsed.mappings || !parsed.version) continue;

      writeFileSync(dest, text);

      const sizeKB = (text.length / 1024).toFixed(0);
      console.log(`  ${fixture.name}: ${sizeKB} KB (from ${url})`);
      downloaded = true;
      break;
    } catch {
      continue;
    }
  }

  if (!downloaded) {
    console.error(`  ${fixture.name}: FAILED — could not download from any URL`);
    console.error(`    Tried: ${fixture.urls.join('\n           ')}`);
    allOk = false;
  }
}

console.log();

if (allOk) {
  console.log('All fixtures ready! Run: npm run bench:real-world');
} else {
  console.error('Some downloads failed. You can manually place source map files in benchmarks/fixtures/');
  process.exit(1);
}

// Adds a recorder to an installed Vite, or removes it again.
//
//   node instrument.mjs <project-dir>            add the recorder
//   node instrument.mjs <project-dir> --restore  put the original file back
//
// The recorder is inactive until SRCMAP_WORKLOAD_OUT is set. See README.md.

import { copyFileSync, existsSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";

const BACKUP_SUFFIX = ".srcmap-workload-orig";

const RECORDER = String.raw`
const __srcmapRec = (() => {
	const g = globalThis;
	if (g.__srcmapWorkloadRecorder) return g.__srcmapWorkloadRecorder;
	const out = process.env.SRCMAP_WORKLOAD_OUT;
	const capture = process.env.SRCMAP_WORKLOAD_CAPTURE === "1";
	const stats = {
		combineCalls: 0, combineMs: 0,
		generateMapCalls: 0, generateMapMs: 0,
		ssrTransformCalls: 0, ssrTransformMs: 0,
		transformCalls: 0, transformMs: 0,
		chainLengths: {},
	};
	const records = [];
	const loadedAt = performance.now();
	let chainDepth = 0;
	let generateDepth = 0;
	const rec = {
		combine(fn, filename, list) {
			if (!out) return fn(filename, list);
			if (capture && chainDepth === 0) records.push(JSON.stringify({ kind: "pair", file: filename, maps: list }));
			const t = performance.now();
			try {
				return fn(filename, list);
			} finally {
				stats.combineMs += performance.now() - t;
				stats.combineCalls++;
			}
		},
		chain(ctx, fn) {
			if (!out) return fn.call(ctx);
			const pending = ctx.sourcemapChain.length;
			const key = pending === 0 ? (ctx.combinedMap ? "cached" : "0") : String(pending + (ctx.combinedMap ? 1 : 0));
			stats.chainLengths[key] = (stats.chainLengths[key] ?? 0) + 1;
			if (capture && pending > 0 && pending + (ctx.combinedMap ? 1 : 0) >= 2) {
				records.push(JSON.stringify({ kind: "chain", file: ctx.filename, combined: ctx.combinedMap ?? null, maps: ctx.sourcemapChain }));
			}
			chainDepth++;
			try {
				return fn.call(ctx);
			} finally {
				chainDepth--;
			}
		},
		generate(self, fn, options) {
			if (!out || generateDepth > 0) return fn.call(self, options);
			generateDepth++;
			const t = performance.now();
			try {
				return fn.call(self, options);
			} finally {
				stats.generateMapMs += performance.now() - t;
				stats.generateMapCalls++;
				generateDepth--;
			}
		},
		async timed(name, fn, args) {
			if (!out) return fn(...args);
			const t = performance.now();
			try {
				return await fn(...args);
			} finally {
				stats[name + "Ms"] += performance.now() - t;
				stats[name + "Calls"]++;
			}
		},
	};
	if (out) {
		const fs = process.getBuiltinModule("node:fs");
		const worker = process.getBuiltinModule("node:worker_threads");
		process.on("exit", () => {
			const fd = fs.openSync(out + "." + process.pid + "." + worker.threadId + ".ndjson", "w");
			fs.writeSync(fd, JSON.stringify({ kind: "summary", ...stats, wallMs: performance.now() - loadedAt }) + "\n");
			for (const record of records) fs.writeSync(fd, record + "\n");
			fs.closeSync(fd);
		});
	}
	g.__srcmapWorkloadRecorder = rec;
	return rec;
})();
`;

/** Each patch replaces one function head with a wrapper that calls the renamed original. */
const PATCHES = [
  {
    name: "combineSourcemaps",
    find: "function combineSourcemaps(filename, sourcemapList) {",
    replace:
      "function combineSourcemaps(filename, sourcemapList) {\n\treturn __srcmapRec.combine(__srcmapCombineOrig, filename, sourcemapList);\n}\nfunction __srcmapCombineOrig(filename, sourcemapList) {",
  },
  {
    name: "_getCombinedSourcemap",
    find: "\t_getCombinedSourcemap() {",
    replace:
      "\t_getCombinedSourcemap() {\n\t\treturn __srcmapRec.chain(this, this.__srcmapChainOrig);\n\t}\n\t__srcmapChainOrig() {",
  },
  {
    name: "MagicString.generateDecodedMap",
    find: "\tgenerateDecodedMap(options) {",
    replace:
      "\tgenerateDecodedMap(options) {\n\t\treturn __srcmapRec.generate(this, this.__srcmapGenDecodedOrig, options);\n\t}\n\t__srcmapGenDecodedOrig(options) {",
  },
  {
    name: "MagicString.generateMap",
    find: "\tgenerateMap(options) {",
    replace:
      "\tgenerateMap(options) {\n\t\treturn __srcmapRec.generate(this, this.__srcmapGenOrig, options);\n\t}\n\t__srcmapGenOrig(options) {",
  },
  {
    name: "ssrTransform",
    find: "async function ssrTransform(code, inMap, url, originalCode, options) {",
    replace:
      'async function ssrTransform(...args) {\n\treturn __srcmapRec.timed("ssrTransform", __srcmapSsrTransformOrig, args);\n}\nasync function __srcmapSsrTransformOrig(code, inMap, url, originalCode, options) {',
  },
  {
    name: "loadAndTransform",
    find: "async function loadAndTransform(environment, id, url, options, timestamp, mod, resolved) {",
    replace:
      'async function loadAndTransform(...args) {\n\treturn __srcmapRec.timed("transform", __srcmapLoadAndTransformOrig, args);\n}\nasync function __srcmapLoadAndTransformOrig(environment, id, url, options, timestamp, mod, resolved) {',
  },
];

const findViteChunk = (projectDir) => {
  const require = createRequire(join(realpathSync(projectDir), "package.json"));
  const viteDir = dirname(require.resolve("vite/package.json"));
  const version = JSON.parse(readFileSync(join(viteDir, "package.json"), "utf8")).version;
  return { file: join(viteDir, "dist/node/chunks/node.js"), version };
};

const countOccurrences = (text, needle) => text.split(needle).length - 1;

const [projectDir, flag] = process.argv.slice(2);
if (!projectDir) {
  console.error("usage: node instrument.mjs <project-dir> [--restore]");
  process.exit(1);
}

const { file, version } = findViteChunk(projectDir);
const backup = file + BACKUP_SUFFIX;

if (flag === "--restore") {
  if (existsSync(backup)) copyFileSync(backup, file);
  console.log(`restored vite ${version}: ${file}`);
  process.exit(0);
}

if (!existsSync(file)) {
  console.error(
    `vite ${version} has no dist/node/chunks/node.js; this Vite layout is not supported`,
  );
  process.exit(1);
}
if (!existsSync(backup)) copyFileSync(file, backup);

let source = readFileSync(backup, "utf8");
for (const patch of PATCHES) {
  const count = countOccurrences(source, patch.find);
  if (count !== 1) {
    console.error(`vite ${version}: expected one "${patch.name}" anchor, found ${count}`);
    process.exit(1);
  }
  source = source.replace(patch.find, () => patch.replace);
}
writeFileSync(file, RECORDER + source);
console.log(`instrumented vite ${version}: ${file}`);

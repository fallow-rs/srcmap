// Transforms a module graph through an unbundled Vite dev server, one module at a time.
//
//   node crawl.mjs --root <project-dir> --env client|ssr --entry <file> [--entry <file>] [--config <file>]
//
// The crawl needs no browser. It starts at the entries and follows the imports that
// Vite records in the module graph. It skips modules in node_modules.

import { realpathSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";

const { values } = parseArgs({
  options: {
    root: { type: "string" },
    env: { type: "string", default: "client" },
    entry: { type: "string", multiple: true },
    config: { type: "string" },
  },
});

if (!values.root || !values.entry?.length) {
  console.error(
    "usage: node crawl.mjs --root <dir> --env client|ssr --entry <file> [--config <file>]",
  );
  process.exit(1);
}

const root = realpathSync(values.root);
const require = createRequire(join(root, "package.json"));
const viteDir = dirname(require.resolve("vite/package.json"));
const { createServer } = await import(pathToFileURL(join(viteDir, "dist/node/index.js")).href);

const toUrl = (file) => {
  const abs = realpathSync(resolve(root, file));
  const rel = relative(root, abs);
  return rel.startsWith("..") || isAbsolute(rel) ? `/@fs${abs}` : `/${rel}`;
};

const server = await createServer({
  root,
  configFile: values.config ? resolve(values.config) : undefined,
  logLevel: "error",
  appType: "custom",
  server: { middlewareMode: true, ws: false, hmr: false, watch: null, preTransformRequests: false },
  optimizeDeps: { noDiscovery: true, include: [] },
  experimental: { bundledDev: false },
});

const environment = server.environments[values.env];
if (!environment) {
  console.error(
    `no "${values.env}" environment; found: ${Object.keys(server.environments).join(", ")}`,
  );
  process.exit(1);
}

const queue = values.entry.map(toUrl);
const seen = new Set(queue);
const failures = [];
let transformed = 0;
const startedAt = performance.now();

while (queue.length > 0) {
  const url = queue.shift();
  try {
    await environment.transformRequest(url);
    transformed++;
  } catch (error) {
    failures.push({ url, message: String(error?.message ?? error).split("\n")[0] });
    continue;
  }
  const mod = await environment.moduleGraph.getModuleByUrl(url);
  for (const imported of mod?.importedModules ?? []) {
    if (seen.has(imported.url)) continue;
    seen.add(imported.url);
    if (imported.id?.includes("/node_modules/")) continue;
    queue.push(imported.url);
  }
}

const crawlMs = performance.now() - startedAt;
await server.close();

console.log(
  JSON.stringify({
    env: values.env,
    transformed,
    failed: failures.length,
    crawlMs: Math.round(crawlMs),
  }),
);
for (const failure of failures.slice(0, 10))
  console.log(`failed: ${failure.url}: ${failure.message}`);

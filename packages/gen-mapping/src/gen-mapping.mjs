import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { createGenMappingApi } = require("./gen-mapping-core.cjs");
const { SourceMapGenerator } = require("@srcmap/generator-wasm");
const { SourceMap } = require("@srcmap/sourcemap-wasm");
const {
  GenMapping,
  addMapping,
  maybeAddMapping,
  setSourceContent,
  setIgnore,
  allMappings,
  toEncodedMap,
  toDecodedMap,
  fromMap,
} = createGenMappingApi({ SourceMapGenerator, SourceMap });

export {
  GenMapping,
  addMapping,
  maybeAddMapping,
  setSourceContent,
  setIgnore,
  allMappings,
  toEncodedMap,
  toDecodedMap,
  fromMap,
};

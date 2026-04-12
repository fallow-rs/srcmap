"use strict";

const { createGenMappingApi } = require("./gen-mapping-core.cjs");

let SourceMapGenerator;
try {
  SourceMapGenerator = require("@srcmap/generator-wasm").SourceMapGenerator;
} catch {
  // fallow-ignore-next-line unresolved-import
  const GeneratedSourceMapGenerator =
    require("../../generator-wasm/pkg/srcmap_generator_wasm.js").SourceMapGenerator;
  SourceMapGenerator = GeneratedSourceMapGenerator;
}

let SourceMap;
try {
  SourceMap = require("@srcmap/sourcemap-wasm").SourceMap;
} catch {
  // fallow-ignore-next-line unresolved-import
  SourceMap = require("../../sourcemap-wasm/pkg/srcmap_sourcemap_wasm.js").SourceMap;
}
module.exports = createGenMappingApi({ SourceMapGenerator, SourceMap });

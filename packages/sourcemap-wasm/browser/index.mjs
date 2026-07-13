/**
 * Browser-friendly async wrapper for @srcmap/sourcemap-wasm.
 *
 * Usage:
 *   import init, { SourceMap } from '@srcmap/sourcemap-wasm/browser'
 *   await init()
 *   const sm = new SourceMap(jsonString)
 *
 * For bundlers (webpack, vite, etc.), the default export handles
 * WASM initialization automatically. For direct browser usage,
 * call init() before using any exports.
 */

import initWasm, {
  LazySourceMap,
  SourceMap,
  resultPtr,
  wasmMemory,
} from "../web/srcmap_sourcemap_wasm.js";

export { LazySourceMap, SourceMap, resultPtr, wasmMemory };

let initPromise = null;

/**
 * Initialize the WASM module. Must be called before using any exports.
 * Safe to call multiple times, subsequent calls return the same promise.
 * @param {string|URL|Request|BufferSource} [input] Optional WASM module source
 * @returns {Promise<void>}
 */
export default function init(input) {
  if (!initPromise) {
    initPromise = initWasm(input).then(() => undefined);
  }

  return initPromise;
}

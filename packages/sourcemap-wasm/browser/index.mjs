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

let initialized = false
let initPromise = null

/**
 * Initialize the WASM module. Must be called before using any exports.
 * Safe to call multiple times — subsequent calls return immediately.
 * @param {string|URL|Request|BufferSource} [input] - Optional WASM module source
 * @returns {Promise<void>}
 */
export default async function init(input) {
  if (initialized) return
  if (initPromise) return initPromise

  initPromise = (async () => {
    const wasm = await import('../web/srcmap_sourcemap_wasm.js')
    await wasm.default(input)
    initialized = true

    // Re-export all WASM exports
    Object.assign(exports, wasm)
  })()

  return initPromise
}

const exports = {}

export { exports as wasm }

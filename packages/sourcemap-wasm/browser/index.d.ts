import type { InitInput } from "../web/srcmap_sourcemap_wasm.js";

export { LazySourceMap, SourceMap, resultPtr, wasmMemory } from "../web/srcmap_sourcemap_wasm.js";

declare const init: (
  input?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>,
) => Promise<void>;

export default init;

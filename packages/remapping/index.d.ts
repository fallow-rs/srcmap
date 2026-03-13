/**
 * @srcmap/remapping — drop-in replacement for @jridgewell/remapping
 * and @ampproject/remapping powered by Rust via WebAssembly.
 */

// ── Source map types ────────────────────────────────────────────

export interface RawSourceMap {
  version: number
  file?: string
  sourceRoot?: string
  sources: (string | null)[]
  sourcesContent?: (string | null)[]
  names: string[]
  mappings: string
  ignoreList?: number[]
}

export type EncodedSourceMap = RawSourceMap

export interface DecodedSourceMap {
  version: number
  file?: string
  sourceRoot?: string
  sources: (string | null)[]
  sourcesContent?: (string | null)[]
  names: string[]
  mappings: number[][][]
  ignoreList?: number[]
}

export type SourceMapInput = RawSourceMap | DecodedSourceMap | string

// ── Options ─────────────────────────────────────────────────────

export interface Options {
  excludeContent?: boolean
}

// ── SourceMap result class ──────────────────────────────────────

export declare class SourceMap {
  version: number
  file: string | undefined
  mappings: string
  names: string[]
  sources: (string | null)[]
  sourcesContent: (string | null)[] | undefined
  sourceRoot: string | undefined
  ignoreList: number[] | undefined

  constructor(raw: RawSourceMap)
  toString(): string
  toJSON(): RawSourceMap
}

// ── Loader type ─────────────────────────────────────────────────

export type Loader = (
  sourcefile: string,
) => SourceMapInput | null | undefined | void

// ── Main API ────────────────────────────────────────────────────

/**
 * Remap/compose source maps. Drop-in replacement for @jridgewell/remapping.
 *
 * Supports two calling conventions:
 * - `remapping(singleMap, loader)` — remap a single source map through a loader
 * - `remapping([map1, map2, ...], loader)` — compose an array of source maps
 */
declare function remapping(
  input: SourceMapInput,
  loader: Loader,
  options?: Options,
): SourceMap

declare function remapping(
  input: SourceMapInput[],
  loader: Loader,
  options?: Options,
): SourceMap

export default remapping
export { remapping }

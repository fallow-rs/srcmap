/** The @jridgewell/remapping and @ampproject/remapping API on the srcmap Rust remapping code, compiled to WebAssembly. */

// ── Source map types ────────────────────────────────────────────

export interface RawSourceMap {
  version: number;
  file?: string;
  sourceRoot?: string;
  sources: (string | null)[];
  sourcesContent?: (string | null)[];
  names: string[];
  mappings: string;
  ignoreList?: number[];
}

export type EncodedSourceMap = RawSourceMap;

export interface DecodedSourceMap {
  version: number;
  file?: string;
  sourceRoot?: string;
  sources: (string | null)[];
  sourcesContent?: (string | null)[];
  names: string[];
  mappings: number[][][];
  ignoreList?: number[];
}

export type SourceMapInput = RawSourceMap | DecodedSourceMap | string;

// ── Options ─────────────────────────────────────────────────────

export interface Options {
  /** Leave `sourcesContent` out of the result. */
  excludeContent?: boolean;
  /** Give `mappings` as decoded segment arrays, not as a VLQ string. */
  decodedMappings?: boolean;
}

// ── SourceMap result class ──────────────────────────────────────

export declare class SourceMap {
  version: 3;
  file: string | undefined;
  mappings: string | number[][][];
  names: string[];
  sources: (string | null)[];
  sourcesContent?: (string | null)[];
  sourceRoot: string | undefined;
  ignoreList: number[];

  constructor(raw: RawSourceMap, options?: Options);
  toString(): string;
  toJSON(): RawSourceMap;
}

// ── Loader type ─────────────────────────────────────────────────

/** The second loader argument. The loader can change `source`, `content` and `ignore`. */
export interface LoaderContext {
  readonly importer: string;
  readonly depth: number;
  source: string;
  content: string | null | undefined;
  ignore: boolean | undefined;
}

export type Loader = (
  sourcefile: string,
  ctx: LoaderContext,
) => SourceMapInput | null | undefined | void;

// ── Main API ────────────────────────────────────────────────────

/**
 * Remap/compose source maps. Takes the arguments of @jridgewell/remapping.
 *
 * Supports two calling conventions:
 * - `remapping(singleMap, loader)`: remap a single source map through a loader
 * - `remapping([map1, map2, ...], loader)`: compose an array of source maps
 *
 * `options` set to `true` means `{ excludeContent: true }`.
 */
declare function remapping(
  input: SourceMapInput | SourceMapInput[],
  loader: Loader,
  options?: boolean | Options,
): SourceMap;

export default remapping;
export { remapping };

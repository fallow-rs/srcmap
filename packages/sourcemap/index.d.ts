/**
 * An original source position resolved from a generated position.
 *
 * All values are 0-based. Use `source` and `name` directly as resolved strings.
 */
export interface OriginalPosition {
  /** Original source filename (e.g. `"src/app.ts"`). */
  source: string | null;
  /** 0-based line in the original source. */
  line: number;
  /** 0-based column in the original source. */
  column: number;
  /** Original identifier name, if available. */
  name: string | null;
}

/**
 * A generated position resolved from an original source position.
 *
 * All values are 0-based.
 */
export interface GeneratedPosition {
  /** 0-based line in the generated output. */
  line: number;
  /** 0-based column in the generated output. */
  column: number;
}

/**
 * High-performance source map parser and consumer powered by Rust via NAPI.
 *
 * Parses source map JSON (v3 / ECMA-426) and provides O(log n) position lookups.
 * Supports regular and indexed (sectioned) source maps.
 *
 * @example
 * ```ts
 * import { SourceMap } from '@srcmap/sourcemap'
 *
 * const sm = new SourceMap(jsonString)
 *
 * // Forward lookup: generated -> original (0-based)
 * const loc = sm.originalPositionFor(42, 10)
 * if (loc) {
 *   console.log(`${loc.source}:${loc.line}:${loc.column}`)
 * }
 *
 * // Reverse lookup: original -> generated (0-based)
 * const pos = sm.generatedPositionFor('src/app.ts', 10, 4)
 * ```
 */
export declare class SourceMap {
  /**
   * Parse a source map from a JSON string.
   *
   * Accepts both regular source maps and indexed source maps with `sections`.
   *
   * @param json - Source map JSON string (must have `"version": 3`)
   * @throws If the JSON is invalid or the source map version is unsupported
   */
  constructor(json: string);

  /**
   * Look up the original source position for a generated position.
   *
   * Uses greatest-lower-bound search (finds the closest mapping at or before the column).
   *
   * @param line - 0-based generated line
   * @param column - 0-based generated column
   * @returns The original position, or `null` if no mapping exists
   */
  originalPositionFor(line: number, column: number): OriginalPosition | null;

  /**
   * Look up the original source position with a search bias.
   *
   * @param line - 0-based generated line
   * @param column - 0-based generated column
   * @param bias - `0` for greatest-lower-bound (default), `-1` for least-upper-bound
   * @returns The original position, or `null` if no mapping exists
   */
  originalPositionForWithBias(line: number, column: number, bias: 0 | -1): OriginalPosition | null;

  /**
   * Look up the generated position for an original source position.
   *
   * Uses least-upper-bound search (finds the first mapping at or after the position).
   *
   * @param source - Original source filename (e.g. `"src/app.ts"`)
   * @param line - 0-based original line
   * @param column - 0-based original column
   * @returns The generated position, or `null` if no mapping exists
   */
  generatedPositionFor(source: string, line: number, column: number): GeneratedPosition | null;

  /**
   * Look up the generated position with a search bias.
   *
   * @param source - Original source filename
   * @param line - 0-based original line
   * @param column - 0-based original column
   * @param bias - `0` for default, `-1` for least-upper-bound, `1` for greatest-lower-bound
   * @returns The generated position, or `null` if no mapping exists
   */
  generatedPositionForWithBias(
    source: string,
    line: number,
    column: number,
    bias: -1 | 0 | 1,
  ): GeneratedPosition | null;

  /**
   * Batch forward lookup for multiple generated positions.
   *
   * Amortizes NAPI overhead by processing all positions in a single call.
   * Input is a flat array of `[line0, col0, line1, col1, ...]` pairs.
   * Output is a flat array of `[srcIdx0, line0, col0, nameIdx0, ...]` quads.
   * `-1` indicates no mapping found or no name.
   *
   * @param positions - Flat array of 0-based `[line, column]` pairs
   * @returns Flat array of `[sourceIndex, line, column, nameIndex]` quads
   *
   * @example
   * ```ts
   * const positions = [42, 10, 43, 0, 44, 5]
   * const results = sm.originalPositionsFor(positions)
   * // results: [srcIdx, line, col, nameIdx, srcIdx, line, col, nameIdx, ...]
   * const source = results[0] >= 0 ? sm.source(results[0]) : null
   * ```
   */
  originalPositionsFor(positions: number[]): number[];

  /**
   * Resolve a source index to a source filename.
   *
   * @param index - Source index from a lookup result
   * @returns The source filename
   */
  source(index: number): string;

  /**
   * Resolve a name index to a name string.
   *
   * @param index - Name index from a lookup result
   * @returns The name string
   */
  name(index: number): string;

  /** All source filenames in the source map. */
  readonly sources: string[];

  /** All names in the source map. */
  readonly names: string[];

  /** Debug ID (UUID) for associating generated files with source maps (ECMA-426). */
  readonly debugId: string | null;

  /** Total number of decoded mappings. */
  readonly mappingCount: number;

  /** Number of generated lines covered by mappings. */
  readonly lineCount: number;

  /** Whether the source map contains range mappings. */
  readonly hasRangeMappings: boolean;

  /** Number of range mappings in the source map. */
  readonly rangeMappingCount: number;

  /** Get the encoded range mappings string, or null if none. */
  encodedRangeMappings(): string | null;
}

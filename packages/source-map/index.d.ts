/**
 * @srcmap/source-map — drop-in replacement for Mozilla's source-map v0.6 API
 * powered by Rust via WebAssembly.
 */

// ── Constants (match Mozilla source-map v0.6) ───────────────────

export declare const GREATEST_LOWER_BOUND: 1
export declare const LEAST_UPPER_BOUND: 2

// ── Position types ──────────────────────────────────────────────

export interface Position {
  line: number
  column: number
}

export interface MappingItem {
  generatedLine: number
  generatedColumn: number
  source: string | null
  originalLine: number | null
  originalColumn: number | null
  name: string | null
  lastGeneratedColumn: number | null
}

export interface NullableMappedPosition {
  source: string | null
  line: number | null
  column: number | null
  name: string | null
}

export interface NullablePosition {
  line: number | null
  column: number | null
  lastColumn: number | null
}

// ── SourceMapConsumer ───────────────────────────────────────────

export declare class SourceMapConsumer {
  constructor(
    rawSourceMap: string | RawSourceMap,
    sourceMapUrl?: string,
  )

  /** The generated file this source map is associated with. */
  file: string | null

  /** The sourceRoot prefix for all sources. */
  sourceRoot: string | null

  /** Resolved source file URLs. */
  readonly sources: string[]

  /** Inline source contents (parallel to sources). */
  sourcesContent: (string | null)[] | null

  /**
   * Look up the original position for a generated position.
   * Lines are 1-based, columns are 0-based.
   */
  originalPositionFor(needle: {
    line: number
    column: number
    bias?: typeof GREATEST_LOWER_BOUND | typeof LEAST_UPPER_BOUND
  }): NullableMappedPosition

  /**
   * Look up the generated position for an original source position.
   * Lines are 1-based, columns are 0-based.
   */
  generatedPositionFor(needle: {
    source: string
    line: number
    column: number
    bias?: typeof GREATEST_LOWER_BOUND | typeof LEAST_UPPER_BOUND
  }): NullablePosition

  /**
   * Iterate all mappings in generated position order.
   */
  eachMapping(
    callback: (mapping: MappingItem) => void,
    context?: unknown,
    order?: number,
  ): void

  /**
   * Get source content for a source file.
   */
  sourceContentFor(source: string): string | null

  /**
   * Free WASM resources.
   */
  destroy(): void
}

// ── SourceMapGenerator ──────────────────────────────────────────

export declare class SourceMapGenerator {
  constructor(opts?: { file?: string; sourceRoot?: string })

  /**
   * Add a mapping (Mozilla v0.6 object-based API).
   */
  addMapping(mapping: {
    generated: Position
    original?: Position | null
    source?: string | null
    name?: string | null
  }): void

  /**
   * Set source content for a source file.
   */
  setSourceContent(source: string, content: string | null): void

  /**
   * Returns the source map as a parsed object.
   */
  toJSON(): RawSourceMap

  /**
   * Returns the source map as a JSON string.
   */
  toString(): string

  /**
   * Apply a source map from a consumer to this generator.
   */
  applySourceMap(
    consumer: SourceMapConsumer,
    sourceFile?: string,
    sourceMapPath?: string,
  ): void

  /**
   * Free WASM resources.
   */
  destroy(): void
}

// ── Raw source map type ─────────────────────────────────────────

export interface RawSourceMap {
  version: number
  file?: string
  sourceRoot?: string
  sources: string[]
  sourcesContent?: (string | null)[]
  names: string[]
  mappings: string
}

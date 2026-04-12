/**
 * @srcmap/gen-mapping — drop-in replacement for @jridgewell/gen-mapping
 * powered by Rust via WebAssembly.
 */

// ── Segment types ────────────────────────────────────────────────

/** A segment with only a generated column (unmapped). */
export type UnmappedSegment = [generatedColumn: number];

/** A segment with source location but no name. */
export type MappedSegment = [
  generatedColumn: number,
  sourcesIndex: number,
  sourceLine: number,
  sourceColumn: number,
];

/** A segment with source location and name. */
export type NamedMappedSegment = [
  generatedColumn: number,
  sourcesIndex: number,
  sourceLine: number,
  sourceColumn: number,
  namesIndex: number,
];

export type SourceMapSegment = UnmappedSegment | MappedSegment | NamedMappedSegment;

// ── Source map types ─────────────────────────────────────────────

export interface SourceMapV3 {
  version: 3;
  file?: string | undefined;
  sourceRoot?: string | undefined;
  sources: string[];
  sourcesContent?: (string | null)[];
  names: string[];
  ignoreList?: number[];
}

export interface EncodedSourceMap extends SourceMapV3 {
  mappings: string;
}

export interface DecodedSourceMap extends SourceMapV3 {
  mappings: SourceMapSegment[][];
}

// ── Position types ───────────────────────────────────────────────

export interface Pos {
  line: number;
  column: number;
}

// ── Mapping type ─────────────────────────────────────────────────

export type Mapping =
  | {
      generated: Pos;
      source: undefined;
      original: undefined;
      name: undefined;
    }
  | {
      generated: Pos;
      source: string;
      original: Pos;
      name: string;
    }
  | {
      generated: Pos;
      source: string;
      original: Pos;
      name: undefined;
    };

// ── Options ──────────────────────────────────────────────────────

export interface Options {
  file?: string | null;
  sourceRoot?: string | null;
}

// ── Mapping input ────────────────────────────────────────────────

export interface MappingInput {
  generated: Pos;
  source?: string;
  original?: Pos;
  name?: string;
  content?: string | null;
}

// ── GenMapping class ─────────────────────────────────────────────

export declare class GenMapping {
  constructor(opts?: Options);

  file: string | undefined;
  sourceRoot: string | undefined;

  free(): void;
  [Symbol.dispose](): void;
}

// ── Free functions ───────────────────────────────────────────────

/**
 * Add a mapping to the source map.
 * Lines are 1-based, columns are 0-based.
 */
export declare function addMapping(map: GenMapping, mapping: MappingInput): void;

/**
 * Add a mapping only if it differs from the previous mapping on the same line.
 * Requires mappings to be added in order.
 */
export declare function maybeAddMapping(map: GenMapping, mapping: MappingInput): void;

/**
 * Set the source content for a source file by source name.
 */
export declare function setSourceContent(
  map: GenMapping,
  source: string,
  content: string | null,
): void;

/**
 * Mark a source as ignored (or not).
 */
export declare function setIgnore(map: GenMapping, source: string, ignore?: boolean): void;

/**
 * Return all mappings as an array of Mapping objects.
 * Lines are 1-based, columns are 0-based.
 */
export declare function allMappings(map: GenMapping): Mapping[];

/**
 * Return the source map as an encoded source map object (with VLQ string mappings).
 */
export declare function toEncodedMap(map: GenMapping): EncodedSourceMap;

/**
 * Return the source map as a decoded source map object (with decoded mappings array).
 */
export declare function toDecodedMap(map: GenMapping): DecodedSourceMap;

/**
 * Construct a GenMapping from an existing source map input.
 */
export declare function fromMap(input: EncodedSourceMap | DecodedSourceMap | string): GenMapping;

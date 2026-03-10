export interface OriginalPosition {
  source: string | null
  line: number
  column: number
  name: string | null
}

export interface GeneratedPosition {
  line: number
  column: number
}

export declare class SourceMap {
  /** Parse a source map from a JSON string. Supports regular and indexed (sectioned) maps. */
  constructor(json: string)

  /** Look up the original source position for a generated position. 0-based line and column. */
  originalPositionFor(line: number, column: number): OriginalPosition | null

  /** Look up the generated position for an original source position. 0-based line and column. */
  generatedPositionFor(source: string, line: number, column: number): GeneratedPosition | null

  /**
   * Batch lookup: find original positions for multiple generated positions.
   * Takes a flat array [line0, col0, line1, col1, ...].
   * Returns a flat array [srcIdx0, line0, col0, nameIdx0, srcIdx1, ...].
   * -1 means no mapping found / no name.
   */
  originalPositionsFor(positions: number[]): number[]

  /** Source file paths. */
  get sources(): string[]

  /** Name identifiers. */
  get names(): string[]

  /** Total number of decoded mapping segments. */
  get mappingCount(): number

  /** Number of generated lines. */
  get lineCount(): number
}

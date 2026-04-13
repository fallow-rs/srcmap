export type GeneratedPosition = {
  line: number;
  column: number;
};

export type SourceMapLike = {
  originalPositionFor(line: number, column: number): unknown;
  originalPositionForWithBias?(line: number, column: number, bias: number): unknown;
  originalPositionsFor?(positions: Int32Array | number[]): Int32Array | number[];
};

export type BatchPositions = Int32Array | number[];

export class GeneratedOffsetLookup {
  constructor(code: string);
  get lineCount(): number;
  get totalBytes(): number;
  generatedPositionFor(offset: number): GeneratedPosition;
  generatedPositionsFor(offsets: ArrayLike<number>): Int32Array;
  originalPositionFor(sourceMap: SourceMapLike, offset: number, bias?: number): unknown;
  originalPositionsFor(sourceMap: SourceMapLike, offsets: ArrayLike<number>): BatchPositions;
}

export function generatedPositionForOffset(code: string, offset: number): GeneratedPosition;
export function originalPositionForOffset(
  sourceMap: SourceMapLike,
  code: string,
  offset: number,
  bias?: number,
): unknown;
export function originalPositionsForOffsets(
  sourceMap: SourceMapLike,
  code: string,
  offsets: ArrayLike<number>,
): BatchPositions;

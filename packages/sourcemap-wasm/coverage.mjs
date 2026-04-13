const NEWLINE_LF = 0x0a;
const NEWLINE_CR = 0x0d;

function assertInteger(value, name) {
  if (!Number.isInteger(value)) {
    throw new TypeError(`${name} must be an integer`);
  }
}

function utf8ByteLength(codePoint) {
  if (codePoint <= 0x7f) return 1;
  if (codePoint <= 0x7ff) return 2;
  if (codePoint <= 0xffff) return 3;
  return 4;
}

function codePointWidth(codePoint) {
  return codePoint > 0xffff ? 2 : 1;
}

function binarySearchFloor(values, needle) {
  let low = 0;
  let high = values.length - 1;

  while (low <= high) {
    const mid = low + ((high - low) >> 1);
    const value = values[mid];
    if (value === needle) return mid;
    if (value < needle) low = mid + 1;
    else high = mid - 1;
  }

  return Math.max(0, high);
}

function binarySearchFloorFrom(values, needle, low) {
  let high = values.length - 1;

  while (low <= high) {
    const mid = low + ((high - low) >> 1);
    const value = values[mid];
    if (value === needle) return mid;
    if (value < needle) low = mid + 1;
    else high = mid - 1;
  }

  return Math.max(0, high);
}

function toLength(value) {
  if (ArrayBuffer.isView(value)) return value.length;
  if (Array.isArray(value)) return value.length;
  throw new TypeError("offsets must be an array or typed array");
}

function getOffsetValue(offsets, index) {
  return offsets[index];
}

export class GeneratedOffsetLookup {
  #code;
  #lineAsciiOnly;
  #lineEndBytes;
  #lineStartBytes;
  #lineStartIndices;
  #napiBatchPositions;
  #napiBatchPositionsLength;
  #totalBytes;
  #wasmBatchPositions;
  #wasmBatchPositionsLength;

  constructor(code) {
    if (typeof code !== "string") {
      throw new TypeError("generated code must be a string");
    }

    this.#code = code;

    const lineStartBytes = [0];
    const lineEndBytes = [];
    const lineStartIndices = [0];
    const lineAsciiOnly = [];

    let byteOffset = 0;
    let codeUnitIndex = 0;
    let currentLineAsciiOnly = true;

    while (codeUnitIndex < code.length) {
      const codePoint = code.codePointAt(codeUnitIndex);
      const width = codePointWidth(codePoint);

      if (codePoint === NEWLINE_LF) {
        lineEndBytes.push(byteOffset);
        lineAsciiOnly.push(currentLineAsciiOnly ? 1 : 0);
        byteOffset += 1;
        codeUnitIndex += 1;
        lineStartBytes.push(byteOffset);
        lineStartIndices.push(codeUnitIndex);
        currentLineAsciiOnly = true;
        continue;
      }

      if (codePoint === NEWLINE_CR) {
        lineEndBytes.push(byteOffset);
        lineAsciiOnly.push(currentLineAsciiOnly ? 1 : 0);
        const next = code.charCodeAt(codeUnitIndex + 1);
        if (next === NEWLINE_LF) {
          byteOffset += 2;
          codeUnitIndex += 2;
        } else {
          byteOffset += 1;
          codeUnitIndex += 1;
        }
        lineStartBytes.push(byteOffset);
        lineStartIndices.push(codeUnitIndex);
        currentLineAsciiOnly = true;
        continue;
      }

      if (codePoint > 0x7f) {
        currentLineAsciiOnly = false;
      }

      byteOffset += utf8ByteLength(codePoint);
      codeUnitIndex += width;
    }

    lineEndBytes.push(byteOffset);
    lineAsciiOnly.push(currentLineAsciiOnly ? 1 : 0);

    this.#lineAsciiOnly = Uint8Array.from(lineAsciiOnly);
    this.#lineEndBytes = Uint32Array.from(lineEndBytes);
    this.#lineStartBytes = Uint32Array.from(lineStartBytes);
    this.#lineStartIndices = Uint32Array.from(lineStartIndices);
    this.#napiBatchPositions = null;
    this.#napiBatchPositionsLength = 0;
    this.#totalBytes = byteOffset;
    this.#wasmBatchPositions = null;
    this.#wasmBatchPositionsLength = 0;
  }

  get lineCount() {
    return this.#lineStartBytes.length;
  }

  get totalBytes() {
    return this.#totalBytes;
  }

  generatedPositionFor(offset) {
    assertInteger(offset, "offset");

    if (offset < 0 || offset > this.#totalBytes) {
      throw new RangeError(`offset ${offset} is outside generated code bounds`);
    }

    const line = binarySearchFloor(this.#lineStartBytes, offset);
    const lineStartByte = this.#lineStartBytes[line];
    const lineEndByte = this.#lineEndBytes[line];

    if (this.#lineAsciiOnly[line]) {
      return {
        line,
        column: offset <= lineEndByte ? offset - lineStartByte : lineEndByte - lineStartByte,
      };
    }

    const lineStartIndex = this.#lineStartIndices[line];
    const lineEndIndex =
      line + 1 < this.#lineStartIndices.length
        ? this.#lineStartIndices[line + 1]
        : this.#code.length;

    let byteCursor = lineStartByte;
    let column = 0;
    let codeUnitIndex = lineStartIndex;

    while (codeUnitIndex < lineEndIndex && byteCursor < offset) {
      const codePoint = this.#code.codePointAt(codeUnitIndex);
      const width = codePointWidth(codePoint);

      if (codePoint === NEWLINE_LF || codePoint === NEWLINE_CR) {
        return { line, column };
      }

      const charBytes = utf8ByteLength(codePoint);
      if (byteCursor + charBytes > offset) {
        throw new RangeError(`offset ${offset} falls in the middle of a UTF-8 code point`);
      }

      byteCursor += charBytes;
      column += width;
      codeUnitIndex += width;
    }

    return { line, column };
  }

  generatedPositionsFor(offsets) {
    const length = toLength(offsets);
    const positions = new Int32Array(length * 2);

    this.#fillBatchPositions(offsets, positions);

    return positions;
  }

  originalPositionFor(sourceMap, offset, bias = 0) {
    const { line, column } = this.generatedPositionFor(offset);
    if (typeof sourceMap?.originalPositionForWithBias === "function") {
      return sourceMap.originalPositionForWithBias(line, column, bias);
    }
    if (bias !== 0) {
      throw new TypeError("sourceMap does not support biased lookups");
    }
    return sourceMap.originalPositionFor(line, column);
  }

  originalPositionsFor(sourceMap, offsets) {
    if (typeof sourceMap?.originalPositionsFor !== "function") {
      throw new TypeError("sourceMap must expose originalPositionsFor");
    }

    const length = toLength(offsets);
    const positions = this.#getReusableBatchPositions(sourceMap, length);

    this.#fillBatchPositions(offsets, positions);

    return sourceMap.originalPositionsFor(positions);
  }

  #fillBatchPositions(offsets, positions) {
    const length = toLength(offsets);
    let previousOffset = -1;
    let previousLine = 0;

    for (let i = 0; i < length; i++) {
      const offset = getOffsetValue(offsets, i);
      assertInteger(offset, "offset");

      if (offset < 0 || offset > this.#totalBytes) {
        throw new RangeError(`offset ${offset} is outside generated code bounds`);
      }

      const line =
        offset >= previousOffset
          ? binarySearchFloorFrom(this.#lineStartBytes, offset, previousLine)
          : binarySearchFloor(this.#lineStartBytes, offset);
      const lineStartByte = this.#lineStartBytes[line];
      const lineEndByte = this.#lineEndBytes[line];
      const base = i * 2;

      previousOffset = offset;
      previousLine = line;

      positions[base] = line;
      if (this.#lineAsciiOnly[line]) {
        positions[base + 1] =
          offset <= lineEndByte ? offset - lineStartByte : lineEndByte - lineStartByte;
        continue;
      }

      const lineStartIndex = this.#lineStartIndices[line];
      const lineEndIndex =
        line + 1 < this.#lineStartIndices.length
          ? this.#lineStartIndices[line + 1]
          : this.#code.length;

      let byteCursor = lineStartByte;
      let column = 0;
      let codeUnitIndex = lineStartIndex;

      while (codeUnitIndex < lineEndIndex && byteCursor < offset) {
        const codePoint = this.#code.codePointAt(codeUnitIndex);
        const width = codePointWidth(codePoint);

        if (codePoint === NEWLINE_LF || codePoint === NEWLINE_CR) {
          break;
        }

        const charBytes = utf8ByteLength(codePoint);
        if (byteCursor + charBytes > offset) {
          throw new RangeError(`offset ${offset} falls in the middle of a UTF-8 code point`);
        }

        byteCursor += charBytes;
        column += width;
        codeUnitIndex += width;
      }

      positions[base + 1] = column;
    }
  }

  #getReusableBatchPositions(sourceMap, length) {
    const positionCount = length * 2;

    if (typeof sourceMap?.free === "function") {
      if (this.#wasmBatchPositions === null || this.#wasmBatchPositionsLength < positionCount) {
        this.#wasmBatchPositions = new Int32Array(positionCount);
        this.#wasmBatchPositionsLength = positionCount;
      }

      return this.#wasmBatchPositions.subarray(0, positionCount);
    }

    if (this.#napiBatchPositions === null || this.#napiBatchPositionsLength < positionCount) {
      this.#napiBatchPositions = new Array(positionCount);
      this.#napiBatchPositionsLength = positionCount;
    } else {
      this.#napiBatchPositions.length = positionCount;
    }

    return this.#napiBatchPositions;
  }
}

export function generatedPositionForOffset(code, offset) {
  return new GeneratedOffsetLookup(code).generatedPositionFor(offset);
}

export function originalPositionForOffset(sourceMap, code, offset, bias = 0) {
  return new GeneratedOffsetLookup(code).originalPositionFor(sourceMap, offset, bias);
}

export function originalPositionsForOffsets(sourceMap, code, offsets) {
  return new GeneratedOffsetLookup(code).originalPositionsFor(sourceMap, offsets);
}

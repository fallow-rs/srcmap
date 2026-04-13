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
  #lineStartBytes;
  #lineStartIndices;
  #totalBytes;

  constructor(code) {
    if (typeof code !== "string") {
      throw new TypeError("generated code must be a string");
    }

    this.#code = code;

    const lineStartBytes = [0];
    const lineStartIndices = [0];

    let byteOffset = 0;
    let codeUnitIndex = 0;

    while (codeUnitIndex < code.length) {
      const codePoint = code.codePointAt(codeUnitIndex);
      const width = codePointWidth(codePoint);

      if (codePoint === NEWLINE_LF) {
        byteOffset += 1;
        codeUnitIndex += 1;
        lineStartBytes.push(byteOffset);
        lineStartIndices.push(codeUnitIndex);
        continue;
      }

      if (codePoint === NEWLINE_CR) {
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
        continue;
      }

      byteOffset += utf8ByteLength(codePoint);
      codeUnitIndex += width;
    }

    this.#lineStartBytes = Uint32Array.from(lineStartBytes);
    this.#lineStartIndices = Uint32Array.from(lineStartIndices);
    this.#totalBytes = byteOffset;
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

    for (let i = 0; i < length; i++) {
      const { line, column } = this.generatedPositionFor(getOffsetValue(offsets, i));
      const base = i * 2;
      positions[base] = line;
      positions[base + 1] = column;
    }

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
    return sourceMap.originalPositionsFor(Array.from(this.generatedPositionsFor(offsets)));
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

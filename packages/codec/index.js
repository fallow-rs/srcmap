/* auto-generated NAPI-RS loader */
const { existsSync, readFileSync } = require('fs')
const { join } = require('path')
const { platform, arch } = process

let nativeBinding = null

const triples = {
  'darwin-arm64': 'srcmap-codec.darwin-arm64.node',
  'darwin-x64': 'srcmap-codec.darwin-x64.node',
  'linux-x64-gnu': 'srcmap-codec.linux-x64-gnu.node',
  'linux-x64-musl': 'srcmap-codec.linux-x64-musl.node',
  'linux-arm64-gnu': 'srcmap-codec.linux-arm64-gnu.node',
  'linux-arm64-musl': 'srcmap-codec.linux-arm64-musl.node',
  'win32-x64-msvc': 'srcmap-codec.win32-x64-msvc.node',
}

function getTripleKey() {
  const platformArch = `${platform}-${arch}`
  if (platform === 'linux') {
    const { familySync } = require('detect-libc')
    const libc = familySync() === 'musl' ? 'musl' : 'gnu'
    return `${platformArch}-${libc}`
  }
  return platformArch
}

const tripleKey = getTripleKey()
const bindingFile = triples[tripleKey]

if (bindingFile) {
  const bindingPath = join(__dirname, bindingFile)
  if (existsSync(bindingPath)) {
    nativeBinding = require(bindingPath)
  } else {
    try {
      nativeBinding = require(`@srcmap/codec-${tripleKey}`)
    } catch {
      throw new Error(`Failed to load native binding for ${tripleKey}. File not found: ${bindingPath}`)
    }
  }
} else {
  throw new Error(`Unsupported platform: ${tripleKey}`)
}

module.exports.decode = nativeBinding.decode
module.exports.encode = nativeBinding.encode

// Experimental: JSON string approach
module.exports.decodeJson = function decodeJson(mappings) {
  return JSON.parse(nativeBinding.decodeJson(mappings))
}
module.exports.encodeJson = function encodeJson(mappings) {
  return nativeBinding.encodeJson(JSON.stringify(mappings))
}

// Experimental: Packed buffer approach
module.exports.decodeBuf = function decodeBuf(mappings) {
  const buf = nativeBinding.decodeBuf(mappings)
  const i32 = new Int32Array(buf.buffer, buf.byteOffset, buf.byteLength >> 2)
  let pos = 0

  const nLines = i32[pos++]
  const lineSegCounts = new Array(nLines)
  for (let i = 0; i < nLines; i++) {
    lineSegCounts[i] = i32[pos++]
  }

  const result = new Array(nLines)
  for (let i = 0; i < nLines; i++) {
    const nSegs = lineSegCounts[i]
    const line = new Array(nSegs)
    for (let j = 0; j < nSegs; j++) {
      const nFields = i32[pos++]
      const seg = new Array(nFields)
      for (let k = 0; k < nFields; k++) {
        seg[k] = i32[pos++]
      }
      line[j] = seg
    }
    result[i] = line
  }
  return result
}
module.exports.encodeBuf = function encodeBuf(mappings) {
  // Pack nested arrays into flat i32 buffer
  let totalInts = 1 + mappings.length // n_lines + seg counts
  for (const line of mappings) {
    for (const seg of line) {
      totalInts += 1 + seg.length // n_fields + values
    }
  }

  const buf = Buffer.alloc(totalInts * 4)
  const i32 = new Int32Array(buf.buffer, buf.byteOffset, totalInts)
  let pos = 0

  i32[pos++] = mappings.length
  for (const line of mappings) {
    i32[pos++] = line.length
  }
  for (const line of mappings) {
    for (const seg of line) {
      i32[pos++] = seg.length
      for (const val of seg) {
        i32[pos++] = val
      }
    }
  }

  return nativeBinding.encodeBuf(buf)
}

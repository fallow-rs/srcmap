const { existsSync } = require('fs')
const { join } = require('path')
const { platform, arch } = process

let nativeBinding = null

const triples = {
  'darwin-arm64': 'srcmap-sourcemap.darwin-arm64.node',
  'darwin-x64': 'srcmap-sourcemap.darwin-x64.node',
  'linux-x64-gnu': 'srcmap-sourcemap.linux-x64-gnu.node',
  'linux-x64-musl': 'srcmap-sourcemap.linux-x64-musl.node',
  'linux-arm64-gnu': 'srcmap-sourcemap.linux-arm64-gnu.node',
  'linux-arm64-musl': 'srcmap-sourcemap.linux-arm64-musl.node',
  'win32-x64-msvc': 'srcmap-sourcemap.win32-x64-msvc.node',
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
      nativeBinding = require(`@srcmap/sourcemap-${tripleKey}`)
    } catch {
      throw new Error(`Failed to load native binding for ${tripleKey}. File not found: ${bindingPath}`)
    }
  }
} else {
  throw new Error(`Unsupported platform: ${tripleKey}`)
}

module.exports.SourceMap = nativeBinding.SourceMap

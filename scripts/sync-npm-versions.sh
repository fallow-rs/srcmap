#!/usr/bin/env bash
# Sync npm package.json versions with the Rust workspace version.
# Called by cargo-release as a pre-release hook.
# Arguments: $1 = old version, $2 = new version
set -euo pipefail

VERSION="${2:-$1}"
ROOT="$(git rev-parse --show-toplevel)"

# Use node to safely update version fields in package.json files.
# When requested, also rewrite any internal @srcmap dependency versions so the
# published npm packages stay aligned with the workspace release version.
update_package_json() {
  local pkg="$1"
  local update_internal_deps="${2:-false}"

  node - "$pkg" "$VERSION" "$update_internal_deps" <<'NODE'
const fs = require('fs');

const [, , file, version, updateInternalDeps] = process.argv;
const pkg = JSON.parse(fs.readFileSync(file, 'utf8'));

pkg.version = version;

if (updateInternalDeps === 'true') {
  for (const section of [
    'dependencies',
    'optionalDependencies',
    'peerDependencies',
    'devDependencies',
  ]) {
    const deps = pkg[section];
    if (!deps) continue;

    for (const key of Object.keys(deps)) {
      if (key.startsWith('@srcmap/')) {
        deps[key] = version;
      }
    }
  }
}

fs.writeFileSync(file, JSON.stringify(pkg, null, 2) + '\n');
NODE
}

# Update all top-level npm package.json files.
# These are the releaseable JS/WASM packages plus their versioned optional deps.
for pkg in "$ROOT"/packages/*/package.json; do
  [ -f "$pkg" ] || continue
  update_internal_deps=true
  update_package_json "$pkg" "$update_internal_deps"
  echo "  Updated $(basename "$(dirname "$pkg")")/package.json → $VERSION"
done

# Update platform-specific npm package.json files used by the NAPI wrappers.
for pkg in "$ROOT"/packages/*/npm/*/package.json; do
  [ -f "$pkg" ] || continue
  update_package_json "$pkg"
done

echo "  Updated all platform package versions → $VERSION"

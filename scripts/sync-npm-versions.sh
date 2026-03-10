#!/usr/bin/env bash
# Sync npm package.json versions with the Rust workspace version.
# Called by cargo-release as a pre-release hook.
# Arguments: $1 = old version, $2 = new version
set -euo pipefail

VERSION="${2:-$1}"
ROOT="$(git rev-parse --show-toplevel)"

# Use node to safely update version fields in package.json files
update_version() {
  node -e "
    const fs = require('fs');
    const pkg = JSON.parse(fs.readFileSync('$1', 'utf8'));
    pkg.version = '$VERSION';
    fs.writeFileSync('$1', JSON.stringify(pkg, null, 2) + '\n');
  "
}

update_optional_deps() {
  node -e "
    const fs = require('fs');
    const pkg = JSON.parse(fs.readFileSync('$1', 'utf8'));
    pkg.version = '$VERSION';
    if (pkg.optionalDependencies) {
      for (const key of Object.keys(pkg.optionalDependencies)) {
        if (key.startsWith('@srcmap/')) {
          pkg.optionalDependencies[key] = '$VERSION';
        }
      }
    }
    fs.writeFileSync('$1', JSON.stringify(pkg, null, 2) + '\n');
  "
}

# Update main package.json files (version + optionalDependencies)
for pkg in "$ROOT"/packages/codec/package.json "$ROOT"/packages/sourcemap/package.json; do
  [ -f "$pkg" ] || continue
  update_optional_deps "$pkg"
  echo "  Updated $(basename "$(dirname "$pkg")")/package.json → $VERSION"
done

# Update sourcemap-wasm package.json (version only)
for pkg in "$ROOT"/packages/sourcemap-wasm/package.json; do
  [ -f "$pkg" ] || continue
  update_version "$pkg"
  echo "  Updated $(basename "$(dirname "$pkg")")/package.json → $VERSION"
done

# Update platform-specific npm package.json files
for pkg in "$ROOT"/packages/*/npm/*/package.json; do
  [ -f "$pkg" ] || continue
  update_version "$pkg"
done

echo "  Updated all platform package versions → $VERSION"

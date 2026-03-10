#!/usr/bin/env bash
# Sync npm package.json versions with the Rust workspace version.
# Called by cargo-release as a pre-release hook.
# Arguments: $1 = old version, $2 = new version
set -euo pipefail

VERSION="${2:-$1}"
ROOT="$(git rev-parse --show-toplevel)"

# Update main package.json files (top-level version only)
for pkg in "$ROOT"/packages/*/package.json; do
  sed -i '' '0,/"version": "[^"]*"/{s/"version": "[^"]*"/"version": "'"$VERSION"'"/;}' "$pkg"
  echo "  Updated $(basename "$(dirname "$pkg")")/package.json → $VERSION"
done

# Update platform-specific npm package.json files
for pkg in "$ROOT"/packages/*/npm/*/package.json; do
  [ -f "$pkg" ] || continue
  sed -i '' 's/"version": "[^"]*"/"version": "'"$VERSION"'"/' "$pkg"
done

# Update optionalDependencies versions in main package.json files
for pkg in "$ROOT"/packages/codec/package.json "$ROOT"/packages/sourcemap/package.json; do
  [ -f "$pkg" ] || continue
  sed -i '' 's/\("@srcmap\/[^"]*": "\)[^"]*"/\1'"$VERSION"'"/' "$pkg"
done

echo "  Updated all platform package versions → $VERSION"

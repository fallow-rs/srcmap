#!/usr/bin/env bash
# Sync npm package.json versions with the Rust workspace version.
# Called by cargo-release as a pre-release hook.
# Arguments: $1 = old version, $2 = new version, $5 = workspace root
set -euo pipefail

VERSION="${2:-$1}"
ROOT="$(git rev-parse --show-toplevel)"

for pkg in "$ROOT"/packages/*/package.json; do
  sed -i '' "s/\"version\": \"[^\"]*\"/\"version\": \"$VERSION\"/" "$pkg"
  echo "  Updated $(basename "$(dirname "$pkg")")/package.json → $VERSION"
done

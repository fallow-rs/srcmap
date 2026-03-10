#!/usr/bin/env bash
# Release script for srcmap
# Usage: ./scripts/release.sh [patch|minor|major]
#
# This script:
# 1. Bumps versions, commits, tags, pushes (cargo-release)
# 2. Publishes Rust crates to crates.io
# 3. Waits for CI to build cross-platform binaries
# 4. Downloads artifacts and publishes npm packages (with OTP)
set -euo pipefail

LEVEL="${1:-patch}"
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

# Preflight checks
command -v cargo-release >/dev/null 2>&1 || { echo "Install cargo-release: cargo install cargo-release"; exit 1; }
command -v gh >/dev/null 2>&1 || { echo "Install GitHub CLI: brew install gh"; exit 1; }
command -v napi >/dev/null 2>&1 || { echo "Install napi-rs CLI: npm install -g @napi-rs/cli"; exit 1; }

if [ -n "$(git status --porcelain)" ]; then
  echo "Error: working directory is not clean"
  git status --short
  exit 1
fi

echo "==> Releasing $LEVEL version bump"
echo ""

# Step 1: Bump versions, commit, tag, push
echo "==> Step 1: Bump versions and publish Rust crates"
cargo release "$LEVEL" --execute --no-confirm

# Get the new version from Cargo.toml
VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
TAG="v$VERSION"
echo "==> Released $TAG"

# Step 2: Publish to crates.io in dependency order
echo ""
echo "==> Step 2: Publishing to crates.io"
echo "  Publishing srcmap-codec..."
cargo publish -p srcmap-codec || echo "  Already published, skipping"
echo "  Waiting for crates.io index..."
sleep 30

echo "  Publishing srcmap-sourcemap..."
cargo publish -p srcmap-sourcemap || echo "  Already published, skipping"
echo "  Publishing srcmap-generator..."
cargo publish -p srcmap-generator || echo "  Already published, skipping"
echo "  Waiting for crates.io index..."
sleep 30

echo "  Publishing srcmap-remapping..."
cargo publish -p srcmap-remapping || echo "  Already published, skipping"

# Step 3: Wait for CI builds
echo ""
echo "==> Step 3: Waiting for CI to build cross-platform binaries..."

# Find the release workflow run for this tag
sleep 10
RUN_ID=$(gh run list --workflow=release.yml --limit 1 --json databaseId --jq '.[0].databaseId')
echo "  Workflow run: $RUN_ID"
echo "  https://github.com/BartWaardenburg/srcmap/actions/runs/$RUN_ID"

while true; do
  STATUS=$(gh run view "$RUN_ID" --json status --jq '.status')
  if [ "$STATUS" = "completed" ]; then
    CONCLUSION=$(gh run view "$RUN_ID" --json conclusion --jq '.conclusion')
    if [ "$CONCLUSION" = "success" ]; then
      echo "  CI builds completed successfully!"
      break
    else
      echo "  CI builds failed. Check: https://github.com/BartWaardenburg/srcmap/actions/runs/$RUN_ID"
      echo "  Fix the issue, then rerun with: gh run rerun $RUN_ID"
      echo "  After CI passes, run: ./scripts/publish-npm.sh $VERSION"
      exit 1
    fi
  fi
  echo "  Still building... ($(date +%H:%M:%S))"
  sleep 30
done

# Step 4: Download artifacts
echo ""
echo "==> Step 4: Downloading build artifacts..."
ARTIFACTS_DIR="$ROOT/.artifacts"
rm -rf "$ARTIFACTS_DIR"
mkdir -p "$ARTIFACTS_DIR"

gh run download "$RUN_ID" --dir "$ARTIFACTS_DIR"
echo "  Downloaded to $ARTIFACTS_DIR"

# Move NAPI artifacts into npm directories
echo "  Moving codec artifacts..."
mkdir -p packages/codec/artifacts
for dir in "$ARTIFACTS_DIR"/codec-bindings-*/; do
  cp "$dir"/*.node packages/codec/artifacts/ 2>/dev/null || true
done
cd packages/codec
napi artifacts -d artifacts
cd "$ROOT"

echo "  Moving sourcemap artifacts..."
mkdir -p packages/sourcemap/artifacts
for dir in "$ARTIFACTS_DIR"/sourcemap-bindings-*/; do
  cp "$dir"/*.node packages/sourcemap/artifacts/ 2>/dev/null || true
done
cd packages/sourcemap
napi artifacts -d artifacts
cd "$ROOT"

# Copy WASM package
echo "  Copying WASM package..."
cp -r "$ARTIFACTS_DIR"/wasm-package/* packages/sourcemap-wasm/pkg/ 2>/dev/null || true

# Step 5: Publish to npm
echo ""
echo "==> Step 5: Publishing to npm"
echo "  Enter your npm OTP code:"
read -r OTP

echo "  Publishing @srcmap/codec platform packages..."
for dir in packages/codec/npm/*/; do
  name=$(node -p "require('./${dir}package.json').name")
  echo "    $name"
  npm publish "$dir" --access public --otp "$OTP" 2>/dev/null || echo "    Skipped (already published)"
done

echo "  Publishing @srcmap/codec..."
npm publish packages/codec --access public --ignore-scripts --otp "$OTP"

echo "  Publishing @srcmap/sourcemap platform packages..."
for dir in packages/sourcemap/npm/*/; do
  name=$(node -p "require('./${dir}package.json').name")
  echo "    $name"
  npm publish "$dir" --access public --otp "$OTP" 2>/dev/null || echo "    Skipped (already published)"
done

echo "  Publishing @srcmap/sourcemap..."
npm publish packages/sourcemap --access public --ignore-scripts --otp "$OTP"

echo "  Publishing @srcmap/sourcemap-wasm..."
npm publish packages/sourcemap-wasm --access public --ignore-scripts --otp "$OTP"

# Cleanup
rm -rf "$ARTIFACTS_DIR" packages/codec/artifacts packages/sourcemap/artifacts

echo ""
echo "==> Done! Released $TAG"
echo "  crates.io: https://crates.io/crates/srcmap-codec/$VERSION"
echo "  npm: https://www.npmjs.com/package/@srcmap/codec/v/$VERSION"
echo "  GitHub: https://github.com/BartWaardenburg/srcmap/releases/tag/$TAG"

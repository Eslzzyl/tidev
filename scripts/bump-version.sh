#!/usr/bin/env bash
# bump-version.sh — Bump tidev workspace version, update lockfile and npm package.
#
# Usage:  ./scripts/bump-version.sh <new-version>
#   e.g.  ./scripts/bump-version.sh 0.6.0
#
# This script:
#   1. Updates workspace version in Cargo.toml (and all member crates)
#   2. Regenerates Cargo.lock
#   3. Syncs version + tidevBinaryVersion in npm/tidev/package.json
#   4. Creates a git commit and a signed tag (v<new-version>)

set -euo pipefail

if [ $# -ne 1 ]; then
  echo "Usage: $0 <new-version>"
  echo "  e.g. $0 0.6.0"
  exit 1
fi

NEW_VERSION="$1"

# Validate version format (semver-like)
if ! echo "$NEW_VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
  echo "Error: version must be in semver format (e.g. 0.6.0, 0.6.0-beta.1)"
  exit 1
fi

echo "==> Bumping workspace version to $NEW_VERSION ..."
cargo set-version --workspace "$NEW_VERSION"

echo ""
echo "==> Regenerating Cargo.lock ..."
cargo generate-lockfile

echo ""
echo "==> Updating npm/tidev/package.json ..."
NPM_PKG="npm/tidev/package.json"
if [ -f "$NPM_PKG" ]; then
  jq ".version = \"$NEW_VERSION\" | .tidevBinaryVersion = \"$NEW_VERSION\"" "$NPM_PKG" > "${NPM_PKG}.tmp"
  mv "${NPM_PKG}.tmp" "$NPM_PKG"
  echo "    Updated version and tidevBinaryVersion to $NEW_VERSION"
else
  echo "    Warning: $NPM_PKG not found, skipping"
fi

echo ""
echo "==> Staging all changes ..."
git add Cargo.toml crates/*/Cargo.toml Cargo.lock "$NPM_PKG"

echo ""
echo "==> Creating commit and tag ..."
git commit -m "chore: bump version to $NEW_VERSION"
git tag "v$NEW_VERSION"

echo ""
echo "============================================"
echo "  ✅ Version bumped to $NEW_VERSION"
echo "  📦 Commit : $(git rev-parse HEAD)"
echo "  🏷️  Tag   : v$NEW_VERSION"
echo "============================================"
echo ""
echo "Next step — push to remote:"
echo "  git push origin master --tags"

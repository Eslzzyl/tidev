#!/usr/bin/env pwsh
# bump-version.ps1 — Bump tidev workspace version, update lockfile and npm package.
#
# Usage:  .\scripts\bump-version.ps1 <new-version>
#   e.g.  .\scripts\bump-version.ps1 0.7.0
#
# This script:
#   1. Updates workspace version in Cargo.toml (and all member crates)
#   2. Regenerates Cargo.lock
#   3. Syncs version + tidevBinaryVersion in npm/tidev/package.json
#   4. Creates a git commit and a tag (v<new-version>)
#
# Requirements: cargo-edit (for `cargo set-version`)

$ErrorActionPreference = 'Stop'

if ($args.Count -ne 1) {
    Write-Host "Usage: $($MyInvocation.MyCommand.Name) <new-version>"
    Write-Host "  e.g. $($MyInvocation.MyCommand.Name) 0.7.0"
    exit 1
}

$NewVersion = $args[0]

# Validate version format (semver-like)
if ($NewVersion -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$') {
    Write-Host "Error: version must be in semver format (e.g. 0.7.0, 0.7.0-beta.1)"
    exit 1
}

$RootDir = Split-Path -Parent $PSScriptRoot
$NpmPkg = Join-Path $RootDir 'npm/tidev/package.json'

Write-Host "==> Bumping workspace version to $NewVersion ..."
cargo set-version --workspace $NewVersion
if ($LASTEXITCODE -ne 0) { throw "cargo set-version failed (exit code $LASTEXITCODE)" }

Write-Host ""
Write-Host "==> Regenerating Cargo.lock ..."
cargo generate-lockfile
if ($LASTEXITCODE -ne 0) { throw "cargo generate-lockfile failed (exit code $LASTEXITCODE)" }

Write-Host ""
Write-Host "==> Updating npm/tidev/package.json ..."
if (Test-Path -Path $NpmPkg) {
    $Json = Get-Content -Raw -Path $NpmPkg
    $Json = $Json -replace '("version"\s*:\s*")[^"]*(")', "`${1}$NewVersion`${2}"
    $Json = $Json -replace '("tidevBinaryVersion"\s*:\s*")[^"]*(")', "`${1}$NewVersion`${2}"
    Set-Content -NoNewline -Path $NpmPkg -Value $Json -Encoding utf8
    Write-Host "    Updated version and tidevBinaryVersion to $NewVersion"
} else {
    Write-Host "    Warning: $NpmPkg not found, skipping"
}

Write-Host ""
Write-Host "==> Staging all changes ..."
git add Cargo.toml crates/*/Cargo.toml Cargo.lock $NpmPkg
if ($LASTEXITCODE -ne 0) { throw "git add failed (exit code $LASTEXITCODE)" }

Write-Host ""
Write-Host "==> Creating commit and tag ..."
git commit -m "chore: bump version to $NewVersion"
if ($LASTEXITCODE -ne 0) { throw "git commit failed (exit code $LASTEXITCODE)" }
git tag "v$NewVersion"
if ($LASTEXITCODE -ne 0) { throw "git tag failed (exit code $LASTEXITCODE)" }

$Head = git rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw "git rev-parse failed (exit code $LASTEXITCODE)" }

Write-Host ""
Write-Host "============================================"
Write-Host "  ✅ Version bumped to $NewVersion"
Write-Host "  📦 Commit : $Head"
Write-Host "  🏷️  Tag   : v$NewVersion"
Write-Host "============================================"
Write-Host ""
Write-Host "Next step — push to remote:"
Write-Host "  git push origin master --tags"

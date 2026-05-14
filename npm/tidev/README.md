# tidev

AI coding agent built in Rust — now available via npm!

```
npm install -g tidev
```

This npm package downloads the prebuilt tidev binary from the [GitHub Releases](https://github.com/Eslzzyl/tidev/releases) page and makes the `tidev` command available on your PATH.

## Usage

```bash
tidev [options]
```

See the [main project README](https://github.com/Eslzzyl/tidev#readme) for full documentation.

## Requirements

- Node.js 18 or later
- Linux x64/arm64, macOS x64/arm64, or Windows x64

## How it works

- `postinstall` downloads the correct binary for your platform.
- On subsequent runs, the cached binary is reused unless a newer version is published.
- If the download fails during `npm install`, it falls back silently — the binary is downloaded on first `tidev` invocation.

## Environment variables

| Variable | Purpose |
|----------|---------|
| `TIDEV_VERSION` | Override the binary version to download |
| `TIDEV_GITHUB_REPO` | Override the GitHub repository (e.g., `your-org/tidev`) |
| `TIDEV_RELEASE_BASE_URL` | Use a mirror URL for release assets |
| `TIDEV_DISABLE_INSTALL` | Set to `1` to skip binary download |

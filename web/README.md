# tidev Web

This directory is the standalone Vite frontend project. The current page is
only a shell; product UI will be added without changing the Rust integration
boundary.

## Development

Run `cargo run -- web` from the repository root, then open
`http://127.0.0.1:26502/`. The Rust server starts Vite, proxies the browser
requests to it, and exposes the backend under `/api`. Vite's `5173` port is an
internal development port and is not the normal tidev entrypoint.
When pnpm or the frontend dependencies are unavailable, the Rust server keeps
running and serves a diagnostic fallback page.

To work on the frontend directly:

```text
pnpm install --frozen-lockfile
pnpm dev
```

## Release builds

`cargo build --release` runs `pnpm install --frozen-lockfile` and `pnpm build`
from this directory. The generated `dist` files are compressed while being
embedded into the Rust binary, so users do not need a separate web directory.

The project intentionally does not pin a top-level `packageManager` version.
The `engines.pnpm` range documents the minimum supported pnpm version
(`>=11.0.0`) for package managers that validate engine ranges. The lockfile
continues to control dependency resolution during release builds.

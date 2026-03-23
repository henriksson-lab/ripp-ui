# RIPP (Rust Image Processing Platform)

This the user interface for RIPP. under development

- **Desktop** (`claude-ui`) — native window via [Slint](https://slint.dev/)
- **Server** (`server`) — headless render streamed as MJPEG over HTTP; any browser can view and interact with it

## Architecture

```
ui/app.slint          UI definition (shared by both binaries)
src/teapot.rs         wgpu offscreen teapot renderer (shared library)
src/main.rs           Desktop binary
src/bin/server.rs     Server binary
assets/index.html     Browser client for the server stream
```

Both binaries render from the same `ui/app.slint` definition. The desktop uses Slint natively; the server uses Slint's `MinimalSoftwareWindow` software renderer headlessly, encoding each frame as PNG and broadcasting it as `multipart/x-mixed-replace` MJPEG.

The teapot is rendered offscreen each frame by `TeapotRenderer` (wgpu, flat-shaded) and pushed into Slint as an image property. Slint handles layout, the menubar, dropdowns, and the About dialog.

The server renders only when at least one client is connected.

## Running

```bash
make local    # desktop app
make server   # web server → open http://127.0.0.1:8080
```

Both targets build in release mode. The server also accepts `--fps` to print per-step render timings to stdout once per second:

```bash
cargo run --release --bin server -- --fps
# fps=30  teapot=  6ms  slint=  4ms  expand= 0ms  png=  8ms  total= 18ms
```

## Server interaction

The browser page forwards mouse and keyboard events to the server via `POST /input`. The server dispatches them to Slint on the render thread, so the menubar, dropdowns, and About dialog work the same as in the desktop app.

The browser reports its viewport size on load and on resize via `GET /viewport?w=N&h=N`; the server adjusts the render resolution to match.

## Dependencies

| Crate | Purpose |
|---|---|
| `slint` | UI framework (desktop + headless software renderer) |
| `wgpu` | GPU teapot rendering |
| `actix-web` | HTTP server |
| `png` | Lossless frame encoding |
| `tokio` | Async runtime for the HTTP server thread |
| `mozjpeg` | — (removed; replaced by `png`) |

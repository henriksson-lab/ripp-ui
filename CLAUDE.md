# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
make local        # run the desktop app (Slint GUI)
make server       # run the web server (stream at http://127.0.0.1:8080)

cargo build       # build
cargo run                    # desktop app
cargo run -- --server        # web server mode
cargo run -- --server --fps  # web server with FPS counter
```

There are no tests.

## Architecture

**RIPP** (Rust Imaging Processing Platform) is a single binary (`ripp`) with two modes:

### `src/main.rs` — entry point (`ripp` binary)
Built with **Slint 1.x**. The UI is defined in `ui/app.slint`, compiled at build time by `build.rs` via `slint-build`, and accessed in Rust via `slint::include_modules!()`.

Layout: left pane split vertically — top half shows "Hello World" and live mouse coordinates (tracked via a `TouchArea`); bottom half is a scrollable `ListView` of the current directory populated at startup. Right pane shows a rotating teapot rendered offscreen by `TeapotRenderer` and pushed into Slint as a `SharedPixelBuffer<Rgba8Pixel>` every 16 ms via a `slint::Timer`.

Dropping `TeapotRenderer` hangs (wgpu cleanup deadlock), so both exit paths call `std::process::exit(0)`.

### `src/server.rs` — server mode (`--server` flag)
Uses **Slint's headless software renderer** (`MinimalSoftwareWindow` / `Rgb565Pixel`) on the main thread — the same `AppWindow` from `ui/app.slint` with `server_mode = true`. Each frame: renders the teapot via `TeapotRenderer`, pushes it into Slint as an image property, renders the full UI to an RGB565 buffer, expands to RGB888, encodes as PNG, and broadcasts over a `tokio::sync::broadcast` channel.

actix-web runs on a background thread. Endpoints: `/` (index.html), `/stream` (multipart PNG stream), `/viewport` (resize), `/ws` (WebSocket input). Browser mouse/keyboard events are forwarded to Slint via the WebSocket → `VecDeque<InputEvent>` → `window.dispatch_event()`.

The File menu shows "Shut down server" instead of "Quit" when `server_mode` is true.

### `src/teapot.rs`
Shared wgpu offscreen renderer. `TeapotRenderer::new(w, h)` initialises wgpu and loads `assets/teapot.obj`. `render_frame(rotation)` submits a render pass and does a blocking readback via `map_async` + `device.poll`, returning destrided `Vec<u8>` (RGBA8, `w*h*4` bytes).

### `ui/app.slint`
Single source of truth for the UI, used by both binaries. Key properties set from Rust: `teapot-image`, `server-mode`, `file-list`. Menubar dropdowns and About dialog are pure Slint using `private property <bool>` state and `z`-layered `if` elements (z: 1 dismiss overlay, z: 2 dropdowns, z: 10/11 About backdrop and dialog).

### `assets/`
- `teapot.obj` — mesh loaded at runtime
- `shaders/teapot.wgsl` — WGSL shader used by both binaries
- `index.html` — served by the web server; forwards input events over WebSocket

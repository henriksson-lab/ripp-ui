# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
make local        # run the desktop app (Slint GUI)
make server       # run the web server (MJPEG stream at http://127.0.0.1:8080)

cargo build --bin claude-ui   # build desktop app only
cargo build --bin server      # build web server only
cargo build                   # build both
```

There are no tests.

## Architecture

Two independent binaries share the `claude-ui` crate:

### `src/main.rs` — desktop app (`claude-ui` binary)
Built with **Slint 1.x**. Renders a two-pane layout: "Hello World" text on the left, a rotating teapot on the right.

The UI is defined in **`ui/app.slint`** and compiled into Rust by `build.rs` via `slint-build`. Rust code accesses the generated `AppWindow` struct via `slint::include_modules!()`.

The teapot is rendered offscreen with wgpu each frame (via `TeapotRenderer`) and pushed into Slint as a `slint::Image` (`SharedPixelBuffer<Rgba8Pixel>`) on a 16 ms timer.

Menubar dropdowns and the About modal are implemented inside the .slint file using `private property <bool>` state and conditional `if` elements with explicit `z` ordering:
- `z: 0` — main VerticalLayout
- `z: 1` — full-window dismiss `TouchArea` (shown when any dropdown is open)
- `z: 2` — dropdown `Rectangle` positioned at `y: 36px` (below menubar)
- `z: 10/11` — About backdrop and dialog

### `src/bin/server.rs` — web server (`server` binary)
Headless wgpu renderer (no Slint) that renders the teapot to an offscreen texture, encodes each frame as JPEG via mozjpeg, and streams them as `multipart/x-mixed-replace` MJPEG over HTTP (actix-web). Serves `assets/index.html` at `/` and the stream at `/stream?w=N&h=N`.

### `src/teapot.rs`
Standalone wgpu offscreen teapot renderer used by the desktop app. `TeapotRenderer::new(w, h)` initialises wgpu and loads `assets/teapot.obj`. `render_frame(rotation)` renders one frame synchronously and returns destrided RGBA8 bytes.

The server binary has its own inline copy of equivalent pipeline code.

### `ui/app.slint`
Slint UI definition for the desktop app. Compiled at build time by `build.rs`.

### `build.rs`
Compiles `ui/app.slint` → Rust via `slint_build::compile`.

### `assets/`
- `teapot.obj` — mesh loaded at runtime by both binaries
- `shaders/teapot.wgsl` — wgsl shader used by both binaries
- `index.html` — served by the web server; contains its own HTML/CSS/JS menubar mirroring the desktop app's

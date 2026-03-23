// Headless Slint renderer streamed as MJPEG over HTTP.
//
// Slint's software renderer runs on the main thread and broadcasts encoded JPEG frames.
// The actix-web HTTP server runs on a background thread and subscribes to those frames.
// Mouse and keyboard events POSTed to /input are forwarded to Slint each render frame.
//
// Run with:   cargo run --bin server
// Then open:  http://127.0.0.1:8080
slint::include_modules!();

use actix_web::{web, App, HttpResponse, HttpServer};
use bytes::Bytes;
use ripp::teapot::TeapotRenderer;
use ripp::camera::{start_camera_thread, CameraHandle, CameraImage, DeviceProp};
use futures::stream;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel};
use slint::platform::{WindowAdapter, WindowEvent};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

// Default render dimensions (used until the browser reports its viewport size).
const DEFAULT_W: u32 = 960;
const DEFAULT_H: u32 = 540;
// TeapotRenderer is created once at this fixed size; Slint scales the image in layout.
const TEAPOT_W: u32 = DEFAULT_W / 2;
const TEAPOT_H: u32 = DEFAULT_H - 36;

// ── File browser helper ───────────────────────────────────────────────────────

fn fmt_size(bytes: u64) -> String {
    if bytes < 1_000 {
        format!("{} B", bytes)
    } else if bytes < 1_000_000 {
        format!("{:.1} KB", bytes as f64 / 1_000.0)
    } else if bytes < 1_000_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    }
}

fn load_dir(path: &std::path::Path) -> Vec<FileEntry> {
    let mut entries: Vec<FileEntry> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let size = if is_dir {
                "—".into()
            } else {
                e.metadata().map(|m| fmt_size(m.len())).unwrap_or_default().into()
            };
            FileEntry {
                name: e.file_name().to_string_lossy().to_string().into(),
                is_dir,
                size,
            }
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    entries
}

// ── Headless Slint platform ──────────────────────────────────────────────────

struct HeadlessPlatform {
    window: Rc<MinimalSoftwareWindow>,
}

impl slint::platform::Platform for HeadlessPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
}

// ── Input events ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum InputEvent {
    PointerMoved     { x: f32, y: f32 },
    PointerPressed   { x: f32, y: f32, button: String },
    PointerReleased  { x: f32, y: f32, button: String },
    PointerScrolled  { x: f32, y: f32, delta_x: f32, delta_y: f32 },
    PointerExited,
    KeyPressed       { text: String },
    KeyReleased      { text: String },
}

type EventQueue = Arc<(Mutex<VecDeque<InputEvent>>, Condvar)>;

fn to_button(s: &str) -> slint::platform::PointerEventButton {
    match s {
        "left"   => slint::platform::PointerEventButton::Left,
        "right"  => slint::platform::PointerEventButton::Right,
        "middle" => slint::platform::PointerEventButton::Middle,
        _        => slint::platform::PointerEventButton::Other,
    }
}

// ── Session helpers ───────────────────────────────────────────────────────────

fn build_tree(session: &ripp::session::RippSession) -> slint::ModelRc<ProjectTreeEntry> {
    let entries: Vec<ProjectTreeEntry> = ripp::session::flatten_session(session)
        .into_iter()
        .map(|(label, indent, id)| ProjectTreeEntry {
            label: label.into(),
            indent,
            object_id: id as i32,
        })
        .collect();
    Rc::new(slint::VecModel::from(entries)).into()
}

// ── Render loop (main thread) ────────────────────────────────────────────────

fn run_render_loop(
    ui: AppWindow,
    window: Rc<MinimalSoftwareWindow>,
    frame_tx: Arc<tokio::sync::broadcast::Sender<Vec<u8>>>,
    viewport: Arc<Mutex<(u32, u32)>>,
    event_queue: EventQueue,
    camera: CameraHandle,
    show_fps: bool,
) {
    let session = Rc::new(std::cell::RefCell::new({
        let mut s = ripp::session::RippSession::new();
        s.add_project("Demo Project");
        s
    }));
    ui.set_project_tree(build_tree(&session.borrow()));

    ui.on_new_project({
        let session = session.clone();
        let ui_weak = ui.as_weak();
        move || {
            session.borrow_mut().add_project("New Project");
            if let Some(u) = ui_weak.upgrade() {
                u.set_project_tree(build_tree(&session.borrow()));
            }
        }
    });

    ui.on_project_tree_selected(|_object_id| {});
    ui.on_open_file(|_filename| {});

    ui.on_close_project({
        let session = session.clone();
        let ui_weak = ui.as_weak();
        move || {
            let proj_id = ui_weak.upgrade()
                .map(|u| u.get_selected_project_id())
                .unwrap_or(-1);
            if proj_id >= 0 {
                session.borrow_mut().projects.remove(&(proj_id as u32));
                if let Some(u) = ui_weak.upgrade() {
                    u.set_selected_project_id(-1);
                    u.set_project_tree(build_tree(&session.borrow()));
                }
            }
        }
    });

    let teapot = TeapotRenderer::new(TEAPOT_W, TEAPOT_H);
    let start  = Instant::now();
    let (mut w, mut h) = *viewport.lock().unwrap();
    let mut buf = vec![Rgb565Pixel::default(); (w * h) as usize];
    let mut last_print  = Instant::now();
    let mut frame_count = 0u32;

    // Pending camera image slot: snap threads write here; render loop drains it.
    // CameraImage is Send; conversion to slint::Image happens on the render thread.
    let pending_snap: Arc<Mutex<Option<CameraImage>>> = Arc::new(Mutex::new(None));

    // Pending device-props slot: move threads write here; render loop drains it.
    let pending_props: Arc<Mutex<Option<Vec<DeviceProp>>>> = Arc::new(Mutex::new(None));
    let props_refreshing = Arc::new(AtomicBool::new(false));

    ui.on_snap_requested({
        let camera = camera.clone();
        let slot = pending_snap.clone();
        move || {
            let camera = camera.clone();
            let slot = slot.clone();
            std::thread::spawn(move || {
                *slot.lock().unwrap() = Some(camera.snap());
            });
        }
    });

    let live_running = Arc::new(AtomicBool::new(false));
    ui.on_live_toggled({
        let camera = camera.clone();
        let slot = pending_snap.clone();
        let live_running = live_running.clone();
        move |enabled| {
            live_running.store(enabled, Ordering::SeqCst);
            if enabled {
                let camera = camera.clone();
                let slot = slot.clone();
                let live_running = live_running.clone();
                std::thread::spawn(move || {
                    while live_running.load(Ordering::SeqCst) {
                        *slot.lock().unwrap() = Some(camera.snap());
                    }
                });
            }
        }
    });

    ui.on_camera_panned({
        let camera = camera.clone();
        let slot = pending_props.clone();
        let refreshing = props_refreshing.clone();
        move |dx, dy| {
            camera.move_xy(dx as f64, dy as f64);
            if !refreshing.swap(true, Ordering::SeqCst) {
                let camera = camera.clone();
                let slot = slot.clone();
                let refreshing = refreshing.clone();
                std::thread::spawn(move || {
                    let props = camera.device_props();
                    refreshing.store(false, Ordering::SeqCst);
                    *slot.lock().unwrap() = Some(props);
                });
            }
        }
    });

    ui.on_camera_scrolled({
        let camera = camera.clone();
        let slot = pending_props.clone();
        let refreshing = props_refreshing.clone();
        move |delta| {
            camera.move_z(delta as f64);
            if !refreshing.swap(true, Ordering::SeqCst) {
                let camera = camera.clone();
                let slot = slot.clone();
                let refreshing = refreshing.clone();
                std::thread::spawn(move || {
                    let props = camera.device_props();
                    refreshing.store(false, Ordering::SeqCst);
                    *slot.lock().unwrap() = Some(props);
                });
            }
        }
    });

    loop {
        let deadline = Instant::now() + Duration::from_millis(33);

        if frame_tx.receiver_count() == 0 {
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
            continue;
        }

        // Wait until an input event arrives or the frame deadline is reached,
        // then drain all queued events. This means input wakes the render loop
        // immediately rather than waiting up to 33 ms.
        {
            let (lock, cvar) = event_queue.as_ref();
            let mut guard = lock.lock().unwrap();
            while guard.is_empty() {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() { break; }
                let (g, _) = cvar.wait_timeout(guard, remaining).unwrap();
                guard = g;
            }
            for ev in guard.drain(..) {
                let slint_ev = match ev {
                    InputEvent::PointerMoved { x, y } =>
                        WindowEvent::PointerMoved { position: slint::LogicalPosition::new(x, y) },
                    InputEvent::PointerPressed { x, y, button } =>
                        WindowEvent::PointerPressed {
                            position: slint::LogicalPosition::new(x, y),
                            button: to_button(&button),
                        },
                    InputEvent::PointerReleased { x, y, button } =>
                        WindowEvent::PointerReleased {
                            position: slint::LogicalPosition::new(x, y),
                            button: to_button(&button),
                        },
                    InputEvent::PointerScrolled { x, y, delta_x, delta_y } =>
                        WindowEvent::PointerScrolled {
                            position: slint::LogicalPosition::new(x, y),
                            delta_x,
                            delta_y,
                        },
                    InputEvent::PointerExited =>
                        WindowEvent::PointerExited,
                    InputEvent::KeyPressed  { text } =>
                        WindowEvent::KeyPressed  { text: text.into() },
                    InputEvent::KeyReleased { text } =>
                        WindowEvent::KeyReleased { text: text.into() },
                };
                window.window().dispatch_event(slint_ev);
            }
        }

        // Resize Slint window if the browser reported new dimensions.
        let (new_w, new_h) = *viewport.lock().unwrap();
        if new_w != w || new_h != h {
            w = new_w;
            h = new_h;
            window.set_size(slint::PhysicalSize::new(w, h));
            buf.resize((w * h) as usize, Rgb565Pixel::default());
        }

        if let Some(raw) = pending_snap.lock().unwrap().take() {
            ui.set_camera_image(raw.to_slint_image());
        }

        if let Some(props) = pending_props.lock().unwrap().take() {
            let rows: Vec<DevicePropEntry> = props.into_iter().map(|p| DevicePropEntry {
                device: p.device.into(), property: p.property.into(), value: p.value.into(),
            }).collect();
            ui.set_device_props(Rc::new(slint::VecModel::from(rows)).into());
        }

        slint::platform::update_timers_and_animations();

        let rotation = start.elapsed().as_secs_f32() * 0.8;
        let t0 = Instant::now();

        // 1. Render teapot → Slint image property (Slint scales it in layout).
        let pixels = teapot.render_frame(rotation);
        let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(TEAPOT_W, TEAPOT_H);
        pb.make_mut_bytes().copy_from_slice(&pixels);
        ui.set_teapot_image(slint::Image::from_rgba8(pb));
        let t1 = Instant::now();

        // 2. Render full Slint UI to RGB565 buffer.
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buf, w as usize);
        });
        let t2 = Instant::now();

        // 3. Expand RGB565 → RGB888 for PNG encoding.
        let rgb: Vec<u8> = buf
            .iter()
            .flat_map(|p| {
                let raw = p.0;
                let r = ((raw & 0xF800) >> 8) as u8;
                let g = ((raw & 0x07E0) >> 3) as u8;
                let b = ((raw & 0x001F) << 3) as u8;
                [r, g, b]
            })
            .collect();
        let t3 = Instant::now();

        let frame = encode_png(&rgb, w, h);
        let t4 = Instant::now();

        let _ = frame_tx.send(frame);

        if show_fps {
            frame_count += 1;
            if t4.duration_since(last_print) >= Duration::from_secs(1) {
                println!(
                    "fps={:2}  teapot={:4}ms  slint={:4}ms  expand={:3}ms  png={:4}ms  total={:4}ms",
                    frame_count,
                    t1.duration_since(t0).as_millis(),
                    t2.duration_since(t1).as_millis(),
                    t3.duration_since(t2).as_millis(),
                    t4.duration_since(t3).as_millis(),
                    t4.duration_since(t0).as_millis(),
                );
                frame_count = 0;
                last_print  = t4;
            }
        }

        // Sleep until the frame deadline or the next input event.
        let remaining = deadline.saturating_duration_since(Instant::now());
        if !remaining.is_zero() {
            let (lock, cvar) = event_queue.as_ref();
            let _ = cvar.wait_timeout(lock.lock().unwrap(), remaining);
        }
    }
}

// ── PNG encoding ──────────────────────────────────────────────────────────────

fn encode_png(rgb: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    enc.set_filter(png::FilterType::NoFilter);
    enc.write_header().unwrap()
        .write_image_data(rgb).unwrap();
    out
}

// ── HTTP handlers ──────────────────────────────────────────────────────────

async fn index() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(include_str!("../../assets/index.html"))
}

#[derive(serde::Deserialize)]
struct ViewportQuery {
    w: Option<u32>,
    h: Option<u32>,
}

async fn set_viewport(
    viewport: web::Data<Arc<Mutex<(u32, u32)>>>,
    query: web::Query<ViewportQuery>,
) -> HttpResponse {
    let w = query.w.unwrap_or(DEFAULT_W).clamp(320, 3840);
    let h = query.h.unwrap_or(DEFAULT_H).clamp(240, 2160);
    *viewport.lock().unwrap() = (w, h);
    HttpResponse::NoContent().finish()
}

/// WebSocket endpoint for input events.
/// Each browser tab opens one persistent WS connection and sends JSON-encoded
/// InputEvent messages. This avoids per-event HTTP round-trip overhead.
async fn ws_input(
    req: actix_web::HttpRequest,
    stream: web::Payload,
    queue: web::Data<EventQueue>,
) -> actix_web::Result<HttpResponse> {
    let (res, session, mut msg_stream) = actix_ws::handle(&req, stream)?;
    let queue = queue.into_inner();
    actix_web::rt::spawn(async move {
        use futures::StreamExt;
        while let Some(Ok(msg)) = msg_stream.next().await {
            if let actix_ws::Message::Text(text) = msg {
                if let Ok(event) = serde_json::from_str::<InputEvent>(&text) {
                    let (lock, cvar) = queue.as_ref().as_ref();
                    lock.lock().unwrap().push_back(event);
                    cvar.notify_one();
                }
            }
        }
        let _ = session.close(None).await;
    });
    Ok(res)
}

/// Each HTTP client subscribes to the broadcast channel and streams JPEG frames.
async fn mjpeg_stream(
    frame_tx: web::Data<Arc<tokio::sync::broadcast::Sender<Vec<u8>>>>,
) -> HttpResponse {
    let rx = frame_tx.subscribe();
    let body = stream::unfold(rx, |mut rx| async move {
        let jpeg = loop {
            match rx.recv().await {
                Ok(jpeg) => break jpeg,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return None,
            }
        };
        let header = format!(
            "--frame\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n",
            jpeg.len()
        );
        let mut frame = Vec::with_capacity(header.len() + jpeg.len() + 2);
        frame.extend_from_slice(header.as_bytes());
        frame.extend_from_slice(&jpeg);
        frame.extend_from_slice(b"\r\n");
        Some((Ok::<_, actix_web::Error>(Bytes::from(frame)), rx))
    });
    HttpResponse::Ok()
        .content_type("multipart/x-mixed-replace; boundary=frame")
        .streaming(body)
}

// ── Main ───────────────────────────────────────────────────────────────────

fn main() {
    let show_fps = std::env::args().any(|a| a == "--fps");
    let use_sim  = std::env::args().any(|a| a == "--sim-camera");

    let viewport: Arc<Mutex<(u32, u32)>> = Arc::new(Mutex::new((DEFAULT_W, DEFAULT_H)));
    let event_queue: EventQueue = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));

    // Set up headless Slint software-renderer platform on the main thread.
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(slint::PhysicalSize::new(DEFAULT_W, DEFAULT_H));
    slint::platform::set_platform(Box::new(HeadlessPlatform { window: window.clone() }))
        .expect("Slint platform already initialised");

    let ui = AppWindow::new().unwrap();
    ui.set_server_mode(true);

    let cwd = Rc::new(std::cell::RefCell::new(
        std::fs::canonicalize(".").unwrap_or_else(|_| std::path::PathBuf::from("."))
    ));
    let entries = load_dir(&cwd.borrow());
    ui.set_current_path(cwd.borrow().to_string_lossy().to_string().into());
    ui.set_file_list(Rc::new(slint::VecModel::from(entries)).into());
    ui.on_navigate_to({
        let ui_weak = ui.as_weak();
        let cwd = cwd.clone();
        move |segment| {
            let mut cwd = cwd.borrow_mut();
            if segment == ".." {
                cwd.pop();
            } else {
                cwd.push(segment.as_str());
            }
            let entries = load_dir(&cwd);
            if let Some(u) = ui_weak.upgrade() {
                u.set_current_path(cwd.to_string_lossy().to_string().into());
                u.set_file_list(Rc::new(slint::VecModel::from(entries)).into());
            }
        }
    });
    let cam = start_camera_thread(use_sim);
    ui.set_camera_image(cam.snap().to_slint_image());

    let rows: Vec<DevicePropEntry> = cam.device_props().into_iter().map(|p| DevicePropEntry {
        device: p.device.into(),
        property: p.property.into(),
        value: p.value.into(),
    }).collect();
    ui.set_device_props(Rc::new(slint::VecModel::from(rows)).into());

    ui.on_quit(|| std::process::exit(0));

    let (frame_tx, _) = tokio::sync::broadcast::channel::<Vec<u8>>(4);
    let frame_tx = Arc::new(frame_tx);

    // HTTP server on a background thread with its own Tokio runtime.
    let frame_tx_http  = frame_tx.clone();
    let viewport_http  = viewport.clone();
    let queue_http     = event_queue.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            println!("RIPP server listening on http://127.0.0.1:8080  (Ctrl+C to stop)");
            let server = HttpServer::new(move || {
                App::new()
                    .app_data(web::Data::new(frame_tx_http.clone()))
                    .app_data(web::Data::new(viewport_http.clone()))
                    .app_data(web::Data::new(queue_http.clone()))
                    .route("/",        web::get().to(index))
                    .route("/stream",  web::get().to(mjpeg_stream))
                    .route("/viewport",web::get().to(set_viewport))
                    .route("/ws",      web::get().to(ws_input))
            })
            .bind("127.0.0.1:8080")
            .expect("bind failed")
            .run();

            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.expect("ctrl_c");
                println!("\nShutting down…");
                std::process::exit(0);
            });

            server.await.expect("server error");
        });
    });

    // Main thread: Slint render loop. Runs until the process is killed.
    run_render_loop(ui, window, frame_tx, viewport, event_queue, cam, show_fps);
}

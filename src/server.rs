// Headless Slint renderer streamed as MJPEG over HTTP.
//
// Slint's software renderer runs on the main thread and broadcasts encoded PNG frames.
// The actix-web HTTP server runs on a background thread and subscribes to those frames.
// Mouse and keyboard events posted to /ws are forwarded to Slint each render frame.
//
// Invoked via:  ripp --server [--fps] [--sim-camera]
// Then open:    http://127.0.0.1:8080

use actix_web::{web, App, HttpResponse, HttpServer};
use bytes::Bytes;
use ripp::renderer3d::Renderer3d;
use ripp::micromanager::{start_camera_thread, CameraHandle, CameraImage, DeviceProp};
use ripp::{AppWindow, CameraGlobal, CamPropGlobal, DevicePropEntry, Viewer3dGlobal};
use slint::ComponentHandle;
use ripp::app_logic::AppLogic;
use ripp::session::{Tab3d, Camera3d};
use futures::stream;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel};
use slint::platform::{WindowAdapter, WindowEvent};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

// Default render dimensions (used until the browser reports its viewport size).
const DEFAULT_W: u32 = 960;
const DEFAULT_H: u32 = 540;
// TeapotRenderer is created once at this fixed size; Slint scales the image in layout.
const TEAPOT_W: u32 = DEFAULT_W / 2;
const TEAPOT_H: u32 = DEFAULT_H - 36;

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
    // All shared logic: session, viewer2d, file browser, callbacks.
    // Pass `|| {}` for start_live — the server restarts live via on_live_toggled directly.
    let logic = AppLogic::new(camera.clone());
    logic.register_all(&ui, || {});

    // ── Server-specific camera snap/live (pending-slot approach) ─────────────
    // Snap threads write here; the render loop drains and applies lo/hi each frame.
    let pending_snap:  Arc<Mutex<Option<CameraImage>>>    = Arc::new(Mutex::new(None));
    let pending_props: Arc<Mutex<Option<Vec<DeviceProp>>>> = Arc::new(Mutex::new(None));
    let props_refreshing = logic.props_refreshing.clone();
    let live_running     = logic.live_running.clone();

    ui.global::<CameraGlobal>().on_snap_requested({
        let camera = camera.clone();
        let slot   = pending_snap.clone();
        move || {
            let camera = camera.clone();
            let slot   = slot.clone();
            std::thread::spawn(move || { *slot.lock().unwrap() = Some(camera.snap()); });
        }
    });

    ui.global::<CameraGlobal>().on_live_toggled({
        let camera       = camera.clone();
        let slot         = pending_snap.clone();
        let live_running = live_running.clone();
        move |enabled| {
            live_running.store(enabled, Ordering::SeqCst);
            if enabled {
                let camera       = camera.clone();
                let slot         = slot.clone();
                let live_running = live_running.clone();
                std::thread::spawn(move || {
                    while live_running.load(Ordering::SeqCst) {
                        *slot.lock().unwrap() = Some(camera.snap());
                    }
                });
            }
        }
    });

    ui.global::<CameraGlobal>().on_camera_panned({
        let camera       = camera.clone();
        let slot         = pending_props.clone();
        let refreshing   = props_refreshing.clone();
        move |dx, dy| {
            camera.move_xy(-dx as f64, -dy as f64);
            if !refreshing.swap(true, Ordering::SeqCst) {
                let camera     = camera.clone();
                let slot       = slot.clone();
                let refreshing = refreshing.clone();
                std::thread::spawn(move || {
                    let props = camera.device_props();
                    refreshing.store(false, Ordering::SeqCst);
                    *slot.lock().unwrap() = Some(props);
                });
            }
        }
    });

    ui.global::<CameraGlobal>().on_camera_scrolled({
        let camera     = camera.clone();
        let slot       = pending_props.clone();
        let refreshing = props_refreshing.clone();
        move |delta| {
            camera.move_z(delta as f64);
            if !refreshing.swap(true, Ordering::SeqCst) {
                let camera     = camera.clone();
                let slot       = slot.clone();
                let refreshing = refreshing.clone();
                std::thread::spawn(move || {
                    let props = camera.device_props();
                    refreshing.store(false, Ordering::SeqCst);
                    *slot.lock().unwrap() = Some(props);
                });
            }
        }
    });

    // ── Render loop ──────────────────────────────────────────────────────────
    let teapot = Renderer3d::new(TEAPOT_W, TEAPOT_H);
    let (mut w, mut h) = *viewport.lock().unwrap();
    let mut buf = vec![Rgb565Pixel::default(); (w * h) as usize];
    let mut last_print  = Instant::now();
    let mut frame_count = 0u32;

    loop {
        let deadline = Instant::now() + Duration::from_millis(33);

        if frame_tx.receiver_count() == 0 {
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
            continue;
        }

        // Drain input events.
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

        // Resize if browser reported new dimensions.
        let (new_w, new_h) = *viewport.lock().unwrap();
        if new_w != w || new_h != h {
            w = new_w; h = new_h;
            window.set_size(slint::PhysicalSize::new(w, h));
            buf.resize((w * h) as usize, Rgb565Pixel::default());
        }

        // Drain pending snap (apply current lo/hi).
        if let Some(raw) = pending_snap.lock().unwrap().take() {
            let lo = ui.global::<CameraGlobal>().get_camera_lo();
            let hi = ui.global::<CameraGlobal>().get_camera_hi();
            *logic.last_camera_frame.lock().unwrap() =
                Some((raw.data.clone(), raw.width, raw.height));
            ui.global::<CameraGlobal>().set_camera_image(raw.to_slint_image(ripp::session::ColorMappingRange { lo, hi }));
        }

        // Drain pending device props.
        if let Some(props) = pending_props.lock().unwrap().take() {
            let rows: Vec<DevicePropEntry> = props.into_iter().map(|p| DevicePropEntry {
                device: p.device.into(), property: p.property.into(), value: p.value.into(),
            }).collect();
            ui.global::<CamPropGlobal>().set_device_props(Rc::new(slint::VecModel::from(rows)).into());
        }

        slint::platform::update_timers_and_animations();

        let t0 = Instant::now();

        // Render teapot → Slint image property.
        let camera = {
            let s = logic.session.borrow();
            s.tabs_left.iter().find_map(|t| {
                t.as_any().downcast_ref::<Tab3d>().map(|t3| Camera3d {
                    yaw:      t3.camera.yaw,
                    pitch:    t3.camera.pitch,
                    distance: t3.camera.distance,
                })
            }).unwrap_or_default()
        };
        let pixels = teapot.render_frame(&camera);
        let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(TEAPOT_W, TEAPOT_H);
        pb.make_mut_bytes().copy_from_slice(&pixels);
        ui.global::<Viewer3dGlobal>().set_teapot_image(slint::Image::from_rgba8(pb));
        let t1 = Instant::now();

        // Render full Slint UI to RGB565 buffer.
        window.draw_if_needed(|renderer| { renderer.render(&mut buf, w as usize); });
        let t2 = Instant::now();

        // Expand RGB565 → RGB888 for PNG encoding.
        let rgb: Vec<u8> = buf.iter()
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

        // Sleep until deadline or next input event.
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
    enc.write_header().unwrap().write_image_data(rgb).unwrap();
    out
}

// ── HTTP handlers ──────────────────────────────────────────────────────────

async fn index() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(include_str!("../assets/index.html"))
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

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run(show_fps: bool, use_sim: bool) {
    let viewport:    Arc<Mutex<(u32, u32)>> = Arc::new(Mutex::new((DEFAULT_W, DEFAULT_H)));
    let event_queue: EventQueue             = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));

    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(slint::PhysicalSize::new(DEFAULT_W, DEFAULT_H));
    slint::platform::set_platform(Box::new(HeadlessPlatform { window: window.clone() }))
        .expect("Slint platform already initialised");

    let ui = AppWindow::new().unwrap();
    ui.set_server_mode(true);

    let cam = start_camera_thread(use_sim);

    let (frame_tx, _) = tokio::sync::broadcast::channel::<Vec<u8>>(4);
    let frame_tx = Arc::new(frame_tx);

    // HTTP server on a background thread.
    let frame_tx_http = frame_tx.clone();
    let viewport_http = viewport.clone();
    let queue_http    = event_queue.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            println!("RIPP server listening on http://127.0.0.1:8080  (Ctrl+C to stop)");
            let server = HttpServer::new(move || {
                App::new()
                    .app_data(web::Data::new(frame_tx_http.clone()))
                    .app_data(web::Data::new(viewport_http.clone()))
                    .app_data(web::Data::new(queue_http.clone()))
                    .route("/",         web::get().to(index))
                    .route("/stream",   web::get().to(mjpeg_stream))
                    .route("/viewport", web::get().to(set_viewport))
                    .route("/ws",       web::get().to(ws_input))
            })
            .bind("127.0.0.1:8080").expect("bind failed")
            .run();

            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.expect("ctrl_c");
                println!("\nShutting down…");
                std::process::exit(0);
            });

            server.await.expect("server error");
        });
    });

    run_render_loop(ui, window, frame_tx, viewport, event_queue, cam, show_fps);
}

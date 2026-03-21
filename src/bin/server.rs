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
use claude_ui::teapot::TeapotRenderer;
use futures::stream;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel};
use slint::platform::{WindowAdapter, WindowEvent};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
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

type EventQueue = Arc<Mutex<VecDeque<InputEvent>>>;

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
) {
    let teapot = TeapotRenderer::new(TEAPOT_W, TEAPOT_H);
    let start  = Instant::now();
    let (mut w, mut h) = *viewport.lock().unwrap();
    let mut buf = vec![Rgb565Pixel::default(); (w * h) as usize];

    loop {
        let deadline = Instant::now() + Duration::from_millis(33);

        // Resize Slint window if the browser reported new dimensions.
        let (new_w, new_h) = *viewport.lock().unwrap();
        if new_w != w || new_h != h {
            w = new_w;
            h = new_h;
            window.set_size(slint::PhysicalSize::new(w, h));
            buf.resize((w * h) as usize, Rgb565Pixel::default());
        }

        // Drain input events from the HTTP thread and dispatch to Slint.
        {
            let mut q = event_queue.lock().unwrap();
            while let Some(ev) = q.pop_front() {
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

        slint::platform::update_timers_and_animations();

        let rotation = start.elapsed().as_secs_f32() * 0.8;

        // 1. Render teapot → Slint image property (Slint scales it in layout).
        let pixels = teapot.render_frame(rotation);
        let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(TEAPOT_W, TEAPOT_H);
        pb.make_mut_bytes().copy_from_slice(&pixels);
        ui.set_teapot_image(slint::Image::from_rgba8(pb));

        // 2. Render full Slint UI to RGB565 buffer.
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buf, w as usize);
        });

        // 3. Expand RGB565 → RGB888 for JPEG encoding.
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

        let jpeg = encode_jpeg(&rgb, w, h);
        let _    = frame_tx.send(jpeg);

        let now = Instant::now();
        if now < deadline {
            std::thread::sleep(deadline - now);
        }
    }
}

// ── JPEG encoding ─────────────────────────────────────────────────────────────

fn encode_jpeg(rgb: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
    comp.set_size(w as usize, h as usize);
    comp.set_quality(82.0);
    let mut comp = comp.start_compress(Vec::new()).expect("mozjpeg start");
    comp.write_scanlines(rgb).expect("mozjpeg write");
    comp.finish().expect("mozjpeg finish")
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

async fn post_input(
    queue: web::Data<EventQueue>,
    event: web::Json<InputEvent>,
) -> HttpResponse {
    queue.lock().unwrap().push_back(event.into_inner());
    HttpResponse::NoContent().finish()
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
            "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
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
    let viewport: Arc<Mutex<(u32, u32)>> = Arc::new(Mutex::new((DEFAULT_W, DEFAULT_H)));
    let event_queue: EventQueue = Arc::new(Mutex::new(VecDeque::new()));

    // Set up headless Slint software-renderer platform on the main thread.
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(slint::PhysicalSize::new(DEFAULT_W, DEFAULT_H));
    slint::platform::set_platform(Box::new(HeadlessPlatform { window: window.clone() }))
        .expect("Slint platform already initialised");

    let ui = AppWindow::new().unwrap();
    ui.on_quit(|| {});

    let (frame_tx, _) = tokio::sync::broadcast::channel::<Vec<u8>>(4);
    let frame_tx = Arc::new(frame_tx);

    // HTTP server on a background thread with its own Tokio runtime.
    let frame_tx_http  = frame_tx.clone();
    let viewport_http  = viewport.clone();
    let queue_http     = event_queue.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            println!("Listening on http://127.0.0.1:8080  (Ctrl+C to stop)");
            let server = HttpServer::new(move || {
                App::new()
                    .app_data(web::Data::new(frame_tx_http.clone()))
                    .app_data(web::Data::new(viewport_http.clone()))
                    .app_data(web::Data::new(queue_http.clone()))
                    .route("/",        web::get().to(index))
                    .route("/stream",  web::get().to(mjpeg_stream))
                    .route("/viewport",web::get().to(set_viewport))
                    .route("/input",   web::post().to(post_input))
            })
            .bind("127.0.0.1:8080")
            .expect("bind failed")
            .run();

            let handle = server.handle();
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.expect("ctrl_c");
                println!("\nShutting down…");
                handle.stop(true).await;
                std::process::exit(0);
            });

            server.await.expect("server error");
        });
    });

    // Main thread: Slint render loop. Runs until the process is killed.
    run_render_loop(ui, window, frame_tx, viewport, event_queue);
}

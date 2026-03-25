use std::rc::Rc;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use slint::ComponentHandle;
use ripp::{AppWindow, DevicePropEntry};
use ripp::app_logic::AppLogic;
use ripp::renderer3d::Renderer3d;
use ripp::micromanager::start_camera_thread;
use ripp::session::{Tab3d, Camera3d, TabCamera};

mod server;

const TEAPOT_W: u32 = 480;
const TEAPOT_H: u32 = 400;

fn main() {
    let show_fps  = std::env::args().any(|a| a == "--fps");
    let use_sim   = std::env::args().any(|a| a == "--sim-camera");
    let server_mode = std::env::args().any(|a| a == "--server");

    if server_mode {
        server::run(show_fps, use_sim);
        return;
    }

    let app = AppWindow::new().unwrap();
    app.window().set_size(slint::LogicalSize::new(1024.0, 768.0));

    let cam = start_camera_thread(use_sim);

    let logic = AppLogic::new(cam.clone());

    // Build the desktop-specific start_live closure (uses upgrade_in_event_loop).
    let last_camera_frame = logic.last_camera_frame.clone();
    let live_running      = logic.live_running.clone();
    let app_weak          = app.as_weak();

    let start_live = {
        let cam               = cam.clone();
        let last_camera_frame = last_camera_frame.clone();
        let app_weak          = app_weak.clone();
        let live_running      = live_running.clone();
        move || {
            if live_running.swap(true, Ordering::SeqCst) { return; } // already running
            let cam               = cam.clone();
            let last_camera_frame = last_camera_frame.clone();
            let app_weak          = app_weak.clone();
            let live_running      = live_running.clone();
            std::thread::spawn(move || {
                while live_running.load(Ordering::SeqCst) {
                    let raw   = cam.snap();
                    let frame = (raw.data.clone(), raw.width, raw.height);
                    let lcf   = last_camera_frame.clone();
                    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
                    if app_weak.upgrade_in_event_loop(move |ui: AppWindow| {
                        let lo = ui.get_camera_lo();
                        let hi = ui.get_camera_hi();
                        *lcf.lock().unwrap() = Some(frame);
                        ui.set_camera_image(raw.to_slint_image(ripp::session::ColorMappingRange { lo, hi }));
                        let _ = done_tx.send(());
                    }).is_err() { break; }
                    done_rx.recv().ok();
                }
            });
        }
    };

    // Register all shared callbacks.
    logic.register_all(&app, start_live.clone());

    // ── Desktop-specific callbacks ────────────────────────────────────────────

    // Manual snap: spawn thread, use upgrade_in_event_loop for lo/hi.
    app.on_snap_requested({
        let cam               = cam.clone();
        let last_camera_frame = last_camera_frame.clone();
        let app_weak          = app_weak.clone();
        move || {
            let cam               = cam.clone();
            let last_camera_frame = last_camera_frame.clone();
            let app_weak          = app_weak.clone();
            std::thread::spawn(move || {
                let raw   = cam.snap();
                let frame = (raw.data.clone(), raw.width, raw.height);
                app_weak.upgrade_in_event_loop(move |ui: AppWindow| {
                    let lo = ui.get_camera_lo();
                    let hi = ui.get_camera_hi();
                    *last_camera_frame.lock().unwrap() = Some(frame);
                    ui.set_camera_image(raw.to_slint_image(ripp::session::ColorMappingRange { lo, hi }));
                }).ok();
            });
        }
    });

    // Live toggle: start/stop the live loop.
    app.on_live_toggled({
        let session      = logic.session.clone();
        let app_weak     = app_weak.clone();
        let live_running = live_running.clone();
        let start_live   = start_live.clone();
        move |enabled| {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                let mut s = session.borrow_mut();
                if let Some(t) = s.tabs_left.get_mut(tab_idx) {
                    if let Some(tc) = t.as_any_mut().downcast_mut::<TabCamera>() {
                        tc.live = enabled;
                    }
                }
            }
            if enabled { start_live(); } else { live_running.store(false, Ordering::SeqCst); }
        }
    });

    // Camera panned: move stage, refresh props via upgrade_in_event_loop.
    app.on_camera_panned({
        let cam        = cam.clone();
        let app_weak   = app_weak.clone();
        let refreshing = logic.props_refreshing.clone();
        move |dx, dy| {
            cam.move_xy(-dx as f64, -dy as f64);
            spawn_props_refresh(&cam, &app_weak, &refreshing);
        }
    });

    // Camera scrolled: move Z stage, refresh props.
    app.on_camera_scrolled({
        let cam        = cam.clone();
        let app_weak   = app_weak.clone();
        let refreshing = logic.props_refreshing.clone();
        move |delta| {
            cam.move_z(delta as f64);
            spawn_props_refresh(&cam, &app_weak, &refreshing);
        }
    });

    // ── Teapot rendering timer ────────────────────────────────────────────────
    let mut renderer = Renderer3d::new(TEAPOT_W, TEAPOT_H);
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(16),
        {
            let session  = logic.session.clone();
            let app_weak = app_weak.clone();
            move || {
                let Some(ui) = app_weak.upgrade() else { return };
                let tab_idx = ui.get_active_left_tab() as usize;
                let camera = {
                    let s = session.borrow();
                    match s.tabs_left.get(tab_idx)
                        .and_then(|t| t.as_any().downcast_ref::<Tab3d>())
                    {
                        Some(t3) => Camera3d {
                            yaw:      t3.camera.yaw,
                            pitch:    t3.camera.pitch,
                            distance: t3.camera.distance,
                        },
                        None => return,
                    }
                };
                let vw = ui.get_viewer3d_viewport_w().max(1) as u32;
                let vh = ui.get_viewer3d_viewport_h().max(1) as u32;
                renderer.resize(vw, vh);
                let pixels = renderer.render_frame(&camera);
                let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(vw, vh);
                pb.make_mut_bytes().copy_from_slice(&pixels);
                ui.set_teapot_image(slint::Image::from_rgba8(pb));
            }
        },
    );

    app.run().unwrap();
    std::process::exit(0);
}

/// Spawn a background thread to refresh device props; guard against overlapping refreshes.
fn spawn_props_refresh(
    cam:        &ripp::micromanager::CameraHandle,
    app_weak:   &slint::Weak<AppWindow>,
    refreshing: &Arc<AtomicBool>,
) {
    if !refreshing.swap(true, Ordering::SeqCst) {
        let cam        = cam.clone();
        let app_weak   = app_weak.clone();
        let refreshing = refreshing.clone();
        std::thread::spawn(move || {
            let props = cam.device_props();
            refreshing.store(false, Ordering::SeqCst);
            app_weak.upgrade_in_event_loop(move |ui: AppWindow| {
                let rows: Vec<DevicePropEntry> = props.into_iter().map(|p| DevicePropEntry {
                    device: p.device.into(), property: p.property.into(), value: p.value.into(),
                }).collect();
                ui.set_device_props(Rc::new(slint::VecModel::from(rows)).into());
            }).ok();
        });
    }
}

slint::include_modules!();

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Instant;
use ripp::teapot::TeapotRenderer;
use ripp::camera::start_camera_thread;

const TEAPOT_W: u32 = 480;
const TEAPOT_H: u32 = 400;

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

fn main() {
    let app = AppWindow::new().unwrap();

    let cwd = Rc::new(RefCell::new(
        std::fs::canonicalize(".").unwrap_or_else(|_| PathBuf::from("."))
    ));

    let entries = load_dir(&cwd.borrow());
    app.set_current_path(cwd.borrow().to_string_lossy().to_string().into());
    app.set_file_list(Rc::new(slint::VecModel::from(entries)).into());

    app.on_navigate_to({
        let app_weak = app.as_weak();
        let cwd = cwd.clone();
        move |segment| {
            let mut cwd = cwd.borrow_mut();
            if segment == ".." {
                cwd.pop();
            } else {
                cwd.push(segment.as_str());
            }
            let entries = load_dir(&cwd);
            if let Some(ui) = app_weak.upgrade() {
                ui.set_current_path(cwd.to_string_lossy().to_string().into());
                ui.set_file_list(Rc::new(slint::VecModel::from(entries)).into());
            }
        }
    });

    // Headless wgpu teapot renderer
    let renderer = TeapotRenderer::new(TEAPOT_W, TEAPOT_H);
    let start    = Instant::now();
    let app_weak = app.as_weak();

    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(16),
        move || {
            let rotation = start.elapsed().as_secs_f32() * 0.8;
            let pixels   = renderer.render_frame(rotation);

            let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(TEAPOT_W, TEAPOT_H);
            pb.make_mut_bytes().copy_from_slice(&pixels);

            if let Some(ui) = app_weak.upgrade() {
                ui.set_teapot_image(slint::Image::from_rgba8(pb));
            }
        },
    );

    let cam = start_camera_thread();
    app.set_camera_image(cam.snap().to_slint_image());

    let rows: Vec<DevicePropEntry> = cam.device_props().into_iter().map(|p| DevicePropEntry {
        device: p.device.into(),
        property: p.property.into(),
        value: p.value.into(),
    }).collect();
    app.set_device_props(Rc::new(slint::VecModel::from(rows)).into());

    // Manual snap
    app.on_snap_requested({
        let cam = cam.clone();
        let app_weak = app.as_weak();
        move || {
            let cam = cam.clone();
            let app_weak = app_weak.clone();
            std::thread::spawn(move || {
                let raw = cam.snap(); // CameraImage is Send
                app_weak.upgrade_in_event_loop(move |ui| ui.set_camera_image(raw.to_slint_image())).ok();
            });
        }
    });

    // Live (continuous) snap
    let live_running = Arc::new(AtomicBool::new(false));
    app.on_live_toggled({
        let cam = cam.clone();
        let app_weak = app.as_weak();
        let live_running = live_running.clone();
        move |enabled| {
            live_running.store(enabled, Ordering::SeqCst);
            if enabled {
                let cam = cam.clone();
                let app_weak = app_weak.clone();
                let live_running = live_running.clone();
                std::thread::spawn(move || {
                    while live_running.load(Ordering::SeqCst) {
                        let raw = cam.snap();
                        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
                        if app_weak.upgrade_in_event_loop(move |ui| {
                            ui.set_camera_image(raw.to_slint_image());
                            let _ = done_tx.send(());
                        }).is_err() { break; }
                        done_rx.recv().ok(); // wait until the frame is actually displayed
                    }
                });
            }
        }
    });

    let props_refreshing = Arc::new(AtomicBool::new(false));

    app.on_camera_panned({
        let cam = cam.clone();
        let app_weak = app.as_weak();
        let refreshing = props_refreshing.clone();
        move |dx, dy| {
            cam.move_xy(dx as f64, dy as f64);
            if !refreshing.swap(true, Ordering::SeqCst) {
                let cam = cam.clone();
                let app_weak = app_weak.clone();
                let refreshing = refreshing.clone();
                std::thread::spawn(move || {
                    let props = cam.device_props();
                    refreshing.store(false, Ordering::SeqCst);
                    app_weak.upgrade_in_event_loop(move |ui| {
                        let rows: Vec<DevicePropEntry> = props.into_iter().map(|p| DevicePropEntry {
                            device: p.device.into(), property: p.property.into(), value: p.value.into(),
                        }).collect();
                        ui.set_device_props(Rc::new(slint::VecModel::from(rows)).into());
                    }).ok();
                });
            }
        }
    });

    app.on_camera_scrolled({
        let cam = cam.clone();
        let app_weak = app.as_weak();
        let refreshing = props_refreshing.clone();
        move |delta| {
            cam.move_z(delta as f64);
            if !refreshing.swap(true, Ordering::SeqCst) {
                let cam = cam.clone();
                let app_weak = app_weak.clone();
                let refreshing = refreshing.clone();
                std::thread::spawn(move || {
                    let props = cam.device_props();
                    refreshing.store(false, Ordering::SeqCst);
                    app_weak.upgrade_in_event_loop(move |ui| {
                        let rows: Vec<DevicePropEntry> = props.into_iter().map(|p| DevicePropEntry {
                            device: p.device.into(), property: p.property.into(), value: p.value.into(),
                        }).collect();
                        ui.set_device_props(Rc::new(slint::VecModel::from(rows)).into());
                    }).ok();
                });
            }
        }
    });

    app.on_quit(|| std::process::exit(0));

    app.run().unwrap();
    std::process::exit(0);
}

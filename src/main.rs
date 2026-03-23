slint::include_modules!();

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
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

fn build_left_tabs(session: &ripp::session::RippSession) -> slint::ModelRc<LeftTabEntry> {
    let entries: Vec<LeftTabEntry> = session.tabs.iter()
        .map(|t| LeftTabEntry { label: t.label().into(), tab_type: t.type_id() })
        .collect();
    Rc::new(slint::VecModel::from(entries)).into()
}

fn build_tree(session: &ripp::session::RippSession) -> slint::ModelRc<ProjectTreeEntry> {
    let entries: Vec<ProjectTreeEntry> = ripp::session::flatten_session(session)
        .into_iter()
        .map(|(label, indent, obj_id, proj_id)| ProjectTreeEntry {
            label: label.into(),
            indent,
            object_id: obj_id as i32,
            project_id: proj_id as i32,
        })
        .collect();
    Rc::new(slint::VecModel::from(entries)).into()
}

fn render_bioformats_image(
    bf: &mut ripp::session::BioformatsData,
    z: u32,
    cam_x: f64,
    cam_y: f64,
    zoom: f64,
    brightness: f32,
    contrast: f32,
) -> Option<slint::Image> {
    let meta = bf.reader.metadata();
    let w = meta.size_x;
    let h = meta.size_y;
    let bytes = bf.reader.open_bytes(z).ok()?;

    let is_gray = bytes.len() == (w * h) as usize;
    let is_rgb  = bytes.len() == (w * h * 3) as usize;
    if !is_gray && !is_rgb { return None; }

    let apply = |v: u8| -> u8 {
        (v as f32 * contrast + brightness * 255.0).clamp(0.0, 255.0) as u8
    };

    let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
    let out = pb.make_mut_bytes();

    for oy in 0..h {
        for ox in 0..w {
            let src_x = (cam_x + (ox as f64 - w as f64 / 2.0) / zoom).round() as i64;
            let src_y = (cam_y + (oy as f64 - h as f64 / 2.0) / zoom).round() as i64;
            let i = (oy * w + ox) as usize;
            if src_x >= 0 && src_x < w as i64 && src_y >= 0 && src_y < h as i64 {
                let si = src_y as usize * w as usize + src_x as usize;
                let (r, g, b) = if is_gray {
                    let v = bytes[si]; (v, v, v)
                } else {
                    (bytes[si * 3], bytes[si * 3 + 1], bytes[si * 3 + 2])
                };
                out[i * 4]     = apply(r);
                out[i * 4 + 1] = apply(g);
                out[i * 4 + 2] = apply(b);
            } else {
                out[i * 4]     = 0;
                out[i * 4 + 1] = 0;
                out[i * 4 + 2] = 0;
            }
            out[i * 4 + 3] = 255;
        }
    }
    Some(slint::Image::from_rgba8(pb))
}

fn do_render_viewer2d(
    session: &std::rc::Rc<std::cell::RefCell<ripp::session::RippSession>>,
    selected_obj: (i32, i32),
    tab_idx: usize,
    brightness: f32,
    contrast: f32,
    ui: &AppWindow,
) {
    let (proj_id, obj_id) = selected_obj;
    if proj_id < 0 { return; }

    let (z, cam_x, cam_y, zoom) = {
        let s = session.borrow();
        match s.tabs.get(tab_idx) {
            Some(ripp::session::RippTab::Tab2d(t)) =>
                (t.camera.z as u32, t.camera.x, t.camera.y, t.camera.zoom),
            _ => return,
        }
    };

    let image_opt = {
        let mut s = session.borrow_mut();
        if let Some(proj) = s.projects.get_mut(&(proj_id as u32)) {
            if let Some(obj) = ripp::session::find_object_mut(&mut proj.root, obj_id as u32) {
                if let ripp::session::ProjectData::Bioformats(bf) = &mut obj.data {
                    render_bioformats_image(bf, z, cam_x, cam_y, zoom, brightness, contrast)
                } else { None }
            } else { None }
        } else { None }
    };

    if let Some(img) = image_opt {
        ui.set_viewer2d_image(img);
        ui.set_viewer2d_image_loaded(true);
    }
}

fn main() {
    let app = AppWindow::new().unwrap();

    let session = Rc::new(RefCell::new({
        let mut s = ripp::session::RippSession::new();
        s.add_project("Demo Project");
        s
    }));
    app.set_project_tree(build_tree(&session.borrow()));
    app.set_left_tabs(build_left_tabs(&session.borrow()));

    app.on_close_left_tab({
        let session = session.clone();
        let app_weak = app.as_weak();
        move |index| {
            let index = index as usize;
            let mut s = session.borrow_mut();
            if index < s.tabs.len() {
                s.tabs.remove(index);
            }
            drop(s);
            if let Some(ui) = app_weak.upgrade() {
                ui.set_left_tabs(build_left_tabs(&session.borrow()));
                let new_len = session.borrow().tabs.len() as i32;
                if ui.get_active_left_tab() >= new_len {
                    ui.set_active_left_tab((new_len - 1).max(0));
                }
            }
        }
    });

    app.on_new_project({
        let session = session.clone();
        let app_weak = app.as_weak();
        move || {
            session.borrow_mut().add_project("New Project");
            if let Some(ui) = app_weak.upgrade() {
                ui.set_project_tree(build_tree(&session.borrow()));
            }
        }
    });

    app.on_project_tree_selected(|_object_id| {});

    let selected_obj: Rc<RefCell<(i32, i32)>> = Rc::new(RefCell::new((-1, -1)));

    app.on_viewer2d_object_selected({
        let session = session.clone();
        let app_weak = app.as_weak();
        let selected_obj = selected_obj.clone();
        move |project_id, object_id| {
            *selected_obj.borrow_mut() = (project_id, object_id);
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                // Read image dimensions and z range, then center the camera
                let (img_w, img_h, z_max) = {
                    let s = session.borrow();
                    if let Some(proj) = s.projects.get(&(project_id as u32)) {
                        if let Some(obj) = ripp::session::find_object_ref(&proj.root, object_id as u32) {
                            if let ripp::session::ProjectData::Bioformats(bf) = &obj.data {
                                let meta = bf.reader.metadata();
                                (meta.size_x as f64, meta.size_y as f64,
                                 (meta.size_z as i32 - 1).max(0))
                            } else { (0.0, 0.0, 0) }
                        } else { (0.0, 0.0, 0) }
                    } else { (0.0, 0.0, 0) }
                };
                // Center camera and reset z
                {
                    let mut s = session.borrow_mut();
                    if let Some(ripp::session::RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                        t.camera.x    = img_w / 2.0;
                        t.camera.y    = img_h / 2.0;
                        t.camera.zoom = 1.0;
                        t.camera.z    = 0.0;
                    }
                }
                ui.set_viewer2d_z(0.0);
                ui.set_viewer2d_z_max(z_max as f32);
                let brightness = ui.get_viewer2d_brightness();
                let contrast = ui.get_viewer2d_contrast();
                do_render_viewer2d(&session, (project_id, object_id), tab_idx, brightness, contrast, &ui);
            }
        }
    });

    app.on_viewer2d_panned({
        let session = session.clone();
        let app_weak = app.as_weak();
        let selected_obj = selected_obj.clone();
        move |dx, dy| {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                {
                    let mut s = session.borrow_mut();
                    if let Some(ripp::session::RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                        t.camera.x -= dx as f64 / t.camera.zoom;
                        t.camera.y -= dy as f64 / t.camera.zoom;
                    }
                }
                let obj = *selected_obj.borrow();
                let brightness = ui.get_viewer2d_brightness();
                let contrast = ui.get_viewer2d_contrast();
                do_render_viewer2d(&session, obj, tab_idx, brightness, contrast, &ui);
            }
        }
    });

    app.on_viewer2d_scrolled({
        let session = session.clone();
        let app_weak = app.as_weak();
        let selected_obj = selected_obj.clone();
        move |delta| {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                {
                    let mut s = session.borrow_mut();
                    if let Some(ripp::session::RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                        t.camera.zoom *= (delta as f64 * 0.005_f64).exp();
                        t.camera.zoom = t.camera.zoom.clamp(0.01, 100.0);
                    }
                }
                let obj = *selected_obj.borrow();
                let brightness = ui.get_viewer2d_brightness();
                let contrast = ui.get_viewer2d_contrast();
                do_render_viewer2d(&session, obj, tab_idx, brightness, contrast, &ui);
            }
        }
    });

    app.on_viewer2d_settings_changed({
        let session = session.clone();
        let app_weak = app.as_weak();
        let selected_obj = selected_obj.clone();
        move || {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                let obj = *selected_obj.borrow();
                let brightness = ui.get_viewer2d_brightness();
                let contrast = ui.get_viewer2d_contrast();
                do_render_viewer2d(&session, obj, tab_idx, brightness, contrast, &ui);
            }
        }
    });

    app.on_viewer2d_z_changed({
        let session = session.clone();
        let app_weak = app.as_weak();
        let selected_obj = selected_obj.clone();
        move |z| {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                {
                    let mut s = session.borrow_mut();
                    if let Some(ripp::session::RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                        t.camera.z = z.round() as f64;
                    }
                }
                let obj = *selected_obj.borrow();
                let brightness = ui.get_viewer2d_brightness();
                let contrast = ui.get_viewer2d_contrast();
                do_render_viewer2d(&session, obj, tab_idx, brightness, contrast, &ui);
            }
        }
    });

    app.on_close_project({
        let session = session.clone();
        let app_weak = app.as_weak();
        move || {
            let proj_id = app_weak.upgrade()
                .map(|u| u.get_selected_project_id())
                .unwrap_or(-1);
            if proj_id >= 0 {
                session.borrow_mut().projects.remove(&(proj_id as u32));
                if let Some(ui) = app_weak.upgrade() {
                    ui.set_selected_project_id(-1);
                    ui.set_project_tree(build_tree(&session.borrow()));
                }
            }
        }
    });

    let cwd = Rc::new(RefCell::new(
        std::fs::canonicalize(".").unwrap_or_else(|_| PathBuf::from("."))
    ));

    let entries = load_dir(&cwd.borrow());
    app.set_current_path(cwd.borrow().to_string_lossy().to_string().into());
    app.set_file_list(Rc::new(slint::VecModel::from(entries)).into());

    app.on_open_file({
        let session = session.clone();
        let cwd = cwd.clone();
        let app_weak = app.as_weak();
        move |filename| {
            let full_path = cwd.borrow().join(filename.as_str());
            match ripp::session::BioformatsData::open(&full_path) {
                Ok(bf_data) => {
                    let name = full_path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| full_path.to_string_lossy().into_owned());
                    let proj_id = session.borrow_mut().add_project(&name);
                    session.borrow_mut().projects.get_mut(&proj_id).unwrap()
                        .root.data = ripp::session::ProjectData::Bioformats(bf_data);
                    if let Some(ui) = app_weak.upgrade() {
                        ui.set_project_tree(build_tree(&session.borrow()));
                        ui.set_active_tab(0);
                    }
                }
                Err(e) => {
                    if let Some(ui) = app_weak.upgrade() {
                        ui.set_error_message(e.to_string().into());
                    }
                }
            }
        }
    });

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
    let app_weak = app.as_weak();

    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(16),
        {
            let session = session.clone();
            move || {
                let (yaw, pitch, distance) = {
                    let s = session.borrow();
                    s.tabs.iter().find_map(|t| {
                        if let ripp::session::RippTab::Tab3d(t3) = t {
                            Some((t3.camera.yaw, t3.camera.pitch, t3.camera.distance))
                        } else { None }
                    }).unwrap_or((0.0, 0.3, 6.0))
                };
                let pixels = renderer.render_frame(yaw, pitch, distance);

                let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(TEAPOT_W, TEAPOT_H);
                pb.make_mut_bytes().copy_from_slice(&pixels);

                if let Some(ui) = app_weak.upgrade() {
                    ui.set_teapot_image(slint::Image::from_rgba8(pb));
                }
            }
        },
    );

    app.on_viewer3d_panned({
        let session = session.clone();
        move |dx, dy| {
            let mut s = session.borrow_mut();
            if let Some(t3) = s.tabs.iter_mut().find_map(|t| {
                if let ripp::session::RippTab::Tab3d(t3) = t { Some(t3) } else { None }
            }) {
                t3.camera.yaw   -= dx * 0.005;
                t3.camera.pitch  = (t3.camera.pitch + dy * 0.005).clamp(-1.5, 1.5);
            }
        }
    });

    app.on_viewer3d_scrolled({
        let session = session.clone();
        move |delta| {
            let mut s = session.borrow_mut();
            if let Some(t3) = s.tabs.iter_mut().find_map(|t| {
                if let ripp::session::RippTab::Tab3d(t3) = t { Some(t3) } else { None }
            }) {
                t3.camera.distance = (t3.camera.distance * (-(delta * 0.005_f32)).exp())
                    .clamp(0.5, 100.0);
            }
        }
    });

    let use_sim = std::env::args().any(|a| a == "--sim-camera");
    let cam = start_camera_thread(use_sim);
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

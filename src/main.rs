slint::include_modules!();

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
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
    lo: f32,
    hi: f32,
) -> Option<slint::Image> {
    let meta = bf.reader.metadata();
    let w = meta.size_x;
    let h = meta.size_y;
    let bytes = bf.reader.open_bytes(z).ok()?;

    let is_gray = bytes.len() == (w * h) as usize;
    let is_rgb  = bytes.len() == (w * h * 3) as usize;
    if !is_gray && !is_rgb { return None; }

    let range = (hi - lo).max(1.0);
    let apply = |v: u8| -> u8 {
        ((v as f32 - lo) / range * 255.0).clamp(0.0, 255.0) as u8
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
    tab_idx: usize,
    lo: f32,
    hi: f32,
    ui: &AppWindow,
) {
    let (proj_id, obj_id, z, cam_x, cam_y, zoom) = {
        let s = session.borrow();
        match s.tabs.get(tab_idx) {
            Some(ripp::session::RippTab::Tab2d(t)) => (
                t.selected_proj_id, t.selected_obj_id,
                t.camera.z as u32, t.camera.x, t.camera.y, t.camera.zoom,
            ),
            _ => return,
        }
    };
    if proj_id < 0 { return; }

    let image_opt = {
        let mut s = session.borrow_mut();
        if let Some(proj) = s.projects.get_mut(&(proj_id as u32)) {
            if let Some(obj) = ripp::session::find_object_mut(&mut proj.root, obj_id as u32) {
                if let ripp::session::ProjectData::Bioformats(bf) = &mut obj.data {
                    render_bioformats_image(bf, z, cam_x, cam_y, zoom, lo, hi)
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

    app.on_add_tab_3d({
        let session = session.clone();
        let app_weak = app.as_weak();
        move || {
            session.borrow_mut().tabs.push(ripp::session::RippTab::Tab3d(
                ripp::session::Tab3d { camera: ripp::session::Camera3d::default() }
            ));
            if let Some(ui) = app_weak.upgrade() {
                let new_idx = (session.borrow().tabs.len() as i32) - 1;
                ui.set_left_tabs(build_left_tabs(&session.borrow()));
                ui.set_active_left_tab(new_idx);
            }
        }
    });
    app.on_add_tab_2d({
        let session = session.clone();
        let app_weak = app.as_weak();
        move || {
            session.borrow_mut().tabs.push(ripp::session::RippTab::Tab2d(
                ripp::session::Tab2d::default()
            ));
            if let Some(ui) = app_weak.upgrade() {
                let new_idx = (session.borrow().tabs.len() as i32) - 1;
                ui.set_left_tabs(build_left_tabs(&session.borrow()));
                ui.set_active_left_tab(new_idx);
            }
        }
    });
    app.on_add_tab_camera({
        let session = session.clone();
        let app_weak = app.as_weak();
        move || {
            session.borrow_mut().tabs.push(ripp::session::RippTab::Camera(
                ripp::session::TabCamera { live: false, lo: 0.0, hi: 255.0 }
            ));
            if let Some(ui) = app_weak.upgrade() {
                let new_idx = (session.borrow().tabs.len() as i32) - 1;
                ui.set_left_tabs(build_left_tabs(&session.borrow()));
                ui.set_active_left_tab(new_idx);
            }
        }
    });

    app.on_viewer2d_object_selected({
        let session = session.clone();
        let app_weak = app.as_weak();
        move |project_id, object_id| {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
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
                {
                    let mut s = session.borrow_mut();
                    if let Some(ripp::session::RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                        t.selected_proj_id = project_id;
                        t.selected_obj_id  = object_id;
                        t.z_max            = z_max;
                        t.camera.x    = img_w / 2.0;
                        t.camera.y    = img_h / 2.0;
                        t.camera.zoom = 1.0;
                        t.camera.z    = 0.0;
                    }
                }
                ui.set_viewer2d_z(0.0);
                ui.set_viewer2d_z_max(z_max as f32);
                let lo = ui.get_viewer2d_lo();
                let hi = ui.get_viewer2d_hi();
                do_render_viewer2d(&session, tab_idx, lo, hi, &ui);
            }
        }
    });

    app.on_viewer2d_panned({
        let session = session.clone();
        let app_weak = app.as_weak();
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
                let lo = ui.get_viewer2d_lo();
                let hi = ui.get_viewer2d_hi();
                do_render_viewer2d(&session, tab_idx, lo, hi, &ui);
            }
        }
    });

    app.on_viewer2d_scrolled({
        let session = session.clone();
        let app_weak = app.as_weak();
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
                let lo = ui.get_viewer2d_lo();
                let hi = ui.get_viewer2d_hi();
                do_render_viewer2d(&session, tab_idx, lo, hi, &ui);
            }
        }
    });

    app.on_viewer2d_settings_changed({
        let session = session.clone();
        let app_weak = app.as_weak();
        move || {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                let lo = ui.get_viewer2d_lo();
                let hi = ui.get_viewer2d_hi();
                {
                    let mut s = session.borrow_mut();
                    if let Some(ripp::session::RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                        t.lo = lo;
                        t.hi = hi;
                    }
                }
                do_render_viewer2d(&session, tab_idx, lo, hi, &ui);
            }
        }
    });

    app.on_viewer2d_z_changed({
        let session = session.clone();
        let app_weak = app.as_weak();
        move |z| {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                {
                    let mut s = session.borrow_mut();
                    if let Some(ripp::session::RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                        t.camera.z = z.round() as f64;
                    }
                }
                let lo = ui.get_viewer2d_lo();
                let hi = ui.get_viewer2d_hi();
                do_render_viewer2d(&session, tab_idx, lo, hi, &ui);
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
                    let tab_idx = app_weak.upgrade()
                        .map(|u| u.get_active_left_tab() as usize)
                        .unwrap_or(0);
                    match s.tabs.get(tab_idx) {
                        Some(ripp::session::RippTab::Tab3d(t3)) =>
                            (t3.camera.yaw, t3.camera.pitch, t3.camera.distance),
                        _ => return,
                    }
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
        let app_weak = app.as_weak();
        move |dx, dy| {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                let mut s = session.borrow_mut();
                if let Some(ripp::session::RippTab::Tab3d(t3)) = s.tabs.get_mut(tab_idx) {
                    t3.camera.yaw   -= dx * 0.005;
                    t3.camera.pitch  = (t3.camera.pitch + dy * 0.005).clamp(-1.5, 1.5);
                }
            }
        }
    });

    app.on_viewer3d_scrolled({
        let session = session.clone();
        let app_weak = app.as_weak();
        move |delta| {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                let mut s = session.borrow_mut();
                if let Some(ripp::session::RippTab::Tab3d(t3)) = s.tabs.get_mut(tab_idx) {
                    t3.camera.distance = (t3.camera.distance * (-(delta * 0.005_f32)).exp())
                        .clamp(0.5, 100.0);
                }
            }
        }
    });

    let use_sim = std::env::args().any(|a| a == "--sim-camera");
    let cam = start_camera_thread(use_sim);
    // Shared storage for last captured camera frame (Send-safe for use in threads)
    let last_camera_frame: Arc<Mutex<Option<(Vec<u8>, u32, u32)>>> = Arc::new(Mutex::new(None));
    app.set_camera_image(cam.snap().to_slint_image(0.0, 255.0));

    let rows: Vec<DevicePropEntry> = cam.device_props().into_iter().map(|p| DevicePropEntry {
        device: p.device.into(),
        property: p.property.into(),
        value: p.value.into(),
    }).collect();
    app.set_device_props(Rc::new(slint::VecModel::from(rows)).into());

    // Manual snap
    app.on_snap_requested({
        let cam = cam.clone();
        let last_camera_frame = last_camera_frame.clone();
        let app_weak = app.as_weak();
        move || {
            let cam = cam.clone();
            let last_camera_frame = last_camera_frame.clone();
            let app_weak = app_weak.clone();
            std::thread::spawn(move || {
                let raw = cam.snap();
                let frame = (raw.data.clone(), raw.width, raw.height);
                app_weak.upgrade_in_event_loop(move |ui| {
                    let lo = ui.get_camera_lo();
                    let hi = ui.get_camera_hi();
                    *last_camera_frame.lock().unwrap() = Some(frame);
                    ui.set_camera_image(raw.to_slint_image(lo, hi));
                }).ok();
            });
        }
    });

    // Live (continuous) snap
    let live_running = Arc::new(AtomicBool::new(false));

    // Factored out so on_left_tab_activated can also start the loop
    let start_live = {
        let cam = cam.clone();
        let last_camera_frame = last_camera_frame.clone();
        let app_weak = app.as_weak();
        let live_running = live_running.clone();
        move || {
            if live_running.swap(true, Ordering::SeqCst) { return; } // already running
            let cam = cam.clone();
            let last_camera_frame = last_camera_frame.clone();
            let app_weak = app_weak.clone();
            let live_running = live_running.clone();
            std::thread::spawn(move || {
                while live_running.load(Ordering::SeqCst) {
                    let raw = cam.snap();
                    let frame = (raw.data.clone(), raw.width, raw.height);
                    let last_camera_frame = last_camera_frame.clone();
                    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
                    if app_weak.upgrade_in_event_loop(move |ui| {
                        let lo = ui.get_camera_lo();
                        let hi = ui.get_camera_hi();
                        *last_camera_frame.lock().unwrap() = Some(frame);
                        ui.set_camera_image(raw.to_slint_image(lo, hi));
                        let _ = done_tx.send(());
                    }).is_err() { break; }
                    done_rx.recv().ok();
                }
            });
        }
    };

    app.on_live_toggled({
        let session = session.clone();
        let app_weak = app.as_weak();
        let live_running = live_running.clone();
        let start_live = start_live.clone();
        move |enabled| {
            // Save per-tab
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                let mut s = session.borrow_mut();
                if let Some(ripp::session::RippTab::Camera(tc)) = s.tabs.get_mut(tab_idx) {
                    tc.live = enabled;
                }
            }
            if enabled {
                start_live();
            } else {
                live_running.store(false, Ordering::SeqCst);
            }
        }
    });

    app.on_camera_settings_changed({
        let session = session.clone();
        let last_camera_frame = last_camera_frame.clone();
        let app_weak = app.as_weak();
        move || {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                let lo = ui.get_camera_lo();
                let hi = ui.get_camera_hi();
                {
                    let mut s = session.borrow_mut();
                    if let Some(ripp::session::RippTab::Camera(tc)) = s.tabs.get_mut(tab_idx) {
                        tc.lo = lo;
                        tc.hi = hi;
                    }
                }
                if let Some((ref data, w, h)) = *last_camera_frame.lock().unwrap() {
                    let img = ripp::camera::CameraImage { data: data.clone(), width: w, height: h };
                    ui.set_camera_image(img.to_slint_image(lo, hi));
                }
            }
        }
    });

    // Restore per-tab state when the active tab changes
    let prev_tab_idx: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
    app.on_left_tab_activated({
        let session = session.clone();
        let app_weak = app.as_weak();
        let live_running = live_running.clone();
        let start_live = start_live.clone();
        let prev_tab_idx = prev_tab_idx.clone();
        move |new_idx| {
            let new_idx = new_idx as usize;
            let old_idx = *prev_tab_idx.borrow();
            *prev_tab_idx.borrow_mut() = new_idx;

            // Save live state of the old camera tab and stop the loop
            {
                let mut s = session.borrow_mut();
                if let Some(ripp::session::RippTab::Camera(tc)) = s.tabs.get_mut(old_idx) {
                    tc.live = live_running.load(Ordering::SeqCst);
                    live_running.store(false, Ordering::SeqCst);
                }
            }

            if let Some(ui) = app_weak.upgrade() {
                let s = session.borrow();
                match s.tabs.get(new_idx) {
                    Some(ripp::session::RippTab::Tab2d(t)) => {
                        ui.set_viewer2d_lo(t.lo);
                        ui.set_viewer2d_hi(t.hi);
                        ui.set_viewer2d_z(t.camera.z as f32);
                        ui.set_viewer2d_z_max(t.z_max as f32);
                        let has_obj = t.selected_proj_id >= 0;
                        if !has_obj {
                            ui.set_viewer2d_image_loaded(false);
                        }
                        // Re-render needs mutable borrow — drop shared borrow first
                        let (lo, hi) = (t.lo, t.hi);
                        drop(s);
                        if has_obj {
                            do_render_viewer2d(&session, new_idx, lo, hi, &ui);
                        }
                    }
                    Some(ripp::session::RippTab::Camera(tc)) => {
                        let want_live = tc.live;
                        let (lo, hi) = (tc.lo, tc.hi);
                        ui.set_live_snap(want_live);
                        ui.set_camera_lo(lo);
                        ui.set_camera_hi(hi);
                        drop(s);
                        if want_live { start_live(); }
                    }
                    _ => {}
                }
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

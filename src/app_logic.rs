use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use slint::ComponentHandle;
use crate::{AppWindow, FileEntry, LeftTabEntry, ProjectTreeEntry, DevicePropEntry};
use crate::session::*;
use crate::viewer2d::Viewer2dRenderer;
use crate::camera::{CameraHandle, CameraImage};

// ── Shared helpers ────────────────────────────────────────────────────────────

pub fn fmt_size(bytes: u64) -> String {
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

pub fn load_dir(path: &Path) -> Vec<FileEntry> {
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

pub fn build_left_tabs(session: &RippSession) -> slint::ModelRc<LeftTabEntry> {
    let entries: Vec<LeftTabEntry> = session.tabs.iter()
        .map(|t| LeftTabEntry { label: t.label().into(), tab_type: t.type_id() })
        .collect();
    Rc::new(slint::VecModel::from(entries)).into()
}

pub fn build_tree(session: &RippSession) -> slint::ModelRc<ProjectTreeEntry> {
    let entries: Vec<ProjectTreeEntry> = flatten_session(session)
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

pub fn upload_viewer2d_image(
    session: &Rc<RefCell<RippSession>>,
    viewer2d: &mut Viewer2dRenderer,
    tab_idx: usize,
) {
    let (proj_id, obj_id, z) = {
        let s = session.borrow();
        match s.tabs.get(tab_idx) {
            Some(RippTab::Tab2d(t)) if t.selected_proj_id >= 0 =>
                (t.selected_proj_id, t.selected_obj_id, t.camera.z as u32),
            _ => return,
        }
    };
    let mut s = session.borrow_mut();
    if let Some(proj) = s.projects.get_mut(&(proj_id as u32)) {
        if let Some(obj) = find_object_mut(&mut proj.root, obj_id as u32) {
            if let ProjectData::Bioformats(bf) = &mut obj.data {
                let meta = bf.reader.metadata();
                let w = meta.size_x;
                let h = meta.size_y;
                if let Ok(bytes) = bf.reader.open_bytes(z) {
                    let is_gray = bytes.len() == (w * h) as usize;
                    let is_rgb  = bytes.len() == (w * h * 3) as usize;
                    if is_gray || is_rgb {
                        viewer2d.upload(&bytes, w, h, is_gray);
                    }
                }
            }
        }
    }
}

pub fn render_viewer2d(
    session: &Rc<RefCell<RippSession>>,
    viewer2d: &Viewer2dRenderer,
    tab_idx: usize,
    lo: f32,
    hi: f32,
    ui: &AppWindow,
) {
    let (cam_x, cam_y, zoom) = {
        let s = session.borrow();
        match s.tabs.get(tab_idx) {
            Some(RippTab::Tab2d(t)) => (t.camera.x, t.camera.y, t.camera.zoom),
            _ => return,
        }
    };
    if let Some(pixels) = viewer2d.render(cam_x, cam_y, zoom, lo, hi) {
        let w = viewer2d.out_w();
        let h = viewer2d.out_h();
        let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
        pb.make_mut_bytes().copy_from_slice(&pixels);
        ui.set_viewer2d_image(slint::Image::from_rgba8(pb));
        ui.set_viewer2d_image_loaded(true);
    }
}

// ── AppLogic struct ───────────────────────────────────────────────────────────

pub struct AppLogic {
    pub session:           Rc<RefCell<RippSession>>,
    pub viewer2d:          Rc<RefCell<Viewer2dRenderer>>,
    pub cam:               CameraHandle,
    pub last_camera_frame: Arc<Mutex<Option<(Vec<u8>, u32, u32)>>>,
    pub live_running:      Arc<AtomicBool>,
    pub props_refreshing:  Arc<AtomicBool>,
    pub prev_tab_idx:      Rc<RefCell<usize>>,
    pub cwd:               Rc<RefCell<PathBuf>>,
}

impl AppLogic {
    pub fn new(cam: CameraHandle) -> Self {
        let mut s = RippSession::new();
        s.add_project("Demo Project");
        let cwd = std::fs::canonicalize(".").unwrap_or_else(|_| PathBuf::from("."));
        Self {
            session:           Rc::new(RefCell::new(s)),
            viewer2d:          Rc::new(RefCell::new(Viewer2dRenderer::new())),
            cam,
            last_camera_frame: Arc::new(Mutex::new(None)),
            live_running:      Arc::new(AtomicBool::new(false)),
            props_refreshing:  Arc::new(AtomicBool::new(false)),
            prev_tab_idx:      Rc::new(RefCell::new(0)),
            cwd:               Rc::new(RefCell::new(cwd)),
        }
    }

    /// Register all shared Slint callbacks on `app`.
    ///
    /// `start_live` is called when switching to a Camera tab that had live mode enabled.
    /// Pass a real live-loop starter on desktop; pass `|| {}` on the server where the
    /// render loop handles live frames via a pending-snap slot.
    pub fn register_all<F: Fn() + 'static>(&self, app: &AppWindow, start_live: F) {
        let start_live = Rc::new(start_live);

        let session      = &self.session;
        let viewer2d     = &self.viewer2d;
        let cwd          = &self.cwd;
        let lcf          = &self.last_camera_frame;
        let live_running = &self.live_running;
        let prev_tab     = &self.prev_tab_idx;

        // ── Initial UI state ─────────────────────────────────────────────────
        app.set_project_tree(build_tree(&session.borrow()));
        app.set_left_tabs(build_left_tabs(&session.borrow()));
        app.set_current_path(cwd.borrow().to_string_lossy().to_string().into());
        app.set_file_list(Rc::new(slint::VecModel::from(load_dir(&cwd.borrow()))).into());

        let snap = self.cam.snap();
        *lcf.lock().unwrap() = Some((snap.data.clone(), snap.width, snap.height));
        app.set_camera_image(snap.to_slint_image(0.0, 255.0));

        let rows: Vec<DevicePropEntry> = self.cam.device_props().into_iter().map(|p| {
            DevicePropEntry { device: p.device.into(), property: p.property.into(), value: p.value.into() }
        }).collect();
        app.set_device_props(Rc::new(slint::VecModel::from(rows)).into());

        // ── Callbacks ────────────────────────────────────────────────────────

        app.on_close_left_tab({
            let session = session.clone();
            let app_weak = app.as_weak();
            move |index| {
                let index = index as usize;
                let mut s = session.borrow_mut();
                if index < s.tabs.len() { s.tabs.remove(index); }
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
                session.borrow_mut().tabs.push(RippTab::Tab3d(Tab3d { camera: Camera3d::default() }));
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
                session.borrow_mut().tabs.push(RippTab::Tab2d(Tab2d::default()));
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
                session.borrow_mut().tabs.push(RippTab::Camera(TabCamera { live: false, lo: 0.0, hi: 255.0 }));
                if let Some(ui) = app_weak.upgrade() {
                    let new_idx = (session.borrow().tabs.len() as i32) - 1;
                    ui.set_left_tabs(build_left_tabs(&session.borrow()));
                    ui.set_active_left_tab(new_idx);
                }
            }
        });

        app.on_viewer2d_object_selected({
            let session  = session.clone();
            let viewer2d = viewer2d.clone();
            let app_weak = app.as_weak();
            move |project_id, object_id| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let (img_w, img_h, z_max) = {
                        let s = session.borrow();
                        if let Some(proj) = s.projects.get(&(project_id as u32)) {
                            if let Some(obj) = find_object_ref(&proj.root, object_id as u32) {
                                if let ProjectData::Bioformats(bf) = &obj.data {
                                    let meta = bf.reader.metadata();
                                    (meta.size_x as f64, meta.size_y as f64,
                                     (meta.size_z as i32 - 1).max(0))
                                } else { (0.0, 0.0, 0) }
                            } else { (0.0, 0.0, 0) }
                        } else { (0.0, 0.0, 0) }
                    };
                    {
                        let mut s = session.borrow_mut();
                        if let Some(RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
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
                    upload_viewer2d_image(&session, &mut viewer2d.borrow_mut(), tab_idx);
                    render_viewer2d(&session, &viewer2d.borrow(), tab_idx, lo, hi, &ui);
                }
            }
        });

        app.on_viewer2d_panned({
            let session  = session.clone();
            let viewer2d = viewer2d.clone();
            let app_weak = app.as_weak();
            move |dx, dy| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    {
                        let mut s = session.borrow_mut();
                        if let Some(RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                            t.camera.x -= dx as f64 / t.camera.zoom;
                            t.camera.y -= dy as f64 / t.camera.zoom;
                        }
                    }
                    let lo = ui.get_viewer2d_lo();
                    let hi = ui.get_viewer2d_hi();
                    render_viewer2d(&session, &viewer2d.borrow(), tab_idx, lo, hi, &ui);
                }
            }
        });

        app.on_viewer2d_scrolled({
            let session  = session.clone();
            let viewer2d = viewer2d.clone();
            let app_weak = app.as_weak();
            move |delta| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    {
                        let mut s = session.borrow_mut();
                        if let Some(RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                            t.camera.zoom *= (delta as f64 * 0.005_f64).exp();
                            t.camera.zoom = t.camera.zoom.clamp(0.01, 100.0);
                        }
                    }
                    let lo = ui.get_viewer2d_lo();
                    let hi = ui.get_viewer2d_hi();
                    render_viewer2d(&session, &viewer2d.borrow(), tab_idx, lo, hi, &ui);
                }
            }
        });

        app.on_viewer2d_settings_changed({
            let session  = session.clone();
            let viewer2d = viewer2d.clone();
            let app_weak = app.as_weak();
            move || {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let lo = ui.get_viewer2d_lo();
                    let hi = ui.get_viewer2d_hi();
                    {
                        let mut s = session.borrow_mut();
                        if let Some(RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                            t.lo = lo;
                            t.hi = hi;
                        }
                    }
                    render_viewer2d(&session, &viewer2d.borrow(), tab_idx, lo, hi, &ui);
                }
            }
        });

        app.on_viewer2d_z_changed({
            let session  = session.clone();
            let viewer2d = viewer2d.clone();
            let app_weak = app.as_weak();
            move |z| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    {
                        let mut s = session.borrow_mut();
                        if let Some(RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                            t.camera.z = z.round() as f64;
                        }
                    }
                    let lo = ui.get_viewer2d_lo();
                    let hi = ui.get_viewer2d_hi();
                    upload_viewer2d_image(&session, &mut viewer2d.borrow_mut(), tab_idx);
                    render_viewer2d(&session, &viewer2d.borrow(), tab_idx, lo, hi, &ui);
                }
            }
        });

        app.on_close_project({
            let session  = session.clone();
            let app_weak = app.as_weak();
            move || {
                let proj_id = app_weak.upgrade().map(|u| u.get_selected_project_id()).unwrap_or(-1);
                if proj_id >= 0 {
                    session.borrow_mut().projects.remove(&(proj_id as u32));
                    if let Some(ui) = app_weak.upgrade() {
                        ui.set_selected_project_id(-1);
                        ui.set_project_tree(build_tree(&session.borrow()));
                    }
                }
            }
        });

        app.on_open_file({
            let session  = session.clone();
            let cwd      = cwd.clone();
            let app_weak = app.as_weak();
            move |filename| {
                let full_path = cwd.borrow().join(filename.as_str());
                match BioformatsData::open(&full_path) {
                    Ok(bf_data) => {
                        let name = full_path.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| full_path.to_string_lossy().into_owned());
                        let proj_id = session.borrow_mut().add_project(&name);
                        session.borrow_mut().projects.get_mut(&proj_id).unwrap()
                            .root.data = ProjectData::Bioformats(bf_data);
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
            let cwd      = cwd.clone();
            let app_weak = app.as_weak();
            move |segment| {
                let mut cwd = cwd.borrow_mut();
                if segment == ".." { cwd.pop(); } else { cwd.push(segment.as_str()); }
                let entries = load_dir(&cwd);
                if let Some(ui) = app_weak.upgrade() {
                    ui.set_current_path(cwd.to_string_lossy().to_string().into());
                    ui.set_file_list(Rc::new(slint::VecModel::from(entries)).into());
                }
            }
        });

        app.on_viewer3d_panned({
            let session  = session.clone();
            let app_weak = app.as_weak();
            move |dx, dy| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let mut s = session.borrow_mut();
                    if let Some(RippTab::Tab3d(t3)) = s.tabs.get_mut(tab_idx) {
                        t3.camera.yaw   -= dx * 0.005;
                        t3.camera.pitch  = (t3.camera.pitch + dy * 0.005).clamp(-1.5, 1.5);
                    }
                }
            }
        });

        app.on_viewer3d_scrolled({
            let session  = session.clone();
            let app_weak = app.as_weak();
            move |delta| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let mut s = session.borrow_mut();
                    if let Some(RippTab::Tab3d(t3)) = s.tabs.get_mut(tab_idx) {
                        t3.camera.distance = (t3.camera.distance * (-(delta * 0.005_f32)).exp())
                            .clamp(0.5, 100.0);
                    }
                }
            }
        });

        app.on_camera_settings_changed({
            let session  = session.clone();
            let lcf      = lcf.clone();
            let app_weak = app.as_weak();
            move || {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let lo = ui.get_camera_lo();
                    let hi = ui.get_camera_hi();
                    {
                        let mut s = session.borrow_mut();
                        if let Some(RippTab::Camera(tc)) = s.tabs.get_mut(tab_idx) {
                            tc.lo = lo;
                            tc.hi = hi;
                        }
                    }
                    if let Some((ref data, w, h)) = *lcf.lock().unwrap() {
                        let img = CameraImage { data: data.clone(), width: w, height: h };
                        ui.set_camera_image(img.to_slint_image(lo, hi));
                    }
                }
            }
        });

        app.on_left_tab_activated({
            let session      = session.clone();
            let viewer2d     = viewer2d.clone();
            let app_weak     = app.as_weak();
            let live_running = live_running.clone();
            let prev_tab     = prev_tab.clone();
            let start_live   = start_live.clone();
            move |new_idx| {
                let new_idx = new_idx as usize;
                let old_idx = *prev_tab.borrow();
                *prev_tab.borrow_mut() = new_idx;

                // Save live state on old Camera tab and stop the loop
                {
                    let mut s = session.borrow_mut();
                    if let Some(RippTab::Camera(tc)) = s.tabs.get_mut(old_idx) {
                        tc.live = live_running.load(Ordering::SeqCst);
                        live_running.store(false, Ordering::SeqCst);
                    }
                }

                if let Some(ui) = app_weak.upgrade() {
                    let s = session.borrow();
                    match s.tabs.get(new_idx) {
                        Some(RippTab::Tab2d(t)) => {
                            ui.set_viewer2d_lo(t.lo);
                            ui.set_viewer2d_hi(t.hi);
                            ui.set_viewer2d_z(t.camera.z as f32);
                            ui.set_viewer2d_z_max(t.z_max as f32);
                            let has_obj = t.selected_proj_id >= 0;
                            if !has_obj { ui.set_viewer2d_image_loaded(false); }
                            let (lo, hi) = (t.lo, t.hi);
                            drop(s);
                            if has_obj {
                                upload_viewer2d_image(&session, &mut viewer2d.borrow_mut(), new_idx);
                                render_viewer2d(&session, &viewer2d.borrow(), new_idx, lo, hi, &ui);
                            }
                        }
                        Some(RippTab::Camera(tc)) => {
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

        app.on_quit(|| std::process::exit(0));
    }
}

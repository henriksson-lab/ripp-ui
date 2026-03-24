use std::cell::RefCell;
use slint::ComponentHandle;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, atomic::AtomicBool};
use crate::{AppWindow, DevicePropEntry, LeftTabEntry, ProjectTreeEntry, TabMenuAction};
use crate::session::{RippSession, RippTab, flatten_session};
use crate::renderer2d::Viewer2dRenderer;
use crate::micromanager::CameraHandle;
use crate::panes;

// ── Shared model builders (used by multiple panes) ────────────────────────────

pub fn build_tabs(tabs: &[RippTab]) -> slint::ModelRc<LeftTabEntry> {
    let entries: Vec<LeftTabEntry> = tabs.iter()
        .map(|t| {
            let actions: Vec<TabMenuAction> = t.menu_actions().into_iter()
                .map(|(l, id)| TabMenuAction { label: l.into(), action_id: id })
                .collect();
            LeftTabEntry {
                label:        t.label().into(),
                tab_type:     t.type_id(),
                menu_actions: Rc::new(slint::VecModel::from(actions)).into(),
            }
        })
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

// ── AppLogic ──────────────────────────────────────────────────────────────────

pub struct AppLogic {
    pub session:                Rc<RefCell<RippSession>>,
    pub viewer2d:               Rc<RefCell<Viewer2dRenderer>>,
    pub cam:                    CameraHandle,
    pub last_camera_frame:      Arc<Mutex<Option<(Vec<u8>, u32, u32)>>>,
    pub live_running:           Arc<AtomicBool>,
    pub props_refreshing:       Arc<AtomicBool>,
    pub prev_tab_idx:           Rc<RefCell<usize>>,
    pub prev_right_top_idx:     Rc<RefCell<usize>>,
    pub prev_right_bottom_idx:  Rc<RefCell<usize>>,
    pub cwd:                    Rc<RefCell<PathBuf>>,
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
            live_running:           Arc::new(AtomicBool::new(false)),
            props_refreshing:       Arc::new(AtomicBool::new(false)),
            prev_tab_idx:           Rc::new(RefCell::new(0)),
            prev_right_top_idx:     Rc::new(RefCell::new(0)),
            prev_right_bottom_idx:  Rc::new(RefCell::new(0)),
            cwd:                    Rc::new(RefCell::new(cwd)),
        }
    }

    /// Register all shared Slint callbacks on `app`, delegating to per-pane modules.
    ///
    /// `start_live` is called when switching to a Camera tab that had live mode active.
    /// Pass the real live-loop starter on desktop; `|| {}` on the server.
    pub fn register_all<F: Fn() + 'static>(&self, app: &AppWindow, start_live: F) {
        let cam1  = self.cam.clone();
        let cam2  = self.cam.clone();
        let cam3  = self.cam.clone();
        let weak1 = app.as_weak();
        let weak2 = app.as_weak();
        let weak3 = app.as_weak();

        let refresh_props = |cam: &crate::micromanager::CameraHandle, ui: &AppWindow| {
            let rows: Vec<DevicePropEntry> = cam.device_props().into_iter()
                .map(|p| DevicePropEntry { device: p.device.into(), property: p.property.into(), value: p.value.into() })
                .collect();
            ui.set_device_props(Rc::new(slint::VecModel::from(rows)).into());
        };

        let add_demo_camera: Option<Rc<dyn Fn()>> = Some(Rc::new(move || {
            cam1.load_demo_camera();
            if let Some(ui) = weak1.upgrade() { refresh_props(&cam1, &ui); }
        }));
        let add_sim_camera: Option<Rc<dyn Fn()>> = Some(Rc::new(move || {
            cam2.load_sim_camera();
            if let Some(ui) = weak2.upgrade() { refresh_props(&cam2, &ui); }
        }));
        let disconnect_all: Option<Rc<dyn Fn()>> = Some(Rc::new(move || {
            cam3.disconnect_all();
            if let Some(ui) = weak3.upgrade() { refresh_props(&cam3, &ui); }
        }));

        panes::tabs::register(app, &self.session, &self.viewer2d,
                              &self.live_running,
                              &self.prev_tab_idx,
                              &self.prev_right_top_idx,
                              &self.prev_right_bottom_idx,
                              add_demo_camera,
                              add_sim_camera,
                              disconnect_all,
                              start_live);
        panes::viewer3d::register(app, &self.session);
        panes::viewer2d::register(app, &self.session, &self.viewer2d);
        panes::camera_view::register(app, &self.cam, &self.session, &self.last_camera_frame);
        panes::project::register(app, &self.session);
        panes::file_browser::register(app, &self.session, &self.cwd);
        app.on_quit(|| std::process::exit(0));
    }
}

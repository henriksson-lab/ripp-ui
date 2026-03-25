use std::cell::RefCell;
use slint::ComponentHandle;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, atomic::AtomicBool};
use crate::{AppWindow, DevicePropEntry, LeftTabEntry, ProjectTreeEntry, TabMenuAction};
use crate::session::{RippSession, TabPane, TabType, CallbackCtx, flatten_session};
use crate::renderer2d::Viewer2dRenderer;
use crate::micromanager::CameraHandle;
use crate::panes;

// ── Shared model builders ─────────────────────────────────────────────────────

pub fn build_tabs(tabs: &[Box<dyn TabPane>]) -> slint::ModelRc<LeftTabEntry> {
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
    pub session:               Rc<RefCell<RippSession>>,
    pub viewer2d:              Rc<RefCell<Viewer2dRenderer>>,
    pub panscan_viewer:        Rc<RefCell<Viewer2dRenderer>>,
    pub cam:                   CameraHandle,
    pub last_camera_frame:     Arc<Mutex<Option<(Vec<u8>, u32, u32)>>>,
    pub live_running:          Arc<AtomicBool>,
    pub props_refreshing:      Arc<AtomicBool>,
    pub prev_tab_idx:          Rc<RefCell<usize>>,
    pub prev_right_top_idx:    Rc<RefCell<usize>>,
    pub prev_right_bottom_idx: Rc<RefCell<usize>>,
    pub cwd:                   Rc<RefCell<PathBuf>>,
    pub tab_types:             Vec<Box<dyn TabType>>,
}

impl AppLogic {
    pub fn new(cam: CameraHandle) -> Self {
        let tab_types: Vec<Box<dyn TabType>> = vec![
            Box::new(panes::viewer3d::TabTypeViewer3d),
            Box::new(panes::viewer2d::TabTypeViewer2d),
            Box::new(panes::camera_view::TabTypeCamera),
            Box::new(panes::cam_prop::TabTypeCamProp),
            Box::new(panes::particle_tracking::TabTypeParticleTracking),
            Box::new(panes::project::TabTypeProject),
            Box::new(panes::file_browser::TabTypeFileBrowser),
            Box::new(panes::plots::TabTypePlots),
            Box::new(panes::help::TabTypeHelp),
            Box::new(panes::pan_scan::TabTypePanScan),
        ];

        let mut session = RippSession::new();
        session.add_project("Demo Project");

        // Populate startup tabs from TabType::visible_on_startup()
        for tt in &tab_types {
            if tt.visible_on_startup() {
                session.tabs_mut(tt.default_location()).push(tt.create());
            }
        }

        let cwd = std::fs::canonicalize(".").unwrap_or_else(|_| PathBuf::from("."));
        Self {
            session:           Rc::new(RefCell::new(session)),
            viewer2d:          Rc::new(RefCell::new(Viewer2dRenderer::new())),
            panscan_viewer:    Rc::new(RefCell::new(Viewer2dRenderer::new())),
            cam,
            last_camera_frame: Arc::new(Mutex::new(None)),
            live_running:           Arc::new(AtomicBool::new(false)),
            props_refreshing:       Arc::new(AtomicBool::new(false)),
            prev_tab_idx:           Rc::new(RefCell::new(0)),
            prev_right_top_idx:     Rc::new(RefCell::new(0)),
            prev_right_bottom_idx:  Rc::new(RefCell::new(0)),
            cwd:                    Rc::new(RefCell::new(cwd)),
            tab_types,
        }
    }

    pub fn register_all<F: Fn() + 'static>(&self, app: &AppWindow, start_live: F) {
        let cam1  = self.cam.clone();
        let cam2  = self.cam.clone();
        let cam3  = self.cam.clone();
        let weak1 = app.as_weak();
        let weak2 = app.as_weak();
        let weak3 = app.as_weak();

        let refresh_props = |cam: &CameraHandle, ui: &AppWindow| {
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

        let ctx = CallbackCtx {
            session:           self.session.clone(),
            viewer2d:          self.viewer2d.clone(),
            panscan_viewer:    self.panscan_viewer.clone(),
            cam:               self.cam.clone(),
            live_running:      self.live_running.clone(),
            start_live:        Rc::new(start_live),
            add_demo_camera,
            add_sim_camera,
            disconnect_all,
            cwd:               self.cwd.clone(),
            last_camera_frame: self.last_camera_frame.clone(),
        };

        // Tab lifecycle (close, activate, move, add-pane)
        panes::tabs::register(
            app, &self.session, &self.tab_types, &ctx,
            &self.prev_tab_idx, &self.prev_right_top_idx, &self.prev_right_bottom_idx,
        );

        // Per-type callbacks
        for tt in &self.tab_types {
            tt.register_callbacks(app, &ctx);
        }

        app.on_quit(|| std::process::exit(0));
    }
}

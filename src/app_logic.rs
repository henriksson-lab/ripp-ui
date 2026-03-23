use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, atomic::AtomicBool};
use crate::{AppWindow, LeftTabEntry, ProjectTreeEntry};
use crate::session::{RippSession, flatten_session};
use crate::viewer2d::Viewer2dRenderer;
use crate::camera::CameraHandle;
use crate::panes;

// ── Shared model builders (used by multiple panes) ────────────────────────────

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

// ── AppLogic ──────────────────────────────────────────────────────────────────

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

    /// Register all shared Slint callbacks on `app`, delegating to per-pane modules.
    ///
    /// `start_live` is called when switching to a Camera tab that had live mode active.
    /// Pass the real live-loop starter on desktop; `|| {}` on the server.
    pub fn register_all<F: Fn() + 'static>(&self, app: &AppWindow, start_live: F) {
        panes::tabs::register(app, &self.session, &self.viewer2d,
                              &self.live_running, &self.prev_tab_idx, start_live);
        panes::viewer3d::register(app, &self.session);
        panes::viewer2d::register(app, &self.session, &self.viewer2d);
        panes::camera_view::register(app, &self.cam, &self.session, &self.last_camera_frame);
        panes::project::register(app, &self.session);
        panes::file_browser::register(app, &self.session, &self.cwd);
        app.on_quit(|| std::process::exit(0));
    }
}

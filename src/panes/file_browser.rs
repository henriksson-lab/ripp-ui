use std::any::Any;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, atomic::AtomicBool};
use slint::ComponentHandle;
use crate::{AppWindow, FileBrowserGlobal, FileEntry, ProjectGlobal};
use crate::session::{TabFileBrowser, TabPane, TabType, CallbackCtx, ActivationContext, PaneLocation, BioformatsData, ProjectData};
use crate::app_logic::build_tree;

impl TabPane for TabFileBrowser {
    fn label(&self)            -> &str         { "Files" }
    fn type_id(&self)          -> i32          { 6 }
    fn default_location(&self) -> PaneLocation { PaneLocation::RightBottom }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
    fn as_any(&self)         -> &dyn Any     { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

// ── File system helpers ───────────────────────────────────────────────────────

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

// ── TabType ───────────────────────────────────────────────────────────────────

pub struct TabTypeFileBrowser;

impl TabType for TabTypeFileBrowser {
    fn type_id(&self)            -> i32          { 6 }
    fn label(&self)              -> &str         { "Files" }
    fn default_location(&self)   -> PaneLocation { PaneLocation::RightBottom }
    fn visible_on_startup(&self) -> bool         { true }
    fn create(&self)             -> Box<dyn TabPane> { Box::new(TabFileBrowser) }
    fn register_callbacks(&self, app: &AppWindow, ctx: &CallbackCtx) {
        let session = ctx.session.clone();
        let cwd     = ctx.cwd.clone();

        // ── Initial state ─────────────────────────────────────────────────────
        app.global::<FileBrowserGlobal>().set_current_path(cwd.borrow().to_string_lossy().to_string().into());
        app.global::<FileBrowserGlobal>().set_file_list(Rc::new(slint::VecModel::from(load_dir(&cwd.borrow()))).into());

        // ── Callbacks ─────────────────────────────────────────────────────────
        app.global::<FileBrowserGlobal>().on_navigate_to({
            let cwd      = cwd.clone();
            let app_weak = app.as_weak();
            move |segment| {
                let mut cwd = cwd.borrow_mut();
                if segment == ".." { cwd.pop(); } else { cwd.push(segment.as_str()); }
                let entries = load_dir(&cwd);
                if let Some(ui) = app_weak.upgrade() {
                    ui.global::<FileBrowserGlobal>().set_current_path(cwd.to_string_lossy().to_string().into());
                    ui.global::<FileBrowserGlobal>().set_file_list(Rc::new(slint::VecModel::from(entries)).into());
                }
            }
        });

        app.global::<FileBrowserGlobal>().on_open_file({
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
                            ui.global::<ProjectGlobal>().set_project_tree(build_tree(&session.borrow()));
                            ui.set_active_right_bottom_tab(0);
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
    }
}

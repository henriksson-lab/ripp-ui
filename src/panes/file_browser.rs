use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use slint::ComponentHandle;
use crate::{AppWindow, FileEntry};
use crate::session::{RippSession, BioformatsData, ProjectData};
use crate::app_logic::build_tree;

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

// ── Callback registration ─────────────────────────────────────────────────────

pub fn register(
    app: &AppWindow,
    session: &Rc<RefCell<RippSession>>,
    cwd: &Rc<RefCell<PathBuf>>,
) {
    // ── Initial state ─────────────────────────────────────────────────────────
    app.set_current_path(cwd.borrow().to_string_lossy().to_string().into());
    app.set_file_list(Rc::new(slint::VecModel::from(load_dir(&cwd.borrow()))).into());

    // ── Callbacks ─────────────────────────────────────────────────────────────
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
}

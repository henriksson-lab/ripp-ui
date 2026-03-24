use std::cell::RefCell;
use std::rc::Rc;
use slint::ComponentHandle;
use crate::AppWindow;
use crate::session::RippSession;
use crate::app_logic::{build_tree, build_tabs};

pub fn register(app: &AppWindow, session: &Rc<RefCell<RippSession>>) {
    // ── Initial state ─────────────────────────────────────────────────────────
    app.set_project_tree(build_tree(&session.borrow()));

    // ── Callbacks ─────────────────────────────────────────────────────────────
    app.on_new_project({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move || {
            session.borrow_mut().add_project("New Project");
            if let Some(ui) = app_weak.upgrade() {
                ui.set_project_tree(build_tree(&session.borrow()));
            }
        }
    });

    app.on_project_tree_selected(|_object_id| {});

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
                    // Rebuild left tabs in case open files reference the removed project
                    ui.set_left_tabs(build_tabs(&session.borrow().tabs_left));
                }
            }
        }
    });
}

use std::any::Any;
use std::sync::{Arc, atomic::AtomicBool};
use slint::ComponentHandle;
use crate::AppWindow;
use crate::session::{TabProject, TabPane, TabType, CallbackCtx, ActivationContext, PaneLocation};
use crate::app_logic::{build_tree, build_tabs};

impl TabPane for TabProject {
    fn label(&self)            -> &str         { "Project" }
    fn type_id(&self)          -> i32          { 5 }
    fn default_location(&self) -> PaneLocation { PaneLocation::RightBottom }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
    fn as_any(&self)         -> &dyn Any     { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

pub struct TabTypeProject;

impl TabType for TabTypeProject {
    fn type_id(&self)            -> i32          { 5 }
    fn label(&self)              -> &str         { "Project" }
    fn default_location(&self)   -> PaneLocation { PaneLocation::RightBottom }
    fn visible_on_startup(&self) -> bool         { true }
    fn create(&self)             -> Box<dyn TabPane> { Box::new(TabProject) }
    fn register_callbacks(&self, app: &AppWindow, ctx: &CallbackCtx) {
        let session = ctx.session.clone();

        // ── Initial state ─────────────────────────────────────────────────────
        app.set_project_tree(build_tree(&session.borrow()));

        // ── Callbacks ─────────────────────────────────────────────────────────
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
                        ui.set_left_tabs(build_tabs(&session.borrow().tabs_left));
                    }
                }
            }
        });
    }
}

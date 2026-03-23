use std::cell::RefCell;
use std::rc::Rc;
use slint::ComponentHandle;
use crate::AppWindow;
use std::sync::{Arc, atomic::AtomicBool};
use crate::session::{RippSession, RippTab, Tab3d, TabPane, ActivationContext};

impl TabPane for Tab3d {
    fn label(&self)   -> &str { "3D View" }
    fn type_id(&self) -> i32  { 0 }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
}

pub fn register(app: &AppWindow, session: &Rc<RefCell<RippSession>>) {
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
}

use std::any::Any;
use slint::ComponentHandle;
use crate::AppWindow;
use std::sync::{Arc, atomic::AtomicBool};
use crate::session::{Tab3d, Camera3d, TabPane, TabType, CallbackCtx, ActivationContext, PaneLocation};
use crate::renderer3d::bounding_sphere_radius;

impl TabPane for Tab3d {
    fn label(&self)            -> &str         { "3D View" }
    fn type_id(&self)          -> i32          { 0 }
    fn default_location(&self) -> PaneLocation { PaneLocation::Left }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
    fn as_any(&self)         -> &dyn Any       { self }
    fn as_any_mut(&mut self) -> &mut dyn Any   { self }
}

pub struct TabTypeViewer3d;

impl TabType for TabTypeViewer3d {
    fn type_id(&self)            -> i32          { 0 }
    fn label(&self)              -> &str         { "3D View" }
    fn default_location(&self)   -> PaneLocation { PaneLocation::Left }
    fn visible_on_startup(&self) -> bool         { true }
    fn create(&self) -> Box<dyn TabPane> {
        // Fit the bounding sphere in the vertical FOV (π/4 rad = 45°).
        let r   = bounding_sphere_radius("assets/teapot.obj");
        let fov_half = std::f32::consts::FRAC_PI_4 / 2.0; // half of 45°
        let distance = (r / fov_half.sin()) * 1.1;        // +10% margin
        Box::new(Tab3d { camera: Camera3d { yaw: 0.0, pitch: 0.3, distance } })
    }
    fn register_callbacks(&self, app: &AppWindow, ctx: &CallbackCtx) {
        let session  = ctx.session.clone();
        let app_weak = app.as_weak();
        app.on_viewer3d_panned({
            let session  = session.clone();
            let app_weak = app_weak.clone();
            move |dx, dy| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let mut s = session.borrow_mut();
                    if let Some(t) = s.tabs_left.get_mut(tab_idx) {
                        if let Some(t3) = t.as_any_mut().downcast_mut::<Tab3d>() {
                            t3.camera.yaw   -= dx * 0.005;
                            t3.camera.pitch  = (t3.camera.pitch + dy * 0.005).clamp(-1.5, 1.5);
                        }
                    }
                }
            }
        });

        app.on_viewer3d_scrolled({
            move |delta| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let mut s = session.borrow_mut();
                    if let Some(t) = s.tabs_left.get_mut(tab_idx) {
                        if let Some(t3) = t.as_any_mut().downcast_mut::<Tab3d>() {
                            t3.camera.distance = (t3.camera.distance * (-(delta * 0.005_f32)).exp())
                                .clamp(0.5, 100.0);
                        }
                    }
                }
            }
        });
    }
}

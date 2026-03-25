use std::any::Any;
use std::rc::Rc;
use std::sync::Arc;
use slint::ComponentHandle;
use crate::{AppWindow, DevicePropEntry};
use std::sync::atomic::{AtomicBool, Ordering};
use crate::session::{TabCamera, ColorMappingRange, TabPane, TabType, CallbackCtx, ActivationContext, PaneLocation};
use crate::micromanager::CameraImage;

impl TabPane for TabCamera {
    fn label(&self)            -> &str         { "Camera" }
    fn type_id(&self)          -> i32          { 2 }
    fn default_location(&self) -> PaneLocation { PaneLocation::Left }
    fn on_deactivating(&mut self, live_running: &Arc<AtomicBool>) {
        self.live = live_running.load(Ordering::SeqCst);
        live_running.store(false, Ordering::SeqCst);
    }
    fn on_activated(&self, ui: &AppWindow, ctx: &ActivationContext) {
        ui.set_live_snap(self.live);
        ui.set_camera_lo(self.color.lo);
        ui.set_camera_hi(self.color.hi);
        if self.live { (ctx.start_live)(); }
    }
    fn as_any(&self)         -> &dyn Any     { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

pub struct TabTypeCamera;

impl TabType for TabTypeCamera {
    fn type_id(&self)            -> i32          { 2 }
    fn label(&self)              -> &str         { "Camera" }
    fn default_location(&self)   -> PaneLocation { PaneLocation::Left }
    fn visible_on_startup(&self) -> bool         { false }
    fn create(&self) -> Box<dyn TabPane> {
        Box::new(TabCamera { live: false, color: ColorMappingRange::default() })
    }
    fn register_callbacks(&self, app: &AppWindow, ctx: &CallbackCtx) {
        let session            = ctx.session.clone();
        let cam                = ctx.cam.clone();
        let last_camera_frame  = ctx.last_camera_frame.clone();

        // ── Initial state ─────────────────────────────────────────────────────
        let snap = cam.snap();
        *last_camera_frame.lock().unwrap() = Some((snap.data.clone(), snap.width, snap.height));
        app.set_camera_image(snap.to_slint_image(ColorMappingRange::default()));

        let rows: Vec<DevicePropEntry> = cam.device_props().into_iter().map(|p| {
            DevicePropEntry { device: p.device.into(), property: p.property.into(), value: p.value.into() }
        }).collect();
        app.set_device_props(Rc::new(slint::VecModel::from(rows)).into());

        // ── Callbacks ─────────────────────────────────────────────────────────
        app.on_camera_settings_changed({
            let session  = session.clone();
            let lcf      = last_camera_frame.clone();
            let app_weak = app.as_weak();
            move || {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let color = ColorMappingRange { lo: ui.get_camera_lo(), hi: ui.get_camera_hi() };
                    {
                        let mut s = session.borrow_mut();
                        if let Some(t) = s.tabs_left.get_mut(tab_idx) {
                            if let Some(tc) = t.as_any_mut().downcast_mut::<TabCamera>() {
                                tc.color = color;
                            }
                        }
                    }
                    if let Some((ref data, w, h)) = *lcf.lock().unwrap() {
                        let img = CameraImage { data: data.clone(), width: w, height: h };
                        ui.set_camera_image(img.to_slint_image(color));
                    }
                }
            }
        });
    }
}

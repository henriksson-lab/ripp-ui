use std::sync::{Arc, atomic::AtomicBool};
use crate::AppWindow;
use crate::session::{TabCamProp, TabPane, ActivationContext, PaneLocation};

impl TabPane for TabCamProp {
    fn label(&self)            -> &str         { "Cam prop" }
    fn type_id(&self)          -> i32          { 3 }
    fn default_location(&self) -> PaneLocation { PaneLocation::RightTop }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
    fn menu_actions(&self) -> Vec<(String, i32)> {
        vec![
            ("Add demo microscope".into(), 1),
            ("Add simulated microscope".into(), 2),
            ("Disconnect all hardware".into(), 3),
        ]
    }
    fn on_menu_action(&mut self, action_id: i32, _ui: &AppWindow, ctx: &ActivationContext) {
        match action_id {
            1 => { if let Some(f) = &ctx.add_demo_camera { f(); } }
            2 => { if let Some(f) = &ctx.add_sim_camera  { f(); } }
            3 => { if let Some(f) = &ctx.disconnect_all  { f(); } }
            _ => {}
        }
    }
}

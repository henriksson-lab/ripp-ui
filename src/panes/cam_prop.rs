use std::sync::{Arc, atomic::AtomicBool};
use crate::AppWindow;
use crate::session::{TabCamProp, TabPane, ActivationContext, PaneLocation};

impl TabPane for TabCamProp {
    fn label(&self)            -> &str         { "Cam prop" }
    fn type_id(&self)          -> i32          { 3 }
    fn default_location(&self) -> PaneLocation { PaneLocation::RightTop }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
}

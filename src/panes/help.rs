use std::sync::{Arc, atomic::AtomicBool};
use crate::AppWindow;
use crate::session::{TabHelp, TabPane, ActivationContext, PaneLocation};

impl TabPane for TabHelp {
    fn label(&self)            -> &str         { "Help" }
    fn type_id(&self)          -> i32          { 8 }
    fn default_location(&self) -> PaneLocation { PaneLocation::RightBottom }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
}

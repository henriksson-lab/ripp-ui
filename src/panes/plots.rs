use std::sync::{Arc, atomic::AtomicBool};
use crate::AppWindow;
use crate::session::{TabPlots, TabPane, ActivationContext, PaneLocation};

impl TabPane for TabPlots {
    fn label(&self)            -> &str         { "Plots" }
    fn type_id(&self)          -> i32          { 7 }
    fn default_location(&self) -> PaneLocation { PaneLocation::RightBottom }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
}

use std::any::Any;
use std::sync::{Arc, atomic::AtomicBool};
use crate::AppWindow;
use crate::session::{TabPlots, TabPane, TabType, CallbackCtx, ActivationContext, PaneLocation};

impl TabPane for TabPlots {
    fn label(&self)            -> &str         { "Plots" }
    fn type_id(&self)          -> i32          { 7 }
    fn default_location(&self) -> PaneLocation { PaneLocation::RightBottom }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
    fn as_any(&self)         -> &dyn Any     { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

pub struct TabTypePlots;

impl TabType for TabTypePlots {
    fn type_id(&self)            -> i32          { 7 }
    fn label(&self)              -> &str         { "Plots" }
    fn default_location(&self)   -> PaneLocation { PaneLocation::RightBottom }
    fn visible_on_startup(&self) -> bool         { false }
    fn create(&self)             -> Box<dyn TabPane> { Box::new(TabPlots) }
    fn register_callbacks(&self, _app: &AppWindow, _ctx: &CallbackCtx) {}
}

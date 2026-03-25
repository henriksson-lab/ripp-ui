use std::any::Any;
use std::sync::{Arc, atomic::AtomicBool};
use crate::AppWindow;
use crate::session::{TabHelp, TabPane, TabType, CallbackCtx, ActivationContext, PaneLocation};

impl TabPane for TabHelp {
    fn label(&self)            -> &str         { "Help" }
    fn type_id(&self)          -> i32          { 8 }
    fn default_location(&self) -> PaneLocation { PaneLocation::RightBottom }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
    fn as_any(&self)         -> &dyn Any     { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

pub struct TabTypeHelp;

impl TabType for TabTypeHelp {
    fn type_id(&self)            -> i32          { 8 }
    fn label(&self)              -> &str         { "Help" }
    fn default_location(&self)   -> PaneLocation { PaneLocation::RightBottom }
    fn visible_on_startup(&self) -> bool         { false }
    fn create(&self)             -> Box<dyn TabPane> { Box::new(TabHelp) }
    fn register_callbacks(&self, _app: &AppWindow, _ctx: &CallbackCtx) {}
}

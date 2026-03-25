use std::any::Any;
use std::sync::{Arc, atomic::AtomicBool};
use crate::AppWindow;
use crate::session::{TabParticleTracking, TabPane, TabType, CallbackCtx, ActivationContext, PaneLocation};

impl TabPane for TabParticleTracking {
    fn label(&self)            -> &str         { "Part. tracking" }
    fn type_id(&self)          -> i32          { 4 }
    fn default_location(&self) -> PaneLocation { PaneLocation::RightTop }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
    fn as_any(&self)         -> &dyn Any     { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

pub struct TabTypeParticleTracking;

impl TabType for TabTypeParticleTracking {
    fn type_id(&self)            -> i32          { 4 }
    fn label(&self)              -> &str         { "Part. tracking" }
    fn default_location(&self)   -> PaneLocation { PaneLocation::RightTop }
    fn visible_on_startup(&self) -> bool         { false }
    fn create(&self)             -> Box<dyn TabPane> { Box::new(TabParticleTracking) }
    fn register_callbacks(&self, _app: &AppWindow, _ctx: &CallbackCtx) {}
}

use std::sync::{Arc, atomic::AtomicBool};
use crate::AppWindow;
use crate::session::{TabCamProp, TabParticleTracking, TabPane, ActivationContext};

impl TabPane for TabCamProp {
    fn label(&self)   -> &str { "Cam prop" }
    fn type_id(&self) -> i32  { 3 }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
}

impl TabPane for TabParticleTracking {
    fn label(&self)   -> &str { "Part. tracking" }
    fn type_id(&self) -> i32  { 4 }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
}

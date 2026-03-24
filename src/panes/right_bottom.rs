use std::sync::{Arc, atomic::AtomicBool};
use crate::AppWindow;
use crate::session::{TabProject, TabFileBrowser, TabPlots, TabHelp, TabPane, ActivationContext};

impl TabPane for TabProject {
    fn label(&self)   -> &str { "Project" }
    fn type_id(&self) -> i32  { 5 }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
}

impl TabPane for TabFileBrowser {
    fn label(&self)   -> &str { "Files" }
    fn type_id(&self) -> i32  { 6 }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
}

impl TabPane for TabPlots {
    fn label(&self)   -> &str { "Plots" }
    fn type_id(&self) -> i32  { 7 }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
}

impl TabPane for TabHelp {
    fn label(&self)   -> &str { "Help" }
    fn type_id(&self) -> i32  { 8 }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, _: &AppWindow, _: &ActivationContext) {}
}

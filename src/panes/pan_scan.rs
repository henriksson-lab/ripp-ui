use std::any::Any;
use std::sync::{Arc, atomic::AtomicBool};
use slint::ComponentHandle;
use crate::{AppWindow, PanScanGlobal};
use crate::session::{
    TabPanScan, Camera2d, ColorMappingRange,
    TabPane, TabType, CallbackCtx, ActivationContext, PaneLocation,
};
use crate::renderer2d::Viewer2dRenderer;

// ── Image generation ──────────────────────────────────────────────────────────

fn make_sin_image() -> Vec<u8> {
    (0..512_u32).flat_map(|y| (0..512_u32).map(move |x| {
        let v = (x as f32 / 50.0).sin() + (y as f32 / 50.0).sin();
        (v.abs() * 127.5) as u8
    })).collect()
}

// ── Render helper ─────────────────────────────────────────────────────────────

fn render_panscan(panscan_viewer: &Viewer2dRenderer, cam: Camera2d, color: ColorMappingRange, ui: &AppWindow) {
    if let Some(pixels) = panscan_viewer.render(cam, color) {
        let sz = panscan_viewer.size();
        let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(sz.w, sz.h);
        pb.make_mut_bytes().copy_from_slice(&pixels);
        ui.global::<PanScanGlobal>().set_panscan_image(slint::Image::from_rgba8(pb));
    }
}

// ── TabPane impl ──────────────────────────────────────────────────────────────

impl TabPane for TabPanScan {
    fn label(&self)            -> &str         { "Panorama scan" }
    fn type_id(&self)          -> i32          { 9 }
    fn default_location(&self) -> PaneLocation { PaneLocation::Left }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, ui: &AppWindow, ctx: &ActivationContext) {
        if !self.uploaded {
            ctx.panscan_viewer.borrow_mut().upload(&make_sin_image(), 512, 512, true);
        }
        ui.global::<PanScanGlobal>().set_panscan_lo(self.color.lo);
        ui.global::<PanScanGlobal>().set_panscan_hi(self.color.hi);
        ui.global::<PanScanGlobal>().set_panscan_min_x(self.min_x.clone().into());
        ui.global::<PanScanGlobal>().set_panscan_max_x(self.max_x.clone().into());
        ui.global::<PanScanGlobal>().set_panscan_min_y(self.min_y.clone().into());
        ui.global::<PanScanGlobal>().set_panscan_max_y(self.max_y.clone().into());
        render_panscan(&ctx.panscan_viewer.borrow(),
                       Camera2d { x: self.camera.x, y: self.camera.y, zoom: self.camera.zoom },
                       self.color, ui);
    }
    fn as_any(&self)         -> &dyn Any     { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

// ── TabType ───────────────────────────────────────────────────────────────────

pub struct TabTypePanScan;

impl TabType for TabTypePanScan {
    fn type_id(&self)            -> i32          { 9 }
    fn label(&self)              -> &str         { "Panorama scan" }
    fn default_location(&self)   -> PaneLocation { PaneLocation::Left }
    fn visible_on_startup(&self) -> bool         { false }
    fn create(&self)             -> Box<dyn TabPane> { Box::new(TabPanScan::default()) }
    fn register_callbacks(&self, app: &AppWindow, ctx: &CallbackCtx) {
        let session        = ctx.session.clone();
        let panscan_viewer = ctx.panscan_viewer.clone();
        let cam            = ctx.cam.clone();

        // Upload the sin image once so it's ready before first activation.
        panscan_viewer.borrow_mut().upload(&make_sin_image(), 512, 512, true);
        {
            let mut s = session.borrow_mut();
            for t in &mut s.tabs_left         { if let Some(ps) = t.as_any_mut().downcast_mut::<TabPanScan>() { ps.uploaded = true; } }
            for t in &mut s.tabs_right_top    { if let Some(ps) = t.as_any_mut().downcast_mut::<TabPanScan>() { ps.uploaded = true; } }
            for t in &mut s.tabs_right_bottom { if let Some(ps) = t.as_any_mut().downcast_mut::<TabPanScan>() { ps.uploaded = true; } }
        }

        app.global::<PanScanGlobal>().on_panscan_panned({
            let session        = session.clone();
            let panscan_viewer = panscan_viewer.clone();
            let app_weak       = app.as_weak();
            move |dx, dy| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let color = ColorMappingRange {
                        lo: ui.global::<PanScanGlobal>().get_panscan_lo(),
                        hi: ui.global::<PanScanGlobal>().get_panscan_hi(),
                    };
                    let cam = {
                        let mut s = session.borrow_mut();
                        s.tabs_left.get_mut(tab_idx)
                            .and_then(|t| t.as_any_mut().downcast_mut::<TabPanScan>())
                            .map(|t| {
                                t.camera.x -= dx as f64 / t.camera.zoom;
                                t.camera.y -= dy as f64 / t.camera.zoom;
                                Camera2d { x: t.camera.x, y: t.camera.y, zoom: t.camera.zoom }
                            })
                    };
                    if let Some(cam) = cam {
                        render_panscan(&panscan_viewer.borrow(), cam, color, &ui);
                    }
                }
            }
        });

        app.global::<PanScanGlobal>().on_panscan_scrolled({
            let session        = session.clone();
            let panscan_viewer = panscan_viewer.clone();
            let app_weak       = app.as_weak();
            move |delta| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let color = ColorMappingRange {
                        lo: ui.global::<PanScanGlobal>().get_panscan_lo(),
                        hi: ui.global::<PanScanGlobal>().get_panscan_hi(),
                    };
                    let cam = {
                        let mut s = session.borrow_mut();
                        s.tabs_left.get_mut(tab_idx)
                            .and_then(|t| t.as_any_mut().downcast_mut::<TabPanScan>())
                            .map(|t| {
                                t.camera.zoom *= (delta as f64 * 0.005_f64).exp();
                                t.camera.zoom = t.camera.zoom.clamp(0.01, 100.0);
                                Camera2d { x: t.camera.x, y: t.camera.y, zoom: t.camera.zoom }
                            })
                    };
                    if let Some(cam) = cam {
                        render_panscan(&panscan_viewer.borrow(), cam, color, &ui);
                    }
                }
            }
        });

        app.global::<PanScanGlobal>().on_panscan_settings_changed({
            let session        = session.clone();
            let panscan_viewer = panscan_viewer.clone();
            let app_weak       = app.as_weak();
            move || {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let color = ColorMappingRange {
                        lo: ui.global::<PanScanGlobal>().get_panscan_lo(),
                        hi: ui.global::<PanScanGlobal>().get_panscan_hi(),
                    };
                    let cam = {
                        let mut s = session.borrow_mut();
                        s.tabs_left.get_mut(tab_idx)
                            .and_then(|t| t.as_any_mut().downcast_mut::<TabPanScan>())
                            .map(|t| {
                                t.color = color;
                                Camera2d { x: t.camera.x, y: t.camera.y, zoom: t.camera.zoom }
                            })
                    };
                    if let Some(cam) = cam {
                        render_panscan(&panscan_viewer.borrow(), cam, color, &ui);
                    }
                }
            }
        });

        app.global::<PanScanGlobal>().on_panscan_field_edited({
            let session  = session.clone();
            let app_weak = app.as_weak();
            move |field, text| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let text = text.to_string();
                    let mut s = session.borrow_mut();
                    if let Some(t) = s.tabs_left.get_mut(tab_idx)
                        .and_then(|t| t.as_any_mut().downcast_mut::<TabPanScan>())
                    {
                        match field {
                            0 => t.min_x = text,
                            1 => t.max_x = text,
                            2 => t.min_y = text,
                            3 => t.max_y = text,
                            _ => {}
                        }
                    }
                }
            }
        });

        app.global::<PanScanGlobal>().on_panscan_set_corner({
            let session  = session.clone();
            let app_weak = app.as_weak();
            let cam      = cam.clone();
            move || {
                let Some((new_x, new_y)) = cam.get_xy_position() else { return; };
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let bounds = {
                        let s = session.borrow();
                        s.tabs_left.get(tab_idx)
                            .and_then(|t| t.as_any().downcast_ref::<TabPanScan>())
                            .map(|t| (t.min_x.clone(), t.max_x.clone(), t.min_y.clone(), t.max_y.clone()))
                    };
                    let Some((min_x, max_x, min_y, max_y)) = bounds else { return; };
                    let s_min_x = format!("{:.3}", expand_min(min_x.parse().ok(), new_x));
                    let s_max_x = format!("{:.3}", expand_max(max_x.parse().ok(), new_x));
                    let s_min_y = format!("{:.3}", expand_min(min_y.parse().ok(), new_y));
                    let s_max_y = format!("{:.3}", expand_max(max_y.parse().ok(), new_y));
                    {
                        let mut s = session.borrow_mut();
                        if let Some(t) = s.tabs_left.get_mut(tab_idx)
                            .and_then(|t| t.as_any_mut().downcast_mut::<TabPanScan>())
                        {
                            t.min_x = s_min_x.clone();
                            t.max_x = s_max_x.clone();
                            t.min_y = s_min_y.clone();
                            t.max_y = s_max_y.clone();
                        }
                    }
                    ui.global::<PanScanGlobal>().set_panscan_min_x(s_min_x.into());
                    ui.global::<PanScanGlobal>().set_panscan_max_x(s_max_x.into());
                    ui.global::<PanScanGlobal>().set_panscan_min_y(s_min_y.into());
                    ui.global::<PanScanGlobal>().set_panscan_max_y(s_max_y.into());
                }
            }
        });

        app.global::<PanScanGlobal>().on_panscan_reset({
            let session  = session.clone();
            let app_weak = app.as_weak();
            move || {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    {
                        let mut s = session.borrow_mut();
                        if let Some(t) = s.tabs_left.get_mut(tab_idx)
                            .and_then(|t| t.as_any_mut().downcast_mut::<TabPanScan>())
                        {
                            t.min_x.clear(); t.max_x.clear();
                            t.min_y.clear(); t.max_y.clear();
                        }
                    }
                    ui.global::<PanScanGlobal>().set_panscan_min_x("".into());
                    ui.global::<PanScanGlobal>().set_panscan_max_x("".into());
                    ui.global::<PanScanGlobal>().set_panscan_min_y("".into());
                    ui.global::<PanScanGlobal>().set_panscan_max_y("".into());
                }
            }
        });

        app.global::<PanScanGlobal>().on_panscan_record(|| {});
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn expand_min(current: Option<f64>, new_val: f64) -> f64 {
    match current { Some(v) => v.min(new_val), None => new_val }
}

fn expand_max(current: Option<f64>, new_val: f64) -> f64 {
    match current { Some(v) => v.max(new_val), None => new_val }
}

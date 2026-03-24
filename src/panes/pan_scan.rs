use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, atomic::AtomicBool};
use slint::ComponentHandle;
use crate::AppWindow;
use crate::session::{
    RippSession, RippTab, TabPanScan, Camera2d, ColorMappingRange,
    TabPane, ActivationContext, PaneLocation,
};
use crate::renderer2d::Viewer2dRenderer;
use crate::micromanager::CameraHandle;

// ── Image generation ──────────────────────────────────────────────────────────

fn make_sin_image() -> Vec<u8> {
    (0..512_u32).flat_map(|y| (0..512_u32).map(move |x| {
        let v = (x as f32 / 50.0).sin() + (y as f32 / 50.0).sin();
        (v.abs() * 127.5) as u8
    })).collect()
}

// ── Render helper ─────────────────────────────────────────────────────────────

fn render_panscan(
    session:        &Rc<RefCell<RippSession>>,
    panscan_viewer: &Rc<RefCell<Viewer2dRenderer>>,
    tab_idx:        usize,
    color:          ColorMappingRange,
    ui:             &AppWindow,
) {
    let cam = {
        let s = session.borrow();
        match s.tabs_left.get(tab_idx) {
            Some(RippTab::PanScan(t)) => Camera2d { x: t.camera.x, y: t.camera.y, zoom: t.camera.zoom },
            _ => return,
        }
    };
    if let Some(pixels) = panscan_viewer.borrow().render(cam, color) {
        let sz = panscan_viewer.borrow().size();
        let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(sz.w, sz.h);
        pb.make_mut_bytes().copy_from_slice(&pixels);
        ui.set_panscan_image(slint::Image::from_rgba8(pb));
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
        ui.set_panscan_lo(self.color.lo);
        ui.set_panscan_hi(self.color.hi);
        ui.set_panscan_min_x(self.min_x.clone().into());
        ui.set_panscan_max_x(self.max_x.clone().into());
        ui.set_panscan_min_y(self.min_y.clone().into());
        ui.set_panscan_max_y(self.max_y.clone().into());
        render_panscan(&ctx.session, &ctx.panscan_viewer, ctx.tab_idx, self.color, ui);
    }
}

// Note: on_activated takes &self but we need to set uploaded. We handle the
// upload lazily inside register callbacks instead — see on_activated note below.

// ── Callback registration ─────────────────────────────────────────────────────

pub fn register(
    app:            &AppWindow,
    session:        &Rc<RefCell<RippSession>>,
    panscan_viewer: &Rc<RefCell<Viewer2dRenderer>>,
    cam:            &CameraHandle,
) {
    // Upload the sin image once up front so it's ready before first activation.
    panscan_viewer.borrow_mut().upload(&make_sin_image(), 512, 512, true);
    {
        let mut s = session.borrow_mut();
        for t in &mut s.tabs_left         { if let RippTab::PanScan(ps) = t { ps.uploaded = true; } }
        for t in &mut s.tabs_right_top    { if let RippTab::PanScan(ps) = t { ps.uploaded = true; } }
        for t in &mut s.tabs_right_bottom { if let RippTab::PanScan(ps) = t { ps.uploaded = true; } }
    }

    app.on_panscan_panned({
        let session        = session.clone();
        let panscan_viewer = panscan_viewer.clone();
        let app_weak       = app.as_weak();
        move |dx, dy| {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                {
                    let mut s = session.borrow_mut();
                    if let Some(RippTab::PanScan(t)) = s.tabs_left.get_mut(tab_idx) {
                        t.camera.x -= dx as f64 / t.camera.zoom;
                        t.camera.y -= dy as f64 / t.camera.zoom;
                    }
                }
                let color = ColorMappingRange { lo: ui.get_panscan_lo(), hi: ui.get_panscan_hi() };
                render_panscan(&session, &panscan_viewer, tab_idx, color, &ui);
            }
        }
    });

    app.on_panscan_scrolled({
        let session        = session.clone();
        let panscan_viewer = panscan_viewer.clone();
        let app_weak       = app.as_weak();
        move |delta| {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                {
                    let mut s = session.borrow_mut();
                    if let Some(RippTab::PanScan(t)) = s.tabs_left.get_mut(tab_idx) {
                        t.camera.zoom *= (delta as f64 * 0.005_f64).exp();
                        t.camera.zoom = t.camera.zoom.clamp(0.01, 100.0);
                    }
                }
                let color = ColorMappingRange { lo: ui.get_panscan_lo(), hi: ui.get_panscan_hi() };
                render_panscan(&session, &panscan_viewer, tab_idx, color, &ui);
            }
        }
    });

    app.on_panscan_settings_changed({
        let session        = session.clone();
        let panscan_viewer = panscan_viewer.clone();
        let app_weak       = app.as_weak();
        move || {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                let color = ColorMappingRange { lo: ui.get_panscan_lo(), hi: ui.get_panscan_hi() };
                {
                    let mut s = session.borrow_mut();
                    if let Some(RippTab::PanScan(t)) = s.tabs_left.get_mut(tab_idx) {
                        t.color = color;
                    }
                }
                render_panscan(&session, &panscan_viewer, tab_idx, color, &ui);
            }
        }
    });

    app.on_panscan_field_edited({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move |field, text| {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                let text = text.to_string();
                let mut s = session.borrow_mut();
                if let Some(RippTab::PanScan(t)) = s.tabs_left.get_mut(tab_idx) {
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

    app.on_panscan_set_corner({
        let session  = session.clone();
        let app_weak = app.as_weak();
        let cam      = cam.clone();
        move || {
            let Some((new_x, new_y)) = cam.get_xy_position() else { return; };
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;

                let (min_x, max_x, min_y, max_y) = {
                    let s = session.borrow();
                    if let Some(RippTab::PanScan(t)) = s.tabs_left.get(tab_idx) {
                        (t.min_x.clone(), t.max_x.clone(), t.min_y.clone(), t.max_y.clone())
                    } else { return; }
                };

                let new_min_x = expand_min(min_x.parse().ok(), new_x);
                let new_max_x = expand_max(max_x.parse().ok(), new_x);
                let new_min_y = expand_min(min_y.parse().ok(), new_y);
                let new_max_y = expand_max(max_y.parse().ok(), new_y);

                let fmt = |v: f64| format!("{:.3}", v);
                let s_min_x = fmt(new_min_x);
                let s_max_x = fmt(new_max_x);
                let s_min_y = fmt(new_min_y);
                let s_max_y = fmt(new_max_y);

                {
                    let mut s = session.borrow_mut();
                    if let Some(RippTab::PanScan(t)) = s.tabs_left.get_mut(tab_idx) {
                        t.min_x = s_min_x.clone();
                        t.max_x = s_max_x.clone();
                        t.min_y = s_min_y.clone();
                        t.max_y = s_max_y.clone();
                    }
                }

                ui.set_panscan_min_x(s_min_x.into());
                ui.set_panscan_max_x(s_max_x.into());
                ui.set_panscan_min_y(s_min_y.into());
                ui.set_panscan_max_y(s_max_y.into());
            }
        }
    });

    app.on_panscan_reset({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move || {
            if let Some(ui) = app_weak.upgrade() {
                let tab_idx = ui.get_active_left_tab() as usize;
                {
                    let mut s = session.borrow_mut();
                    if let Some(RippTab::PanScan(t)) = s.tabs_left.get_mut(tab_idx) {
                        t.min_x.clear(); t.max_x.clear();
                        t.min_y.clear(); t.max_y.clear();
                    }
                }
                ui.set_panscan_min_x("".into());
                ui.set_panscan_max_x("".into());
                ui.set_panscan_min_y("".into());
                ui.set_panscan_max_y("".into());
            }
        }
    });

    app.on_panscan_record(|| {
        // stub — does nothing yet
    });
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn expand_min(current: Option<f64>, new_val: f64) -> f64 {
    match current {
        Some(v) => v.min(new_val),
        None    => new_val,
    }
}

fn expand_max(current: Option<f64>, new_val: f64) -> f64 {
    match current {
        Some(v) => v.max(new_val),
        None    => new_val,
    }
}

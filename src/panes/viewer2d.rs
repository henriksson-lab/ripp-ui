use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;
use slint::ComponentHandle;
use crate::AppWindow;
use std::sync::{Arc, atomic::AtomicBool};
use crate::session::{RippSession, Tab2d, ProjectData, Camera2d, ColorMappingRange, TabPane, TabType, CallbackCtx, ActivationContext, PaneLocation, find_object_ref, find_object_mut};
use crate::renderer2d::Viewer2dRenderer;

// ── GPU helpers ───────────────────────────────────────────────────────────────

/// Upload image from session project into the renderer. `proj_id` must be ≥ 0.
pub fn upload(
    session:  &Rc<RefCell<RippSession>>,
    viewer2d: &mut Viewer2dRenderer,
    proj_id:  u32,
    obj_id:   u32,
    z:        u32,
) {
    let mut s = session.borrow_mut();
    if let Some(proj) = s.projects.get_mut(&proj_id) {
        if let Some(obj) = find_object_mut(&mut proj.root, obj_id) {
            if let ProjectData::Bioformats(bf) = &mut obj.data {
                let meta = bf.reader.metadata();
                let w = meta.size_x;
                let h = meta.size_y;
                if let Ok(bytes) = bf.reader.open_bytes(z) {
                    let is_gray = bytes.len() == (w * h) as usize;
                    let is_rgb  = bytes.len() == (w * h * 3) as usize;
                    if is_gray || is_rgb {
                        viewer2d.upload(&bytes, w, h, is_gray);
                    }
                }
            }
        }
    }
}

pub fn render(viewer2d: &Viewer2dRenderer, cam: Camera2d, color: ColorMappingRange, ui: &AppWindow) {
    if let Some(pixels) = viewer2d.render(cam, color) {
        let sz = viewer2d.size();
        let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(sz.w, sz.h);
        pb.make_mut_bytes().copy_from_slice(&pixels);
        ui.set_viewer2d_image(slint::Image::from_rgba8(pb));
        ui.set_viewer2d_image_loaded(true);
    }
}

// ── TabPane impl ──────────────────────────────────────────────────────────────

impl TabPane for Tab2d {
    fn label(&self)            -> &str         { "2D Viewer" }
    fn type_id(&self)          -> i32          { 1 }
    fn default_location(&self) -> PaneLocation { PaneLocation::Left }
    fn on_deactivating(&mut self, _: &Arc<AtomicBool>) {}
    fn on_activated(&self, ui: &AppWindow, ctx: &ActivationContext) {
        ui.set_viewer2d_lo(self.color.lo);
        ui.set_viewer2d_hi(self.color.hi);
        ui.set_viewer2d_z(self.camera.z as f32);
        ui.set_viewer2d_z_max(self.z_max as f32);
        if self.selected_proj_id < 0 {
            ui.set_viewer2d_image_loaded(false);
        } else {
            upload(&ctx.session, &mut ctx.viewer2d.borrow_mut(),
                   self.selected_proj_id as u32, self.selected_obj_id as u32,
                   self.camera.z as u32);
            render(&ctx.viewer2d.borrow(),
                   Camera2d { x: self.camera.x, y: self.camera.y, zoom: self.camera.zoom },
                   self.color, ui);
        }
    }
    fn as_any(&self)         -> &dyn Any     { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

// ── TabType ───────────────────────────────────────────────────────────────────

pub struct TabTypeViewer2d;

impl TabType for TabTypeViewer2d {
    fn type_id(&self)            -> i32          { 1 }
    fn label(&self)              -> &str         { "2D Viewer" }
    fn default_location(&self)   -> PaneLocation { PaneLocation::Left }
    fn visible_on_startup(&self) -> bool         { true }
    fn create(&self)             -> Box<dyn TabPane> { Box::new(Tab2d::default()) }
    fn register_callbacks(&self, app: &AppWindow, ctx: &CallbackCtx) {
        let session  = ctx.session.clone();
        let viewer2d = ctx.viewer2d.clone();

        app.on_viewer2d_object_selected({
            let session  = session.clone();
            let viewer2d = viewer2d.clone();
            let app_weak = app.as_weak();
            move |project_id, object_id| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let (img_w, img_h, z_max) = {
                        let s = session.borrow();
                        if let Some(proj) = s.projects.get(&(project_id as u32)) {
                            if let Some(obj) = find_object_ref(&proj.root, object_id as u32) {
                                if let ProjectData::Bioformats(bf) = &obj.data {
                                    let meta = bf.reader.metadata();
                                    (meta.size_x as f64, meta.size_y as f64,
                                     (meta.size_z as i32 - 1).max(0))
                                } else { (0.0, 0.0, 0) }
                            } else { (0.0, 0.0, 0) }
                        } else { (0.0, 0.0, 0) }
                    };
                    {
                        let mut s = session.borrow_mut();
                        if let Some(t) = s.tabs_left.get_mut(tab_idx) {
                            if let Some(t2d) = t.as_any_mut().downcast_mut::<Tab2d>() {
                                t2d.selected_proj_id = project_id;
                                t2d.selected_obj_id  = object_id;
                                t2d.z_max            = z_max;
                                t2d.camera.x    = img_w / 2.0;
                                t2d.camera.y    = img_h / 2.0;
                                t2d.camera.zoom = 1.0;
                                t2d.camera.z    = 0.0;
                            }
                        }
                    }
                    ui.set_viewer2d_z(0.0);
                    ui.set_viewer2d_z_max(z_max as f32);
                    let color = ColorMappingRange { lo: ui.get_viewer2d_lo(), hi: ui.get_viewer2d_hi() };
                    upload(&session, &mut viewer2d.borrow_mut(),
                           project_id as u32, object_id as u32, 0);
                    render(&viewer2d.borrow(),
                           Camera2d { x: img_w / 2.0, y: img_h / 2.0, zoom: 1.0 },
                           color, &ui);
                }
            }
        });

        app.on_viewer2d_panned({
            let session  = session.clone();
            let viewer2d = viewer2d.clone();
            let app_weak = app.as_weak();
            move |dx, dy| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let color = ColorMappingRange { lo: ui.get_viewer2d_lo(), hi: ui.get_viewer2d_hi() };
                    let cam = {
                        let mut s = session.borrow_mut();
                        if let Some(t) = s.tabs_left.get_mut(tab_idx) {
                            if let Some(t2d) = t.as_any_mut().downcast_mut::<Tab2d>() {
                                t2d.camera.x -= dx as f64 / t2d.camera.zoom;
                                t2d.camera.y -= dy as f64 / t2d.camera.zoom;
                                Some(Camera2d { x: t2d.camera.x, y: t2d.camera.y, zoom: t2d.camera.zoom })
                            } else { None }
                        } else { None }
                    };
                    if let Some(cam) = cam {
                        render(&viewer2d.borrow(), cam, color, &ui);
                    }
                }
            }
        });

        app.on_viewer2d_scrolled({
            let session  = session.clone();
            let viewer2d = viewer2d.clone();
            let app_weak = app.as_weak();
            move |delta| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let color = ColorMappingRange { lo: ui.get_viewer2d_lo(), hi: ui.get_viewer2d_hi() };
                    let cam = {
                        let mut s = session.borrow_mut();
                        if let Some(t) = s.tabs_left.get_mut(tab_idx) {
                            if let Some(t2d) = t.as_any_mut().downcast_mut::<Tab2d>() {
                                t2d.camera.zoom *= (delta as f64 * 0.005_f64).exp();
                                t2d.camera.zoom = t2d.camera.zoom.clamp(0.01, 100.0);
                                Some(Camera2d { x: t2d.camera.x, y: t2d.camera.y, zoom: t2d.camera.zoom })
                            } else { None }
                        } else { None }
                    };
                    if let Some(cam) = cam {
                        render(&viewer2d.borrow(), cam, color, &ui);
                    }
                }
            }
        });

        app.on_viewer2d_settings_changed({
            let session  = session.clone();
            let viewer2d = viewer2d.clone();
            let app_weak = app.as_weak();
            move || {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let color = ColorMappingRange { lo: ui.get_viewer2d_lo(), hi: ui.get_viewer2d_hi() };
                    let cam = {
                        let mut s = session.borrow_mut();
                        if let Some(t) = s.tabs_left.get_mut(tab_idx) {
                            if let Some(t2d) = t.as_any_mut().downcast_mut::<Tab2d>() {
                                t2d.color = color;
                                Some(Camera2d { x: t2d.camera.x, y: t2d.camera.y, zoom: t2d.camera.zoom })
                            } else { None }
                        } else { None }
                    };
                    if let Some(cam) = cam {
                        render(&viewer2d.borrow(), cam, color, &ui);
                    }
                }
            }
        });

        app.on_viewer2d_resized({
            let session  = session.clone();
            let viewer2d = viewer2d.clone();
            let app_weak = app.as_weak();
            move |w, h| {
                if w <= 0.0 || h <= 0.0 { return; }
                viewer2d.borrow_mut().resize(w as u32, h as u32);
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let color = ColorMappingRange { lo: ui.get_viewer2d_lo(), hi: ui.get_viewer2d_hi() };
                    let cam = {
                        let s = session.borrow();
                        s.tabs_left.get(tab_idx)
                            .and_then(|t| t.as_any().downcast_ref::<Tab2d>())
                            .map(|t2d| Camera2d { x: t2d.camera.x, y: t2d.camera.y, zoom: t2d.camera.zoom })
                    };
                    if let Some(cam) = cam {
                        render(&viewer2d.borrow(), cam, color, &ui);
                    }
                }
            }
        });

        app.on_viewer2d_z_changed({
            let session  = session.clone();
            let viewer2d = viewer2d.clone();
            let app_weak = app.as_weak();
            move |z| {
                if let Some(ui) = app_weak.upgrade() {
                    let tab_idx = ui.get_active_left_tab() as usize;
                    let color = ColorMappingRange { lo: ui.get_viewer2d_lo(), hi: ui.get_viewer2d_hi() };
                    let info = {
                        let mut s = session.borrow_mut();
                        if let Some(t) = s.tabs_left.get_mut(tab_idx) {
                            if let Some(t2d) = t.as_any_mut().downcast_mut::<Tab2d>() {
                                t2d.camera.z = z.round() as f64;
                                if t2d.selected_proj_id >= 0 {
                                    Some((t2d.selected_proj_id as u32, t2d.selected_obj_id as u32,
                                          t2d.camera.z as u32,
                                          Camera2d { x: t2d.camera.x, y: t2d.camera.y, zoom: t2d.camera.zoom }))
                                } else { None }
                            } else { None }
                        } else { None }
                    };
                    if let Some((proj_id, obj_id, z_u, cam)) = info {
                        upload(&session, &mut viewer2d.borrow_mut(), proj_id, obj_id, z_u);
                        render(&viewer2d.borrow(), cam, color, &ui);
                    }
                }
            }
        });
    }
}

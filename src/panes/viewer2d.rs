use std::cell::RefCell;
use std::rc::Rc;
use slint::ComponentHandle;
use crate::AppWindow;
use crate::session::{RippSession, RippTab, ProjectData, find_object_ref, find_object_mut};
use crate::viewer2d::Viewer2dRenderer;

// ── GPU helpers ───────────────────────────────────────────────────────────────

pub fn upload(
    session: &Rc<RefCell<RippSession>>,
    viewer2d: &mut Viewer2dRenderer,
    tab_idx: usize,
) {
    let (proj_id, obj_id, z) = {
        let s = session.borrow();
        match s.tabs.get(tab_idx) {
            Some(RippTab::Tab2d(t)) if t.selected_proj_id >= 0 =>
                (t.selected_proj_id, t.selected_obj_id, t.camera.z as u32),
            _ => return,
        }
    };
    let mut s = session.borrow_mut();
    if let Some(proj) = s.projects.get_mut(&(proj_id as u32)) {
        if let Some(obj) = find_object_mut(&mut proj.root, obj_id as u32) {
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

pub fn render(
    session: &Rc<RefCell<RippSession>>,
    viewer2d: &Viewer2dRenderer,
    tab_idx: usize,
    lo: f32,
    hi: f32,
    ui: &AppWindow,
) {
    let (cam_x, cam_y, zoom) = {
        let s = session.borrow();
        match s.tabs.get(tab_idx) {
            Some(RippTab::Tab2d(t)) => (t.camera.x, t.camera.y, t.camera.zoom),
            _ => return,
        }
    };
    if let Some(pixels) = viewer2d.render(cam_x, cam_y, zoom, lo, hi) {
        let w = viewer2d.out_w();
        let h = viewer2d.out_h();
        let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
        pb.make_mut_bytes().copy_from_slice(&pixels);
        ui.set_viewer2d_image(slint::Image::from_rgba8(pb));
        ui.set_viewer2d_image_loaded(true);
    }
}

// ── Callback registration ─────────────────────────────────────────────────────

pub fn register(
    app: &AppWindow,
    session: &Rc<RefCell<RippSession>>,
    viewer2d: &Rc<RefCell<Viewer2dRenderer>>,
) {
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
                    if let Some(RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                        t.selected_proj_id = project_id;
                        t.selected_obj_id  = object_id;
                        t.z_max            = z_max;
                        t.camera.x    = img_w / 2.0;
                        t.camera.y    = img_h / 2.0;
                        t.camera.zoom = 1.0;
                        t.camera.z    = 0.0;
                    }
                }
                ui.set_viewer2d_z(0.0);
                ui.set_viewer2d_z_max(z_max as f32);
                let lo = ui.get_viewer2d_lo();
                let hi = ui.get_viewer2d_hi();
                upload(&session, &mut viewer2d.borrow_mut(), tab_idx);
                render(&session, &viewer2d.borrow(), tab_idx, lo, hi, &ui);
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
                {
                    let mut s = session.borrow_mut();
                    if let Some(RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                        t.camera.x -= dx as f64 / t.camera.zoom;
                        t.camera.y -= dy as f64 / t.camera.zoom;
                    }
                }
                let lo = ui.get_viewer2d_lo();
                let hi = ui.get_viewer2d_hi();
                render(&session, &viewer2d.borrow(), tab_idx, lo, hi, &ui);
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
                {
                    let mut s = session.borrow_mut();
                    if let Some(RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                        t.camera.zoom *= (delta as f64 * 0.005_f64).exp();
                        t.camera.zoom = t.camera.zoom.clamp(0.01, 100.0);
                    }
                }
                let lo = ui.get_viewer2d_lo();
                let hi = ui.get_viewer2d_hi();
                render(&session, &viewer2d.borrow(), tab_idx, lo, hi, &ui);
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
                let lo = ui.get_viewer2d_lo();
                let hi = ui.get_viewer2d_hi();
                {
                    let mut s = session.borrow_mut();
                    if let Some(RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                        t.lo = lo;
                        t.hi = hi;
                    }
                }
                render(&session, &viewer2d.borrow(), tab_idx, lo, hi, &ui);
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
                {
                    let mut s = session.borrow_mut();
                    if let Some(RippTab::Tab2d(t)) = s.tabs.get_mut(tab_idx) {
                        t.camera.z = z.round() as f64;
                    }
                }
                let lo = ui.get_viewer2d_lo();
                let hi = ui.get_viewer2d_hi();
                upload(&session, &mut viewer2d.borrow_mut(), tab_idx);
                render(&session, &viewer2d.borrow(), tab_idx, lo, hi, &ui);
            }
        }
    });
}

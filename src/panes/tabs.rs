use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use slint::ComponentHandle;
use crate::AppWindow;
use crate::session::{RippSession, RippTab, Tab3d, Tab2d, TabCamera, Camera3d};
use crate::renderer2d::Viewer2dRenderer;
use crate::app_logic::build_left_tabs;
use crate::panes::viewer2d as v2d;

fn add_tab(
    session:  &Rc<RefCell<RippSession>>,
    app_weak: &slint::Weak<AppWindow>,
    tab:      RippTab,
) {
    session.borrow_mut().tabs.push(tab);
    if let Some(ui) = app_weak.upgrade() {
        let new_idx = (session.borrow().tabs.len() as i32) - 1;
        ui.set_left_tabs(build_left_tabs(&session.borrow()));
        ui.set_active_left_tab(new_idx);
    }
}

pub fn register<F: Fn() + 'static>(
    app: &AppWindow,
    session: &Rc<RefCell<RippSession>>,
    viewer2d: &Rc<RefCell<Viewer2dRenderer>>,
    live_running: &Arc<AtomicBool>,
    prev_tab_idx: &Rc<RefCell<usize>>,
    start_live: F,
) {
    let start_live = Rc::new(start_live);

    // ── Initial state ─────────────────────────────────────────────────────────
    app.set_left_tabs(build_left_tabs(&session.borrow()));

    // ── Callbacks ─────────────────────────────────────────────────────────────
    app.on_close_left_tab({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move |index| {
            let index = index as usize;
            let mut s = session.borrow_mut();
            if index < s.tabs.len() { s.tabs.remove(index); }
            drop(s);
            if let Some(ui) = app_weak.upgrade() {
                ui.set_left_tabs(build_left_tabs(&session.borrow()));
                let new_len = session.borrow().tabs.len() as i32;
                if ui.get_active_left_tab() >= new_len {
                    ui.set_active_left_tab((new_len - 1).max(0));
                }
            }
        }
    });

    app.on_add_tab_3d({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move || { add_tab(&session, &app_weak, RippTab::Tab3d(Tab3d { camera: Camera3d::default() })); }
    });

    app.on_add_tab_2d({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move || { add_tab(&session, &app_weak, RippTab::Tab2d(Tab2d::default())); }
    });

    app.on_add_tab_camera({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move || { add_tab(&session, &app_weak, RippTab::Camera(TabCamera { live: false, color: crate::session::ColorMappingRange::default() })); }
    });

    app.on_left_tab_activated({
        let session      = session.clone();
        let viewer2d     = viewer2d.clone();
        let app_weak     = app.as_weak();
        let live_running = live_running.clone();
        let prev_tab     = prev_tab_idx.clone();
        let start_live   = start_live.clone();
        move |new_idx| {
            let new_idx = new_idx as usize;
            let old_idx = *prev_tab.borrow();
            *prev_tab.borrow_mut() = new_idx;

            // Save live state on old Camera tab and stop the loop
            {
                let mut s = session.borrow_mut();
                if let Some(RippTab::Camera(tc)) = s.tabs.get_mut(old_idx) {
                    tc.live = live_running.load(Ordering::SeqCst);
                    live_running.store(false, Ordering::SeqCst);
                }
            }

            if let Some(ui) = app_weak.upgrade() {
                let s = session.borrow();
                match s.tabs.get(new_idx) {
                    Some(RippTab::Tab2d(t)) => {
                        ui.set_viewer2d_lo(t.color.lo);
                        ui.set_viewer2d_hi(t.color.hi);
                        ui.set_viewer2d_z(t.camera.z as f32);
                        ui.set_viewer2d_z_max(t.z_max as f32);
                        let has_obj = t.selected_proj_id >= 0;
                        if !has_obj { ui.set_viewer2d_image_loaded(false); }
                        let color = t.color;
                        drop(s);
                        if has_obj {
                            v2d::upload(&session, &mut viewer2d.borrow_mut(), new_idx);
                            v2d::render(&session, &viewer2d.borrow(), new_idx, color, &ui);
                        }
                    }
                    Some(RippTab::Camera(tc)) => {
                        let want_live = tc.live;
                        let color = tc.color;
                        ui.set_live_snap(want_live);
                        ui.set_camera_lo(color.lo);
                        ui.set_camera_hi(color.hi);
                        drop(s);
                        if want_live { start_live(); }
                    }
                    _ => {}
                }
            }
        }
    });
}

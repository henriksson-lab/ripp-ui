use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use slint::ComponentHandle;
use crate::AppWindow;
use crate::session::{RippSession, RippTab, TabArea, Tab3d, Tab2d, TabCamera, Camera3d, ActivationContext};
use crate::renderer2d::Viewer2dRenderer;
use crate::app_logic::build_tabs;

fn add_tab(
    session:  &Rc<RefCell<RippSession>>,
    app_weak: &slint::Weak<AppWindow>,
    tab:      RippTab,
    area:     TabArea,
) {
    session.borrow_mut().tabs_mut(area).push(tab);
    if let Some(ui) = app_weak.upgrade() {
        let s = session.borrow();
        let new_idx = (s.tabs(area).len() as i32) - 1;
        match area {
            TabArea::Left => {
                ui.set_left_tabs(build_tabs(s.tabs(area)));
                ui.set_active_left_tab(new_idx);
            }
            TabArea::RightTop => {
                ui.set_right_top_tabs(build_tabs(s.tabs(area)));
                ui.set_active_right_top_tab(new_idx);
            }
            TabArea::RightBottom => {
                ui.set_right_bottom_tabs(build_tabs(s.tabs(area)));
                ui.set_active_right_bottom_tab(new_idx);
            }
        }
    }
}

fn make_tab_activated_handler(
    session:      Rc<RefCell<RippSession>>,
    viewer2d:     Rc<RefCell<Viewer2dRenderer>>,
    app_weak:     slint::Weak<AppWindow>,
    live_running: Arc<AtomicBool>,
    prev_tab:     Rc<RefCell<usize>>,
    start_live:   Rc<dyn Fn()>,
    area:         TabArea,
) -> impl Fn(i32) + 'static {
    move |new_idx| {
        let new_idx = new_idx as usize;
        let old_idx = *prev_tab.borrow();
        *prev_tab.borrow_mut() = new_idx;

        session.borrow_mut().tabs_mut(area).get_mut(old_idx)
            .map(|t| t.on_deactivating(&live_running));

        if let Some(ui) = app_weak.upgrade() {
            let ctx = ActivationContext {
                session:      session.clone(),
                viewer2d:     viewer2d.clone(),
                start_live:   start_live.clone(),
                live_running: live_running.clone(),
                tab_idx:      new_idx,
                area,
            };
            session.borrow().tabs(area).get(new_idx)
                .map(|t| t.on_activated(&ui, &ctx));
        }
    }
}

pub fn register<F: Fn() + 'static>(
    app:                   &AppWindow,
    session:               &Rc<RefCell<RippSession>>,
    viewer2d:              &Rc<RefCell<Viewer2dRenderer>>,
    live_running:          &Arc<AtomicBool>,
    prev_left_idx:         &Rc<RefCell<usize>>,
    prev_right_top_idx:    &Rc<RefCell<usize>>,
    prev_right_bottom_idx: &Rc<RefCell<usize>>,
    start_live: F,
) {
    let start_live = Rc::new(start_live);

    // ── Initial state ─────────────────────────────────────────────────────────
    app.set_left_tabs(build_tabs(&session.borrow().tabs_left));
    app.set_right_top_tabs(build_tabs(&session.borrow().tabs_right_top));
    app.set_right_bottom_tabs(build_tabs(&session.borrow().tabs_right_bottom));

    // ── Left pane callbacks ───────────────────────────────────────────────────
    app.on_close_left_tab({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move |index| {
            let index = index as usize;
            let mut s = session.borrow_mut();
            if index < s.tabs_left.len() { s.tabs_left.remove(index); }
            drop(s);
            if let Some(ui) = app_weak.upgrade() {
                ui.set_left_tabs(build_tabs(&session.borrow().tabs_left));
                let new_len = session.borrow().tabs_left.len() as i32;
                if ui.get_active_left_tab() >= new_len {
                    ui.set_active_left_tab((new_len - 1).max(0));
                }
            }
        }
    });

    app.on_add_tab_3d({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move || { add_tab(&session, &app_weak, RippTab::Tab3d(Tab3d { camera: Camera3d::default() }), TabArea::Left); }
    });

    app.on_add_tab_2d({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move || { add_tab(&session, &app_weak, RippTab::Tab2d(Tab2d::default()), TabArea::Left); }
    });

    app.on_add_tab_camera({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move || { add_tab(&session, &app_weak, RippTab::Camera(TabCamera { live: false, color: crate::session::ColorMappingRange::default() }), TabArea::Left); }
    });

    app.on_left_tab_activated(make_tab_activated_handler(
        session.clone(), viewer2d.clone(), app.as_weak(),
        live_running.clone(), prev_left_idx.clone(), start_live.clone(),
        TabArea::Left,
    ));

    // ── Right-top pane callbacks ──────────────────────────────────────────────
    app.on_close_right_top_tab({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move |index| {
            let index = index as usize;
            let mut s = session.borrow_mut();
            if index < s.tabs_right_top.len() { s.tabs_right_top.remove(index); }
            drop(s);
            if let Some(ui) = app_weak.upgrade() {
                ui.set_right_top_tabs(build_tabs(&session.borrow().tabs_right_top));
                let new_len = session.borrow().tabs_right_top.len() as i32;
                if ui.get_active_right_top_tab() >= new_len {
                    ui.set_active_right_top_tab((new_len - 1).max(0));
                }
            }
        }
    });

    app.on_right_top_tab_activated(make_tab_activated_handler(
        session.clone(), viewer2d.clone(), app.as_weak(),
        live_running.clone(), prev_right_top_idx.clone(), start_live.clone(),
        TabArea::RightTop,
    ));

    // ── Right-bottom pane callbacks ───────────────────────────────────────────
    app.on_close_right_bottom_tab({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move |index| {
            let index = index as usize;
            let mut s = session.borrow_mut();
            if index < s.tabs_right_bottom.len() { s.tabs_right_bottom.remove(index); }
            drop(s);
            if let Some(ui) = app_weak.upgrade() {
                ui.set_right_bottom_tabs(build_tabs(&session.borrow().tabs_right_bottom));
                let new_len = session.borrow().tabs_right_bottom.len() as i32;
                if ui.get_active_right_bottom_tab() >= new_len {
                    ui.set_active_right_bottom_tab((new_len - 1).max(0));
                }
            }
        }
    });

    app.on_right_bottom_tab_activated(make_tab_activated_handler(
        session.clone(), viewer2d.clone(), app.as_weak(),
        live_running.clone(), prev_right_bottom_idx.clone(), start_live.clone(),
        TabArea::RightBottom,
    ));
}

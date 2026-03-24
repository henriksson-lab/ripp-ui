use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use slint::ComponentHandle;
use crate::AppWindow;
use crate::session::{
    RippSession, RippTab, PaneLocation, Tab3d, Tab2d, TabCamera, Camera3d, ActivationContext,
    TabCamProp, TabParticleTracking, TabProject, TabFileBrowser, TabPlots, TabHelp,
    ColorMappingRange,
};
use crate::renderer2d::Viewer2dRenderer;
use crate::app_logic::build_tabs;

fn area_from_int(n: i32) -> Option<PaneLocation> {
    match n {
        0 => Some(PaneLocation::Left),
        1 => Some(PaneLocation::RightTop),
        2 => Some(PaneLocation::RightBottom),
        _ => None,
    }
}

fn move_tab_between_areas(
    session:  &Rc<RefCell<RippSession>>,
    app_weak: &slint::Weak<AppWindow>,
    from:     PaneLocation,
    idx:      usize,
    to:       PaneLocation,
) {
    {
        let mut s = session.borrow_mut();
        let tab = match from {
            PaneLocation::Left        => { if idx >= s.tabs_left.len()         { return; } s.tabs_left.remove(idx) }
            PaneLocation::RightTop    => { if idx >= s.tabs_right_top.len()    { return; } s.tabs_right_top.remove(idx) }
            PaneLocation::RightBottom => { if idx >= s.tabs_right_bottom.len() { return; } s.tabs_right_bottom.remove(idx) }
        };
        match to {
            PaneLocation::Left        => s.tabs_left.push(tab),
            PaneLocation::RightTop    => s.tabs_right_top.push(tab),
            PaneLocation::RightBottom => s.tabs_right_bottom.push(tab),
        }
    }
    if let Some(ui) = app_weak.upgrade() {
        let s = session.borrow();
        ui.set_left_tabs(build_tabs(s.tabs(PaneLocation::Left)));
        ui.set_right_top_tabs(build_tabs(s.tabs(PaneLocation::RightTop)));
        ui.set_right_bottom_tabs(build_tabs(s.tabs(PaneLocation::RightBottom)));
        let from_len = s.tabs(from).len() as i32;
        let to_new   = (s.tabs(to).len() as i32) - 1;
        drop(s);

        match from {
            PaneLocation::Left => {
                if ui.get_active_left_tab() >= from_len {
                    ui.set_active_left_tab((from_len - 1).max(0));
                }
            }
            PaneLocation::RightTop => {
                if ui.get_active_right_top_tab() >= from_len {
                    ui.set_active_right_top_tab((from_len - 1).max(0));
                }
            }
            PaneLocation::RightBottom => {
                if ui.get_active_right_bottom_tab() >= from_len {
                    ui.set_active_right_bottom_tab((from_len - 1).max(0));
                }
            }
        }
        match to {
            PaneLocation::Left        => ui.set_active_left_tab(to_new),
            PaneLocation::RightTop    => ui.set_active_right_top_tab(to_new),
            PaneLocation::RightBottom => ui.set_active_right_bottom_tab(to_new),
        }
    }
}

fn add_tab(
    session:  &Rc<RefCell<RippSession>>,
    app_weak: &slint::Weak<AppWindow>,
    tab:      RippTab,
    loc:      PaneLocation,
) {
    session.borrow_mut().tabs_mut(loc).push(tab);
    if let Some(ui) = app_weak.upgrade() {
        let s = session.borrow();
        let new_idx = (s.tabs(loc).len() as i32) - 1;
        match loc {
            PaneLocation::Left => {
                ui.set_left_tabs(build_tabs(s.tabs(loc)));
                ui.set_active_left_tab(new_idx);
            }
            PaneLocation::RightTop => {
                ui.set_right_top_tabs(build_tabs(s.tabs(loc)));
                ui.set_active_right_top_tab(new_idx);
            }
            PaneLocation::RightBottom => {
                ui.set_right_bottom_tabs(build_tabs(s.tabs(loc)));
                ui.set_active_right_bottom_tab(new_idx);
            }
        }
    }
}

fn make_tab_activated_handler(
    session:         Rc<RefCell<RippSession>>,
    viewer2d:        Rc<RefCell<Viewer2dRenderer>>,
    app_weak:        slint::Weak<AppWindow>,
    live_running:    Arc<AtomicBool>,
    prev_tab:        Rc<RefCell<usize>>,
    start_live:      Rc<dyn Fn()>,
    add_demo_camera: Option<Rc<dyn Fn()>>,
    add_sim_camera:  Option<Rc<dyn Fn()>>,
    disconnect_all:  Option<Rc<dyn Fn()>>,
    loc:             PaneLocation,
) -> impl Fn(i32) + 'static {
    move |new_idx| {
        let new_idx = new_idx as usize;
        let old_idx = *prev_tab.borrow();
        *prev_tab.borrow_mut() = new_idx;

        session.borrow_mut().tabs_mut(loc).get_mut(old_idx)
            .map(|t| t.on_deactivating(&live_running));

        if let Some(ui) = app_weak.upgrade() {
            let ctx = ActivationContext {
                session:         session.clone(),
                viewer2d:        viewer2d.clone(),
                start_live:      start_live.clone(),
                live_running:    live_running.clone(),
                tab_idx:         new_idx,
                area:            loc,
                add_demo_camera: add_demo_camera.clone(),
                add_sim_camera:  add_sim_camera.clone(),
                disconnect_all:  disconnect_all.clone(),
            };
            session.borrow().tabs(loc).get(new_idx)
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
    add_demo_camera:       Option<Rc<dyn Fn()>>,
    add_sim_camera:        Option<Rc<dyn Fn()>>,
    disconnect_all:        Option<Rc<dyn Fn()>>,
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
        move || {
            let tab = RippTab::Tab3d(Tab3d { camera: Camera3d::default() });
            let loc = tab.default_location();
            add_tab(&session, &app_weak, tab, loc);
        }
    });

    app.on_add_tab_2d({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move || {
            let tab = RippTab::Tab2d(Tab2d::default());
            let loc = tab.default_location();
            add_tab(&session, &app_weak, tab, loc);
        }
    });

    app.on_add_tab_camera({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move || {
            let tab = RippTab::Camera(TabCamera { live: false, color: crate::session::ColorMappingRange::default() });
            let loc = tab.default_location();
            add_tab(&session, &app_weak, tab, loc);
        }
    });

    app.on_left_tab_activated(make_tab_activated_handler(
        session.clone(), viewer2d.clone(), app.as_weak(),
        live_running.clone(), prev_left_idx.clone(), start_live.clone(),
        add_demo_camera.clone(), add_sim_camera.clone(), disconnect_all.clone(), PaneLocation::Left,
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
        add_demo_camera.clone(), add_sim_camera.clone(), disconnect_all.clone(), PaneLocation::RightTop,
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
        add_demo_camera.clone(), add_sim_camera.clone(), disconnect_all.clone(), PaneLocation::RightBottom,
    ));

    // ── Move-tab callbacks ────────────────────────────────────────────────────
    app.on_move_left_tab({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move |idx, target| {
            if let Some(to) = area_from_int(target) {
                move_tab_between_areas(&session, &app_weak, PaneLocation::Left, idx as usize, to);
            }
        }
    });

    app.on_move_right_top_tab({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move |idx, target| {
            if let Some(to) = area_from_int(target) {
                move_tab_between_areas(&session, &app_weak, PaneLocation::RightTop, idx as usize, to);
            }
        }
    });

    app.on_add_pane({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move |area, type_id| {
            let Some(loc) = area_from_int(area) else { return };
            let tab = match type_id {
                0 => RippTab::Tab3d(Tab3d { camera: Camera3d::default() }),
                1 => RippTab::Tab2d(Tab2d::default()),
                2 => RippTab::Camera(TabCamera { live: false, color: ColorMappingRange::default() }),
                3 => RippTab::CamProp(TabCamProp),
                4 => RippTab::ParticleTracking(TabParticleTracking),
                5 => RippTab::Project(TabProject),
                6 => RippTab::FileBrowser(TabFileBrowser),
                7 => RippTab::Plots(TabPlots),
                8 => RippTab::Help(TabHelp),
                _ => return,
            };
            add_tab(&session, &app_weak, tab, loc);
        }
    });

    app.on_move_right_bottom_tab({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move |idx, target| {
            if let Some(to) = area_from_int(target) {
                move_tab_between_areas(&session, &app_weak, PaneLocation::RightBottom, idx as usize, to);
            }
        }
    });

    // ── Tab custom-action callbacks ───────────────────────────────────────────
    let make_action_handler = |loc: PaneLocation| {
        let session         = session.clone();
        let viewer2d        = viewer2d.clone();
        let app_weak        = app.as_weak();
        let live_running    = live_running.clone();
        let start_live      = start_live.clone();
        let add_demo_camera = add_demo_camera.clone();
        let add_sim_camera  = add_sim_camera.clone();
        let disconnect_all  = disconnect_all.clone();
        move |tab_idx: i32, action_id: i32| {
            if let Some(ui) = app_weak.upgrade() {
                let ctx = ActivationContext {
                    session:         session.clone(),
                    viewer2d:        viewer2d.clone(),
                    start_live:      start_live.clone(),
                    live_running:    live_running.clone(),
                    tab_idx:         tab_idx as usize,
                    area:            loc,
                    add_demo_camera: add_demo_camera.clone(),
                    add_sim_camera:  add_sim_camera.clone(),
                    disconnect_all:  disconnect_all.clone(),
                };
                session.borrow_mut().tabs_mut(loc).get_mut(tab_idx as usize)
                    .map(|t| t.on_menu_action(action_id, &ui, &ctx));
            }
        }
    };
    app.on_tab_action_left(make_action_handler(PaneLocation::Left));
    app.on_tab_action_right_top(make_action_handler(PaneLocation::RightTop));
    app.on_tab_action_right_bottom(make_action_handler(PaneLocation::RightBottom));
}

use std::cell::RefCell;
use std::rc::Rc;
use slint::ComponentHandle;
use crate::AppWindow;
use crate::session::{
    RippSession, TabPane, TabType, PaneLocation, ActivationContext, CallbackCtx,
};
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
    tab:      Box<dyn TabPane>,
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
    ctx:      CallbackCtx,
    app_weak: slint::Weak<AppWindow>,
    prev_tab: Rc<RefCell<usize>>,
    loc:      PaneLocation,
) -> impl Fn(i32) + 'static {
    move |new_idx| {
        let new_idx = new_idx as usize;
        let old_idx = *prev_tab.borrow();
        *prev_tab.borrow_mut() = new_idx;

        ctx.session.borrow_mut().tabs_mut(loc).get_mut(old_idx)
            .map(|t| t.on_deactivating(&ctx.live_running));

        if let Some(ui) = app_weak.upgrade() {
            let act_ctx = ActivationContext {
                session:         ctx.session.clone(),
                viewer2d:        ctx.viewer2d.clone(),
                panscan_viewer:  ctx.panscan_viewer.clone(),
                start_live:      ctx.start_live.clone(),
                live_running:    ctx.live_running.clone(),
                tab_idx:         new_idx,
                area:            loc,
                add_demo_camera: ctx.add_demo_camera.clone(),
                add_sim_camera:  ctx.add_sim_camera.clone(),
                disconnect_all:  ctx.disconnect_all.clone(),
            };
            // Obtain a raw pointer so we can drop the borrow before calling
            // on_activated — which may itself borrow the session (e.g. Tab2d
            // calls upload → session.borrow_mut()).  The tab cannot be removed
            // while we are on the single UI thread inside this closure.
            let tab_ptr: Option<*const dyn TabPane> = {
                let s = ctx.session.borrow();
                s.tabs(loc).get(new_idx).map(|t| t.as_ref() as *const dyn TabPane)
            };
            if let Some(ptr) = tab_ptr {
                // SAFETY: single-threaded; tab is not moved or dropped here.
                unsafe { (*ptr).on_activated(&ui, &act_ctx) };
            }
        }
    }
}

pub fn register(
    app:                   &AppWindow,
    session:               &Rc<RefCell<RippSession>>,
    tab_types:             &[Box<dyn TabType>],
    ctx:                   &CallbackCtx,
    prev_left_idx:         &Rc<RefCell<usize>>,
    prev_right_top_idx:    &Rc<RefCell<usize>>,
    prev_right_bottom_idx: &Rc<RefCell<usize>>,
) {
    // ── Initial state ─────────────────────────────────────────────────────────
    app.set_left_tabs(build_tabs(session.borrow().tabs(PaneLocation::Left)));
    app.set_right_top_tabs(build_tabs(session.borrow().tabs(PaneLocation::RightTop)));
    app.set_right_bottom_tabs(build_tabs(session.borrow().tabs(PaneLocation::RightBottom)));

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
                ui.set_left_tabs(build_tabs(session.borrow().tabs(PaneLocation::Left)));
                let new_len = session.borrow().tabs_left.len() as i32;
                if ui.get_active_left_tab() >= new_len {
                    ui.set_active_left_tab((new_len - 1).max(0));
                }
            }
        }
    });

    app.on_left_tab_activated(make_tab_activated_handler(
        clone_ctx(ctx), app.as_weak(), prev_left_idx.clone(), PaneLocation::Left,
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
                ui.set_right_top_tabs(build_tabs(session.borrow().tabs(PaneLocation::RightTop)));
                let new_len = session.borrow().tabs_right_top.len() as i32;
                if ui.get_active_right_top_tab() >= new_len {
                    ui.set_active_right_top_tab((new_len - 1).max(0));
                }
            }
        }
    });

    app.on_right_top_tab_activated(make_tab_activated_handler(
        clone_ctx(ctx), app.as_weak(), prev_right_top_idx.clone(), PaneLocation::RightTop,
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
                ui.set_right_bottom_tabs(build_tabs(session.borrow().tabs(PaneLocation::RightBottom)));
                let new_len = session.borrow().tabs_right_bottom.len() as i32;
                if ui.get_active_right_bottom_tab() >= new_len {
                    ui.set_active_right_bottom_tab((new_len - 1).max(0));
                }
            }
        }
    });

    app.on_right_bottom_tab_activated(make_tab_activated_handler(
        clone_ctx(ctx), app.as_weak(), prev_right_bottom_idx.clone(), PaneLocation::RightBottom,
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

    app.on_move_right_bottom_tab({
        let session  = session.clone();
        let app_weak = app.as_weak();
        move |idx, target| {
            if let Some(to) = area_from_int(target) {
                move_tab_between_areas(&session, &app_weak, PaneLocation::RightBottom, idx as usize, to);
            }
        }
    });

    // ── Add-pane callbacks ────────────────────────────────────────────────────
    // Build a small owned registry for use in closures
    let registry: Rc<Vec<Box<dyn TabType>>> = {
        let v: Vec<Box<dyn TabType>> = tab_types.iter().map(|tt| clone_tab_type(tt.as_ref())).collect();
        Rc::new(v)
    };

    app.on_add_pane({
        let session  = session.clone();
        let app_weak = app.as_weak();
        let registry = registry.clone();
        move |area, type_id| {
            let Some(loc) = area_from_int(area) else { return };
            if let Some(tab) = registry.iter().find(|tt| tt.type_id() == type_id).map(|tt| tt.create()) {
                add_tab(&session, &app_weak, tab, loc);
            }
        }
    });

    app.on_add_pane_at_default({
        let session  = session.clone();
        let app_weak = app.as_weak();
        let registry = registry.clone();
        move |type_id| {
            if let Some(tt) = registry.iter().find(|tt| tt.type_id() == type_id) {
                let loc = tt.default_location();
                let tab = tt.create();
                add_tab(&session, &app_weak, tab, loc);
            }
        }
    });

    // ── Tab custom-action callbacks ───────────────────────────────────────────
    let make_action_handler = |loc: PaneLocation| {
        let ctx      = clone_ctx(ctx);
        let app_weak = app.as_weak();
        move |tab_idx: i32, action_id: i32| {
            if let Some(ui) = app_weak.upgrade() {
                let act_ctx = ActivationContext {
                    session:         ctx.session.clone(),
                    viewer2d:        ctx.viewer2d.clone(),
                    panscan_viewer:  ctx.panscan_viewer.clone(),
                    start_live:      ctx.start_live.clone(),
                    live_running:    ctx.live_running.clone(),
                    tab_idx:         tab_idx as usize,
                    area:            loc,
                    add_demo_camera: ctx.add_demo_camera.clone(),
                    add_sim_camera:  ctx.add_sim_camera.clone(),
                    disconnect_all:  ctx.disconnect_all.clone(),
                };
                let tab_ptr: Option<*mut dyn TabPane> = {
                    let mut s = ctx.session.borrow_mut();
                    s.tabs_mut(loc).get_mut(tab_idx as usize)
                        .map(|t| t.as_mut() as *mut dyn TabPane)
                };
                if let Some(ptr) = tab_ptr {
                    // SAFETY: single-threaded; tab is not moved or dropped here.
                    unsafe { (*ptr).on_menu_action(action_id, &ui, &act_ctx) };
                }
            }
        }
    };
    app.on_tab_action_left(make_action_handler(PaneLocation::Left));
    app.on_tab_action_right_top(make_action_handler(PaneLocation::RightTop));
    app.on_tab_action_right_bottom(make_action_handler(PaneLocation::RightBottom));
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Clone a CallbackCtx (all fields are Rc/Arc so this is cheap).
fn clone_ctx(ctx: &CallbackCtx) -> CallbackCtx {
    CallbackCtx {
        session:            ctx.session.clone(),
        viewer2d:           ctx.viewer2d.clone(),
        panscan_viewer:     ctx.panscan_viewer.clone(),
        cam:                ctx.cam.clone(),
        live_running:       ctx.live_running.clone(),
        start_live:         ctx.start_live.clone(),
        add_demo_camera:    ctx.add_demo_camera.clone(),
        add_sim_camera:     ctx.add_sim_camera.clone(),
        disconnect_all:     ctx.disconnect_all.clone(),
        cwd:                ctx.cwd.clone(),
        last_camera_frame:  ctx.last_camera_frame.clone(),
    }
}

/// Clone a single TabType into a new Box for use in closures.
fn clone_tab_type(tt: &dyn TabType) -> Box<dyn TabType> {
    // We need each TabType to be clonable into a Box. Since all our TabType structs
    // are unit structs, we use type_id-based reconstruction.
    use crate::panes::*;
    match tt.type_id() {
        0 => Box::new(viewer3d::TabTypeViewer3d),
        1 => Box::new(viewer2d::TabTypeViewer2d),
        2 => Box::new(camera_view::TabTypeCamera),
        3 => Box::new(cam_prop::TabTypeCamProp),
        4 => Box::new(particle_tracking::TabTypeParticleTracking),
        5 => Box::new(project::TabTypeProject),
        6 => Box::new(file_browser::TabTypeFileBrowser),
        7 => Box::new(plots::TabTypePlots),
        8 => Box::new(help::TabTypeHelp),
        9 => Box::new(pan_scan::TabTypePanScan),
        _ => unreachable!("unknown type_id {}", tt.type_id()),
    }
}

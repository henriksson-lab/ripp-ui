use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, atomic::AtomicBool};
use glam::Mat4;
use crate::AppWindow;
use crate::renderer2d::Viewer2dRenderer;

// --- color mapping ---

#[derive(Copy, Clone)]
pub struct ColorMappingRange {
    pub lo: f32,
    pub hi: f32,
}

impl Default for ColorMappingRange {
    fn default() -> Self { Self { lo: 0.0, hi: 255.0 } }
}

// --- session level ---

#[derive(Copy, Clone)]
pub enum PaneLocation { Left, RightTop, RightBottom }

pub struct RippSession {
    pub projects:          BTreeMap<u32, Project>,
    pub tabs_left:         Vec<RippTab>,
    pub tabs_right_top:    Vec<RippTab>,
    pub tabs_right_bottom: Vec<RippTab>,
    next_id: u32,
}

// --- tabs ---

pub struct Camera3d {
    pub yaw:      f32,   // azimuth in radians
    pub pitch:    f32,   // elevation in radians, clamped ±PI/2
    pub distance: f32,   // distance from origin
}

impl Default for Camera3d {
    fn default() -> Self {
        Self { yaw: 0.0, pitch: 0.3, distance: 6.0 }
    }
}

impl Camera3d {
    /// Compute view * projection matrix for this camera position.
    pub fn view_matrix(&self, aspect: f32) -> Mat4 {
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 200.0);
        let eye  = glam::Vec3::new(
            self.distance * self.pitch.cos() * self.yaw.sin(),
            self.distance * self.pitch.sin(),
            self.distance * self.pitch.cos() * self.yaw.cos(),
        );
        let view = Mat4::look_at_rh(eye, glam::Vec3::ZERO, glam::Vec3::Y);
        proj * view
    }
}

pub struct Camera2d {
    pub x:    f64,
    pub y:    f64,
    pub zoom: f64,
}

pub struct Camera2dZ {
    pub x:    f64,
    pub y:    f64,
    pub z:    f64,
    pub zoom: f64,
}

#[derive(Copy, Clone)]
pub struct WindowSize {
    pub w: u32,
    pub h: u32,
}

pub struct Tab3d {
    pub camera: Camera3d,
}

pub struct Tab2d {
    pub camera: Camera2dZ,
    pub selected_proj_id: i32,
    pub selected_obj_id:  i32,
    pub color:            ColorMappingRange,
    pub z_max:            i32,
}

impl Default for Tab2d {
    fn default() -> Self {
        Self {
            camera: Camera2dZ { x: 0.0, y: 0.0, z: 0.0, zoom: 1.0 },
            selected_proj_id: -1,
            selected_obj_id:  -1,
            color: ColorMappingRange::default(),
            z_max: 0,
        }
    }
}

pub struct TabCamera {
    pub live:  bool,
    pub color: ColorMappingRange,
}

// Right-top pane tabs
pub struct TabCamProp;
pub struct TabParticleTracking;

// Right-bottom pane tabs
pub struct TabProject;
pub struct TabFileBrowser;
pub struct TabPlots;
pub struct TabHelp;

pub struct TabPanScan {
    pub camera:   Camera2d,
    pub color:    ColorMappingRange,
    pub min_x:    String,
    pub max_x:    String,
    pub min_y:    String,
    pub max_y:    String,
    pub uploaded: bool,
}

impl Default for TabPanScan {
    fn default() -> Self {
        Self {
            camera:   Camera2d { x: 256.0, y: 256.0, zoom: 1.0 },
            color:    ColorMappingRange::default(),
            min_x:    String::new(),
            max_x:    String::new(),
            min_y:    String::new(),
            max_y:    String::new(),
            uploaded: false,
        }
    }
}

pub enum RippTab {
    Tab3d(Tab3d),
    Tab2d(Tab2d),
    Camera(TabCamera),
    CamProp(TabCamProp),
    ParticleTracking(TabParticleTracking),
    Project(TabProject),
    FileBrowser(TabFileBrowser),
    Plots(TabPlots),
    Help(TabHelp),
    PanScan(TabPanScan),
}

// ── Tab plugin trait ──────────────────────────────────────────────────────────

pub struct ActivationContext {
    pub session:          Rc<RefCell<RippSession>>,
    pub viewer2d:         Rc<RefCell<Viewer2dRenderer>>,
    pub panscan_viewer:   Rc<RefCell<Viewer2dRenderer>>,
    pub start_live:       Rc<dyn Fn()>,
    pub live_running:     Arc<AtomicBool>,
    pub tab_idx:          usize,
    pub area:             PaneLocation,
    pub add_demo_camera:  Option<Rc<dyn Fn()>>,
    pub add_sim_camera:   Option<Rc<dyn Fn()>>,
    pub disconnect_all:   Option<Rc<dyn Fn()>>,
}

pub trait TabPane {
    fn label(&self)            -> &str;
    fn type_id(&self)          -> i32;
    fn default_location(&self) -> PaneLocation;
    fn on_deactivating(&mut self, live_running: &Arc<AtomicBool>);
    fn on_activated(&self, ui: &AppWindow, ctx: &ActivationContext);
    fn menu_actions(&self) -> Vec<(String, i32)> { vec![] }
    fn on_menu_action(&mut self, _action_id: i32, _ui: &AppWindow, _ctx: &ActivationContext) {}
}

impl RippTab {
    pub fn type_id(&self)          -> i32          { self.as_pane().type_id() }
    pub fn label(&self)            -> &str          { self.as_pane().label() }
    pub fn default_location(&self) -> PaneLocation  { self.as_pane().default_location() }
    pub fn menu_actions(&self)     -> Vec<(String, i32)> { self.as_pane().menu_actions() }

    pub fn on_activated(&self, ui: &AppWindow, ctx: &ActivationContext) {
        match self {
            Self::Tab3d(t)           => (t as &dyn TabPane).on_activated(ui, ctx),
            Self::Tab2d(t)           => (t as &dyn TabPane).on_activated(ui, ctx),
            Self::Camera(t)          => (t as &dyn TabPane).on_activated(ui, ctx),
            Self::CamProp(t)         => (t as &dyn TabPane).on_activated(ui, ctx),
            Self::ParticleTracking(t)=> (t as &dyn TabPane).on_activated(ui, ctx),
            Self::Project(t)         => (t as &dyn TabPane).on_activated(ui, ctx),
            Self::FileBrowser(t)     => (t as &dyn TabPane).on_activated(ui, ctx),
            Self::Plots(t)           => (t as &dyn TabPane).on_activated(ui, ctx),
            Self::Help(t)            => (t as &dyn TabPane).on_activated(ui, ctx),
            Self::PanScan(t)         => (t as &dyn TabPane).on_activated(ui, ctx),
        }
    }
    pub fn on_deactivating(&mut self, lr: &Arc<AtomicBool>) {
        match self {
            Self::Tab3d(t)           => (t as &mut dyn TabPane).on_deactivating(lr),
            Self::Tab2d(t)           => (t as &mut dyn TabPane).on_deactivating(lr),
            Self::Camera(t)          => (t as &mut dyn TabPane).on_deactivating(lr),
            Self::CamProp(t)         => (t as &mut dyn TabPane).on_deactivating(lr),
            Self::ParticleTracking(t)=> (t as &mut dyn TabPane).on_deactivating(lr),
            Self::Project(t)         => (t as &mut dyn TabPane).on_deactivating(lr),
            Self::FileBrowser(t)     => (t as &mut dyn TabPane).on_deactivating(lr),
            Self::Plots(t)           => (t as &mut dyn TabPane).on_deactivating(lr),
            Self::Help(t)            => (t as &mut dyn TabPane).on_deactivating(lr),
            Self::PanScan(t)         => (t as &mut dyn TabPane).on_deactivating(lr),
        }
    }
    pub fn on_menu_action(&mut self, action_id: i32, ui: &AppWindow, ctx: &ActivationContext) {
        match self {
            Self::Tab3d(t)           => (t as &mut dyn TabPane).on_menu_action(action_id, ui, ctx),
            Self::Tab2d(t)           => (t as &mut dyn TabPane).on_menu_action(action_id, ui, ctx),
            Self::Camera(t)          => (t as &mut dyn TabPane).on_menu_action(action_id, ui, ctx),
            Self::CamProp(t)         => (t as &mut dyn TabPane).on_menu_action(action_id, ui, ctx),
            Self::ParticleTracking(t)=> (t as &mut dyn TabPane).on_menu_action(action_id, ui, ctx),
            Self::Project(t)         => (t as &mut dyn TabPane).on_menu_action(action_id, ui, ctx),
            Self::FileBrowser(t)     => (t as &mut dyn TabPane).on_menu_action(action_id, ui, ctx),
            Self::Plots(t)           => (t as &mut dyn TabPane).on_menu_action(action_id, ui, ctx),
            Self::Help(t)            => (t as &mut dyn TabPane).on_menu_action(action_id, ui, ctx),
            Self::PanScan(t)         => (t as &mut dyn TabPane).on_menu_action(action_id, ui, ctx),
        }
    }
    fn as_pane(&self) -> &dyn TabPane {
        match self {
            Self::Tab3d(t)           => t,
            Self::Tab2d(t)           => t,
            Self::Camera(t)          => t,
            Self::CamProp(t)         => t,
            Self::ParticleTracking(t)=> t,
            Self::Project(t)         => t,
            Self::FileBrowser(t)     => t,
            Self::Plots(t)           => t,
            Self::Help(t)            => t,
            Self::PanScan(t)         => t,
        }
    }
}

impl RippSession {
    pub fn new() -> Self {
        Self {
            projects: BTreeMap::new(),
            tabs_left: vec![
                RippTab::Tab3d(Tab3d { camera: Camera3d::default() }),
                RippTab::Tab2d(Tab2d::default()),
                RippTab::Camera(TabCamera { live: false, color: ColorMappingRange::default() }),
            ],
            tabs_right_top: vec![
                RippTab::CamProp(TabCamProp),
                RippTab::ParticleTracking(TabParticleTracking),
            ],
            tabs_right_bottom: vec![
                RippTab::Project(TabProject),
                RippTab::FileBrowser(TabFileBrowser),
                RippTab::Plots(TabPlots),
                RippTab::Help(TabHelp),
            ],
            next_id: 0,
        }
    }

    pub fn tabs(&self, loc: PaneLocation) -> &Vec<RippTab> {
        match loc {
            PaneLocation::Left        => &self.tabs_left,
            PaneLocation::RightTop    => &self.tabs_right_top,
            PaneLocation::RightBottom => &self.tabs_right_bottom,
        }
    }

    pub fn tabs_mut(&mut self, loc: PaneLocation) -> &mut Vec<RippTab> {
        match loc {
            PaneLocation::Left        => &mut self.tabs_left,
            PaneLocation::RightTop    => &mut self.tabs_right_top,
            PaneLocation::RightBottom => &mut self.tabs_right_bottom,
        }
    }

    pub fn add_project(&mut self, name: impl Into<String>) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.projects.insert(id, Project::new(name));
        id
    }
}

// --- project level ---

pub struct Project {
    pub name: String,
    pub root: ProjectObject,
    next_object_id: u32,
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        let root = ProjectObject { id: 0, children: Vec::new(), data: ProjectData::None };
        Self { name: name.into(), root, next_object_id: 1 }
    }

    pub fn generate_unique_object_id(&mut self) -> u32 {
        let id = self.next_object_id;
        self.next_object_id += 1;
        id
    }
}

// --- object level ---

pub struct ProjectObject {
    pub id: u32,
    pub children: Vec<ProjectObject>,
    pub data: ProjectData,
}

pub struct BioformatsData {
    pub path: String,
    pub reader: bioformats::registry::ImageReader,
}

impl BioformatsData {
    pub fn open(path: impl Into<std::path::PathBuf>) -> bioformats::Result<Self> {
        let path = path.into();
        let reader = bioformats::registry::ImageReader::open(&path)?;
        Ok(Self { path: path.to_string_lossy().into_owned(), reader })
    }
}

pub struct OmeroData {
    pub server: String,
    pub image_id: u64,
}

pub enum ProjectData {
    None,
    Bioformats(BioformatsData),
    Omero(OmeroData),
}

/// Flatten a `RippSession` into `(label, indent, obj_id, proj_id)` tuples for a tree view.
pub fn flatten_session(session: &RippSession) -> Vec<(String, i32, u32, u32)> {
    let mut out = Vec::new();
    for (proj_id, project) in &session.projects {
        out.push((project.name.clone(), 0, *proj_id, *proj_id));
        flatten_object(&project.root, 1, *proj_id, &mut out);
    }
    out
}

fn object_label(obj: &ProjectObject) -> String {
    match &obj.data {
        ProjectData::None => format!("Object {}", obj.id),
        ProjectData::Bioformats(bf) => std::path::Path::new(&bf.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| bf.path.clone()),
        ProjectData::Omero(o) => format!("Omero {} @ {}", o.image_id, o.server),
    }
}

fn flatten_object(obj: &ProjectObject, indent: i32, proj_id: u32, out: &mut Vec<(String, i32, u32, u32)>) {
    out.push((object_label(obj), indent, obj.id, proj_id));
    for child in &obj.children {
        flatten_object(child, indent + 1, proj_id, out);
    }
}

/// Find an immutable reference to the `ProjectObject` with the given id anywhere in the tree.
pub fn find_object_ref(obj: &ProjectObject, id: u32) -> Option<&ProjectObject> {
    if obj.id == id { return Some(obj); }
    for child in &obj.children {
        if let Some(found) = find_object_ref(child, id) {
            return Some(found);
        }
    }
    None
}

/// Find a mutable reference to the `ProjectObject` with the given id anywhere in the tree.
pub fn find_object_mut(obj: &mut ProjectObject, id: u32) -> Option<&mut ProjectObject> {
    if obj.id == id { return Some(obj); }
    for child in &mut obj.children {
        if let Some(found) = find_object_mut(child, id) {
            return Some(found);
        }
    }
    None
}

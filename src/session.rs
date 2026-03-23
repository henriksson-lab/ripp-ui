use std::collections::BTreeMap;

// --- session level ---

pub struct RippSession {
    pub projects: BTreeMap<u32, Project>,
    pub tabs: Vec<RippTab>,
    next_id: u32,
}

// --- tabs ---

pub struct Camera2d {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub zoom: f64,
}

pub struct Tab3d;

pub struct Tab2d {
    pub camera: Camera2d,
}

pub struct TabCamera;

pub enum RippTab {
    Tab3d(Tab3d),
    Tab2d(Tab2d),
    Camera(TabCamera),
}

impl RippTab {
    pub fn type_id(&self) -> i32 {
        match self {
            RippTab::Tab3d(_)  => 0,
            RippTab::Tab2d(_)  => 1,
            RippTab::Camera(_) => 2,
        }
    }
    pub fn label(&self) -> &str {
        match self {
            RippTab::Tab3d(_)  => "3D View",
            RippTab::Tab2d(_)  => "2D Viewer",
            RippTab::Camera(_) => "Camera",
        }
    }
}

impl RippSession {
    pub fn new() -> Self {
        Self {
            projects: BTreeMap::new(),
            tabs: vec![
                RippTab::Tab3d(Tab3d),
                RippTab::Tab2d(Tab2d { camera: Camera2d { x: 0.0, y: 0.0, z: 0.0, zoom: 1.0 } }),
                RippTab::Camera(TabCamera),
            ],
            next_id: 0,
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

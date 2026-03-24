use std::sync::mpsc;
use micromanager::CMMCore;
use micromanager::adapters::demo::DemoAdapter;
use crate::sim_adapter::SimAdapter;
use crate::session::ColorMappingRange;

// ── Public data types ─────────────────────────────────────────────────────────

pub struct CameraImage {
    pub data: Vec<u8>, // GRAY8
    pub width: u32,
    pub height: u32,
}

impl CameraImage {
    /// Convert GRAY8 → RGBA8 and wrap as a Slint `Image`, applying color mapping.
    pub fn to_slint_image(&self, color: ColorMappingRange) -> slint::Image {
        let range = (color.hi - color.lo).max(1.0);
        let lo = color.lo;
        let apply = |g: u8| -> u8 {
            ((g as f32 - lo) / range * 255.0).clamp(0.0, 255.0) as u8
        };
        let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(self.width, self.height);
        let dst = pb.make_mut_bytes();
        for (i, &g) in self.data.iter().enumerate() {
            let v = apply(g);
            dst[i * 4]     = v;
            dst[i * 4 + 1] = v;
            dst[i * 4 + 2] = v;
            dst[i * 4 + 3] = 255;
        }
        slint::Image::from_rgba8(pb)
    }
}

pub struct DeviceProp {
    pub device: String,
    pub property: String,
    pub value: String,
}

// ── Command protocol ──────────────────────────────────────────────────────────

enum CameraCmd {
    Snap(mpsc::Sender<CameraImage>),
    GetProps(mpsc::Sender<Vec<DeviceProp>>),
    MoveXY(f64, f64),
    MoveZ(f64),
    LoadDemoCamera(mpsc::Sender<()>),
    LoadSimCamera(mpsc::Sender<()>),
    DisconnectAll(mpsc::Sender<()>),
    GetXYPosition(mpsc::Sender<Option<(f64, f64)>>),
}

// ── Public handle (cheaply cloneable, Send) ───────────────────────────────────

#[derive(Clone)]
pub struct CameraHandle {
    cmd_tx: mpsc::Sender<CameraCmd>,
}

impl CameraHandle {
    /// Snap one image. Blocks the calling thread until the camera thread replies.
    pub fn snap(&self) -> CameraImage {
        let (tx, rx) = mpsc::channel();
        self.cmd_tx.send(CameraCmd::Snap(tx)).unwrap();
        rx.recv().unwrap()
    }

    /// Fetch all device properties. Blocks until the camera thread replies.
    pub fn device_props(&self) -> Vec<DeviceProp> {
        let (tx, rx) = mpsc::channel();
        self.cmd_tx.send(CameraCmd::GetProps(tx)).unwrap();
        rx.recv().unwrap()
    }

    /// Move XYStage by (dx, dy) µm. Fire-and-forget.
    pub fn move_xy(&self, dx: f64, dy: f64) {
        let _ = self.cmd_tx.send(CameraCmd::MoveXY(dx, dy));
    }

    /// Move Z Stage by dz µm (relative). Fire-and-forget.
    pub fn move_z(&self, dz: f64) {
        let _ = self.cmd_tx.send(CameraCmd::MoveZ(dz));
    }

    /// Load and initialize the demo camera device. Blocks until the camera thread confirms.
    pub fn load_demo_camera(&self) {
        let (tx, rx) = mpsc::channel();
        let _ = self.cmd_tx.send(CameraCmd::LoadDemoCamera(tx));
        rx.recv().ok();
    }

    /// Load and initialize the simulated camera device. Blocks until the camera thread confirms.
    pub fn load_sim_camera(&self) {
        let (tx, rx) = mpsc::channel();
        let _ = self.cmd_tx.send(CameraCmd::LoadSimCamera(tx));
        rx.recv().ok();
    }

    /// Get the current XY stage position. Returns `None` if no XY stage is loaded.
    pub fn get_xy_position(&self) -> Option<(f64, f64)> {
        let (tx, rx) = mpsc::channel();
        let _ = self.cmd_tx.send(CameraCmd::GetXYPosition(tx));
        rx.recv().unwrap_or(None)
    }

    /// Unload all devices. Blocks until the camera thread confirms.
    pub fn disconnect_all(&self) {
        let (tx, rx) = mpsc::channel();
        let _ = self.cmd_tx.send(CameraCmd::DisconnectAll(tx));
        rx.recv().ok();
    }
}

/// Spawn the camera thread and return a handle to it.
/// Pass `use_sim = true` to use the SimulatedCamera instead of the DemoCamera.
/// The thread exits automatically when all handles are dropped.
pub fn start_camera_thread(use_sim: bool) -> CameraHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel::<CameraCmd>();
    std::thread::spawn(move || {
        let mut cam = MicromanagerSession::new(use_sim);
        while let Ok(cmd) = cmd_rx.recv() {
            match cmd {
                CameraCmd::Snap(reply)     => { let _ = reply.send(cam.snap()); }
                CameraCmd::GetProps(reply) => { let _ = reply.send(cam.device_props()); }
                CameraCmd::MoveXY(dx, dy) => {
                    if let Ok((x, y)) = cam.core.get_xy_position() {
                        let _ = cam.core.set_xy_position(x + dx, y + dy);
                    }
                }
                CameraCmd::MoveZ(dz) => { let _ = cam.core.set_relative_position(dz); }
                CameraCmd::LoadDemoCamera(reply) => {
                    cam.core.load_device("DemoCamera", "demo", "DCamera").ok();
                    cam.core.load_device("Stage",      "demo", "DStage").ok();
                    cam.core.load_device("XYStage",    "demo", "DXYStage").ok();
                    cam.core.load_device("Shutter",    "demo", "DShutter").ok();
                    cam.core.load_device("Wheel",      "demo", "DWheel").ok();
                    cam.core.initialize_device("DemoCamera").ok();
                    cam.core.initialize_device("Stage").ok();
                    cam.core.initialize_device("XYStage").ok();
                    cam.core.initialize_device("Shutter").ok();
                    cam.core.initialize_device("Wheel").ok();
                    cam.core.set_camera_device("DemoCamera").ok();
                    cam.core.set_focus_device("Stage").ok();
                    cam.core.set_xy_stage_device("XYStage").ok();
                    cam.core.set_shutter_device("Shutter").ok();
                    reply.send(()).ok();
                }
                CameraCmd::LoadSimCamera(reply) => {
                    cam.core.load_device("SimCamera",  "sim", "SimCamera").ok();
                    cam.core.load_device("SimStage",   "sim", "SimStage").ok();
                    cam.core.load_device("SimXYStage", "sim", "SimXYStage").ok();
                    cam.core.load_device("SimShutter", "sim", "SimShutter").ok();
                    cam.core.load_device("SimWheel",   "sim", "SimWheel").ok();
                    cam.core.initialize_device("SimCamera").ok();
                    cam.core.initialize_device("SimStage").ok();
                    cam.core.initialize_device("SimXYStage").ok();
                    cam.core.initialize_device("SimShutter").ok();
                    cam.core.initialize_device("SimWheel").ok();
                    cam.core.set_camera_device("SimCamera").ok();
                    cam.core.set_focus_device("SimStage").ok();
                    cam.core.set_xy_stage_device("SimXYStage").ok();
                    cam.core.set_shutter_device("SimShutter").ok();
                    reply.send(()).ok();
                }
                CameraCmd::GetXYPosition(reply) => {
                    let pos = cam.core.get_xy_position().ok();
                    let _ = reply.send(pos.map(|(x, y)| (x, y)));
                }
                CameraCmd::DisconnectAll(reply) => {
                    let labels: Vec<String> = cam.core.device_labels().into_iter().map(|s| s.to_string()).collect();
                    for label in labels {
                        cam.core.unload_device(&label).ok();
                    }
                    reply.send(()).ok();
                }
            }
        }
    });
    CameraHandle { cmd_tx }
}

// ── Internal camera wrapper (owns CMMCore) ────────────────────────────────────

struct MicromanagerSession {
    core: CMMCore,
}

impl MicromanagerSession {
    fn new(_use_sim: bool) -> Self {
        let mut core = CMMCore::new();
        core.register_adapter(Box::new(DemoAdapter));
        core.register_adapter(Box::new(SimAdapter));
        Self { core }
    }

    fn snap(&mut self) -> CameraImage {
        if self.core.snap_image().is_ok() {
            if let Ok(frame) = self.core.get_image() {
                return CameraImage { data: frame.data, width: frame.width, height: frame.height };
            }
        }
        CameraImage { data: vec![0u8; 64 * 64], width: 64, height: 64 }
    }

    fn device_props(&self) -> Vec<DeviceProp> {
        let mut rows = Vec::new();
        for label in self.core.device_labels() {
            if let Ok(prop_names) = self.core.get_property_names(label) {
                for prop in prop_names {
                    let value = self.core.get_property(label, &prop)
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    rows.push(DeviceProp {
                        device: label.to_string(),
                        property: prop,
                        value,
                    });
                }
            }
        }
        rows
    }
}

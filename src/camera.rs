use std::sync::mpsc;
use micromanager::CMMCore;
use micromanager::adapters::demo::DemoAdapter;
use crate::sim_adapter::SimAdapter;

// ── Public data types ─────────────────────────────────────────────────────────

pub struct CameraImage {
    pub data: Vec<u8>, // GRAY8
    pub width: u32,
    pub height: u32,
}

impl CameraImage {
    /// Convert GRAY8 → RGBA8 and wrap as a Slint `Image`, applying lo/hi mapping.
    pub fn to_slint_image(&self, lo: f32, hi: f32) -> slint::Image {
        let range = (hi - lo).max(1.0);
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
}

/// Spawn the camera thread and return a handle to it.
/// Pass `use_sim = true` to use the SimulatedCamera instead of the DemoCamera.
/// The thread exits automatically when all handles are dropped.
pub fn start_camera_thread(use_sim: bool) -> CameraHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel::<CameraCmd>();
    std::thread::spawn(move || {
        let mut cam = DemoCamera::new(use_sim);
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
            }
        }
    });
    CameraHandle { cmd_tx }
}

// ── Internal camera wrapper (owns CMMCore) ────────────────────────────────────

struct DemoCamera {
    core: CMMCore,
}

impl DemoCamera {
    fn new(use_sim: bool) -> Self {
        let mut core = CMMCore::new();
        core.register_adapter(Box::new(DemoAdapter));

        if use_sim {
            core.register_adapter(Box::new(SimAdapter));
            core.load_device("Camera", "sim", "SimCamera").unwrap();
        } else {
            core.load_device("Camera", "demo", "DCamera").unwrap();
        }
        core.load_device("Stage",   "demo", "DStage").unwrap();

        core.load_device("XYStage", "demo", "DXYStage").unwrap();
        core.load_device("Shutter", "demo", "DShutter").unwrap();
        core.load_device("Wheel",   "demo", "DWheel").unwrap();

        core.initialize_device("Camera").unwrap();
        core.initialize_device("Stage").unwrap();
        core.initialize_device("XYStage").unwrap();
        core.initialize_device("Shutter").unwrap();
        core.initialize_device("Wheel").unwrap();

        core.set_camera_device("Camera").unwrap();
        core.set_focus_device("Stage").unwrap();
        core.set_xy_stage_device("XYStage").unwrap();
        core.set_shutter_device("Shutter").unwrap();

        Self { core }
    }

    fn snap(&mut self) -> CameraImage {
        self.core.snap_image().unwrap();
        let frame = self.core.get_image().unwrap();
        CameraImage {
            data: frame.data,
            width: frame.width,
            height: frame.height,
        }
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

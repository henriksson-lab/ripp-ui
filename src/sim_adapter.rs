use std::sync::Mutex;

use micromanager::{
    AdapterModule, AnyDevice, Camera, Device, DeviceInfo, DeviceType,
    FocusDirection, ImageRoi, MmError, MmResult, PropertyMap, PropertyValue,
    Shutter, Stage, StateDevice, XYStage,
};

// ── Shared microscope state ────────────────────────────────────────────────────

pub struct SimMicroscopeState {
    pub x_um: f64,
    pub y_um: f64,
    pub z_um: f64,
}

pub static SIM_STATE: Mutex<SimMicroscopeState> = Mutex::new(SimMicroscopeState {
    x_um: 0.0,
    y_um: 0.0,
    z_um: 0.0,
});

// ── SimCamera ─────────────────────────────────────────────────────────────────

/// Simulated fluorescence camera.
///
/// Renders a few Gaussian PSF spots on a noisy background, scaled by exposure.
/// Serves as a prototype for custom camera simulations.
pub struct SimCamera {
    props:       PropertyMap,
    initialized: bool,
    image_buf:   Vec<u8>,
    width:       u32,
    height:      u32,
    exposure_ms: f64,
    capturing:   bool,
    frame_count: u64,
}

/// Fixed bead positions (x, y) and peak amplitudes (0–255).
const BEADS: &[(f32, f32, f32)] = &[
    (128.0, 128.0, 220.0),
    (256.0, 240.0, 255.0),
    (384.0, 150.0, 190.0),
    (100.0, 380.0, 210.0),
    (430.0, 400.0, 170.0),
    (310.0, 370.0, 200.0),
];

/// PSF sigma in pixels.
const SIGMA: f32 = 7.0;

impl SimCamera {
    pub fn new() -> Self {
        let width  = 512u32;
        let height = 512u32;
        let mut props = PropertyMap::new();
        props.define_property("Exposure",   PropertyValue::Float(10.0),                    false).unwrap();
        props.define_property("CameraName", PropertyValue::String("SimCamera".into()), true ).unwrap();

        Self {
            props,
            initialized: false,
            image_buf:   vec![0u8; (width * height) as usize],
            width,
            height,
            exposure_ms: 10.0,
            capturing:   false,
            frame_count: 0,
        }
    }

    fn generate_image(&mut self) {
        let w = self.width  as usize;
        let h = self.height as usize;
        let exposure_scale = (self.exposure_ms / 10.0) as f32;

        // Read shared stage position.
        let (stage_x, stage_y, stage_z) = {
            let s = SIM_STATE.lock().unwrap();
            (s.x_um as f32, s.y_um as f32, s.z_um as f32)
        };

        // Z defocus broadens the PSF and reduces peak amplitude.
        const PIX_PER_UM: f32 = 10.0;   // 100 nm/pixel
        let sigma_eff = SIGMA + stage_z.abs() * 0.3;
        let sigma2    = 2.0 * sigma_eff * sigma_eff;
        let z_amp     = (-stage_z.abs() * 0.05_f32).exp().max(0.1);

        self.image_buf.resize(w * h, 0);
        self.frame_count = self.frame_count.wrapping_add(1);
        let fc = self.frame_count;

        for y in 0..h {
            for x in 0..w {
                // Gaussian PSF contribution from each bead, offset by stage XY.
                let mut signal: f32 = 0.0;
                for &(bx, by, amp) in BEADS {
                    let dx = x as f32 - (bx - stage_x * PIX_PER_UM);
                    let dy = y as f32 - (by - stage_y * PIX_PER_UM);
                    signal += amp * z_amp * (-(dx * dx + dy * dy) / sigma2).exp();
                }

                // Shot-noise via a fast per-pixel hash (no external crate needed).
                let seed = fc
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add((y * w + x) as u64)
                    .wrapping_mul(2685821657736338717);
                let noise = ((seed >> 56) & 0x0F) as f32; // 0..15 counts

                let val = ((signal * exposure_scale) + noise).min(255.0) as u8;
                self.image_buf[y * w + x] = val;
            }
        }
    }
}

impl Default for SimCamera {
    fn default() -> Self {
        Self::new()
    }
}

impl Device for SimCamera {
    fn name(&self)        -> &str { "SimCamera" }
    fn description(&self) -> &str { "Simulated fluorescence camera with Gaussian PSF spots" }

    fn initialize(&mut self) -> MmResult<()> {
        self.initialized = true;
        Ok(())
    }

    fn shutdown(&mut self) -> MmResult<()> {
        self.initialized = false;
        self.capturing   = false;
        Ok(())
    }

    fn get_property(&self, name: &str) -> MmResult<PropertyValue> {
        match name {
            "Exposure" => Ok(PropertyValue::Float(self.exposure_ms)),
            _          => self.props.get(name).cloned(),
        }
    }

    fn set_property(&mut self, name: &str, val: PropertyValue) -> MmResult<()> {
        match name {
            "Exposure" => {
                self.exposure_ms = val.as_f64().ok_or(MmError::InvalidPropertyValue)?;
                Ok(())
            }
            _ => self.props.set(name, val),
        }
    }

    fn property_names(&self)                    -> Vec<String> { self.props.property_names().to_vec() }
    fn has_property(&self, name: &str)          -> bool        { self.props.has_property(name) }
    fn is_property_read_only(&self, name: &str) -> bool        { self.props.entry(name).map(|e| e.read_only).unwrap_or(false) }
    fn device_type(&self)                       -> DeviceType  { DeviceType::Camera }
    fn busy(&self)                              -> bool        { self.capturing }
}

impl Camera for SimCamera {
    fn snap_image(&mut self) -> MmResult<()> {
        if !self.initialized {
            return Err(MmError::NotConnected);
        }
        self.generate_image();
        Ok(())
    }

    fn get_image_buffer(&self)         -> MmResult<&[u8]> { Ok(&self.image_buf) }
    fn get_image_width(&self)          -> u32              { self.width }
    fn get_image_height(&self)         -> u32              { self.height }
    fn get_image_bytes_per_pixel(&self) -> u32             { 1 }
    fn get_bit_depth(&self)            -> u32              { 8 }
    fn get_number_of_components(&self) -> u32              { 1 }
    fn get_number_of_channels(&self)   -> u32              { 1 }

    fn get_exposure(&self)             -> f64              { self.exposure_ms }
    fn set_exposure(&mut self, ms: f64)                    { self.exposure_ms = ms; }

    fn get_binning(&self)              -> i32              { 1 }
    fn set_binning(&mut self, _b: i32) -> MmResult<()>    { Ok(()) }

    fn get_roi(&self) -> MmResult<ImageRoi> {
        Ok(ImageRoi::new(0, 0, self.width, self.height))
    }
    fn set_roi(&mut self, _roi: ImageRoi) -> MmResult<()> { Ok(()) }
    fn clear_roi(&mut self)               -> MmResult<()> { Ok(()) }

    fn start_sequence_acquisition(&mut self, _count: i64, _interval_ms: f64) -> MmResult<()> {
        self.capturing = true;
        Ok(())
    }
    fn stop_sequence_acquisition(&mut self) -> MmResult<()> {
        self.capturing = false;
        Ok(())
    }
    fn is_capturing(&self) -> bool { self.capturing }
}

// ── SimXYStage ────────────────────────────────────────────────────────────────

pub struct SimXYStage {
    props:       PropertyMap,
    initialized: bool,
}

impl SimXYStage {
    pub fn new() -> Self {
        let mut props = PropertyMap::new();
        props.define_property("X_um", PropertyValue::Float(0.0), false).unwrap();
        props.define_property("Y_um", PropertyValue::Float(0.0), false).unwrap();
        Self { props, initialized: false }
    }
}

impl Device for SimXYStage {
    fn name(&self)        -> &str { "SimXYStage" }
    fn description(&self) -> &str { "Simulated XY stage" }

    fn initialize(&mut self) -> MmResult<()> { self.initialized = true; Ok(()) }
    fn shutdown(&mut self)   -> MmResult<()> { self.initialized = false; Ok(()) }

    fn get_property(&self, name: &str) -> MmResult<PropertyValue> {
        let s = SIM_STATE.lock().unwrap();
        match name {
            "X_um" => Ok(PropertyValue::Float(s.x_um)),
            "Y_um" => Ok(PropertyValue::Float(s.y_um)),
            _      => self.props.get(name).cloned(),
        }
    }

    fn set_property(&mut self, name: &str, val: PropertyValue) -> MmResult<()> {
        let mut s = SIM_STATE.lock().unwrap();
        match name {
            "X_um" => { s.x_um = val.as_f64().ok_or(MmError::InvalidPropertyValue)?; Ok(()) }
            "Y_um" => { s.y_um = val.as_f64().ok_or(MmError::InvalidPropertyValue)?; Ok(()) }
            _      => { drop(s); self.props.set(name, val) }
        }
    }

    fn property_names(&self)                    -> Vec<String> { self.props.property_names().to_vec() }
    fn has_property(&self, name: &str)          -> bool        { self.props.has_property(name) }
    fn is_property_read_only(&self, name: &str) -> bool        { self.props.entry(name).map(|e| e.read_only).unwrap_or(false) }
    fn device_type(&self)                       -> DeviceType  { DeviceType::XYStage }
    fn busy(&self)                              -> bool        { false }
}

impl XYStage for SimXYStage {
    fn set_xy_position_um(&mut self, x: f64, y: f64) -> MmResult<()> {
        let mut s = SIM_STATE.lock().unwrap();
        s.x_um = x; s.y_um = y; Ok(())
    }
    fn get_xy_position_um(&self) -> MmResult<(f64, f64)> {
        let s = SIM_STATE.lock().unwrap();
        Ok((s.x_um, s.y_um))
    }
    fn set_relative_xy_position_um(&mut self, dx: f64, dy: f64) -> MmResult<()> {
        let mut s = SIM_STATE.lock().unwrap();
        s.x_um += dx; s.y_um += dy; Ok(())
    }
    fn home(&mut self) -> MmResult<()> {
        let mut s = SIM_STATE.lock().unwrap();
        s.x_um = 0.0; s.y_um = 0.0; Ok(())
    }
    fn stop(&mut self) -> MmResult<()> { Ok(()) }
    fn get_limits_um(&self) -> MmResult<(f64, f64, f64, f64)> { Ok((-50_000.0, 50_000.0, -50_000.0, 50_000.0)) }
    fn get_step_size_um(&self) -> (f64, f64) { (0.1, 0.1) }
    fn set_origin(&mut self) -> MmResult<()> {
        let mut s = SIM_STATE.lock().unwrap();
        s.x_um = 0.0; s.y_um = 0.0; Ok(())
    }
}

// ── SimStage ──────────────────────────────────────────────────────────────────

pub struct SimStage {
    props:       PropertyMap,
    initialized: bool,
}

impl SimStage {
    pub fn new() -> Self {
        let mut props = PropertyMap::new();
        props.define_property("Position_um", PropertyValue::Float(0.0), false).unwrap();
        Self { props, initialized: false }
    }
}

impl Device for SimStage {
    fn name(&self)        -> &str { "SimStage" }
    fn description(&self) -> &str { "Simulated Z stage" }

    fn initialize(&mut self) -> MmResult<()> { self.initialized = true; Ok(()) }
    fn shutdown(&mut self)   -> MmResult<()> { self.initialized = false; Ok(()) }

    fn get_property(&self, name: &str) -> MmResult<PropertyValue> {
        match name {
            "Position_um" => Ok(PropertyValue::Float(SIM_STATE.lock().unwrap().z_um)),
            _             => self.props.get(name).cloned(),
        }
    }

    fn set_property(&mut self, name: &str, val: PropertyValue) -> MmResult<()> {
        match name {
            "Position_um" => {
                SIM_STATE.lock().unwrap().z_um = val.as_f64().ok_or(MmError::InvalidPropertyValue)?;
                Ok(())
            }
            _ => self.props.set(name, val),
        }
    }

    fn property_names(&self)                    -> Vec<String> { self.props.property_names().to_vec() }
    fn has_property(&self, name: &str)          -> bool        { self.props.has_property(name) }
    fn is_property_read_only(&self, name: &str) -> bool        { self.props.entry(name).map(|e| e.read_only).unwrap_or(false) }
    fn device_type(&self)                       -> DeviceType  { DeviceType::Stage }
    fn busy(&self)                              -> bool        { false }
}

impl Stage for SimStage {
    fn set_position_um(&mut self, pos: f64) -> MmResult<()> {
        SIM_STATE.lock().unwrap().z_um = pos; Ok(())
    }
    fn get_position_um(&self) -> MmResult<f64> {
        Ok(SIM_STATE.lock().unwrap().z_um)
    }
    fn set_relative_position_um(&mut self, d: f64) -> MmResult<()> {
        SIM_STATE.lock().unwrap().z_um += d; Ok(())
    }
    fn home(&mut self) -> MmResult<()> { SIM_STATE.lock().unwrap().z_um = 0.0; Ok(()) }
    fn stop(&mut self) -> MmResult<()> { Ok(()) }
    fn get_limits(&self) -> MmResult<(f64, f64)> { Ok((-1000.0, 1000.0)) }
    fn get_focus_direction(&self) -> FocusDirection { FocusDirection::TowardSample }
    fn is_continuous_focus_drive(&self) -> bool { false }
}

// ── SimShutter ────────────────────────────────────────────────────────────────

pub struct SimShutter {
    props:       PropertyMap,
    initialized: bool,
    open:        bool,
}

impl SimShutter {
    pub fn new() -> Self {
        let mut props = PropertyMap::new();
        props.define_property("State", PropertyValue::String("Closed".into()), false).unwrap();
        props.set_allowed_values("State", &["Open", "Closed"]).unwrap();
        Self { props, initialized: false, open: false }
    }
}

impl Device for SimShutter {
    fn name(&self)        -> &str { "SimShutter" }
    fn description(&self) -> &str { "Simulated shutter" }

    fn initialize(&mut self) -> MmResult<()> { self.initialized = true; Ok(()) }
    fn shutdown(&mut self)   -> MmResult<()> { self.open = false; self.initialized = false; Ok(()) }

    fn get_property(&self, name: &str) -> MmResult<PropertyValue> {
        match name {
            "State" => Ok(PropertyValue::String(if self.open { "Open" } else { "Closed" }.into())),
            _       => self.props.get(name).cloned(),
        }
    }

    fn set_property(&mut self, name: &str, val: PropertyValue) -> MmResult<()> {
        match name {
            "State" => {
                match val.as_str() {
                    "Open"   => { self.open = true;  Ok(()) }
                    "Closed" => { self.open = false; Ok(()) }
                    _        => Err(MmError::InvalidPropertyValue),
                }
            }
            _ => self.props.set(name, val),
        }
    }

    fn property_names(&self)                    -> Vec<String> { self.props.property_names().to_vec() }
    fn has_property(&self, name: &str)          -> bool        { self.props.has_property(name) }
    fn is_property_read_only(&self, name: &str) -> bool        { self.props.entry(name).map(|e| e.read_only).unwrap_or(false) }
    fn device_type(&self)                       -> DeviceType  { DeviceType::Shutter }
    fn busy(&self)                              -> bool        { false }
}

impl Shutter for SimShutter {
    fn set_open(&mut self, open: bool) -> MmResult<()> { self.open = open; Ok(()) }
    fn get_open(&self) -> MmResult<bool> { Ok(self.open) }
    fn fire(&mut self, _delta_t: f64) -> MmResult<()> { self.open = true; self.open = false; Ok(()) }
}

// ── SimWheel ──────────────────────────────────────────────────────────────────

const NUM_WHEEL_POSITIONS: u64 = 6;

pub struct SimWheel {
    props:       PropertyMap,
    initialized: bool,
    position:    u64,
    labels:      Vec<String>,
    gate_open:   bool,
}

impl SimWheel {
    pub fn new() -> Self {
        let labels: Vec<String> = (0..NUM_WHEEL_POSITIONS).map(|i| format!("State-{}", i)).collect();
        let mut props = PropertyMap::new();
        props.define_property("State", PropertyValue::Integer(0), false).unwrap();
        props.define_property("Label", PropertyValue::String(labels[0].clone()), false).unwrap();
        Self { props, initialized: false, position: 0, labels, gate_open: true }
    }
}

impl Device for SimWheel {
    fn name(&self)        -> &str { "SimWheel" }
    fn description(&self) -> &str { "Simulated filter wheel" }

    fn initialize(&mut self) -> MmResult<()> { self.initialized = true; Ok(()) }
    fn shutdown(&mut self)   -> MmResult<()> { self.initialized = false; Ok(()) }

    fn get_property(&self, name: &str) -> MmResult<PropertyValue> {
        match name {
            "State" => Ok(PropertyValue::Integer(self.position as i64)),
            "Label" => Ok(PropertyValue::String(self.labels[self.position as usize].clone())),
            _       => self.props.get(name).cloned(),
        }
    }

    fn set_property(&mut self, name: &str, val: PropertyValue) -> MmResult<()> {
        match name {
            "State" => {
                let pos = val.as_i64().ok_or(MmError::InvalidPropertyValue)? as u64;
                if pos >= NUM_WHEEL_POSITIONS { return Err(MmError::UnknownPosition); }
                self.position = pos;
                Ok(())
            }
            "Label" => {
                let label = val.as_str().to_string();
                let pos = self.labels.iter().position(|l| l == &label)
                    .ok_or_else(|| MmError::UnknownLabel(label))? as u64;
                self.position = pos;
                Ok(())
            }
            _ => self.props.set(name, val),
        }
    }

    fn property_names(&self)                    -> Vec<String> { self.props.property_names().to_vec() }
    fn has_property(&self, name: &str)          -> bool        { self.props.has_property(name) }
    fn is_property_read_only(&self, name: &str) -> bool        { self.props.entry(name).map(|e| e.read_only).unwrap_or(false) }
    fn device_type(&self)                       -> DeviceType  { DeviceType::State }
    fn busy(&self)                              -> bool        { false }
}

impl StateDevice for SimWheel {
    fn set_position(&mut self, pos: u64) -> MmResult<()> {
        if pos >= NUM_WHEEL_POSITIONS { return Err(MmError::UnknownPosition); }
        self.position = pos;
        Ok(())
    }
    fn get_position(&self) -> MmResult<u64> { Ok(self.position) }
    fn get_number_of_positions(&self) -> u64 { NUM_WHEEL_POSITIONS }
    fn get_position_label(&self, pos: u64) -> MmResult<String> {
        self.labels.get(pos as usize).cloned().ok_or(MmError::UnknownPosition)
    }
    fn set_position_by_label(&mut self, label: &str) -> MmResult<()> {
        let pos = self.labels.iter().position(|l| l == label)
            .ok_or_else(|| MmError::UnknownLabel(label.to_string()))? as u64;
        self.position = pos;
        Ok(())
    }
    fn set_position_label(&mut self, pos: u64, label: &str) -> MmResult<()> {
        if pos >= NUM_WHEEL_POSITIONS { return Err(MmError::UnknownPosition); }
        self.labels[pos as usize] = label.to_string();
        Ok(())
    }
    fn set_gate_open(&mut self, open: bool) -> MmResult<()> { self.gate_open = open; Ok(()) }
    fn get_gate_open(&self) -> MmResult<bool> { Ok(self.gate_open) }
}

// ── SimAdapter ────────────────────────────────────────────────────────────────

const DEVICES: &[DeviceInfo] = &[
    DeviceInfo { name: "SimCamera",  description: "Simulated fluorescence camera with Gaussian PSF spots", device_type: DeviceType::Camera },
    DeviceInfo { name: "SimXYStage", description: "Simulated XY stage",      device_type: DeviceType::XYStage },
    DeviceInfo { name: "SimStage",   description: "Simulated Z stage",        device_type: DeviceType::Stage },
    DeviceInfo { name: "SimShutter", description: "Simulated shutter",        device_type: DeviceType::Shutter },
    DeviceInfo { name: "SimWheel",   description: "Simulated filter wheel",   device_type: DeviceType::State },
];

pub struct SimAdapter;

impl AdapterModule for SimAdapter {
    fn module_name(&self)              -> &'static str          { "sim" }
    fn devices(&self)                  -> &'static [DeviceInfo] { DEVICES }
    fn create_device(&self, name: &str) -> Option<AnyDevice> {
        match name {
            "SimCamera"  => Some(AnyDevice::Camera(Box::new(SimCamera::new()))),
            "SimXYStage" => Some(AnyDevice::XYStage(Box::new(SimXYStage::new()))),
            "SimStage"   => Some(AnyDevice::Stage(Box::new(SimStage::new()))),
            "SimShutter" => Some(AnyDevice::Shutter(Box::new(SimShutter::new()))),
            "SimWheel"   => Some(AnyDevice::StateDevice(Box::new(SimWheel::new()))),
            _            => None,
        }
    }
}

use micromanager::{
    AdapterModule, AnyDevice, Camera, Device, DeviceInfo, DeviceType,
    ImageRoi, MmError, MmResult, PropertyMap, PropertyValue,
};

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
        let sigma2 = 2.0 * SIGMA * SIGMA;

        self.image_buf.resize(w * h, 0);
        self.frame_count = self.frame_count.wrapping_add(1);
        let fc = self.frame_count;

        for y in 0..h {
            for x in 0..w {
                // Gaussian PSF contribution from each bead.
                let mut signal: f32 = 0.0;
                for &(bx, by, amp) in BEADS {
                    let dx = x as f32 - bx;
                    let dy = y as f32 - by;
                    signal += amp * (-(dx * dx + dy * dy) / sigma2).exp();
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

// ── SimAdapter ────────────────────────────────────────────────────────────────

const DEVICES: &[DeviceInfo] = &[DeviceInfo {
    name:        "SimCamera",
    description: "Simulated fluorescence camera with Gaussian PSF spots",
    device_type: DeviceType::Camera,
}];

pub struct SimAdapter;

impl AdapterModule for SimAdapter {
    fn module_name(&self)              -> &'static str          { "sim" }
    fn devices(&self)                  -> &'static [DeviceInfo] { DEVICES }
    fn create_device(&self, name: &str) -> Option<AnyDevice> {
        match name {
            "SimCamera" => Some(AnyDevice::Camera(Box::new(SimCamera::new()))),
            _           => None,
        }
    }
}

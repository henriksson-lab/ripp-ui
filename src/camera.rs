use micromanager::CMMCore;
use micromanager::adapters::demo::DemoAdapter;

pub struct CameraImage {
    pub data: Vec<u8>, // GRAY8
    pub width: u32,
    pub height: u32,
}

pub struct DeviceProp {
    pub device: String,
    pub property: String,
    pub value: String,
}

pub struct DemoCamera {
    core: CMMCore,
}

impl DemoCamera {
    pub fn new() -> Self {
        let mut core = CMMCore::new();
        core.register_adapter(Box::new(DemoAdapter));

        core.load_device("Camera",  "demo", "DCamera").unwrap();
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

    pub fn snap(&mut self) -> CameraImage {
        self.core.snap_image().unwrap();
        let frame = self.core.get_image().unwrap();
        CameraImage {
            data: frame.data,
            width: frame.width,
            height: frame.height,
        }
    }

    pub fn device_props(&self) -> Vec<DeviceProp> {
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

slint::include_modules!();

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;
use ripp::teapot::TeapotRenderer;
use micromanager::CMMCore;
use micromanager::adapters::demo::DemoAdapter;

const TEAPOT_W: u32 = 480;
const TEAPOT_H: u32 = 400;

fn fmt_size(bytes: u64) -> String {
    if bytes < 1_000 {
        format!("{} B", bytes)
    } else if bytes < 1_000_000 {
        format!("{:.1} KB", bytes as f64 / 1_000.0)
    } else if bytes < 1_000_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    }
}

fn load_dir(path: &std::path::Path) -> Vec<FileEntry> {
    let mut entries: Vec<FileEntry> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let size = if is_dir {
                "—".into()
            } else {
                e.metadata().map(|m| fmt_size(m.len())).unwrap_or_default().into()
            };
            FileEntry {
                name: e.file_name().to_string_lossy().to_string().into(),
                is_dir,
                size,
            }
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    entries
}

fn main() {
    let app = AppWindow::new().unwrap();

    let cwd = Rc::new(RefCell::new(
        std::fs::canonicalize(".").unwrap_or_else(|_| PathBuf::from("."))
    ));

    let entries = load_dir(&cwd.borrow());
    app.set_current_path(cwd.borrow().to_string_lossy().to_string().into());
    app.set_file_list(Rc::new(slint::VecModel::from(entries)).into());

    app.on_navigate_to({
        let app_weak = app.as_weak();
        let cwd = cwd.clone();
        move |segment| {
            let mut cwd = cwd.borrow_mut();
            if segment == ".." {
                cwd.pop();
            } else {
                cwd.push(segment.as_str());
            }
            let entries = load_dir(&cwd);
            if let Some(ui) = app_weak.upgrade() {
                ui.set_current_path(cwd.to_string_lossy().to_string().into());
                ui.set_file_list(Rc::new(slint::VecModel::from(entries)).into());
            }
        }
    });

    // Headless wgpu teapot renderer
    let renderer = TeapotRenderer::new(TEAPOT_W, TEAPOT_H);
    let start    = Instant::now();
    let app_weak = app.as_weak();

    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(16),
        move || {
            let rotation = start.elapsed().as_secs_f32() * 0.8;
            let pixels   = renderer.render_frame(rotation);

            let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(TEAPOT_W, TEAPOT_H);
            pb.make_mut_bytes().copy_from_slice(&pixels);

            if let Some(ui) = app_weak.upgrade() {
                ui.set_teapot_image(slint::Image::from_rgba8(pb));
            }
        },
    );

    // Snap one image from the demo camera
    let mut core = CMMCore::new();
    core.register_adapter(Box::new(DemoAdapter));
    core.load_device("Camera", "demo", "DCamera").unwrap();
    core.initialize_device("Camera").unwrap();
    core.set_camera_device("Camera").unwrap();
    core.snap_image().unwrap();
    let frame = core.get_image().unwrap();

    let w = frame.width;
    let h = frame.height;
    let mut pb = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
    let dst = pb.make_mut_bytes();
    for (i, &g) in frame.data.iter().enumerate() {
        dst[i * 4]     = g;
        dst[i * 4 + 1] = g;
        dst[i * 4 + 2] = g;
        dst[i * 4 + 3] = 255;
    }
    app.set_camera_image(slint::Image::from_rgba8(pb));

    app.on_quit(|| std::process::exit(0));

    app.run().unwrap();
    std::process::exit(0);
}

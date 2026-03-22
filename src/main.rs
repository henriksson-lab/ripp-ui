slint::include_modules!();

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;
use ripp::teapot::TeapotRenderer;

const TEAPOT_W: u32 = 480;
const TEAPOT_H: u32 = 400;

fn load_dir(path: &std::path::Path) -> Vec<FileEntry> {
    let mut entries: Vec<FileEntry> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| FileEntry {
            name: e.file_name().to_string_lossy().to_string().into(),
            is_dir: e.file_type().map(|t| t.is_dir()).unwrap_or(false),
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

    app.on_quit(|| std::process::exit(0));

    app.run().unwrap();
    std::process::exit(0);
}

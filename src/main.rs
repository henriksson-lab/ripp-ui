slint::include_modules!();

use std::time::Instant;
use claude_ui::teapot::TeapotRenderer;

const TEAPOT_W: u32 = 480;
const TEAPOT_H: u32 = 400;

fn main() {
    let app = AppWindow::new().unwrap();

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
}

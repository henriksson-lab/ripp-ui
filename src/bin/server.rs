/// Headless wgpu teapot renderer streamed as MJPEG over HTTP.
///
/// Run with:   cargo run --bin server
/// Then open:  http://127.0.0.1:8080
use actix_web::{web, App, HttpResponse, HttpServer};
use bytes::Bytes;
use bytemuck::{Pod, Zeroable};
use futures::stream;
use glam::Mat4;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::broadcast;
use wgpu::util::DeviceExt;

// ── Frame dimensions ───────────────────────────────────────────────────────
// 960 * 4 = 3840 bytes/row  →  3840 / 256 = 15 (exact), zero staging padding.
const W: u32 = 960;
const H: u32 = 540;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const BPR: u32 = W * 4; // bytes per row (already 256-aligned)

// ── GPU data types ─────────────────────────────────────────────────────────
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
}

// ── Headless renderer ──────────────────────────────────────────────────────
struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    ubuf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    staging: wgpu::Buffer,
    index_count: u32,
}

impl Renderer {
    async fn new() -> Self {
        // ── wgpu init ────────────────────────────────────────────────────────
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .expect("no GPU adapter found");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .expect("failed to create device");

        // ── Load & triangulate mesh ──────────────────────────────────────────
        let (models, _) = tobj::load_obj(
            "assets/teapot.obj",
            &tobj::LoadOptions { triangulate: true, ..Default::default() },
        )
        .expect("failed to load teapot.obj");
        let mesh = &models[0].mesh;

        // Expand to flat-shaded triangles (OBJ has no normals)
        let pos = &mesh.positions;
        let idx = &mesh.indices;
        let mut verts: Vec<Vertex> = Vec::with_capacity(idx.len());
        let mut flat_idx: Vec<u32> = Vec::with_capacity(idx.len());
        for (ti, tri) in idx.chunks(3).enumerate() {
            let p = |i: u32| {
                let b = i as usize * 3;
                glam::Vec3::new(pos[b], pos[b + 1], pos[b + 2])
            };
            let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
            let n = (b - a).cross(c - a).normalize_or_zero().to_array();
            let base = (ti * 3) as u32;
            verts.push(Vertex { pos: a.to_array(), normal: n });
            verts.push(Vertex { pos: b.to_array(), normal: n });
            verts.push(Vertex { pos: c.to_array(), normal: n });
            flat_idx.extend_from_slice(&[base, base + 1, base + 2]);
        }
        let index_count = flat_idx.len() as u32;

        // ── Buffers ──────────────────────────────────────────────────────────
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&flat_idx),
            usage: wgpu::BufferUsages::INDEX,
        });
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Bind group ───────────────────────────────────────────────────────
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: ubuf.as_entire_binding(),
            }],
        });

        // ── Shader ───────────────────────────────────────────────────────────
        let shader_src = include_str!("../../assets/shaders/teapot.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });

        // ── Pipeline ─────────────────────────────────────────────────────────
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(FORMAT.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let ext = wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 };

        // ── Colour target ─────────────────────────────────────────────────────
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: ext,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        // ── Depth target ──────────────────────────────────────────────────────
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: ext,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());

        // ── Staging buffer ────────────────────────────────────────────────────
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (BPR * H) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Seed uniform buffer
        queue.write_buffer(
            &ubuf,
            0,
            bytemuck::cast_slice(&[Uniforms { mvp: Mat4::IDENTITY.to_cols_array_2d() }]),
        );

        Self { device, queue, pipeline, vbuf, ibuf, ubuf, bind_group,
               target, target_view, depth_view, staging, index_count }
    }

    /// Render one frame and return it as a JPEG byte buffer.
    fn render_frame(&self, rotation: f32) -> Vec<u8> {
        // Update MVP
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, W as f32 / H as f32, 0.1, 100.0);
        let view = Mat4::look_at_rh(glam::Vec3::new(0.0, 2.0, 6.0), glam::Vec3::ZERO, glam::Vec3::Y);
        let mvp = proj * view * Mat4::from_rotation_y(rotation);
        self.queue.write_buffer(
            &self.ubuf, 0,
            bytemuck::cast_slice(&[Uniforms { mvp: mvp.to_cols_array_2d() }]),
        );

        // Encode render pass
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.1, b: 0.15, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vbuf.slice(..));
            pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }

        // Copy texture → staging buffer
        enc.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &self.staging,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(BPR),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        );

        self.queue.submit([enc.finish()]);

        // CPU readback (blocking)
        let slice = self.staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();

        // BPR is already aligned (960*4=3840, 3840/256=15 exact), so direct copy
        let pixels: Vec<u8> = slice.get_mapped_range().to_vec();
        self.staging.unmap();

        // RGBA → RGB JPEG
        let rgba = image::RgbaImage::from_raw(W, H, pixels).unwrap();
        let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
        let mut jpeg = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut jpeg);
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, 82);
        encoder.encode(rgb.as_raw(), W, H, image::ExtendedColorType::Rgb8).unwrap();
        jpeg
    }
}

// Renderer is sent into a blocking thread — verify it is Send/Sync.
unsafe impl Send for Renderer {}
unsafe impl Sync for Renderer {}

// ── HTTP handlers ──────────────────────────────────────────────────────────

async fn index() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(include_str!("../../assets/index.html"))
}

async fn mjpeg_stream(
    tx: web::Data<Arc<broadcast::Sender<Bytes>>>,
) -> HttpResponse {
    let rx = tx.subscribe();

    let body = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(jpeg) => {
                    let mut frame = Vec::with_capacity(64 + jpeg.len());
                    frame.extend_from_slice(
                        format!(
                            "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                            jpeg.len()
                        )
                        .as_bytes(),
                    );
                    frame.extend_from_slice(&jpeg);
                    frame.extend_from_slice(b"\r\n");
                    return Some((Ok::<_, actix_web::Error>(Bytes::from(frame)), rx));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    HttpResponse::Ok()
        .content_type("multipart/x-mixed-replace; boundary=frame")
        .streaming(body)
}

// ── Main ───────────────────────────────────────────────────────────────────

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    println!("Initialising GPU…");
    let renderer = Renderer::new().await;
    println!("GPU ready. Starting render loop.");

    let (tx, _) = broadcast::channel::<Bytes>(4);
    let tx: Arc<broadcast::Sender<Bytes>> = Arc::new(tx);

    let shutdown = Arc::new(AtomicBool::new(false));

    // Render loop in a dedicated blocking thread
    let tx_render = tx.clone();
    let shutdown_render = shutdown.clone();
    tokio::task::spawn_blocking(move || {
        let start = Instant::now();
        while !shutdown_render.load(Ordering::Relaxed) {
            // Only render when someone is watching
            if tx_render.receiver_count() > 0 {
                let rotation = start.elapsed().as_secs_f32() * 0.8;
                let jpeg = renderer.render_frame(rotation);
                let _ = tx_render.send(Bytes::from(jpeg));
            }
            std::thread::sleep(std::time::Duration::from_millis(33)); // ~30 fps
        }
    });

    let addr = "127.0.0.1:8080";
    println!("Listening on http://{addr}  (Ctrl+C to stop)");

    let server = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(tx.clone()))
            .route("/", web::get().to(index))
            .route("/stream", web::get().to(mjpeg_stream))
    })
    .bind(addr)?
    .run();

    let handle = server.handle();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.expect("failed to listen for Ctrl+C");
        println!("\nShutting down…");
        shutdown.store(true, Ordering::Relaxed); // unblock the render loop
        handle.stop(true).await;                 // drain active HTTP connections
    });

    server.await
}

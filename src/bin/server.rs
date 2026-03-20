/// Headless wgpu teapot renderer streamed as MJPEG over HTTP.
///
/// Run with:   cargo run --bin server
/// Then open:  http://127.0.0.1:8080
use actix_web::{web, App, HttpResponse, HttpServer};
use bytes::Bytes;
use bytemuck::{Pod, Zeroable};
use futures::stream;
use glam::Mat4;
use serde::Deserialize;
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

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
    w: u32,
    h: u32,
    bpr: u32,
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
    staging: [wgpu::Buffer; 2],
    index_count: u32,
}

impl Renderer {
    async fn new(w: u32, h: u32) -> Self {
        let bpr = (w * 4 + 255) / 256 * 256;

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

        let ext = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };

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

        // ── Two staging buffers (ping-pong) ───────────────────────────────────
        let mk_staging = || device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (bpr * h) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging = [mk_staging(), mk_staging()];

        queue.write_buffer(
            &ubuf,
            0,
            bytemuck::cast_slice(&[Uniforms { mvp: Mat4::IDENTITY.to_cols_array_2d() }]),
        );

        Self { w, h, bpr, device, queue, pipeline, vbuf, ibuf, ubuf, bind_group,
               target, target_view, depth_view, staging, index_count }
    }

    fn submit_frame(&self, rotation: f32, slot: usize) -> wgpu::SubmissionIndex {
        let proj = Mat4::perspective_rh(
            std::f32::consts::FRAC_PI_4,
            self.w as f32 / self.h as f32,
            0.1,
            100.0,
        );
        let view = Mat4::look_at_rh(
            glam::Vec3::new(0.0, 2.0, 6.0),
            glam::Vec3::ZERO,
            glam::Vec3::Y,
        );
        let mvp = proj * view * Mat4::from_rotation_y(rotation);
        self.queue.write_buffer(
            &self.ubuf, 0,
            bytemuck::cast_slice(&[Uniforms { mvp: mvp.to_cols_array_2d() }]),
        );

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
        enc.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &self.staging[slot],
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(self.bpr),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d { width: self.w, height: self.h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()])
    }

    fn readback_and_encode(&self, sub: wgpu::SubmissionIndex, slot: usize) -> Vec<u8> {
        let buf = &self.staging[slot];
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        self.device.poll(wgpu::Maintain::WaitForSubmissionIndex(sub));
        rx.recv().unwrap().unwrap();

        let mapped = slice.get_mapped_range();
        let jpeg = encode_jpeg(&*mapped, self.w, self.h, self.bpr);
        drop(mapped);
        buf.unmap();
        jpeg
    }
}

/// JPEG-encode raw RGBA pixels, handling row-stride padding when bpr > w*4.
fn encode_jpeg(rgba: &[u8], w: u32, h: u32, bpr: u32) -> Vec<u8> {
    let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_EXT_RGBA);
    comp.set_size(w as usize, h as usize);
    comp.set_quality(82.0);
    let mut comp = comp.start_compress(Vec::new()).expect("mozjpeg start");
    if bpr == w * 4 {
        comp.write_scanlines(rgba).expect("mozjpeg write");
    } else {
        for row in 0..h as usize {
            let start = row * bpr as usize;
            let end = start + (w * 4) as usize;
            comp.write_scanlines(&rgba[start..end]).expect("mozjpeg write");
        }
    }
    comp.finish().expect("mozjpeg finish")
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

#[derive(Deserialize)]
struct SizeQuery {
    w: Option<u32>,
    h: Option<u32>,
}

async fn mjpeg_stream(query: web::Query<SizeQuery>) -> HttpResponse {
    let w = query.w.unwrap_or(480).max(64).min(3840);
    let h = query.h.unwrap_or(540).max(64).min(2160);

    let renderer = Renderer::new(w, h).await;
    let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(2);

    tokio::task::spawn_blocking(move || {
        let start = Instant::now();
        let mut prev: Option<(wgpu::SubmissionIndex, usize)> = None;
        let mut frame: usize = 0;
        loop {
            let deadline = Instant::now() + Duration::from_millis(33);
            let slot = frame % 2;
            let rotation = start.elapsed().as_secs_f32() * 0.8;
            let sub = renderer.submit_frame(rotation, slot);
            if let Some((ps, pslot)) = prev.replace((sub, slot)) {
                let jpeg = renderer.readback_and_encode(ps, pslot);
                if tx.blocking_send(Bytes::from(jpeg)).is_err() {
                    break;
                }
            }
            frame += 1;
            let now = Instant::now();
            if now < deadline {
                std::thread::sleep(deadline - now);
            }
        }
    });

    let body = stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|jpeg| {
            let header = format!(
                "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                jpeg.len()
            );
            let mut frame = Vec::with_capacity(header.len() + jpeg.len() + 2);
            frame.extend_from_slice(header.as_bytes());
            frame.extend_from_slice(&jpeg);
            frame.extend_from_slice(b"\r\n");
            (Ok::<_, actix_web::Error>(Bytes::from(frame)), rx)
        })
    });

    HttpResponse::Ok()
        .content_type("multipart/x-mixed-replace; boundary=frame")
        .streaming(body)
}

// ── Main ───────────────────────────────────────────────────────────────────

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    println!("Listening on http://127.0.0.1:8080  (Ctrl+C to stop)");
    let server = HttpServer::new(|| {
        App::new()
            .route("/", web::get().to(index))
            .route("/stream", web::get().to(mjpeg_stream))
    })
    .bind("127.0.0.1:8080")?
    .run();

    let handle = server.handle();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.expect("ctrl_c");
        println!("\nShutting down…");
        handle.stop(true).await;
    });

    server.await
}

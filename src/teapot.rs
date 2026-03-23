/// Standalone wgpu offscreen teapot renderer shared by the desktop and server binaries.
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Vertex {
    pos:    [f32; 3],
    normal: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
}

pub struct TeapotRenderer {
    w:           u32,
    h:           u32,
    bpr:         u32, // bytes-per-row (wgpu-aligned)
    device:      wgpu::Device,
    queue:       wgpu::Queue,
    pipeline:    wgpu::RenderPipeline,
    vbuf:        wgpu::Buffer,
    ibuf:        wgpu::Buffer,
    ubuf:        wgpu::Buffer,
    bind_group:  wgpu::BindGroup,
    color_tex:   wgpu::Texture,
    color_view:  wgpu::TextureView,
    depth_view:  wgpu::TextureView,
    staging:     wgpu::Buffer,
    index_count: u32,
}

impl TeapotRenderer {
    pub fn new(w: u32, h: u32) -> Self {
        futures::executor::block_on(Self::new_async(w, h))
    }

    async fn new_async(w: u32, h: u32) -> Self {
        // wgpu requires bytes-per-row to be a multiple of 256
        let bpr = (w * 4 + 255) / 256 * 256;

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference:       wgpu::PowerPreference::HighPerformance,
                compatible_surface:     None,
                force_fallback_adapter: false,
            })
            .await
            .expect("no GPU adapter");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .expect("device creation failed");

        // ── Load & triangulate mesh ──────────────────────────────────────────
        let (models, _) = tobj::load_obj(
            "assets/teapot.obj",
            &tobj::LoadOptions { triangulate: true, ..Default::default() },
        )
        .expect("failed to load assets/teapot.obj");
        let mesh = &models[0].mesh;
        let pos  = &mesh.positions;
        let idx  = &mesh.indices;

        let mut verts:    Vec<Vertex> = Vec::with_capacity(idx.len());
        let mut flat_idx: Vec<u32>    = Vec::with_capacity(idx.len());
        for (ti, tri) in idx.chunks(3).enumerate() {
            let p = |i: u32| {
                let b = i as usize * 3;
                glam::Vec3::new(pos[b], pos[b + 1], pos[b + 2])
            };
            let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
            let n    = (b - a).cross(c - a).normalize_or_zero().to_array();
            let base = (ti * 3) as u32;
            verts.push(Vertex { pos: a.to_array(), normal: n });
            verts.push(Vertex { pos: b.to_array(), normal: n });
            verts.push(Vertex { pos: c.to_array(), normal: n });
            flat_idx.extend_from_slice(&[base, base + 1, base + 2]);
        }
        let index_count = flat_idx.len() as u32;

        // ── Buffers ──────────────────────────────────────────────────────────
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    None,
            contents: bytemuck::cast_slice(&verts),
            usage:    wgpu::BufferUsages::VERTEX,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    None,
            contents: bytemuck::cast_slice(&flat_idx),
            usage:    wgpu::BufferUsages::INDEX,
        });
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              None,
            size:               std::mem::size_of::<Uniforms>() as u64,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Bind group ───────────────────────────────────────────────────────
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding:    0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty:                 wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size:   None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   None,
            layout:  &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding:  0,
                resource: ubuf.as_entire_binding(),
            }],
        });

        // ── Shader & pipeline ────────────────────────────────────────────────
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  None,
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../assets/shaders/teapot.wgsl").into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:                None,
            bind_group_layouts:   &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module:      &shader,
                entry_point: "vs_main",
                buffers:     &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode:    wgpu::VertexStepMode::Vertex,
                    attributes:   &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module:      &shader,
                entry_point: "fs_main",
                targets:     &[Some(FORMAT.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology:   wgpu::PrimitiveTopology::TriangleList,
                cull_mode:  Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format:              wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare:       wgpu::CompareFunction::Less,
                stencil:             wgpu::StencilState::default(),
                bias:                wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview:   None,
        });

        // ── Render target ────────────────────────────────────────────────────
        let ext = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };

        let color_tex = device.create_texture(&wgpu::TextureDescriptor {
            label:            None,
            size:             ext,
            mip_level_count:  1,
            sample_count:     1,
            dimension:        wgpu::TextureDimension::D2,
            format:           FORMAT,
            usage:            wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::COPY_SRC,
            view_formats:     &[],
        });
        let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
            label:            None,
            size:             ext,
            mip_level_count:  1,
            sample_count:     1,
            dimension:        wgpu::TextureDimension::D2,
            format:           wgpu::TextureFormat::Depth32Float,
            usage:            wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats:     &[],
        });
        let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label:              None,
            size:               (bpr * h) as u64,
            usage:              wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        queue.write_buffer(
            &ubuf,
            0,
            bytemuck::cast_slice(&[Uniforms { mvp: Mat4::IDENTITY.to_cols_array_2d() }]),
        );

        Self {
            w, h, bpr, device, queue, pipeline, vbuf, ibuf, ubuf, bind_group,
            color_tex, color_view, depth_view, staging, index_count,
        }
    }

    /// Render one frame and return destrided RGBA8 bytes (length = w * h * 4).
    /// `yaw` = azimuth (radians), `pitch` = elevation (radians), `distance` = eye distance.
    pub fn render_frame(&self, yaw: f32, pitch: f32, distance: f32) -> Vec<u8> {
        let aspect = self.w as f32 / self.h as f32;
        let proj   = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 200.0);
        let eye    = glam::Vec3::new(
            distance * pitch.cos() * yaw.sin(),
            distance * pitch.sin(),
            distance * pitch.cos() * yaw.cos(),
        );
        let view   = Mat4::look_at_rh(eye, glam::Vec3::ZERO, glam::Vec3::Y);
        let mvp = proj * view;
        self.queue.write_buffer(
            &self.ubuf,
            0,
            bytemuck::cast_slice(&[Uniforms { mvp: mvp.to_cols_array_2d() }]),
        );

        let mut enc = self.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor::default(),
        );
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &self.color_view,
                    resolve_target: None,
                    ops:            wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1, g: 0.1, b: 0.15, a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view:       &self.depth_view,
                    depth_ops:  Some(wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes:   None,
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
                texture:   &self.color_tex,
                mip_level: 0,
                origin:    wgpu::Origin3d::ZERO,
                aspect:    wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &self.staging,
                layout: wgpu::ImageDataLayout {
                    offset:         0,
                    bytes_per_row:  Some(self.bpr),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d { width: self.w, height: self.h, depth_or_array_layers: 1 },
        );
        let sub = self.queue.submit([enc.finish()]);

        // Blocking readback
        let slice = self.staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        self.device.poll(wgpu::Maintain::WaitForSubmissionIndex(sub));
        rx.recv().unwrap().unwrap();

        let mapped = slice.get_mapped_range();
        let raw    = mapped.to_vec();
        drop(mapped);
        self.staging.unmap();

        // Strip stride padding if present
        if self.bpr == self.w * 4 {
            raw
        } else {
            let mut out = Vec::with_capacity((self.w * self.h * 4) as usize);
            for row in 0..self.h as usize {
                let s = row * self.bpr as usize;
                out.extend_from_slice(&raw[s..s + self.w as usize * 4]);
            }
            out
        }
    }
}

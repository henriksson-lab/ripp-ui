/// wgpu offscreen renderer for the 2D image viewer.
///
/// Upload raw image data once per z-slice; re-render with GPU uniforms for
/// pan / zoom / lo / hi without touching the texture.
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Uniforms {
    cam_x: f32,
    cam_y: f32,
    zoom:  f32,
    lo:    f32,
    hi:    f32,
    out_w: f32,
    out_h: f32,
    _pad:  f32,
}

pub struct Viewer2dRenderer {
    device:     wgpu::Device,
    queue:      wgpu::Queue,
    pipeline:   wgpu::RenderPipeline,
    sampler:    wgpu::Sampler,
    bgl:        wgpu::BindGroupLayout,
    ubuf:       wgpu::Buffer,
    // Recreated when image/viewport size changes:
    image_tex:  Option<wgpu::Texture>,
    image_view: Option<wgpu::TextureView>,
    bind_group: Option<wgpu::BindGroup>,
    color_tex:  Option<wgpu::Texture>,
    color_view: Option<wgpu::TextureView>,
    staging:    Option<wgpu::Buffer>,
    out_w:      u32,
    out_h:      u32,
    bpr:        u32,
}

impl Viewer2dRenderer {
    pub fn new() -> Self {
        futures::executor::block_on(Self::new_async())
    }

    async fn new_async() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference:       wgpu::PowerPreference::HighPerformance,
                compatible_surface:     None,
                force_fallback_adapter: false,
            })
            .await
            .expect("no GPU adapter for Viewer2d");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .expect("device creation failed for Viewer2d");

        // Nearest-neighbour sampler, clamped at edges
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter:     wgpu::FilterMode::Nearest,
            min_filter:     wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Uniform buffer (32 bytes)
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("viewer2d-uniforms"),
            size:               std::mem::size_of::<Uniforms>() as u64,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Bind group layout: uniform + texture + sampler
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("viewer2d-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty:                 wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size:   None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding:    1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled:   false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding:    2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("viewer2d-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../assets/shaders/viewer2d.wgsl").into(),
            ),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:                Some("viewer2d-pipeline-layout"),
            bind_group_layouts:   &[&bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("viewer2d-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module:      &shader,
                entry_point: "vs_main",
                buffers:     &[], // fullscreen triangle, no vertex buffers
            },
            fragment: Some(wgpu::FragmentState {
                module:      &shader,
                entry_point: "fs_main",
                targets:     &[Some(FORMAT.into())],
            }),
            primitive:     wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample:   wgpu::MultisampleState::default(),
            multiview:     None,
        });

        Self {
            device,
            queue,
            pipeline,
            sampler,
            bgl,
            ubuf,
            image_tex:  None,
            image_view: None,
            bind_group: None,
            color_tex:  None,
            color_view: None,
            staging:    None,
            out_w: 0,
            out_h: 0,
            bpr:   0,
        }
    }

    pub fn out_w(&self) -> u32 { self.out_w }
    pub fn out_h(&self) -> u32 { self.out_h }

    /// Upload a new image (z-slice).  Converts gray/RGB → RGBA8 on the CPU once.
    /// Recreates the render target only when dimensions change.
    pub fn upload(&mut self, bytes: &[u8], img_w: u32, img_h: u32, is_gray: bool) {
        // Convert to RGBA8
        let rgba: Vec<u8> = if is_gray {
            bytes.iter().flat_map(|&g| [g, g, g, 255u8]).collect()
        } else {
            bytes.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255u8]).collect()
        };

        // Recreate render target if size changed
        if self.out_w != img_w || self.out_h != img_h {
            let bpr = (img_w * 4 + 255) / 256 * 256;
            let ext = wgpu::Extent3d { width: img_w, height: img_h, depth_or_array_layers: 1 };

            let color_tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label:           Some("viewer2d-color"),
                size:            ext,
                mip_level_count: 1,
                sample_count:    1,
                dimension:       wgpu::TextureDimension::D2,
                format:          FORMAT,
                usage:           wgpu::TextureUsages::RENDER_ATTACHMENT
                               | wgpu::TextureUsages::COPY_SRC,
                view_formats:    &[],
            });
            let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());

            let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
                label:              Some("viewer2d-staging"),
                size:               (bpr * img_h) as u64,
                usage:              wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            self.color_tex  = Some(color_tex);
            self.color_view = Some(color_view);
            self.staging    = Some(staging);
            self.out_w      = img_w;
            self.out_h      = img_h;
            self.bpr        = bpr;
        }

        // Upload image texture
        let ext = wgpu::Extent3d { width: img_w, height: img_h, depth_or_array_layers: 1 };
        let image_tex = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label:           Some("viewer2d-image"),
                size:            ext,
                mip_level_count: 1,
                sample_count:    1,
                dimension:       wgpu::TextureDimension::D2,
                format:          FORMAT,
                usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats:    &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &rgba,
        );
        let image_view = image_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // Recreate bind group with new texture
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("viewer2d-bind-group"),
            layout:  &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding:  1,
                    resource: wgpu::BindingResource::TextureView(&image_view),
                },
                wgpu::BindGroupEntry {
                    binding:  2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        self.image_tex  = Some(image_tex);
        self.image_view = Some(image_view);
        self.bind_group = Some(bind_group);
    }

    /// Render current frame with given camera/level parameters.
    /// Returns `None` if no image has been uploaded yet.
    pub fn render(&self, cam_x: f64, cam_y: f64, zoom: f64, lo: f32, hi: f32) -> Option<Vec<u8>> {
        let bind_group = self.bind_group.as_ref()?;
        let color_view = self.color_view.as_ref()?;
        let staging    = self.staging.as_ref()?;
        let color_tex  = self.color_tex.as_ref()?;

        // Update uniforms
        self.queue.write_buffer(
            &self.ubuf,
            0,
            bytemuck::cast_slice(&[Uniforms {
                cam_x: cam_x as f32,
                cam_y: cam_y as f32,
                zoom:  zoom as f32,
                lo,
                hi,
                out_w: self.out_w as f32,
                out_h: self.out_h as f32,
                _pad:  0.0,
            }]),
        );

        let mut enc = self.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("viewer2d-enc") },
        );
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewer2d-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           color_view,
                    resolve_target: None,
                    ops:            wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes:         None,
                occlusion_query_set:      None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        enc.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture:   color_tex,
                mip_level: 0,
                origin:    wgpu::Origin3d::ZERO,
                aspect:    wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: staging,
                layout: wgpu::ImageDataLayout {
                    offset:         0,
                    bytes_per_row:  Some(self.bpr),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d { width: self.out_w, height: self.out_h, depth_or_array_layers: 1 },
        );
        let sub = self.queue.submit([enc.finish()]);

        // Blocking readback
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        self.device.poll(wgpu::Maintain::WaitForSubmissionIndex(sub));
        rx.recv().unwrap().unwrap();

        let mapped = slice.get_mapped_range();
        let raw = mapped.to_vec();
        drop(mapped);
        staging.unmap();

        // Destride
        Some(if self.bpr == self.out_w * 4 {
            raw
        } else {
            let mut out = Vec::with_capacity((self.out_w * self.out_h * 4) as usize);
            for row in 0..self.out_h as usize {
                let s = row * self.bpr as usize;
                out.extend_from_slice(&raw[s..s + self.out_w as usize * 4]);
            }
            out
        })
    }
}

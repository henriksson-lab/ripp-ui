/// wgpu offscreen renderer for the 2D image viewer.
///
/// Upload raw image data once per z-slice; re-render with GPU uniforms for
/// pan / zoom / lo / hi without touching the texture.
///
/// The wgpu device is initialised lazily on the first `upload()` call so that
/// binaries that never show the 2D viewer (e.g. the headless server when no
/// file is opened) do not pay the GPU-device creation cost or contend with the
/// TeapotRenderer's device.
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use crate::session::{Camera2d, ColorMappingRange, WindowSize};

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

/// All wgpu state.  Created on the first `upload()` call.
struct Gpu {
    device:   wgpu::Device,
    queue:    wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    sampler:  wgpu::Sampler,
    bgl:      wgpu::BindGroupLayout,
    ubuf:     wgpu::Buffer,
    // Recreated when image/viewport size changes:
    image_tex:  Option<wgpu::Texture>,
    image_view: Option<wgpu::TextureView>,
    bind_group: Option<wgpu::BindGroup>,
    color_tex:  Option<wgpu::Texture>,
    color_view: Option<wgpu::TextureView>,
    staging:    Option<wgpu::Buffer>,
    size: WindowSize,
    bpr:  u32,
}

pub struct Viewer2dRenderer {
    gpu: Option<Gpu>,
}

impl Viewer2dRenderer {
    /// Create a shell.  No GPU work happens until the first `upload()`.
    pub fn new() -> Self {
        Self { gpu: None }
    }

    pub fn size(&self) -> WindowSize {
        self.gpu.as_ref().map_or(WindowSize { w: 0, h: 0 }, |g| g.size)
    }

    /// Upload a new image (z-slice).  Converts gray/RGB → RGBA8 on the CPU once.
    /// Initialises the wgpu device on the first call.
    pub fn upload(&mut self, bytes: &[u8], img_w: u32, img_h: u32, is_gray: bool) {
        let gpu = self.gpu.get_or_insert_with(|| futures::executor::block_on(Gpu::new()));

        // Convert to RGBA8
        let rgba: Vec<u8> = if is_gray {
            bytes.iter().flat_map(|&g| [g, g, g, 255u8]).collect()
        } else {
            bytes.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255u8]).collect()
        };

        // Recreate render target if size changed
        if gpu.size.w != img_w || gpu.size.h != img_h {
            let bpr = (img_w * 4 + 255) / 256 * 256;
            let ext = wgpu::Extent3d { width: img_w, height: img_h, depth_or_array_layers: 1 };

            let color_tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
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

            let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label:              Some("viewer2d-staging"),
                size:               (bpr * img_h) as u64,
                usage:              wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            gpu.color_tex  = Some(color_tex);
            gpu.color_view = Some(color_view);
            gpu.staging    = Some(staging);
            gpu.size       = WindowSize { w: img_w, h: img_h };
            gpu.bpr        = bpr;
        }

        // Upload image texture
        let ext = wgpu::Extent3d { width: img_w, height: img_h, depth_or_array_layers: 1 };
        let image_tex = gpu.device.create_texture_with_data(
            &gpu.queue,
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
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("viewer2d-bind-group"),
            layout:  &gpu.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: gpu.ubuf.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding:  1,
                    resource: wgpu::BindingResource::TextureView(&image_view),
                },
                wgpu::BindGroupEntry {
                    binding:  2,
                    resource: wgpu::BindingResource::Sampler(&gpu.sampler),
                },
            ],
        });

        gpu.image_tex  = Some(image_tex);
        gpu.image_view = Some(image_view);
        gpu.bind_group = Some(bind_group);
    }

    /// Render current frame with given camera/level parameters.
    /// Returns `None` if no image has been uploaded yet.
    pub fn render(&self, cam: Camera2d, color: ColorMappingRange) -> Option<Vec<u8>> {
        let gpu = self.gpu.as_ref()?;
        let bind_group = gpu.bind_group.as_ref()?;
        let color_view = gpu.color_view.as_ref()?;
        let staging    = gpu.staging.as_ref()?;
        let color_tex  = gpu.color_tex.as_ref()?;

        // Update uniforms
        gpu.queue.write_buffer(
            &gpu.ubuf,
            0,
            bytemuck::cast_slice(&[Uniforms {
                cam_x: cam.x as f32,
                cam_y: cam.y as f32,
                zoom:  cam.zoom as f32,
                lo:    color.lo,
                hi:    color.hi,
                out_w: gpu.size.w as f32,
                out_h: gpu.size.h as f32,
                _pad:  0.0,
            }]),
        );

        let mut enc = gpu.device.create_command_encoder(
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
            pass.set_pipeline(&gpu.pipeline);
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
                    bytes_per_row:  Some(gpu.bpr),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d { width: gpu.size.w, height: gpu.size.h, depth_or_array_layers: 1 },
        );
        let sub = gpu.queue.submit([enc.finish()]);

        // Blocking readback
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        gpu.device.poll(wgpu::Maintain::WaitForSubmissionIndex(sub));
        rx.recv().unwrap().unwrap();

        let mapped = slice.get_mapped_range();
        let raw = mapped.to_vec();
        drop(mapped);
        staging.unmap();

        // Destride
        Some(if gpu.bpr == gpu.size.w * 4 {
            raw
        } else {
            let mut out = Vec::with_capacity((gpu.size.w * gpu.size.h * 4) as usize);
            for row in 0..gpu.size.h as usize {
                let s = row * gpu.bpr as usize;
                out.extend_from_slice(&raw[s..s + gpu.size.w as usize * 4]);
            }
            out
        })
    }
}

impl Gpu {
    async fn new() -> Self {
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter:     wgpu::FilterMode::Nearest,
            min_filter:     wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("viewer2d-uniforms"),
            size:               std::mem::size_of::<Uniforms>() as u64,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

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
                buffers:     &[],
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
            device, queue, pipeline, sampler, bgl, ubuf,
            image_tex:  None,
            image_view: None,
            bind_group: None,
            color_tex:  None,
            color_view: None,
            staging:    None,
            size: WindowSize { w: 0, h: 0 },
            bpr:  0,
        }
    }
}

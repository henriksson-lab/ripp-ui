use iced::widget::shader::{self, Storage, Viewport};
use iced::widget::shader::wgpu;
use iced::widget::shader::wgpu::util::DeviceExt;
use iced::Rectangle;
use glam::Mat4;
use bytemuck::{Pod, Zeroable};

/// Lightweight state cloned into each frame's draw call
#[derive(Debug, Clone)]
pub struct Scene {
    pub rotation: f32,  // radians
}

impl Scene {
    pub fn new() -> Self { Self { rotation: 0.0 } }
}

/// Per-frame GPU primitive
#[derive(Debug)]
pub struct TeapotPrimitive {
    rotation: f32,
}

/// Long-lived GPU resources (created once, stored in shader::Storage)
struct TeapotPipeline {
    pipeline:    wgpu::RenderPipeline,
    vertex_buf:  wgpu::Buffer,
    index_buf:   wgpu::Buffer,
    uniform_buf: wgpu::Buffer,
    bind_group:  wgpu::BindGroup,
    index_count: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Vertex { pos: [f32; 3], normal: [f32; 3] }

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Uniforms { mvp: [[f32; 4]; 4] }

impl TeapotPipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        // Load teapot.obj
        let (models, _) = tobj::load_obj("assets/teapot.obj",
            &tobj::LoadOptions { triangulate: true, ..Default::default() }).unwrap();
        let mesh = &models[0].mesh;

        // Build flat-shaded vertex list: expand triangles so each has its own 3 verts + face normal
        let pos = &mesh.positions;
        let idx = &mesh.indices;

        let mut vertices: Vec<Vertex> = Vec::with_capacity(idx.len());
        let mut flat_indices: Vec<u32> = Vec::with_capacity(idx.len());

        for (tri_i, tri) in idx.chunks(3).enumerate() {
            let p = |i: u32| -> glam::Vec3 {
                let b = (i as usize) * 3;
                glam::Vec3::new(pos[b], pos[b+1], pos[b+2])
            };
            let a = p(tri[0]);
            let b = p(tri[1]);
            let c = p(tri[2]);
            let n = (b - a).cross(c - a).normalize_or_zero();
            let normal = [n.x, n.y, n.z];
            let base = (tri_i * 3) as u32;
            vertices.push(Vertex { pos: a.to_array(), normal });
            vertices.push(Vertex { pos: b.to_array(), normal });
            vertices.push(Vertex { pos: c.to_array(), normal });
            flat_indices.extend_from_slice(&[base, base+1, base+2]);
        }

        let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&flat_indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("../assets/shaders/teapot.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
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
                targets: &[Some(format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        // Initial uniform upload so buffer isn't empty
        let init_mvp = Mat4::IDENTITY;
        queue.write_buffer(&uniform_buf, 0,
            bytemuck::cast_slice(&[Uniforms { mvp: init_mvp.to_cols_array_2d() }]));

        Self { pipeline, vertex_buf, index_buf, uniform_buf, bind_group,
               index_count: flat_indices.len() as u32 }
    }

    fn update_uniforms(&self, queue: &wgpu::Queue, rotation: f32, bounds: &Rectangle) {
        let aspect = bounds.width / bounds.height;
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 100.0);
        let view = Mat4::look_at_rh(
            glam::Vec3::new(0.0, 2.0, 6.0),
            glam::Vec3::ZERO,
            glam::Vec3::Y,
        );
        let model = Mat4::from_rotation_y(rotation);
        let mvp = proj * view * model;
        queue.write_buffer(&self.uniform_buf, 0,
            bytemuck::cast_slice(&[Uniforms { mvp: mvp.to_cols_array_2d() }]));
    }
}

impl shader::Primitive for TeapotPrimitive {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        storage: &mut Storage,
        bounds: &Rectangle,
        _viewport: &Viewport,
    ) {
        if !storage.has::<TeapotPipeline>() {
            storage.store(TeapotPipeline::new(device, queue, format));
        }
        let pipeline = storage.get::<TeapotPipeline>().unwrap();
        pipeline.update_uniforms(queue, self.rotation, bounds);
    }

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        storage: &Storage,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let pipeline = storage.get::<TeapotPipeline>().unwrap();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.1, b: 0.15, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_scissor_rect(clip_bounds.x, clip_bounds.y, clip_bounds.width, clip_bounds.height);
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &pipeline.bind_group, &[]);
        pass.set_vertex_buffer(0, pipeline.vertex_buf.slice(..));
        pass.set_index_buffer(pipeline.index_buf.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..pipeline.index_count, 0, 0..1);
    }
}

impl<Msg> shader::Program<Msg> for Scene {
    type State = ();
    type Primitive = TeapotPrimitive;

    fn draw(&self, _state: &(), _cursor: iced::mouse::Cursor, _bounds: Rectangle) -> TeapotPrimitive {
        TeapotPrimitive { rotation: self.rotation }
    }
}

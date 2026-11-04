//! Draws the 3D viewport into its own texture, which egui then shows as an image.

pub mod meshes;

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use eframe::egui_wgpu::{self, wgpu};
use glam::{Mat4, Vec3, Vec4};
use wgpu::util::DeviceExt;

use meshes::{MeshKind, MeshRange, Vertex};

const COLOR_STORAGE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const COLOR_TARGET: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    shadow_matrix: [[f32; 4]; 4],
    eye: [f32; 4],
    params: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky: [f32; 4],
    ground: [f32; 4],
    background_top: [f32; 4],
    background_bottom: [f32; 4],
    grid_color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    model: [[f32; 4]; 4],
    color: [f32; 4],
    edge: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct DrawItem {
    pub mesh: MeshKind,
    pub model: Mat4,
    /// Linear color; alpha below one draws it in the transparent pass.
    pub color: Vec4,
    /// Linear outline color; alpha is the outline's strength.
    pub edge: Vec4,
}

/// How the scene looks this frame. Colors are linear.
#[derive(Clone, Copy, Debug)]
pub struct SceneStyle {
    pub shaded: bool,
    /// The sun's view of the scene, when shadows are on.
    pub shadow_matrix: Option<Mat4>,
    pub sun_dir: Vec3,
    pub sun_color: Vec3,
    pub sky: Vec3,
    pub ground: Vec3,
    pub shadow_softness: f32,
    pub shadow_strength: f32,
    pub floor_shadows: bool,
    pub shadow_lift: f32,
    pub background_top: Vec3,
    pub background_bottom: Vec3,
    pub background_alpha: f32,
    pub grid_color: Vec3,
    pub grid_opacity: f32,
}

pub struct FrameInput {
    /// Size in physical pixels.
    pub size: [u32; 2],
    pub view_proj: Mat4,
    pub eye: Vec3,
    pub grid_y: f32,
    pub grid_fade: f32,
    pub parts: Vec<DrawItem>,
    /// Drawn after the parts, in the order given.
    pub bones: Vec<DrawItem>,
    /// Draw bones over the parts instead of letting parts hide them.
    pub bones_on_top: bool,
    pub style: SceneStyle,
}

struct Target {
    size: [u32; 2],
    resolved_view: wgpu::TextureView,
    /// Absent without antialiasing, when drawing goes straight to `resolved_view`.
    msaa_view: Option<wgpu::TextureView>,
    depth_view: wgpu::TextureView,
    /// The last step on OpenGL, which can't see one texture as both formats.
    encode: Option<EncodeStep>,
}

/// Draws the finished picture into the texture egui shows, encoding it on the way.
struct EncodeStep {
    shown: wgpu::TextureView,
    source: wgpu::BindGroup,
}

/// Only built on OpenGL.
struct EncodePipeline {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl EncodePipeline {
    fn new(device: &wgpu::Device, shader: &wgpu::ShaderModule) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kaye encode layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kaye encode pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("kaye encode"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_encode"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_STORAGE,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { layout, pipeline }
    }
}

struct Pipelines {
    background: wgpu::RenderPipeline,
    grid: wgpu::RenderPipeline,
    opaque: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    overlay: wgpu::RenderPipeline,
}

pub struct ViewportRenderer {
    shader: wgpu::ShaderModule,
    globals: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    bind_group: wgpu::BindGroup,
    shadow_globals: wgpu::BindGroup,
    shadow_sampler: wgpu::Sampler,
    shadow_pipeline: wgpu::RenderPipeline,
    shadow_view: wgpu::TextureView,
    shadow_size: u32,
    samples: u32,
    pipelines: Pipelines,
    encode_pipeline: Option<EncodePipeline>,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    ranges: HashMap<MeshKind, MeshRange>,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    target: Option<Target>,
    texture_id: Option<egui::TextureId>,
}

fn create_shadow_map(device: &wgpu::Device, size: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("kaye shadow map"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    shadow_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("kaye globals"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(shadow_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

const VERTEX_ATTRS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];
const INSTANCE_ATTRS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4,
    7 => Float32x4, 8 => Float32x4
];

fn vertex_layouts() -> [Option<wgpu::VertexBufferLayout<'static>>; 2] {
    [
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRS,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Instance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRS,
        }),
    ]
}

fn create_pipelines(device: &wgpu::Device, shader: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, samples: u32) -> Pipelines {
    let multisample = wgpu::MultisampleState {
        count: samples,
        mask: !0,
        alpha_to_coverage_enabled: false,
    };
    let depth = |write: bool, compare: wgpu::CompareFunction| wgpu::DepthStencilState {
        format: DEPTH,
        depth_write_enabled: Some(write),
        depth_compare: Some(compare),
        stencil: Default::default(),
        bias: Default::default(),
    };
    let color_target = |blend: Option<wgpu::BlendState>| {
        [Some(wgpu::ColorTargetState {
            format: COLOR_TARGET,
            blend,
            write_mask: wgpu::ColorWrites::ALL,
        })]
    };
    let fullscreen = |label: &str, fragment: &str, depth_state: wgpu::DepthStencilState, blend: Option<wgpu::BlendState>| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(depth_state),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &color_target(blend),
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let buffers = vertex_layouts();
    let mesh = |label: &str, fragment: &str, depth_state: wgpu::DepthStencilState, blend: Option<wgpu::BlendState>| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_mesh"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(depth_state),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &color_target(blend),
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    Pipelines {
        background: fullscreen("kaye background", "fs_background", depth(false, wgpu::CompareFunction::Always), None),
        grid: fullscreen(
            "kaye grid",
            "fs_grid",
            depth(false, wgpu::CompareFunction::LessEqual),
            Some(wgpu::BlendState::ALPHA_BLENDING),
        ),
        opaque: mesh("kaye opaque", "fs_mesh", depth(true, wgpu::CompareFunction::Less), None),
        transparent: mesh(
            "kaye transparent",
            "fs_mesh",
            depth(false, wgpu::CompareFunction::Less),
            Some(wgpu::BlendState::ALPHA_BLENDING),
        ),
        overlay: mesh("kaye overlay", "fs_overlay", depth(true, wgpu::CompareFunction::LessEqual), None),
    }
}

impl ViewportRenderer {
    pub fn new(device: &wgpu::Device, antialiasing: bool, shadow_size: u32) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let samples = if antialiasing { 4 } else { 1 };
        let shadow_size = shadow_size.max(1);

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kaye globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kaye globals layout"),
            entries: &[
                uniform_entry,
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kaye pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        // The shadow pass writes the map, so it can't also have the map bound.
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kaye shadow layout"),
            entries: &[uniform_entry],
        });
        let shadow_globals = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kaye shadow globals"),
            layout: &shadow_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let shadow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kaye shadow pipeline layout"),
            bind_group_layouts: &[Some(&shadow_layout)],
            immediate_size: 0,
        });
        let shadow_buffers = vertex_layouts();
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("kaye shadow"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                compilation_options: Default::default(),
                buffers: &shadow_buffers,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("kaye shadow sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_view = create_shadow_map(device, shadow_size);
        let bind_group = create_bind_group(device, &layout, &globals, &shadow_view, &shadow_sampler);
        let pipelines = create_pipelines(device, &shader, &pipeline_layout, samples);

        let (mesh, ranges) = meshes::build_all();
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("kaye vertices"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("kaye indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let instance_capacity = 256;
        Self {
            shader,
            globals,
            layout,
            pipeline_layout,
            bind_group,
            shadow_globals,
            shadow_sampler,
            shadow_pipeline,
            shadow_view,
            shadow_size,
            samples,
            pipelines,
            encode_pipeline: None,
            vertices,
            indices,
            ranges: ranges.into_iter().collect(),
            instances: create_instance_buffer(device, instance_capacity),
            instance_capacity,
            target: None,
            texture_id: None,
        }
    }

    /// Applies quality settings, rebuilding only what they affect.
    pub fn configure(&mut self, device: &wgpu::Device, antialiasing: bool, shadow_size: u32) {
        let samples = if antialiasing { 4 } else { 1 };
        if samples != self.samples {
            self.samples = samples;
            self.pipelines = create_pipelines(device, &self.shader, &self.pipeline_layout, samples);
            self.target = None;
        }
        let shadow_size = shadow_size.max(1);
        if shadow_size != self.shadow_size {
            self.shadow_size = shadow_size;
            self.shadow_view = create_shadow_map(device, shadow_size);
            self.bind_group = create_bind_group(device, &self.layout, &self.globals, &self.shadow_view, &self.shadow_sampler);
        }
    }

    fn ensure_target(&mut self, rs: &egui_wgpu::RenderState, size: [u32; 2]) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let device = &rs.device;
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let color = |label: &'static str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages, view_formats: &'static [wgpu::TextureFormat]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        // Stored as plain Unorm because that is what egui samples, but rendered
        // through an sRGB view so blending and MSAA resolve happen in linear.
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let (resolved_view, sample_view, encode) = if rs.adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::VIEW_FORMATS) {
            let resolved = color("kaye viewport", COLOR_STORAGE, usage, &[COLOR_TARGET]);
            let view = resolved.create_view(&wgpu::TextureViewDescriptor {
                format: Some(COLOR_TARGET),
                ..Default::default()
            });
            (view, resolved.create_view(&Default::default()), None)
        } else {
            let drawn = color("kaye viewport", COLOR_TARGET, usage, &[]).create_view(&Default::default());
            let shown = color("kaye viewport shown", COLOR_STORAGE, usage, &[]).create_view(&Default::default());
            let pipeline = self.encode_pipeline.get_or_insert_with(|| EncodePipeline::new(device, &self.shader));
            let source = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("kaye encode source"),
                layout: &pipeline.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&drawn),
                }],
            });
            (drawn, shown.clone(), Some(EncodeStep { shown, source }))
        };
        let samples = self.samples;
        let attachment = |label: &str, format: wgpu::TextureFormat| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let msaa_view = (samples > 1).then(|| attachment("kaye viewport msaa", COLOR_TARGET));
        let depth_view = attachment("kaye viewport depth", DEPTH);

        let mut renderer = rs.renderer.write();
        match self.texture_id {
            Some(id) => renderer.update_egui_texture_from_wgpu_texture(device, &sample_view, wgpu::FilterMode::Linear, id),
            None => {
                self.texture_id = Some(renderer.register_native_texture(device, &sample_view, wgpu::FilterMode::Linear));
            }
        }
        self.target = Some(Target {
            size,
            resolved_view,
            msaa_view,
            depth_view,
            encode,
        });
    }

    pub fn render(&mut self, rs: &egui_wgpu::RenderState, input: &FrameInput) -> egui::TextureId {
        let size = [input.size[0].max(1), input.size[1].max(1)];
        self.ensure_target(rs, size);
        let (device, queue) = (&rs.device, &rs.queue);
        let style = &input.style;
        let has_shadows = style.shaded && style.shadow_matrix.is_some();

        queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&Globals {
                view_proj: input.view_proj.to_cols_array_2d(),
                inv_view_proj: input.view_proj.inverse().to_cols_array_2d(),
                shadow_matrix: style.shadow_matrix.unwrap_or(Mat4::IDENTITY).to_cols_array_2d(),
                eye: input.eye.extend(1.0).into(),
                params: [
                    input.grid_y,
                    input.grid_fade,
                    if style.shaded { 1.0 } else { 0.0 },
                    if has_shadows { 1.0 } else { 0.0 },
                ],
                sun_dir: style.sun_dir.extend(style.shadow_softness).into(),
                sun_color: style.sun_color.extend(style.shadow_strength).into(),
                sky: style.sky.extend(if style.floor_shadows { 1.0 } else { 0.0 }).into(),
                ground: style.ground.extend(style.shadow_lift).into(),
                background_top: style.background_top.extend(style.background_alpha).into(),
                background_bottom: style.background_bottom.extend(style.grid_opacity).into(),
                grid_color: style.grid_color.extend(1.0 / self.shadow_size as f32).into(),
            }),
        );

        // Highlighted parts go first in their group. Roblox rigs often have exactly
        // overlapping faces, and first drawn wins, which keeps outlines unbroken.
        let (mut opaque, mut see_through): (Vec<&DrawItem>, Vec<&DrawItem>) =
            input.parts.iter().partition(|p| p.color.w >= 0.999);
        opaque.sort_by_key(|p| (p.mesh, p.edge.w < 0.5));
        // Transparent parts have to go back to front to blend correctly.
        let depth_of = |p: &DrawItem| p.model.w_axis.truncate().distance_squared(input.eye);
        see_through.sort_by(|a, b| depth_of(b).total_cmp(&depth_of(a)));

        let mut data: Vec<Instance> = Vec::with_capacity(input.parts.len() + input.bones.len());
        let push = |data: &mut Vec<Instance>, p: &DrawItem| {
            data.push(Instance {
                model: p.model.to_cols_array_2d(),
                color: p.color.into(),
                edge: p.edge.into(),
            });
        };
        opaque.iter().for_each(|p| push(&mut data, p));
        see_through.iter().for_each(|p| push(&mut data, p));
        input.bones.iter().for_each(|p| push(&mut data, p));

        if data.len() > self.instance_capacity {
            self.instance_capacity = data.len().next_power_of_two();
            self.instances = create_instance_buffer(device, self.instance_capacity);
        }
        if !data.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&data));
        }

        let ranges = &self.ranges;
        let draw_runs = |pass: &mut wgpu::RenderPass<'_>, items: &[&DrawItem], offset: usize| {
            let mut start = 0;
            while start < items.len() {
                let mesh = items[start].mesh;
                let count = items[start..].iter().take_while(|p| p.mesh == mesh).count();
                let r = ranges[&mesh];
                pass.draw_indexed(
                    r.first_index..r.first_index + r.index_count,
                    0,
                    (offset + start) as u32..(offset + start + count) as u32,
                );
                start += count;
            }
        };

        let mut encoder = device.create_command_encoder(&Default::default());
        if has_shadows {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("kaye shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.shadow_globals, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_vertex_buffer(1, self.instances.slice(..));
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            draw_runs(&mut pass, &opaque, 0);
        }

        let target = self.target.as_ref().unwrap();
        {
            let mut pass = begin_pass(&mut encoder, target, true, wgpu::LoadOp::Clear(1.0), false);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_pipeline(&self.pipelines.background);
            pass.draw(0..3, 0..1);

            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_vertex_buffer(1, self.instances.slice(..));
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);

            pass.set_pipeline(&self.pipelines.opaque);
            draw_runs(&mut pass, &opaque, 0);

            pass.set_pipeline(&self.pipelines.grid);
            pass.draw(0..3, 0..1);

            pass.set_pipeline(&self.pipelines.transparent);
            for (i, item) in see_through.iter().enumerate() {
                let r = ranges[&item.mesh];
                let at = (opaque.len() + i) as u32;
                pass.draw_indexed(r.first_index..r.first_index + r.index_count, 0, at..at + 1);
            }
        }
        {
            // Clearing depth here is what puts bones on top of the parts.
            let depth_load = if input.bones_on_top {
                wgpu::LoadOp::Clear(1.0)
            } else {
                wgpu::LoadOp::Load
            };
            let mut pass = begin_pass(&mut encoder, target, false, depth_load, true);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_vertex_buffer(1, self.instances.slice(..));
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_pipeline(&self.pipelines.overlay);
            let bones: Vec<&DrawItem> = input.bones.iter().collect();
            draw_runs(&mut pass, &bones, opaque.len() + see_through.len());
        }
        if let (Some(step), Some(encode)) = (&target.encode, &self.encode_pipeline) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("kaye encode"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &step.shown,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&encode.pipeline);
            pass.set_bind_group(0, &step.source, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
        self.texture_id.unwrap()
    }
}

fn begin_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    target: &'a Target,
    clear_color: bool,
    depth_load: wgpu::LoadOp<f32>,
    resolve: bool,
) -> wgpu::RenderPass<'a> {
    let (view, resolve_target) = match &target.msaa_view {
        Some(msaa) => (msaa, resolve.then_some(&target.resolved_view)),
        None => (&target.resolved_view, None),
    };
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("kaye viewport"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target,
            ops: wgpu::Operations {
                load: if clear_color {
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                } else {
                    wgpu::LoadOp::Load
                },
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &target.depth_view,
            depth_ops: Some(wgpu::Operations {
                load: depth_load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("kaye instances"),
        size: (capacity * std::mem::size_of::<Instance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Converts an sRGB color channel to linear for the shaders.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_color(rgb: [f32; 3], alpha: f32) -> Vec4 {
    Vec4::new(
        srgb_to_linear(rgb[0]),
        srgb_to_linear(rgb[1]),
        srgb_to_linear(rgb[2]),
        alpha,
    )
}

pub fn linear_rgb(rgb: [u8; 3]) -> Vec3 {
    Vec3::new(
        srgb_to_linear(rgb[0] as f32 / 255.0),
        srgb_to_linear(rgb[1] as f32 / 255.0),
        srgb_to_linear(rgb[2] as f32 / 255.0),
    )
}

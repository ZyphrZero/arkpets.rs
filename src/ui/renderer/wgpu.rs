use std::{collections::HashMap, sync::Arc};

use eframe::{
    egui::{self, epaint::Primitive},
    egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor},
    wgpu,
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const UNIFORM_SIZE: u64 = 64;

pub struct Compositor {
    resources: Arc<egui::mutex::Mutex<Resources>>,
}

impl Compositor {
    pub fn new(state: &egui_wgpu::RenderState) -> Self {
        Self {
            resources: Arc::new(egui::mutex::Mutex::new(Resources::new(
                &state.device,
                state.target_format,
            ))),
        }
    }

    pub fn paint(
        &self,
        painter: &egui::Painter,
        meshes: Vec<egui::Mesh>,
        images: Arc<Vec<egui::ColorImage>>,
        effects: super::Effects,
    ) {
        let Some(rect) = super::group_rect(painter, &meshes, &effects) else {
            return;
        };
        painter.add(egui_wgpu::Callback::new_paint_callback(
            rect,
            GroupCallback {
                resources: Arc::clone(&self.resources),
                viewport: painter.ctx().viewport_id(),
                rect,
                meshes,
                images,
                effects,
            },
        ));
    }
}

struct Resources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    images: Option<Arc<Vec<egui::ColorImage>>>,
    mipmaps: bool,
    atlases: Vec<wgpu::TextureView>,
    viewports: HashMap<egui::ViewportId, Viewport>,
}

impl Resources {
    fn new(device: &wgpu::Device, output: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pet group opacity layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(UNIFORM_SIZE),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pet group opacity pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pet group opacity shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pet group opacity pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(if output.is_srgb() {
                    "fs_linear"
                } else {
                    "fs_gamma"
                }),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("pet group opacity sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            images: None,
            mipmaps: false,
            atlases: Vec::new(),
            viewports: HashMap::new(),
        }
    }

    fn upload_atlases(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &Arc<Vec<egui::ColorImage>>,
        mipmaps: bool,
    ) {
        if self.mipmaps == mipmaps
            && self
                .images
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, images))
        {
            return;
        }
        self.viewports.clear();
        self.atlases = images
            .iter()
            .map(|image| {
                let levels = if mipmaps {
                    super::mipmaps(image)
                } else {
                    vec![image.clone()]
                };
                let size = wgpu::Extent3d {
                    width: image.size[0] as u32,
                    height: image.size[1] as u32,
                    depth_or_array_layers: 1,
                };
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("pet shared atlas"),
                    size,
                    mip_level_count: levels.len() as u32,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: FORMAT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                for (level, image) in levels.iter().enumerate() {
                    let size = wgpu::Extent3d {
                        width: image.size[0] as u32,
                        height: image.size[1] as u32,
                        depth_or_array_layers: 1,
                    };
                    let bytes: Vec<u8> = image
                        .pixels
                        .iter()
                        .flat_map(|color| color.to_array())
                        .collect();
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            mip_level: level as u32,
                            ..texture.as_image_copy()
                        },
                        &bytes,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(4 * size.width),
                            rows_per_image: Some(size.height),
                        },
                        size,
                    );
                }
                texture.create_view(&Default::default())
            })
            .collect();
        self.images = Some(Arc::clone(images));
        self.mipmaps = mipmaps;
    }
}

struct Viewport {
    renderer: egui_wgpu::Renderer,
    atlas_ids: Vec<egui::TextureId>,
    uniform: wgpu::Buffer,
    target: Option<Target>,
}

impl Viewport {
    fn new(device: &wgpu::Device, atlases: &[wgpu::TextureView], mipmaps: bool) -> Self {
        let mut renderer = egui_wgpu::Renderer::new(
            device,
            FORMAT,
            egui_wgpu::RendererOptions {
                dithering: false,
                ..Default::default()
            },
        );
        let atlas_ids = atlases
            .iter()
            .map(|view| {
                renderer.register_native_texture_with_sampler_options(
                    device,
                    view,
                    wgpu::SamplerDescriptor {
                        label: Some("pet atlas sampler"),
                        mag_filter: wgpu::FilterMode::Linear,
                        min_filter: wgpu::FilterMode::Linear,
                        mipmap_filter: wgpu::MipmapFilterMode::Linear,
                        lod_max_clamp: if mipmaps { 32.0 } else { 0.0 },
                        ..Default::default()
                    },
                )
            })
            .collect();
        Self {
            renderer,
            atlas_ids,
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pet group opacity uniform"),
                size: UNIFORM_SIZE,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            target: None,
        }
    }
}

struct Target {
    size: [u32; 2],
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

impl Target {
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        uniform: &wgpu::Buffer,
        size: [u32; 2],
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pet group opacity target"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pet group opacity bind group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        Self {
            size,
            view,
            bind_group,
        }
    }
}

struct GroupCallback {
    resources: Arc<egui::mutex::Mutex<Resources>>,
    viewport: egui::ViewportId,
    rect: egui::Rect,
    meshes: Vec<egui::Mesh>,
    images: Arc<Vec<egui::ColorImage>>,
    effects: super::Effects,
}

impl CallbackTrait for GroupCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        _callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let mut resources = self.resources.lock();
        resources.upload_atlases(device, queue, &self.images, self.effects.mipmaps);
        let Resources {
            layout,
            sampler,
            atlases,
            viewports,
            ..
        } = &mut *resources;
        let viewport = viewports
            .entry(self.viewport)
            .or_insert_with(|| Viewport::new(device, atlases, self.effects.mipmaps));
        let pixel_size = [
            (self.rect.width() * screen.pixels_per_point)
                .round()
                .max(1.0) as u32,
            (self.rect.height() * screen.pixels_per_point)
                .round()
                .max(1.0) as u32,
        ];
        let max_size = device.limits().max_texture_dimension_2d;
        assert!(
            pixel_size.iter().all(|size| *size <= max_size),
            "pet opacity target {pixel_size:?} exceeds GPU texture limit {max_size}"
        );
        if viewport
            .target
            .as_ref()
            .is_none_or(|target| target.size[0] < pixel_size[0] || target.size[1] < pixel_size[1])
        {
            let old_size = viewport
                .target
                .as_ref()
                .map_or([0; 2], |target| target.size);
            let size = std::array::from_fn(|axis| {
                pixel_size[axis]
                    .div_ceil(64)
                    .saturating_mul(64)
                    .min(max_size)
                    .max(old_size[axis])
            });
            viewport.target = Some(Target::new(
                device,
                layout,
                sampler,
                &viewport.uniform,
                size,
            ));
        }
        let target = viewport.target.as_ref().expect("opacity target allocated");
        // Four vec4 values keep the Rust byte layout aligned with WGSL uniforms.
        let uniform = [
            pixel_size[0] as f32 / target.size[0] as f32,
            pixel_size[1] as f32 / target.size[1] as f32,
            self.effects.opacity,
            if self.effects.high_quality { 1.0 } else { 0.0 },
            self.effects.outline_color[0],
            self.effects.outline_color[1],
            self.effects.outline_color[2],
            self.effects.outline_color[3],
            self.effects.outline_width,
            self.effects.shadow_opacity,
            2.0 * screen.pixels_per_point,
            0.0,
            1.0 / target.size[0] as f32,
            1.0 / target.size[1] as f32,
            0.0,
            0.0,
        ]
        .map(f32::to_ne_bytes);
        queue.write_buffer(&viewport.uniform, 0, uniform.as_flattened());

        let clip_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, self.rect.size());
        let jobs: Vec<_> = self
            .meshes
            .iter()
            .map(|mesh| {
                let mut mesh = mesh.clone();
                let egui::TextureId::Managed(page) = mesh.texture_id else {
                    unreachable!("group meshes use atlas page indices");
                };
                mesh.texture_id = viewport.atlas_ids[page as usize - 1];
                for vertex in &mut mesh.vertices {
                    vertex.pos -= self.rect.min.to_vec2();
                }
                egui::ClippedPrimitive {
                    clip_rect,
                    primitive: Primitive::Mesh(mesh),
                }
            })
            .collect();
        // A bucket is only spare storage: rendering keeps the real DPI and uses
        // the top-left region, then the final pass samples exactly that region.
        let offscreen = ScreenDescriptor {
            size_in_pixels: target.size,
            pixels_per_point: screen.pixels_per_point,
        };
        let buffers = viewport
            .renderer
            .update_buffers(device, queue, encoder, &jobs, &offscreen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("pet group opacity render"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            viewport.renderer.render(&mut pass, &jobs, &offscreen);
        }
        buffers
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _callback_resources: &CallbackResources,
    ) {
        let resources = self.resources.lock();
        let target = resources.viewports[&self.viewport]
            .target
            .as_ref()
            .expect("opacity target prepared before paint");
        pass.set_pipeline(&resources.pipeline);
        pass.set_bind_group(0, &target.bind_group, &[]);
        pass.draw(0..6, 0..1);
    }
}

const SHADER: &str = r#"
// Effect sampling follows Ark-Pets ComplexFragment.glsl / ComplexFragmentLow.glsl
// (Copyright 2022-2026 Harry Huang, Litwak913; GPL-3.0), with premultiplied compositing.
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
struct Params {
    // Valid UV extent, group opacity, high-quality flag.
    group: vec4<f32>,
    // Straight RGBA; alpha already includes the outline transition.
    outline_color: vec4<f32>,
    // Outline radius in pixels, shadow opacity, shadow radius in pixels, padding.
    effects: vec4<f32>,
    texel: vec4<f32>,
};
@group(0) @binding(2) var<uniform> params: Params;

const GAUSSIAN = array<f32, 25>(
    0.0035434, 0.0158805, 0.0261825, 0.0158805, 0.0035434,
    0.0158805, 0.0711714, 0.1173418, 0.0711714, 0.0158805,
    0.0261825, 0.1173418, 0.0,       0.1173418, 0.0261825,
    0.0158805, 0.0711714, 0.1173418, 0.0711714, 0.0158805,
    0.0035434, 0.0158805, 0.0261825, 0.0158805, 0.0035434,
);
const CIRCLE = array<vec2<f32>, 16>(
    vec2<f32>(-1.000,  0.000), vec2<f32>(-0.924, -0.383),
    vec2<f32>(-0.707, -0.707), vec2<f32>(-0.383, -0.924),
    vec2<f32>( 0.000, -1.000), vec2<f32>( 0.383, -0.924),
    vec2<f32>( 0.707, -0.707), vec2<f32>( 0.924, -0.383),
    vec2<f32>( 1.000,  0.000), vec2<f32>( 0.924,  0.383),
    vec2<f32>( 0.707,  0.707), vec2<f32>( 0.383,  0.924),
    vec2<f32>( 0.000,  1.000), vec2<f32>(-0.383,  0.924),
    vec2<f32>(-0.707,  0.707), vec2<f32>(-0.924,  0.383),
);

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Output {
    let positions = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let point = positions[index];
    var out: Output;
    out.position = vec4<f32>(point.x * 2.0 - 1.0, 1.0 - point.y * 2.0, 0.0, 1.0);
    out.uv = point * params.group.xy;
    return out;
}

fn sample_group(uv: vec2<f32>) -> vec4<f32> {
    // The allocation can exceed the rendered area. Neither the unused bucket
    // nor a clamped border may contribute to the outline or shadow.
    if any(uv < vec2<f32>(0.0)) || any(uv >= params.group.xy) {
        return vec4<f32>(0.0);
    }
    return textureSampleLevel(image, image_sampler, uv, 0.0);
}

fn neighbor_alpha(uv: vec2<f32>, radius: f32) -> f32 {
    let unit = radius * params.texel.xy;
    var sum = 0.0;
    if params.group.w > 0.5 {
        for (var y = -2; y <= 2; y += 1) {
            for (var x = -2; x <= 2; x += 1) {
                if x != 0 || y != 0 {
                    let offset = vec2<f32>(f32(x), f32(y)) * unit;
                    let weight = GAUSSIAN[u32((y + 2) * 5 + x + 2)];
                    sum += sample_group(uv + offset).a * weight;
                }
            }
        }
    } else {
        for (var i = 0u; i < 16u; i += 1u) {
            sum += sample_group(uv + CIRCLE[i] * unit).a;
        }
    }
    return sum;
}

fn composite(uv: vec2<f32>) -> vec4<f32> {
    let base = sample_group(uv);
    var color = base;
    if base.a < 0.1 && params.outline_color.a > 0.0 && params.effects.x > 0.0 {
        let strength = neighbor_alpha(uv, params.effects.x) * 10.0;
        if strength > 0.1 {
            let alpha = clamp(strength, 0.0, 1.0) * params.outline_color.a;
            let outline = vec4<f32>(params.outline_color.rgb * alpha, alpha);
            color += outline * (1.0 - color.a);
        }
    }
    if base.a < 0.9 && params.effects.y > 0.0 {
        let sum = neighbor_alpha(uv, params.effects.z);
        var strength = clamp(sum, 0.0, 1.0);
        if params.group.w > 0.5 {
            strength = sqrt(strength);
        } else if strength <= 0.1 {
            strength = 0.0;
        }
        let shadow_alpha = strength * params.effects.y;
        color += vec4<f32>(0.0, 0.0, 0.0, shadow_alpha) * (1.0 - color.a);
    }
    // Reduce the fully composed image once, including its premultiplied RGB.
    return color * params.group.z;
}

@fragment
fn fs_gamma(in: Output) -> @location(0) vec4<f32> {
    return composite(in.uv);
}

@fragment
fn fs_linear(in: Output) -> @location(0) vec4<f32> {
    let color = composite(in.uv);
    // Match egui's gamma-space colors when the window target encodes sRGB.
    let lower = color.rgb / vec3<f32>(12.92);
    let higher = pow((color.rgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return vec4<f32>(select(higher, lower, color.rgb < vec3<f32>(0.04045)), color.a);
}
"#;

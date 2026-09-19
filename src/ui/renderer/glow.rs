use eframe::{
    egui::{self, ColorImage, Mesh, Pos2, Rect, TextureId},
    egui_glow::{CallbackFn, Painter, ShaderVersion},
    glow::{self, HasContext as _},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// GL allocation and drawing only happen inside a paint callback. The owner must
/// call `destroy` while the GL context is current before dropping this compositor.
pub struct Compositor {
    gl: Arc<glow::Context>,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    resources: Option<Resources>,
    destroyed: bool,
}

struct Resources {
    painter: Painter,
    composite: CompositePass,
    images: Arc<Vec<ColorImage>>,
    mipmaps: bool,
    surfaces: HashMap<egui::ViewportId, Surface>,
}

struct Surface {
    framebuffer: glow::Framebuffer,
    texture: TextureId,
    size: [u32; 2],
}

impl Compositor {
    pub fn new(gl: Arc<glow::Context>) -> Self {
        Self {
            gl,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    pub fn paint(
        &self,
        painter: &egui::Painter,
        mut meshes: Vec<Mesh>,
        images: Arc<Vec<ColorImage>>,
        effects: super::Effects,
    ) {
        let Some(rect) = super::group_rect(painter, &meshes, &effects) else {
            return;
        };
        let viewport = painter.ctx().viewport_id();
        let gl = self.gl.clone();
        let state = self.state.clone();
        // Keep the original slot alpha and order when drawing into the group.
        for mesh in &mut meshes {
            for vertex in &mut mesh.vertices {
                vertex.pos -= rect.min.to_vec2();
            }
        }
        let primitives: Vec<_> = meshes
            .into_iter()
            .map(|mesh| egui::ClippedPrimitive {
                clip_rect: Rect::from_min_size(Pos2::ZERO, rect.size()),
                primitive: egui::epaint::Primitive::Mesh(mesh),
            })
            .collect();

        painter.add(egui::PaintCallback {
            rect,
            callback: Arc::new(CallbackFn::new(move |info, outer| {
                let mut state = state.lock().expect("OpenGL compositor state poisoned");
                assert!(!state.destroyed, "OpenGL compositor has been destroyed");
                let resources = state.resources.get_or_insert_with(|| {
                    Resources::new(gl.clone(), images.clone(), effects.mipmaps)
                        .unwrap_or_else(|error| panic!("Cannot initialize group opacity: {error}"))
                });
                resources.update_images(&images, effects.mipmaps);
                let pixels = info.viewport_in_pixels();
                let requested = [pixels.width_px as u32, pixels.height_px as u32];
                if requested.contains(&0) {
                    return;
                }
                let size = surface_size(requested, resources.painter.max_texture_side());
                let surface = resources
                    .surface(&gl, viewport, size)
                    .unwrap_or_else(|error| panic!("Cannot allocate group opacity: {error}"));
                let framebuffer = surface.framebuffer;
                let texture = surface.texture;
                let size = surface.size;

                // egui_glow uses gamma-space premultiplied RGBA blending. RGBA8
                // preserves that representation across both render passes.
                unsafe {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                }
                resources.painter.clear(size, [0.0; 4]);
                resources
                    .painter
                    .paint_primitives(size, info.pixels_per_point, &primitives);
                unsafe {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, outer.intermediate_fbo());
                }

                resources.composite.paint(
                    &gl,
                    &info,
                    resources
                        .painter
                        .texture(texture)
                        .expect("group framebuffer texture"),
                    size,
                    effects,
                );
                // The outer painter restores viewport, scissor, shader and
                // blending state after the callback returns.
            })),
        });
    }

    /// Call from the GL backend's shutdown hook while its context is current.
    pub fn destroy(&self) {
        let mut state = self.state.lock().expect("OpenGL compositor state poisoned");
        if let Some(mut resources) = state.resources.take() {
            for (_, surface) in resources.surfaces.drain() {
                surface.destroy(&self.gl, &mut resources.painter);
            }
            resources.composite.destroy(&self.gl);
            resources.painter.destroy();
        }
        state.destroyed = true;
    }
}

impl Resources {
    fn new(
        gl: Arc<glow::Context>,
        images: Arc<Vec<ColorImage>>,
        mipmaps: bool,
    ) -> Result<Self, String> {
        // Dithering twice would alter the intermediate transparent pixels.
        let mut painter =
            Painter::new(gl.clone(), "", None, false).map_err(|error| error.to_string())?;
        let composite = match CompositePass::new(&gl) {
            Ok(composite) => composite,
            Err(error) => {
                painter.destroy();
                return Err(error);
            }
        };
        let mut resources = Self {
            painter,
            composite,
            images: Arc::new(Vec::new()),
            mipmaps,
            surfaces: HashMap::new(),
        };
        resources.update_images(&images, mipmaps);
        Ok(resources)
    }

    fn update_images(&mut self, images: &Arc<Vec<ColorImage>>, mipmaps: bool) {
        if Arc::ptr_eq(&self.images, images) && self.mipmaps == mipmaps {
            return;
        }
        for page in 0..self.images.len() {
            self.painter
                .free_texture(TextureId::Managed(page as u64 + 1));
        }
        // egui_glow generates the mip chain only when an atlas is uploaded.
        let options = egui::TextureOptions::LINEAR
            .with_mipmap_mode(mipmaps.then_some(egui::TextureFilter::Linear));
        for (page, image) in images.iter().enumerate() {
            self.painter.set_texture(
                TextureId::Managed(page as u64 + 1),
                &egui::epaint::ImageDelta::full(image.clone(), options),
            );
        }
        self.images = images.clone();
        self.mipmaps = mipmaps;
    }

    fn surface(
        &mut self,
        gl: &glow::Context,
        viewport: egui::ViewportId,
        size: [u32; 2],
    ) -> Result<&Surface, String> {
        let size = self.surfaces.get(&viewport).map_or(size, |surface| {
            [size[0].max(surface.size[0]), size[1].max(surface.size[1])]
        });
        if self
            .surfaces
            .get(&viewport)
            .is_some_and(|surface| surface.size != size)
        {
            self.surfaces
                .remove(&viewport)
                .expect("checked surface")
                .destroy(gl, &mut self.painter);
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = self.surfaces.entry(viewport) {
            entry.insert(Surface::new(gl, &mut self.painter, size)?);
        }
        Ok(&self.surfaces[&viewport])
    }
}

struct CompositePass {
    program: glow::Program,
    vertex_buffer: glow::Buffer,
    vertex_array: Option<glow::VertexArray>,
    sampler: glow::UniformLocation,
    group: glow::UniformLocation,
    outline_color: glow::UniformLocation,
    effects: glow::UniformLocation,
    texel: glow::UniformLocation,
}

impl CompositePass {
    fn new(gl: &glow::Context) -> Result<Self, String> {
        let version = ShaderVersion::get(gl);
        let prefix = format!(
            "{}#define NEW_SHADER_INTERFACE {}\n",
            version.version_declaration(),
            u8::from(version.is_new_shader_interface())
        );
        let program = compile_program(gl, &prefix)?;
        unsafe {
            let names = [
                "u_texture",
                "u_group",
                "u_outline_color",
                "u_effects",
                "u_texel",
            ];
            let locations = names.map(|name| gl.get_uniform_location(program, name));
            if let Some(index) = locations.iter().position(Option::is_none) {
                gl.delete_program(program);
                return Err(format!("Missing composite shader uniform {}", names[index]));
            }
            let [sampler, group, outline_color, effects, texel] =
                locations.map(|location| location.expect("checked uniform"));
            let vertex_buffer = match gl.create_buffer() {
                Ok(buffer) => buffer,
                Err(error) => {
                    gl.delete_program(program);
                    return Err(error);
                }
            };
            let supports_vertex_array = gl.version().major >= 3
                || gl.supported_extensions().iter().any(|extension| {
                    matches!(
                        extension.as_str(),
                        "ARB_vertex_array_object" | "GL_ARB_vertex_array_object"
                    )
                });
            let vertex_array = if supports_vertex_array {
                match gl.create_vertex_array() {
                    Ok(array) => Some(array),
                    Err(error) => {
                        gl.delete_buffer(vertex_buffer);
                        gl.delete_program(program);
                        return Err(error);
                    }
                }
            } else {
                None
            };
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertex_buffer));
            let vertices: Vec<u8> = [
                0.0_f32, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0,
            ]
            .into_iter()
            .flat_map(f32::to_ne_bytes)
            .collect();
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &vertices, glow::STATIC_DRAW);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            Ok(Self {
                program,
                vertex_buffer,
                vertex_array,
                sampler,
                group,
                outline_color,
                effects,
                texel,
            })
        }
    }

    fn paint(
        &self,
        gl: &glow::Context,
        info: &egui::PaintCallbackInfo,
        texture: glow::Texture,
        size: [u32; 2],
        effects: super::Effects,
    ) {
        let viewport = info.viewport_in_pixels();
        let clip = info.clip_rect_in_pixels();
        unsafe {
            gl.viewport(
                viewport.left_px,
                viewport.from_bottom_px,
                viewport.width_px,
                viewport.height_px,
            );
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(
                clip.left_px,
                clip.from_bottom_px,
                clip.width_px,
                clip.height_px,
            );
            gl.enable(glow::BLEND);
            gl.blend_equation_separate(glow::FUNC_ADD, glow::FUNC_ADD);
            gl.blend_func_separate(
                glow::ONE,
                glow::ONE_MINUS_SRC_ALPHA,
                glow::ONE_MINUS_DST_ALPHA,
                glow::ONE,
            );
            gl.use_program(Some(self.program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.uniform_1_i32(Some(&self.sampler), 0);
            gl.uniform_4_f32(
                Some(&self.group),
                viewport.width_px as f32 / size[0] as f32,
                viewport.height_px as f32 / size[1] as f32,
                effects.opacity,
                f32::from(effects.high_quality),
            );
            let [r, g, b, a] = effects.outline_color;
            gl.uniform_4_f32(Some(&self.outline_color), r, g, b, a);
            gl.uniform_4_f32(
                Some(&self.effects),
                effects.outline_width,
                effects.shadow_opacity,
                2.0 * info.pixels_per_point,
                0.0,
            );
            gl.uniform_2_f32(
                Some(&self.texel),
                1.0 / size[0] as f32,
                1.0 / size[1] as f32,
            );
            if let Some(vertex_array) = self.vertex_array {
                gl.bind_vertex_array(Some(vertex_array));
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vertex_buffer));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
            if self.vertex_array.is_some() {
                gl.bind_vertex_array(None);
            } else {
                gl.disable_vertex_attrib_array(0);
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.use_program(None);
        }
    }

    fn destroy(self, gl: &glow::Context) {
        unsafe {
            if let Some(vertex_array) = self.vertex_array {
                gl.delete_vertex_array(vertex_array);
            }
            gl.delete_buffer(self.vertex_buffer);
            gl.delete_program(self.program);
        }
    }
}

fn compile_program(gl: &glow::Context, prefix: &str) -> Result<glow::Program, String> {
    unsafe {
        let vertex = compile_shader(gl, glow::VERTEX_SHADER, &format!("{prefix}{VERTEX_SHADER}"))?;
        let fragment = match compile_shader(
            gl,
            glow::FRAGMENT_SHADER,
            &format!("{prefix}{FRAGMENT_SHADER}"),
        ) {
            Ok(fragment) => fragment,
            Err(error) => {
                gl.delete_shader(vertex);
                return Err(error);
            }
        };
        let program = match gl.create_program() {
            Ok(program) => program,
            Err(error) => {
                gl.delete_shader(vertex);
                gl.delete_shader(fragment);
                return Err(error);
            }
        };
        gl.attach_shader(program, vertex);
        gl.attach_shader(program, fragment);
        gl.bind_attrib_location(program, 0, "a_pos");
        gl.link_program(program);
        gl.detach_shader(program, vertex);
        gl.detach_shader(program, fragment);
        gl.delete_shader(vertex);
        gl.delete_shader(fragment);
        if !gl.get_program_link_status(program) {
            let error = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(format!("Cannot link group effect shader: {error}"));
        }
        Ok(program)
    }
}

fn compile_shader(gl: &glow::Context, kind: u32, source: &str) -> Result<glow::Shader, String> {
    unsafe {
        let shader = gl.create_shader(kind)?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            let error = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            return Err(format!(
                "Cannot compile group effect shader (0x{kind:x}): {error}"
            ));
        }
        Ok(shader)
    }
}

impl Surface {
    fn new(gl: &glow::Context, painter: &mut Painter, size: [u32; 2]) -> Result<Self, String> {
        unsafe {
            let texture = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::LINEAR as i32);
            }
            for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::CLAMP_TO_EDGE as i32);
            }
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                size[0] as i32,
                size[1] as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            let framebuffer = match gl.create_framebuffer() {
                Ok(framebuffer) => framebuffer,
                Err(error) => {
                    gl.delete_texture(texture);
                    return Err(error);
                }
            };
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(framebuffer);
                gl.delete_texture(texture);
                return Err(format!(
                    "incomplete OpenGL framebuffer (0x{status:04x}) at {}x{} pixels",
                    size[0], size[1]
                ));
            }
            Ok(Self {
                framebuffer,
                texture: painter.register_native_texture(texture),
                size,
            })
        }
    }

    fn destroy(self, gl: &glow::Context, painter: &mut Painter) {
        unsafe {
            gl.delete_framebuffer(self.framebuffer);
        }
        painter.free_texture(self.texture);
    }
}

fn surface_size(requested: [u32; 2], max_texture_side: usize) -> [u32; 2] {
    requested.map(|side| {
        assert!(
            side as usize <= max_texture_side,
            "Group opacity requires a {side}-pixel texture, beyond the OpenGL limit {max_texture_side}"
        );
        // Small animated bounds changes should not allocate a new FBO each frame.
        side.div_ceil(64)
            .saturating_mul(64)
            .min(max_texture_side as u32)
    })
}

const VERTEX_SHADER: &str = r#"
#ifdef GL_ES
precision highp float;
#endif
#if NEW_SHADER_INTERFACE
in vec2 a_pos;
out vec2 v_uv;
#else
attribute vec2 a_pos;
varying vec2 v_uv;
#endif
uniform vec4 u_group;
void main() {
    gl_Position = vec4(a_pos.x * 2.0 - 1.0, 1.0 - a_pos.y * 2.0, 0.0, 1.0);
    v_uv = a_pos * u_group.xy;
}
"#;

// Sampling kernels match Ark-Pets ComplexFragment{,Low}.glsl.
// Copyright (c) 2022-2026 Harry Huang, Litwak913; GPL-3.0.
const FRAGMENT_SHADER: &str = r#"
#ifdef GL_ES
precision highp float;
#endif
#if NEW_SHADER_INTERFACE
in vec2 v_uv;
out vec4 frag_color;
#define SAMPLE texture
#else
varying vec2 v_uv;
#define frag_color gl_FragColor
#define SAMPLE texture2D
#endif
uniform sampler2D u_texture;
uniform vec4 u_group;
uniform vec4 u_outline_color;
uniform vec4 u_effects;
uniform vec2 u_texel;

const float GAUSSIAN[25] = float[25](
    0.0035434, 0.0158805, 0.0261825, 0.0158805, 0.0035434,
    0.0158805, 0.0711714, 0.1173418, 0.0711714, 0.0158805,
    0.0261825, 0.1173418, 0.0,       0.1173418, 0.0261825,
    0.0158805, 0.0711714, 0.1173418, 0.0711714, 0.0158805,
    0.0035434, 0.0158805, 0.0261825, 0.0158805, 0.0035434
);
const vec2 CIRCLE[16] = vec2[16](
    vec2(-1.000,  0.000), vec2(-0.924, -0.383),
    vec2(-0.707, -0.707), vec2(-0.383, -0.924),
    vec2( 0.000, -1.000), vec2( 0.383, -0.924),
    vec2( 0.707, -0.707), vec2( 0.924, -0.383),
    vec2( 1.000,  0.000), vec2( 0.924,  0.383),
    vec2( 0.707,  0.707), vec2( 0.383,  0.924),
    vec2( 0.000,  1.000), vec2(-0.383,  0.924),
    vec2(-0.707,  0.707), vec2(-0.924,  0.383)
);

vec4 sample_group(vec2 uv) {
    // Ignore both the unused allocation and GL's clamped texture border.
    if (any(lessThan(uv, vec2(0.0))) || any(greaterThanEqual(uv, u_group.xy))) {
        return vec4(0.0);
    }
    // The group occupies the top-left of a bottom-origin FBO texture.
    return SAMPLE(u_texture, vec2(uv.x, 1.0 - uv.y));
}

float neighbor_alpha(vec2 uv, float radius) {
    vec2 unit = radius * u_texel;
    float sum = 0.0;
    if (u_group.w > 0.5) {
        for (int y = -2; y <= 2; y++) {
            for (int x = -2; x <= 2; x++) {
                if (x != 0 || y != 0) {
                    sum += sample_group(uv + vec2(float(x), float(y)) * unit).a
                        * GAUSSIAN[(y + 2) * 5 + x + 2];
                }
            }
        }
    } else {
        for (int i = 0; i < 16; i++) {
            sum += sample_group(uv + CIRCLE[i] * unit).a;
        }
    }
    return sum;
}

void main() {
    vec4 base = sample_group(v_uv);
    vec4 color = base;
    if (base.a < 0.1 && u_outline_color.a > 0.0 && u_effects.x > 0.0) {
        float strength = neighbor_alpha(v_uv, u_effects.x) * 10.0;
        if (strength > 0.1) {
            float alpha = clamp(strength, 0.0, 1.0) * u_outline_color.a;
            vec4 outline = vec4(u_outline_color.rgb * alpha, alpha);
            color += outline * (1.0 - color.a);
        }
    }
    if (base.a < 0.9 && u_effects.y > 0.0) {
        float strength = clamp(neighbor_alpha(v_uv, u_effects.z), 0.0, 1.0);
        if (u_group.w > 0.5) {
            strength = sqrt(strength);
        } else if (strength <= 0.1) {
            strength = 0.0;
        }
        float alpha = strength * u_effects.y;
        color += vec4(0.0, 0.0, 0.0, alpha) * (1.0 - color.a);
    }
    // Scale the completed premultiplied group once, including all effects.
    frag_color = color * u_group.z;
}
"#;

//! Bounded GPU softening behind utilities, menus and modal decisions. No CPU pixel readback,
//! animation, repaint requests, or changes to interaction are involved.
use egui::{Context, Id, Painter, Rect};
#[cfg(feature = "glow-backdrop")]
use egui_glow::glow::HasContext as _;

#[derive(Clone, Copy, Debug)]
pub struct BackdropBlurInfo {
    pub bounds: Rect,
    pub target_size: [u32; 2],
    pub cached_source: bool,
    pub gpu_available: bool,
}

#[derive(Clone, Default)]
struct BlurFrame {
    pass: u64,
    regions: Vec<BackdropBlurInfo>,
}

fn frame_id() -> Id {
    Id::new("rill_backdrop_blur_frame")
}

fn update_frame<R>(ctx: &Context, update: impl FnOnce(&mut BlurFrame) -> R) -> R {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        let frame = data.get_temp_mut_or_default::<BlurFrame>(frame_id());
        if frame.pass != pass {
            *frame = BlurFrame {
                pass,
                ..Default::default()
            };
        }
        update(frame)
    })
}

/// Physical target size is bounded even for high-density, ultrawide displays.
pub fn backdrop_blur_target_size(bounds: Rect, density: f32) -> [u32; 2] {
    let size = bounds.size() * density.max(0.01);
    let scale = (size.max_elem() / crate::tokens::BACKDROP_BLUR_MAX_SIDE as f32)
        .max(crate::tokens::BACKDROP_BLUR_DOWNSAMPLE);
    [
        (size.x / scale).ceil().max(1.0) as u32,
        (size.y / scale).ceil().max(1.0) as u32,
    ]
}

/// Current-pass diagnostics for executable app tests, including headless runs.
pub fn backdrop_blur_info(ctx: &Context) -> Vec<BackdropBlurInfo> {
    ctx.data(|data| data.get_temp::<BlurFrame>(frame_id()))
        .filter(|frame| frame.pass.saturating_add(1) >= ctx.cumulative_pass_nr())
        .map_or_else(Vec::new, |frame| frame.regions)
}

pub(crate) fn begin_blur_frame(ctx: &Context) {
    update_frame(ctx, |_| {});
}

pub(crate) fn paint_modal_blur(ctx: &Context, painter: &Painter) {
    paint_blur(ctx, painter, ctx.content_rect(), None);
}

fn paint_blur(ctx: &Context, painter: &Painter, bounds: Rect, source: Option<u64>) {
    let bounds = bounds
        .intersect(ctx.content_rect())
        .intersect(painter.clip_rect());
    if !bounds.is_positive() {
        return;
    }
    #[cfg(feature = "glow-backdrop")]
    let backend = ctx.data(|data| data.get_temp::<gpu::Backend>(gpu::backend_id()));
    #[cfg(feature = "glow-backdrop")]
    let available = backend.is_some();
    #[cfg(not(feature = "glow-backdrop"))]
    let available = false;
    let target_size = backdrop_blur_target_size(bounds, ctx.pixels_per_point());
    update_frame(ctx, |frame| {
        frame.regions.push(BackdropBlurInfo {
            bounds,
            target_size,
            cached_source: source.is_some(),
            gpu_available: available,
        })
    });
    #[cfg(feature = "glow-backdrop")]
    if let Some(backend) = backend {
        painter.add(egui::PaintCallback {
            rect: bounds,
            callback: std::sync::Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                backend.paint(info, painter, source);
            })),
        });
    }
}

/// Enable native GPU blur after the renderer has created a compatible context.
/// Call `destroy_backdrop_blur` from the application's renderer shutdown hook.
#[cfg(feature = "glow-backdrop")]
pub fn enable_backdrop_blur(ctx: &Context, gl: &egui_glow::glow::Context) {
    if gl.version().major < 3
        || (!gl.version().is_embedded && gl.version().minor == 0 && gl.version().major == 3)
    {
        return;
    }
    ctx.data_mut(|data| data.insert_temp(gpu::backend_id(), gpu::Backend::default()));
}

#[cfg(feature = "glow-backdrop")]
pub fn destroy_backdrop_blur(ctx: &Context, gl: &egui_glow::glow::Context) {
    if let Some(backend) = ctx.data_mut(|data| data.remove_temp::<gpu::Backend>(gpu::backend_id()))
    {
        backend.destroy(gl);
    }
}

#[cfg(feature = "glow-backdrop")]
mod gpu {
    use super::*;
    use egui_glow::glow::{self, HasContext};
    use std::sync::{Arc, Mutex};

    pub(super) fn backend_id() -> Id {
        Id::new("rill_backdrop_blur_gpu")
    }

    #[derive(Clone, Default)]
    pub(super) struct Backend(Arc<Mutex<State>>);

    #[derive(Default)]
    struct State {
        resources: Option<Resources>,
        failed: bool,
    }

    struct Resources {
        program: glow::Program,
        vao: glow::VertexArray,
        sampler: Option<glow::UniformLocation>,
        direction: Option<glow::UniformLocation>,
        textures: [glow::Texture; 2],
        framebuffers: [glow::Framebuffer; 2],
        size: [u32; 2],
        cached: Option<(u64, [i32; 4], u32)>,
    }

    impl Backend {
        pub(super) fn paint(
            &self,
            info: egui::PaintCallbackInfo,
            painter: &egui_glow::Painter,
            source: Option<u64>,
        ) {
            let gl = painter.gl();
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            if state.failed {
                return;
            }
            // All GL calls execute inside the renderer callback on its owning thread.
            unsafe {
                if state.resources.is_none() {
                    match Resources::new(gl) {
                        Ok(resources) => state.resources = Some(resources),
                        Err(_) => {
                            state.failed = true;
                            return;
                        }
                    }
                }
                let Some(resources) = state.resources.as_mut() else {
                    return;
                };
                let target = painter.intermediate_fbo();
                let viewport = info.viewport_in_pixels();
                let source_rect = [
                    viewport.left_px,
                    viewport.from_bottom_px,
                    viewport.width_px,
                    viewport.height_px,
                ];
                let size = backdrop_blur_target_size(info.viewport, info.pixels_per_point);
                let key =
                    source.map(|source| (source, source_rect, info.pixels_per_point.to_bits()));
                gl.disable(glow::SCISSOR_TEST);
                gl.disable(glow::BLEND);
                gl.disable(glow::DEPTH_TEST);
                gl.active_texture(glow::TEXTURE0);
                gl.bind_vertex_array(Some(resources.vao));
                gl.use_program(Some(resources.program));
                gl.uniform_1_i32(resources.sampler.as_ref(), 0);
                if resources.size != size {
                    if !resources.resize(gl, size) {
                        gl.bind_framebuffer(glow::FRAMEBUFFER, target);
                        state.failed = true;
                        return;
                    }
                    resources.cached = None;
                }
                if key.is_none() || resources.cached != key {
                    gl.bind_framebuffer(glow::READ_FRAMEBUFFER, target);
                    gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(resources.framebuffers[0]));
                    gl.blit_framebuffer(
                        source_rect[0],
                        source_rect[1],
                        source_rect[0] + source_rect[2],
                        source_rect[1] + source_rect[3],
                        0,
                        0,
                        size[0] as i32,
                        size[1] as i32,
                        glow::COLOR_BUFFER_BIT,
                        glow::LINEAR,
                    );
                    gl.viewport(0, 0, size[0] as i32, size[1] as i32);
                    resources.pass(gl, 0, 1, [1.0 / size[0] as f32, 0.0]);
                    resources.pass(gl, 1, 0, [0.0, 1.0 / size[1] as f32]);
                    resources.cached = key;
                }
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(resources.framebuffers[0]));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, target);
                gl.blit_framebuffer(
                    0,
                    0,
                    size[0] as i32,
                    size[1] as i32,
                    source_rect[0],
                    source_rect[1],
                    source_rect[0] + source_rect[2],
                    source_rect[1] + source_rect[3],
                    glow::COLOR_BUFFER_BIT,
                    glow::LINEAR,
                );
                gl.bind_framebuffer(glow::FRAMEBUFFER, target);
                // egui_glow restores its viewport, shader, VAO, texture unit,
                // scissor, and blending immediately after this callback.
            }
        }

        pub(super) fn destroy(&self, gl: &glow::Context) {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(resources) = state.resources.take() {
                // on_exit is called while the rendering context is still current.
                unsafe {
                    resources.destroy(gl);
                }
            }
            state.failed = true;
        }
    }

    impl Resources {
        unsafe fn new(gl: &glow::Context) -> Result<Self, String> {
            unsafe {
                let version = if gl.version().is_embedded {
                    "#version 300 es\nprecision highp float;\n"
                } else {
                    "#version 140\n"
                };
                let vertex = format!(
                    "{version}out vec2 uv; void main() {{ vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2)); uv = p; gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0); }}"
                );
                let fragment = format!(
                    "{version}in vec2 uv; out vec4 color; uniform sampler2D source_image; uniform vec2 direction; void main() {{ color = texture(source_image, uv) * 0.2270270270; color += texture(source_image, uv + direction * 1.3846153846) * 0.3162162162; color += texture(source_image, uv - direction * 1.3846153846) * 0.3162162162; color += texture(source_image, uv + direction * 3.2307692308) * 0.0702702703; color += texture(source_image, uv - direction * 3.2307692308) * 0.0702702703; }}"
                );
                let program = gl.create_program()?;
                let mut shaders = Vec::new();
                for (kind, source) in [
                    (glow::VERTEX_SHADER, vertex),
                    (glow::FRAGMENT_SHADER, fragment),
                ] {
                    let shader = match gl.create_shader(kind) {
                        Ok(shader) => shader,
                        Err(error) => {
                            for shader in shaders {
                                gl.delete_shader(shader);
                            }
                            gl.delete_program(program);
                            return Err(error);
                        }
                    };
                    gl.shader_source(shader, &source);
                    gl.compile_shader(shader);
                    if !gl.get_shader_compile_status(shader) {
                        let error = gl.get_shader_info_log(shader);
                        gl.delete_shader(shader);
                        for shader in shaders {
                            gl.delete_shader(shader);
                        }
                        gl.delete_program(program);
                        return Err(error);
                    }
                    gl.attach_shader(program, shader);
                    shaders.push(shader);
                }
                gl.link_program(program);
                for shader in shaders {
                    gl.detach_shader(program, shader);
                    gl.delete_shader(shader);
                }
                if !gl.get_program_link_status(program) {
                    let error = gl.get_program_info_log(program);
                    gl.delete_program(program);
                    return Err(error);
                }
                let vao = match gl.create_vertex_array() {
                    Ok(vao) => vao,
                    Err(error) => {
                        gl.delete_program(program);
                        return Err(error);
                    }
                };
                let mut textures = Vec::new();
                let mut framebuffers = Vec::new();
                for _ in 0..2 {
                    let result = (|| {
                        let texture = gl.create_texture()?;
                        textures.push(texture);
                        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                        for parameter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                            gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, glow::LINEAR as i32);
                        }
                        for parameter in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                            gl.tex_parameter_i32(
                                glow::TEXTURE_2D,
                                parameter,
                                glow::CLAMP_TO_EDGE as i32,
                            );
                        }
                        framebuffers.push(gl.create_framebuffer()?);
                        Ok::<_, String>(())
                    })();
                    if let Err(error) = result {
                        for texture in textures {
                            gl.delete_texture(texture);
                        }
                        for framebuffer in framebuffers {
                            gl.delete_framebuffer(framebuffer);
                        }
                        gl.delete_vertex_array(vao);
                        gl.delete_program(program);
                        return Err(error);
                    }
                }
                Ok(Self {
                    sampler: gl.get_uniform_location(program, "source_image"),
                    direction: gl.get_uniform_location(program, "direction"),
                    program,
                    vao,
                    textures: [textures[0], textures[1]],
                    framebuffers: [framebuffers[0], framebuffers[1]],
                    size: [0, 0],
                    cached: None,
                })
            }
        }

        unsafe fn resize(&mut self, gl: &glow::Context, size: [u32; 2]) -> bool {
            unsafe {
                for index in 0..2 {
                    gl.bind_texture(glow::TEXTURE_2D, Some(self.textures[index]));
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
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.framebuffers[index]));
                    gl.framebuffer_texture_2d(
                        glow::FRAMEBUFFER,
                        glow::COLOR_ATTACHMENT0,
                        glow::TEXTURE_2D,
                        Some(self.textures[index]),
                        0,
                    );
                    if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE
                    {
                        return false;
                    }
                }
                self.size = size;
                true
            }
        }

        unsafe fn pass(
            &self,
            gl: &glow::Context,
            input: usize,
            output: usize,
            direction: [f32; 2],
        ) {
            unsafe {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.framebuffers[output]));
                gl.bind_texture(glow::TEXTURE_2D, Some(self.textures[input]));
                gl.uniform_2_f32(self.direction.as_ref(), direction[0], direction[1]);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            }
        }

        unsafe fn destroy(self, gl: &glow::Context) {
            unsafe {
                for framebuffer in self.framebuffers {
                    gl.delete_framebuffer(framebuffer);
                }
                for texture in self.textures {
                    gl.delete_texture(texture);
                }
                gl.delete_vertex_array(self.vao);
                gl.delete_program(self.program);
            }
        }
    }
}

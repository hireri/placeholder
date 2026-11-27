//! The visualizer's shader: one fullscreen triangle per effect, with every shape worked out per pixel from the band levels,
//! so a frame costs a handful of uniforms instead of thousands of CPU-built vertices. Needs the GL backend with a
//! shader interface of GLSL 140 / ES 300 or newer; anywhere else `usable` turns false and `viz` draws with meshes.

use eframe::egui::{self, Color32, Context, Rect};
use eframe::egui_glow::{self, glow, glow::HasContext};
use std::sync::{Arc, Mutex};

/// Modes 1..=5 are the visualizer styles, numbered as in `viz::STYLES`.
pub const GLOW: i32 = 6;
const BLIT: i32 = 8;

const VERT: &str = r#"
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const FRAG: &str = r#"
precision highp float;
in vec2 v_uv;
out vec4 o;
uniform int u_mode;
uniform vec2 u_px;      // size of the drawn rect in physical pixels
uniform float u_ppp;    // pixels per point
uniform float u_b[48];  // band levels
uniform float u_pk[48]; // peak caps
uniform vec4 u_col;     // straight rgb + opacity
uniform vec4 u_p;       // bass, kick, gate, spare
uniform sampler2D u_tex;

const float PI = 3.14159265;

float band(float f) {
    float q = clamp(f, 0.0, 1.0) * 47.0;
    int i = int(q);
    return mix(u_b[i], u_b[min(i + 1, 47)], q - float(i));
}
// Catmull-Rom through the band levels: the linear `band` leaves a visible zigzag on a line.
float smooth_band(float f) {
    float q = clamp(f, 0.0, 1.0) * 47.0;
    int i = int(q);
    float t = q - float(i);
    float a = u_b[max(i - 1, 0)], b = u_b[i], c = u_b[min(i + 1, 47)], d = u_b[min(i + 2, 47)];
    return max(0.0, 0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t + (3.0 * (b - c) + d - a) * t * t * t));
}
float peak(float f) { return u_pk[int(clamp(f, 0.0, 1.0) * 47.0 + 0.5)]; }
float cov(float d) { return clamp(0.5 - d, 0.0, 1.0); }
float falloff(float t) { t = clamp(t, 0.0, 1.0); return 1.0 - t * t * (3.0 - 2.0 * t); }
float box(vec2 p, vec2 c, vec2 h, float r) {
    vec2 q = abs(p - c) - h + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
void emit(float a) { o = vec4(u_col.rgb * a * u_col.a, a * u_col.a); }

void main() {
    vec2 p = v_uv * u_px;
    float k = u_ppp;
    if (u_mode == 8) { // a finished image, with rounded corners (u_p.x, in points) and an opacity
        float a = cov(box(p, u_px * 0.5, u_px * 0.5, u_p.x * u_ppp)) * u_col.a;
        o = vec4(texture(u_tex, v_uv).rgb * a, a);
        return;
    }
    if (u_mode == 6) { // soft glow, an ellipse filling the rect
        emit(falloff(length((p - u_px * 0.5) / (u_px * 0.5))));
        return;
    }
    int n = int(clamp(floor(u_px.x / (9.0 * k)), 18.0, 40.0));
    float a = 0.0;
    if (u_mode == 1 || u_mode == 2) {
        float cell = u_px.x / float(n), bw = cell * 0.78;
        float i = clamp(floor(p.x / cell), 0.0, float(n - 1));
        float cx = (i + 0.5) * cell, f = i / float(n - 1);
        float v = max(band(f), 0.05 * u_p.z);
        if (u_mode == 1) {
            float h = v * u_px.y;
            float d = box(p, vec2(cx, h * 0.5), vec2(bw * 0.5, h * 0.5), 2.0 * k);
            float t = clamp((h - p.y) / max(h, 1.0), 0.0, 1.0);
            float g = mix(1.0, 0.62, t * t * (3.0 - 2.0 * t));
            float capy = min(peak(f) * u_px.y, u_px.y - 8.0 * k) + 4.0 * k;
            float cap = cov(box(p, vec2(cx, capy), vec2(bw * 0.5, 1.0 * k), 0.0)) * 0.8;
            a = max(cov(d) * g, cap);
        } else {
            float mid = u_px.y * 0.5, h = v * mid;
            float d = box(p, vec2(cx, mid), vec2(bw * 0.5, h), bw * 0.5);
            float t = clamp(abs(p.y - mid) / max(h, 1.0), 0.0, 1.0);
            float g = p.y > mid ? mix(1.0, 0.75, t) : mix(0.75, 0.4, t);
            a = cov(d) * g;
        }
    } else if (u_mode == 3) {
        float f = p.x / u_px.x, e = 1.0 / 47.0;
        float y = max(smooth_band(f), 0.03 * u_p.z) * u_px.y;
        float slope = (smooth_band(f + e) - smooth_band(f - e)) / (2.0 * e) * u_px.y / u_px.x;
        float dist = abs(p.y - y) / sqrt(1.0 + slope * slope);
        float fill = p.y < y ? 0.34 * pow(clamp(p.y / max(y, 1.0), 0.0, 1.0), 1.6) : 0.0;
        a = max(max(fill, cov(dist - 1.6 * k)), 0.28 * exp(-dist / (7.0 * k)));
    } else if (u_mode == 4) {
        int cols = n / 2;
        float cw = u_px.x / float(cols);
        float rows = clamp(floor(u_px.y / cw + 0.5), 5.0, 14.0);
        float d = min(cw, u_px.y / rows);
        float i = clamp(floor(p.x / cw), 0.0, float(cols - 1));
        float j = floor(p.y / d);
        vec2 c = vec2((i + 0.5) * cw, (j + 0.5) * d);
        float lit = floor(band(i / float(cols - 1)) * rows + 0.5);
        float al = j < lit ? 0.55 + 0.45 * j / rows : 0.13;
        a = cov(length(p - c) - d * 0.31) * al * step(j, rows - 1.0);
    } else if (u_mode == 5) {
        float m = min(u_px.x, u_px.y);
        float r0 = m * 0.26 + u_p.x * m * 0.03, len = m * 0.22;
        vec2 q = p - u_px * 0.5;
        float r = length(q);
        float t = fract(atan(q.x, q.y) / (2.0 * PI)) * 90.0;
        float j = floor(t);
        float f = abs((j + 0.5) / 90.0 * 2.0 - 1.0);
        float l = (0.07 + band(f)) * len;
        float tang = abs(r * sin((t - j - 0.5) / 90.0 * 2.0 * PI));
        float d = max(tang - 1.3 * k, max(r0 - r, r - (r0 + l)));
        a = max(cov(d), falloff(r / (r0 * 1.2)) * (0.12 + 0.25 * u_p.x));
    }
    emit(a);
}
"#;

struct Program {
    prog: glow::Program,
    vao: glow::VertexArray,
}

#[derive(Default)]
pub struct Gl {
    prog: Option<Program>,
    failed: bool,
}

pub type Shared = Arc<Mutex<Gl>>;

pub fn shared(ctx: &Context) -> Shared {
    ctx.data_mut(|d| d.get_temp_mut_or_insert_with::<Shared>(egui::Id::unique("viz-gl"), Default::default).clone())
}

/// False once the shader failed to build here (checked by the callers, which then draw with meshes instead).
pub fn usable(ctx: &Context) -> bool {
    !shared(ctx).lock().unwrap().failed
}

pub struct Params {
    pub mode: i32,
    pub bands: [f32; crate::viz::BANDS],
    pub peaks: [f32; crate::viz::BANDS],
    /// Colour with the opacity in its (premultiplied) alpha.
    pub col: Color32,
    /// bass, kick, gate.
    pub p: [f32; 3],
    /// A GL texture for the blit mode to draw.
    pub raw_tex: Option<glow::Texture>,
}

/// Queues the effect over `rect`.
pub fn paint(painter: &egui::Painter, rect: Rect, params: Params) {
    let sh = shared(painter.ctx());
    let cb = egui_glow::CallbackFn::new(move |info, gp| run(&sh, &params, &info, gp));
    painter.add(egui::PaintCallback { rect, callback: Arc::new(cb) });
}

fn build(gl: &glow::Context) -> Result<Program, String> {
    let v = egui_glow::ShaderVersion::get(gl);
    if !v.is_new_shader_interface() {
        return Err("GLSL too old".into());
    }
    let head = format!("{}\n", v.version_declaration());
    unsafe {
        let compile = |kind, src: &str| -> Result<glow::Shader, String> {
            let s = gl.create_shader(kind)?;
            gl.shader_source(s, &format!("{head}{src}"));
            gl.compile_shader(s);
            if gl.get_shader_compile_status(s) { Ok(s) } else { Err(gl.get_shader_info_log(s)) }
        };
        let (vs, fs) = (compile(glow::VERTEX_SHADER, VERT)?, compile(glow::FRAGMENT_SHADER, FRAG)?);
        let prog = gl.create_program()?;
        gl.attach_shader(prog, vs);
        gl.attach_shader(prog, fs);
        gl.link_program(prog);
        let ok = gl.get_program_link_status(prog);
        gl.detach_shader(prog, vs);
        gl.detach_shader(prog, fs);
        gl.delete_shader(vs);
        gl.delete_shader(fs);
        if !ok {
            return Err(gl.get_program_info_log(prog));
        }
        Ok(Program { prog, vao: gl.create_vertex_array()? })
    }
}

pub fn run(sh: &Shared, p: &Params, info: &egui::PaintCallbackInfo, gp: &egui_glow::Painter) {
    let gl = gp.gl();
    let mut st = sh.lock().unwrap();
    if st.failed {
        return;
    }
    if st.prog.is_none() {
        match build(gl) {
            Ok(pr) => st.prog = Some(pr),
            Err(e) => {
                eprintln!("viz shader: {e}");
                st.failed = true;
                return;
            }
        }
    }
    let pr = st.prog.as_ref().unwrap();
    let vp = info.viewport_in_pixels();
    let ppp = info.pixels_per_point;
    let [r, g, b, a] = p.col.to_array().map(|c| c as f32 / 255.);
    let un = if a > 0. { 1. / a } else { 0. };
    unsafe {
        gl.use_program(Some(pr.prog));
        gl.bind_vertex_array(Some(pr.vao));
        gl.enable(glow::BLEND);
        gl.blend_func_separate(glow::ONE, glow::ONE_MINUS_SRC_ALPHA, glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
        let loc = |n: &str| gl.get_uniform_location(pr.prog, n);
        gl.uniform_1_i32(loc("u_mode").as_ref(), p.mode);
        gl.uniform_2_f32(loc("u_px").as_ref(), vp.width_px as f32, vp.height_px as f32);
        gl.uniform_1_f32(loc("u_ppp").as_ref(), ppp);
        gl.uniform_1_f32_slice(loc("u_b").as_ref(), &p.bands);
        gl.uniform_1_f32_slice(loc("u_pk").as_ref(), &p.peaks);
        gl.uniform_4_f32(loc("u_col").as_ref(), (r * un).min(1.), (g * un).min(1.), (b * un).min(1.), a);
        gl.uniform_4_f32(loc("u_p").as_ref(), p.p[0], p.p[1], p.p[2], 0.);
        if let Some(tex) = p.raw_tex {
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.uniform_1_i32(loc("u_tex").as_ref(), 0);
        }
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
    }
}

/// Draws a finished GL texture over the callback's rect (used for projectM's output and for restoring what it overwrote).
pub fn blit(sh: &Shared, info: &egui::PaintCallbackInfo, gp: &egui_glow::Painter, tex: glow::Texture, radius: f32, opacity: f32) {
    let p = Params { mode: BLIT, bands: [0.; crate::viz::BANDS], peaks: [0.; crate::viz::BANDS], col: Color32::WHITE.gamma_multiply(opacity), p: [radius, 0., 0.], raw_tex: Some(tex) };
    run(sh, &p, info, gp);
}

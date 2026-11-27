//! MilkDrop-compatible visuals through libprojectM, loaded at run time so the app neither builds nor bundles it:
//! the style is offered only when the library is installed (`libprojectM-4`). projectM renders into an offscreen
//! texture inside a GL paint callback; `viz_gl` then blits that texture into the player.

use crate::viz_gl;
use eframe::egui::{self, pos2, Align2, Color32, FontId, Rect, Sense, Ui};
use eframe::egui_glow::{self, glow, glow::HasContext};
use libloading::Library;
use std::ffi::{c_char, c_void, CString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

type Handle = *mut c_void;
const MONO: i32 = 1;
/// A preset plays this long before the next one fades in.
const PRESET_SECS: f64 = 35.;

struct Lib {
    _lib: Library,
    create: unsafe extern "C" fn() -> Handle,
    window_size: unsafe extern "C" fn(Handle, usize, usize),
    load_preset: unsafe extern "C" fn(Handle, *const c_char, bool),
    pcm_add: unsafe extern "C" fn(Handle, *const f32, u32, i32),
    render: unsafe extern "C" fn(Handle),
    fps: unsafe extern "C" fn(Handle, i32),
    soft_cut: unsafe extern "C" fn(Handle, f64),
    hard_cut: unsafe extern "C" fn(Handle, bool),
}

fn lib() -> Option<&'static Lib> {
    static L: OnceLock<Option<Lib>> = OnceLock::new();
    L.get_or_init(|| unsafe {
        let names: &[&str] = if cfg!(windows) {
            &["projectM-4.dll"]
        } else if cfg!(target_os = "macos") {
            &["libprojectM-4.dylib", "libprojectM-4.4.dylib"]
        } else {
            &["libprojectM-4.so.4", "libprojectM-4.so"]
        };
        let env = std::env::var_os("SERAFIN_PROJECTM_LIB");
        let l = env.iter().map(|p| p.to_string_lossy().into_owned()).chain(names.iter().map(|s| s.to_string())).find_map(|n| Library::new(n).ok())?;
        unsafe fn get<T: Copy>(l: &Library, n: &str) -> Option<T> {
            l.get::<T>(format!("{n}\0").as_bytes()).ok().map(|s| *s)
        }
        macro_rules! sym {
            ($n:literal) => {
                get(&l, $n)?
            };
        }
        Some(Lib {
            create: sym!("projectm_create"),
            window_size: sym!("projectm_set_window_size"),
            load_preset: sym!("projectm_load_preset_file"),
            pcm_add: sym!("projectm_pcm_add_float"),
            render: sym!("projectm_opengl_render_frame"),
            fps: sym!("projectm_set_fps"),
            soft_cut: sym!("projectm_set_soft_cut_duration"),
            hard_cut: sym!("projectm_set_hard_cut_enabled"),
            _lib: l,
        })
    })
    .as_ref()
}

/// Whether libprojectM could be loaded; the settings picker only offers the style then.
pub fn available() -> bool {
    lib().is_some()
}

struct Inst {
    h: Handle,
    /// projectM's last frame.
    tex: glow::Texture,
    /// What the window held where projectM draws, put back after each frame.
    save: glow::Texture,
    size: (i32, i32),
    /// When projectM last rendered; it steps on a fixed clock so extra repaints (mouse moves) can't speed it up.
    last: Instant,
    fps: i32,
    /// A new size that has not held still long enough to switch to yet.
    pending: Option<((i32, i32), Instant)>,
}

// The handle is only ever touched from the render thread, inside the paint callback. It lives until the process exits.
unsafe impl Send for Inst {}

#[derive(Default)]
struct Gpu {
    inst: Option<Inst>,
    failed: bool,
    /// Preset to switch to at the next frame.
    request: Option<PathBuf>,
}

/// Preset list and what is playing; UI side.
#[derive(Clone, Default)]
struct Deck {
    list: Vec<PathBuf>,
    cur: usize,
    since: f64,
    started: bool,
    scanned: bool,
}

fn scan(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            scan(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("milk")) {
            out.push(p);
        }
    }
}

/// Where presets are looked for: the app's `presets` folder in its config directory, then the usual install locations.
pub fn dirs() -> Vec<PathBuf> {
    let mut v = vec![crate::backend::data_dir().join("presets")];
    v.extend(["/usr/share/projectM/presets", "/usr/share/projectm/presets"].map(PathBuf::from));
    v
}

fn pcg(seed: u64) -> u64 {
    seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) >> 33
}

/// Draws the visuals over `rect` from the latest mono output `pcm`. Click for the next preset.
pub fn paint(ui: &Ui, rect: Rect, pcm: &[f32], opacity: f32, radius: f32) {
    let ctx = ui.ctx();
    let p = ui.painter();
    let msg = |text: String| {
        p.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(13.), Color32::from_white_alpha(140));
    };
    if !available() {
        return msg("MilkDrop needs libprojectM-4 installed".into());
    }
    let id = egui::Id::unique("viz-pm-deck");
    let mut deck: Deck = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    if !deck.scanned {
        deck.scanned = true;
        for d in dirs() {
            scan(&d, &mut deck.list);
        }
        deck.list.sort();
    }
    if deck.list.is_empty() {
        ctx.data_mut(|d| d.insert_temp(id, deck.clone()));
        return msg(format!("No .milk presets found. Put some in {}.", crate::backend::data_dir().join("presets").display()));
    }
    let now = ctx.input(|i| i.time);
    let resp = ui.interact(rect, id.with("hit"), Sense::click());
    let st: Arc<Mutex<Gpu>> = ctx.data_mut(|d| d.get_temp_mut_or_insert_with::<Arc<Mutex<Gpu>>>(egui::Id::unique("viz-pm"), Default::default).clone());
    if !deck.started || resp.clicked() || now - deck.since > PRESET_SECS {
        deck.cur = if deck.started { (deck.cur + 1 + pcg(now.to_bits()) as usize % deck.list.len().max(2).saturating_sub(1)) % deck.list.len() } else { pcg(now.to_bits()) as usize % deck.list.len() };
        (deck.since, deck.started) = (now, true);
        st.lock().unwrap().request = Some(deck.list[deck.cur].clone());
    }
    ctx.data_mut(|d| d.insert_temp(id, deck.clone()));
    // Presets are stepped per frame, so a fixed rate keeps their speed the same whatever the frame rate setting or focus.
    let fps = 30;
    let gl_shared = viz_gl::shared(ctx);
    let pcm = pcm.to_vec();
    let cb = egui_glow::CallbackFn::new(move |info, gp| run(&st, &gl_shared, &pcm, (radius, opacity, fps), &info, gp));
    p.add(egui::PaintCallback { rect, callback: Arc::new(cb) });
    // The preset's name shows for a few seconds after each change.
    let shown = ((4. - (now - deck.since)) / 1.).clamp(0., 1.) as f32;
    if shown > 0. {
        let name = deck.list[deck.cur].file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        p.text(pos2(rect.min.x + 16., rect.min.y + 118.), Align2::LEFT_TOP, name, FontId::proportional(12.), Color32::WHITE.gamma_multiply(0.8 * shown * opacity));
    }
    ctx.request_repaint_after(Duration::from_millis(1000 / fps as u64));
}

fn make_tex(gl: &glow::Context, w: i32, h: i32, filter: u32) -> Option<glow::Texture> {
    unsafe {
        let tex = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, w, h, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
        for (k, v) in [(glow::TEXTURE_MIN_FILTER, filter), (glow::TEXTURE_MAG_FILTER, filter), (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE), (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE)] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
        }
        Some(tex)
    }
}

/// Copies the framebuffer's lower-left `w`x`h` pixels into `tex`.
unsafe fn grab(gl: &glow::Context, tex: glow::Texture, w: i32, h: i32) {
    // projectM leaves its own framebuffer bound for reading.
    gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
    gl.copy_tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, 0, 0, 0, w, h);
}

// projectM 4.1 always draws its final image to the default framebuffer at the window origin, so it can't be pointed at
// an offscreen target: the pixels there are saved first, projectM draws, the result is copied out, and the saved pixels
// go back before egui carries on.
fn run(st: &Arc<Mutex<Gpu>>, gl_shared: &viz_gl::Shared, pcm: &[f32], (radius, opacity, fps): (f32, f32, i32), info: &egui::PaintCallbackInfo, gp: &egui_glow::Painter) {
    let Some(l) = lib() else { return };
    let gl = gp.gl();
    let mut s = st.lock().unwrap();
    if s.failed {
        return;
    }
    let vp = info.viewport_in_pixels();
    let want = (vp.width_px.max(1), vp.height_px.max(1));
    let screen = (info.screen_size_px[0] as i32, info.screen_size_px[1] as i32);
    unsafe {
        // Resizing reallocates projectM's buffers, so it keeps rendering at the old size (the picture just stretches)
        // until the size has held for a moment. A size larger than the window can't be read back, so that switches at once.
        if let Some(i) = s.inst.as_mut().filter(|i| i.size != want) {
            let since = match i.pending {
                Some((w, t)) if w == want => t,
                _ => Instant::now(),
            };
            i.pending = Some((want, since));
            if since.elapsed() >= Duration::from_millis(150) || i.size.0 > screen.0 || i.size.1 > screen.1 {
                gl.delete_texture(i.tex);
                gl.delete_texture(i.save);
                let (Some(tex), Some(save)) = (make_tex(gl, want.0, want.1, glow::LINEAR), make_tex(gl, want.0, want.1, glow::NEAREST)) else { return s.failed = true };
                (i.tex, i.save, i.size, i.pending) = (tex, save, want, None);
                (l.window_size)(i.h, want.0 as usize, want.1 as usize);
            }
        }
        let (w, h) = s.inst.as_ref().map_or(want, |i| i.size);
        if s.inst.is_none() {
            let handle = (l.create)();
            let (tex, save) = (make_tex(gl, w, h, glow::LINEAR), make_tex(gl, w, h, glow::NEAREST));
            let (Some(tex), Some(save)) = (tex, save) else { return s.failed = true };
            if handle.is_null() {
                eprintln!("viz: projectM could not start (it needs OpenGL 3.3)");
                return s.failed = true;
            }
            (l.fps)(handle, fps);
            (l.soft_cut)(handle, 3.);
            (l.hard_cut)(handle, false);
            (l.window_size)(handle, w as usize, h as usize);
            s.inst = Some(Inst { h: handle, tex, save, size: (w, h), last: Instant::now() - Duration::from_secs(1), fps, pending: None });
        }
        if let Some(path) = s.request.take() {
            if let Ok(c) = CString::new(path.to_string_lossy().as_bytes()) {
                (l.load_preset)(s.inst.as_ref().unwrap().h, c.as_ptr(), true);
            }
        }
        let i = s.inst.as_mut().unwrap();
        // Render only when a frame is due; otherwise the last one is drawn again.
        if i.fps != fps {
            // projectM advances one step per rendered frame, so it is told the rate it is now being run at.
            (l.fps)(i.h, fps);
            i.fps = fps;
        }
        if i.last.elapsed() >= Duration::from_micros(1_000_000 / fps as u64 - 500) {
            i.last = Instant::now();
            (l.pcm_add)(i.h, pcm.as_ptr(), pcm.len() as u32, MONO);
            grab(gl, i.save, w, h);
            gl.disable(glow::SCISSOR_TEST);
            (l.render)(i.h);
            grab(gl, i.tex, w, h);
            gl.viewport(0, 0, w, h);
            viz_gl::blit(gl_shared, info, gp, i.save, 0., 1.);
        }
        gl.viewport(vp.left_px, vp.from_bottom_px, vp.width_px, vp.height_px);
        let c = info.clip_rect_in_pixels();
        gl.enable(glow::SCISSOR_TEST);
        gl.scissor(c.left_px, c.from_bottom_px, c.width_px, c.height_px);
        viz_gl::blit(gl_shared, info, gp, i.tex, radius, opacity);
    }
}

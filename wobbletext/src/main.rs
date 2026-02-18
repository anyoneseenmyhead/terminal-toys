use anyhow::{anyhow, Context, Result};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Color, ResetColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use fontdue::{
    layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle},
    Font,
};
use glam::{Mat4, Vec2, Vec3, Vec4};
use glam::Vec4Swizzles;
use std::{
    collections::HashMap,
    env,
    io::{self, Write},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
struct MicroPixel {
    z: f32,    // smaller = closer
    luma: f32, // 0..1
}
struct MicroBuffer {
    w: usize, // term_cols * 2
    h: usize, // term_rows * 4
    px: Vec<MicroPixel>,
}
impl MicroBuffer {
    fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![
                MicroPixel {
                    z: f32::INFINITY,
                    luma: 0.0
                };
                w * h
            ],
        }
    }
    fn resize(&mut self, w: usize, h: usize) {
        self.w = w;
        self.h = h;
        self.px.resize(
            w * h,
            MicroPixel {
                z: f32::INFINITY,
                luma: 0.0,
            },
        );
    }
    fn clear(&mut self) {
        for p in &mut self.px {
            p.z = f32::INFINITY;
            p.luma = 0.0;
        }
    }
    #[inline]
    fn idx(&self, x: usize, y: usize) -> usize {
        y * self.w + x
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Cell {
    ch: char,
    fg: u8, // grayscale 0..255
}
struct CellBuffer {
    cols: u16,
    rows: u16,
    cells: Vec<Cell>,
}
impl CellBuffer {
    fn new(cols: u16, rows: u16) -> Self {
        let n = cols as usize * rows as usize;
        Self {
            cols,
            rows,
            cells: vec![
                Cell {
                    ch: '\u{2800}',
                    fg: 255
                };
                n
            ],
        }
    }
    fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        let n = cols as usize * rows as usize;
        self.cells.resize(
            n,
            Cell {
                ch: '\u{2800}',
                fg: 255,
            },
        );
        // Clear on resize for safety
        for c in &mut self.cells {
            c.ch = '\u{2800}';
            c.fg = 255;
        }
    }
    #[inline]
    fn idx(&self, col: u16, row: u16) -> usize {
        row as usize * self.cols as usize + col as usize
    }
}

#[derive(Clone)]
struct Mesh {
    verts: Vec<Vec3>,
    tris: Vec<(u32, u32, u32)>,
}

#[derive(Clone, Copy)]
struct Camera {
    pos: Vec3,
    target: Vec3,
    fov_y_radians: f32,
    near: f32,
    far: f32,
}
impl Camera {
    fn vp(&self, aspect: f32) -> Mat4 {
        let view = Mat4::look_at_rh(self.pos, self.target, Vec3::Y);
        let proj = Mat4::perspective_rh(self.fov_y_radians, aspect, self.near, self.far);
        proj * view
    }
}

fn main() -> Result<()> {
    let text = env::args().nth(1).unwrap_or_else(|| "DEFAULT".to_string());

    // Font: try a few common paths, or use DEFAULT_FONT env var.
    let font_bytes = load_font_bytes().context(
        "Could not find a font. Set DEFAULT_FONT to a .ttf path, or install DejaVuSans.ttf.",
    )?;
    let font = Font::from_bytes(font_bytes, fontdue::FontSettings::default())
        .map_err(|e| anyhow!("font parse error: {e:?}"))?;

    // Build once: rasterize -> solid grid -> mesh
    let mask = rasterize_text_mask(&font, &text, 72.0)?;
    // Extrude deeper so thickness reads clearly in 3D.
    let mesh = build_pixel_extrusion_mesh(&mask, 10.0);

    run_screensaver(mesh)?;

    Ok(())
}

fn load_font_bytes() -> Result<Vec<u8>> {
    if let Ok(p) = env::var("DEFAULT_FONT") {
        return std::fs::read(p).context("failed to read DEFAULT_FONT file");
    }

    // Common Linux paths first
    let candidates = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/Library/Fonts/Arial.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        "C:\\Windows\\Fonts\\consola.ttf",
    ];

    for p in candidates {
        if let Ok(b) = std::fs::read(p) {
            return Ok(b);
        }
    }

    Err(anyhow!("no font found"))
}

struct Mask2D {
    w: usize,
    h: usize,
    alpha: Vec<u8>, // 0..255
}

#[cfg(test)]
mod debug_bounds {
    use super::*;

    #[test]
    fn print_ndc_bounds() {
        let font_bytes = load_font_bytes().expect("font");
        let font = Font::from_bytes(font_bytes, fontdue::FontSettings::default()).unwrap();
        let mask = rasterize_text_mask(&font, "Test", 72.0).unwrap();
        let mesh = build_pixel_extrusion_mesh(&mask, 0.25);

        let cam = Camera {
            pos: Vec3::new(0.0, 0.0, 6.0),
            target: Vec3::ZERO,
            fov_y_radians: 45.0f32.to_radians(),
            near: 0.1,
            far: 50.0,
        };
        let cols = 120.0;
        let rows = 32.0;
        let aspect = (cols * 2.0) / (rows * 4.0); // micro aspect
        let vp = cam.vp(aspect);
        let model = screensaver_model(0.0);

        let mut min_ndc = Vec3::splat(f32::INFINITY);
        let mut max_ndc = Vec3::splat(f32::NEG_INFINITY);
        let mut min_v = Vec3::splat(f32::INFINITY);
        let mut max_v = Vec3::splat(f32::NEG_INFINITY);
        for v in &mesh.verts {
            let clip = vp * model * Vec4::new(v.x, v.y, v.z, 1.0);
            let ndc = clip.xyz() / clip.w;
            min_ndc = min_ndc.min(ndc);
            max_ndc = max_ndc.max(ndc);

            min_v = min_v.min(*v);
            max_v = max_v.max(*v);
        }

        println!(
            "mesh span: {:?}, ndc bounds: min {:?}, max {:?}",
            max_v - min_v,
            min_ndc,
            max_ndc
        );
    }

    #[test]
    fn print_ascii_snapshot() {
        let text = "Test";
        let font_bytes = load_font_bytes().expect("font");
        let font = Font::from_bytes(font_bytes, fontdue::FontSettings::default()).unwrap();
        let mask = rasterize_text_mask(&font, text, 72.0).unwrap();
        let mesh = build_pixel_extrusion_mesh(&mask, 0.25);

        let cols = 80u16;
        let rows = 24u16;
        let mut micro = MicroBuffer::new(cols as usize * 2, rows as usize * 4);
        let mut cells = CellBuffer::new(cols, rows);

        let cam = Camera {
            pos: Vec3::new(0.0, 0.0, 4.0),
            target: Vec3::ZERO,
            fov_y_radians: 45.0f32.to_radians(),
            near: 0.1,
            far: 50.0,
        };
        let aspect = micro.w as f32 / micro.h as f32;
        let vp = cam.vp(aspect);
        let model = screensaver_model(0.0);

        micro.clear();
        render_mesh_to_micro(&mesh, model, vp, &mut micro);
        pack_micro_into_cells(&micro, &mut cells);

        for row in 0..rows {
            for col in 0..cols {
                let c = cells.cells[cells.idx(col, row)];
                print!("{}", c.ch);
            }
            println!();
        }
    }

    #[test]
    fn mask_vertical_span() {
        let text = "Test";
        let font_bytes = load_font_bytes().expect("font");
        let font = Font::from_bytes(font_bytes, fontdue::FontSettings::default()).unwrap();
        let mask = rasterize_text_mask(&font, text, 72.0).unwrap();

        let mut min_y = mask.h;
        let mut max_y = 0usize;
        let mut min_x = mask.w;
        let mut max_x = 0usize;
        let mut count = 0usize;
        for y in 0..mask.h {
            for x in 0..mask.w {
                if mask.alpha[y * mask.w + x] > 0 {
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    count += 1;
                }
            }
        }
        println!(
            "mask filled: count {}, x:[{}..{}]/{}, y:[{}..{}]/{}",
            count, min_x, max_x, mask.w, min_y, max_y, mask.h
        );
    }
}

fn rasterize_text_mask(font: &Font, text: &str, px: f32) -> Result<Mask2D> {
    // Use fontdue's line layout so all glyphs share the same baseline.
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings::default());
    layout.append(&[font], &TextStyle::new(text, px, 0));

    let glyphs = layout.glyphs();
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;

    for g in glyphs {
        if g.width == 0 || g.height == 0 {
            continue;
        }
        let gx = g.x as i32;
        let gy = g.y as i32;
        min_x = min_x.min(gx);
        min_y = min_y.min(gy);
        max_x = max_x.max(gx + g.width as i32);
        max_y = max_y.max(gy + g.height as i32);
    }

    if min_x == i32::MAX || min_y == i32::MAX {
        return Ok(Mask2D {
            w: 1,
            h: 1,
            alpha: vec![0],
        });
    }

    let pad = 8i32;
    let w = (max_x - min_x + pad * 2).max(1) as usize;
    let h = (max_y - min_y + pad * 2).max(1) as usize;
    let off_x = pad - min_x;
    let off_y = pad - min_y;

    let mut alpha = vec![0u8; w * h];
    for g in glyphs {
        if g.width == 0 || g.height == 0 {
            continue;
        }
        let (metrics, bitmap) = font.rasterize_config(g.key);
        let gx0 = off_x + g.x as i32;
        let gy0 = off_y + g.y as i32;

        for y in 0..metrics.height as i32 {
            for x in 0..metrics.width as i32 {
                let sx = gx0 + x;
                let sy = gy0 + y;
                if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
                    let src = bitmap[(y as usize) * metrics.width + (x as usize)];
                    let di = (sy as usize) * w + (sx as usize);
                    if src > alpha[di] {
                        alpha[di] = src;
                    }
                }
            }
        }
    }

    Ok(Mask2D { w, h, alpha })
}

fn build_pixel_extrusion_mesh(mask: &Mask2D, depth: f32) -> Mesh {
    let w = mask.w as i32;
    let h = mask.h as i32;
    let threshold: u8 = 110;

    let is_solid = |x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x >= w || y >= h {
            return false;
        }
        let a = mask.alpha[(y as usize) * mask.w + (x as usize)];
        a >= threshold
    };

    let mut verts: Vec<Vec3> = Vec::new();
    let mut tris: Vec<(u32, u32, u32)> = Vec::new();
    let mut vmap: HashMap<(i32, i32, i32), u32> = HashMap::new();

    let mut push_v = |p: Vec3| -> u32 {
        // Quantize to integer grid key so we de-duplicate voxel vertices.
        let key = (p.x.round() as i32, p.y.round() as i32, (p.z * 1000.0).round() as i32);
        if let Some(&idx) = vmap.get(&key) {
            return idx;
        }
        let idx = verts.len() as u32;
        verts.push(p);
        vmap.insert(key, idx);
        idx
    };

    let mut add_quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| {
        let i0 = push_v(a);
        let i1 = push_v(b);
        let i2 = push_v(c);
        let i3 = push_v(d);
        tris.push((i0, i1, i2));
        tris.push((i0, i2, i3));
    };

    // Build in mask pixel coords, then normalize to centered world later.
    for y in 0..h {
        for x in 0..w {
            if !is_solid(x, y) {
                continue;
            }

            // voxel corners in XY, z in {0, -depth}
            let x0 = x as f32;
            let y0 = y as f32;
            let x1 = (x + 1) as f32;
            let y1 = (y + 1) as f32;

            let zf = 0.0f32;
            let zb = -depth;

            let a = Vec3::new(x0, y0, zf);
            let b = Vec3::new(x1, y0, zf);
            let c = Vec3::new(x1, y1, zf);
            let d = Vec3::new(x0, y1, zf);

            let ap = Vec3::new(x0, y0, zb);
            let bp = Vec3::new(x1, y0, zb);
            let cp = Vec3::new(x1, y1, zb);
            let dp = Vec3::new(x0, y1, zb);

            // Front and back
            add_quad(a, b, c, d);
            add_quad(ap, dp, cp, bp);

            // Sides only when neighbor is empty
            if !is_solid(x - 1, y) {
                add_quad(ap, a, d, dp);
            }
            if !is_solid(x + 1, y) {
                add_quad(b, bp, cp, c);
            }
            if !is_solid(x, y - 1) {
                add_quad(ap, bp, b, a);
            }
            if !is_solid(x, y + 1) {
                add_quad(d, c, cp, dp);
            }
        }
    }

    // Center and scale mesh to reasonable world size.
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for v in &verts {
        min = min.min(*v);
        max = max.max(*v);
    }
    // Anchor Y to the ink top (avoid descenders pulling the word down) and center only in X.
    let center_x = (min.x + max.x) * 0.5;
    let top_y = min.y; // mask coords grow downward
    let span = (max - min).max(Vec3::splat(1.0));

    // Fit primarily by height with a generous width cap so long strings don't explode.
    let target_h = 2.0;
    let target_w = 12.0;
    let scale_h = target_h / span.y;
    let scale_w = target_w / span.x;
    let scale = scale_h.min(scale_w);

    for v in &mut verts {
        v.x = (v.x - center_x) * scale;
        v.y = (v.y - top_y) * scale; // top anchored at 0 before flip
        v.z = v.z * scale;
        v.y = -v.y; // flip upright
        v.y += target_h * 0.5; // shift to vertical center-ish region
    }

    Mesh { verts, tris }
}

fn run_screensaver(mesh: Mesh) -> Result<()> {
    let mut stdout = io::stdout();

    terminal::enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, DisableLineWrap, cursor::Hide)?;

    let mut cols_rows = terminal::size()?;
    let mut micro = MicroBuffer::new(cols_rows.0 as usize * 2, cols_rows.1 as usize * 4);
    let mut curr = CellBuffer::new(cols_rows.0, cols_rows.1);
    let mut prev = CellBuffer::new(cols_rows.0, cols_rows.1);

    // Clear once
    execute!(stdout, terminal::Clear(terminal::ClearType::All))?;

    let cam = Camera {
        pos: Vec3::new(0.0, 0.0, 7.5),
        target: Vec3::ZERO,
        fov_y_radians: 45.0f32.to_radians(),
        near: 0.1,
        far: 50.0,
    };

    let start = Instant::now();
    let mut last_frame = Instant::now();
    let target_dt = Duration::from_millis(16);

    'outer: loop {
        // Input
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Esc | KeyCode::Char('q') => break 'outer,
                    _ => {}
                },
                _ => {}
            }
        }

        // Resize
        let new_size = terminal::size()?;
        if new_size != cols_rows {
            cols_rows = new_size;
            micro.resize(cols_rows.0 as usize * 2, cols_rows.1 as usize * 4);
            curr.resize(cols_rows.0, cols_rows.1);
            prev.resize(cols_rows.0, cols_rows.1);
            execute!(stdout, terminal::Clear(terminal::ClearType::All))?;
        }

        let now = Instant::now();
        let dt = now.saturating_duration_since(last_frame);
        last_frame = now;

        let t = start.elapsed().as_secs_f32();

        // Render
        micro.clear();

        let aspect = micro.w as f32 / micro.h.max(1) as f32;
        let vp = cam.vp(aspect);
        let model = screensaver_model(t);

        render_mesh_to_micro(&mesh, model, vp, &mut micro);

        pack_micro_into_cells(&micro, &mut curr);

        diff_flush(&mut stdout, &curr, &mut prev)?;

        // Sleep for fps
        if dt < target_dt {
            std::thread::sleep(target_dt - dt);
        }
    }

    // Cleanup
    execute!(
        stdout,
        ResetColor,
        cursor::Show,
        EnableLineWrap,
        LeaveAlternateScreen
    )?;
    terminal::disable_raw_mode()?;
    Ok(())
}

fn screensaver_model(t: f32) -> Mat4 {
    let yaw = t * 0.55;
    // Keep the text level to avoid baseline tilt; only yaw + gentle vertical bob.
    let pitch = 0.0;
    let roll = 0.0;
    let bob = (t * 0.6).sin() * 0.12;

    Mat4::from_translation(Vec3::new(0.0, bob, 0.0))
        * Mat4::from_rotation_y(yaw)
        * Mat4::from_rotation_x(pitch)
        * Mat4::from_rotation_z(roll)
}

fn render_mesh_to_micro(mesh: &Mesh, model: Mat4, vp: Mat4, micro: &mut MicroBuffer) {
    let light_dir = Vec3::new(0.25, 0.7, 0.6).normalize();

    for &(i0, i1, i2) in &mesh.tris {
        let v0 = (model * Vec4::new(mesh.verts[i0 as usize].x, mesh.verts[i0 as usize].y, mesh.verts[i0 as usize].z, 1.0)).xyz();
        let v1 = (model * Vec4::new(mesh.verts[i1 as usize].x, mesh.verts[i1 as usize].y, mesh.verts[i1 as usize].z, 1.0)).xyz();
        let v2 = (model * Vec4::new(mesh.verts[i2 as usize].x, mesh.verts[i2 as usize].y, mesh.verts[i2 as usize].z, 1.0)).xyz();

        let n = (v1 - v0).cross(v2 - v0);
        let nlen = n.length();
        if nlen < 1e-6 {
            continue;
        }
        let n = n / nlen;
        let mut luma = n.dot(light_dir) * 0.5 + 0.5;
        luma = luma.clamp(0.0, 1.0);

        let p0 = project_to_micro(v0, vp, micro.w, micro.h);
        let p1 = project_to_micro(v1, vp, micro.w, micro.h);
        let p2 = project_to_micro(v2, vp, micro.w, micro.h);

        // Drop trivially offscreen triangles
        if p0.is_none() || p1.is_none() || p2.is_none() {
            continue;
        }
        let p0 = p0.unwrap();
        let p1 = p1.unwrap();
        let p2 = p2.unwrap();

        rasterize_triangle_micro(micro, p0, p1, p2, luma);
    }
}

fn project_to_micro(v: Vec3, vp: Mat4, w: usize, h: usize) -> Option<(Vec2, f32)> {
    let clip = vp * Vec4::new(v.x, v.y, v.z, 1.0);
    if clip.w <= 1e-6 {
        return None;
    }
    let ndc = clip.xyz() / clip.w; // -1..1
    // Basic clip
    if ndc.x < -1.3 || ndc.x > 1.3 || ndc.y < -1.3 || ndc.y > 1.3 {
        // allow some slack, but reject far offscreen
    }
    let x = (ndc.x * 0.5 + 0.5) * (w.saturating_sub(1) as f32);
    let y = (1.0 - (ndc.y * 0.5 + 0.5)) * (h.saturating_sub(1) as f32);
    Some((Vec2::new(x, y), ndc.z))
}

fn rasterize_triangle_micro(
    micro: &mut MicroBuffer,
    p0: (Vec2, f32),
    p1: (Vec2, f32),
    p2: (Vec2, f32),
    luma: f32,
) {
    let (a, az) = p0;
    let (b, bz) = p1;
    let (c, cz) = p2;

    let min_x = a.x.min(b.x).min(c.x).floor().max(0.0) as i32;
    let max_x = a.x.max(b.x).max(c.x).ceil().min((micro.w.saturating_sub(1)) as f32) as i32;
    let min_y = a.y.min(b.y).min(c.y).floor().max(0.0) as i32;
    let max_y = a.y.max(b.y).max(c.y).ceil().min((micro.h.saturating_sub(1)) as f32) as i32;

    let area = edge(b, c, a);
    if area.abs() < 1e-6 {
        return;
    }
    let accept = |w0: f32, w1: f32, w2: f32| -> bool {
        if area > 0.0 {
            w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
        } else {
            w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0
        }
    };

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let w0 = edge(b, c, p) / area;
            let w1 = edge(c, a, p) / area;
            let w2 = edge(a, b, p) / area;

            if accept(w0, w1, w2) {
                let z = w0 * az + w1 * bz + w2 * cz;
                let idx = micro.idx(x as usize, y as usize);
                if z < micro.px[idx].z {
                    micro.px[idx].z = z;
                    micro.px[idx].luma = luma;
                }
            }
        }
    }
}

#[inline]
fn edge(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x)
}

fn pack_micro_into_cells(micro: &MicroBuffer, out: &mut CellBuffer) {
    let cols = out.cols as usize;
    let rows = out.rows as usize;

    for row in 0..rows {
        for col in 0..cols {
            let base_x = col * 2;
            let base_y = row * 4;

            let mut mask: u8 = 0;
            let mut lsum = 0.0f32;
            let mut lcnt = 0u32;

            for y in 0..4usize {
                for x in 0..2usize {
                    let mx = base_x + x;
                    let my = base_y + y;
                    if mx >= micro.w || my >= micro.h {
                        continue;
                    }
                    let p = micro.px[micro.idx(mx, my)];
                    if p.z.is_finite() {
                        mask |= 1 << braille_bit(x, y);
                    }
                    if p.z.is_finite() {
                        lsum += p.luma;
                        lcnt += 1;
                    }
                }
            }

            let ch = char::from_u32(0x2800 + mask as u32).unwrap_or('\u{2800}');
            let luma = if lcnt > 0 { (lsum / lcnt as f32).clamp(0.0, 1.0) } else { 0.0 };

            // Screensaver-ish grayscale, keep readable
            let fg = (80.0 + 175.0 * luma) as u8;

            let cell_idx = out.idx(col as u16, row as u16);
            out.cells[cell_idx] = Cell { ch, fg };
        }
    }
}

#[inline]
fn braille_bit(x: usize, y: usize) -> u8 {
    // (x,y) within 2x4 block
    // y=0: x=0 dot1 bit0, x=1 dot4 bit3
    // y=1: x=0 dot2 bit1, x=1 dot5 bit4
    // y=2: x=0 dot3 bit2, x=1 dot6 bit5
    // y=3: x=0 dot7 bit6, x=1 dot8 bit7
    const T: [[u8; 2]; 4] = [[0, 3], [1, 4], [2, 5], [6, 7]];
    T[y][x]
}

fn diff_flush<W: Write>(out: &mut W, curr: &CellBuffer, prev: &mut CellBuffer) -> Result<()> {
    queue!(out, BeginSynchronizedUpdate)?;
    for row in 0..curr.rows {
        for col in 0..curr.cols {
            let i = curr.idx(col, row);
            let c = curr.cells[i];
            if c != prev.cells[i] {
                queue!(
                    out,
                    cursor::MoveTo(col, row),
                    SetForegroundColor(Color::Rgb {
                        r: c.fg,
                        g: c.fg,
                        b: c.fg
                    }),
                )?;
                write!(out, "{}", c.ch)?;
                prev.cells[i] = c;
            }
        }
    }
    queue!(out, ResetColor, EndSynchronizedUpdate)?;
    out.flush()?;
    Ok(())
}

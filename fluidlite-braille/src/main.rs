use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

const PANEL_W: usize = 26;
const FPS_CAP: u64 = 30;

// Braille subpixel layout: terminal cell = 2x4 subcells.
const BRAILLE_W: usize = 2;
const BRAILLE_H: usize = 4;

#[inline]
fn idx(w: usize, x: usize, y: usize) -> usize {
    y * w + x
}

fn boundary_copy_inward(w: usize, h: usize, a: &mut [f32]) {
    if w < 2 || h < 2 {
        return;
    }
    for x in 0..w {
        a[idx(w, x, 0)] = a[idx(w, x, 1)];
        a[idx(w, x, h - 1)] = a[idx(w, x, h - 2)];
    }
    for y in 0..h {
        a[idx(w, 0, y)] = a[idx(w, 1, y)];
        a[idx(w, w - 1, y)] = a[idx(w, w - 2, y)];
    }
}

fn advect_scalar(
    w: usize,
    h: usize,
    u: &[f32],
    v: &[f32],
    dst: &mut [f32],
    src: &[f32],
    dt: f32,
) {
    let wf = w as f32;
    let hf = h as f32;

    for y in 0..h {
        for x in 0..w {
            let i = idx(w, x, y);

            let mut px = x as f32 - dt * u[i];
            let mut py = y as f32 - dt * v[i];

            px = px.clamp(0.5, wf - 1.5);
            py = py.clamp(0.5, hf - 1.5);

            let x0 = px.floor() as usize;
            let y0 = py.floor() as usize;
            let x1 = x0 + 1;
            let y1 = y0 + 1;

            let sx = px - x0 as f32;
            let sy = py - y0 as f32;

            let i00 = idx(w, x0, y0);
            let i10 = idx(w, x1, y0);
            let i01 = idx(w, x0, y1);
            let i11 = idx(w, x1, y1);

            let v0 = src[i00] * (1.0 - sx) + src[i10] * sx;
            let v1 = src[i01] * (1.0 - sx) + src[i11] * sx;
            dst[i] = v0 * (1.0 - sy) + v1 * sy;
        }
    }
}

fn advect_velocity(
    w: usize,
    h: usize,
    u_dst: &mut [f32],
    v_dst: &mut [f32],
    u_src: &[f32],
    v_src: &[f32],
    dt: f32,
) {
    let wf = w as f32;
    let hf = h as f32;

    for y in 0..h {
        for x in 0..w {
            let i = idx(w, x, y);

            let mut px = x as f32 - dt * u_src[i];
            let mut py = y as f32 - dt * v_src[i];

            px = px.clamp(0.5, wf - 1.5);
            py = py.clamp(0.5, hf - 1.5);

            let x0 = px.floor() as usize;
            let y0 = py.floor() as usize;
            let x1 = x0 + 1;
            let y1 = y0 + 1;

            let sx = px - x0 as f32;
            let sy = py - y0 as f32;

            let i00 = idx(w, x0, y0);
            let i10 = idx(w, x1, y0);
            let i01 = idx(w, x0, y1);
            let i11 = idx(w, x1, y1);

            let u0 = u_src[i00] * (1.0 - sx) + u_src[i10] * sx;
            let u1 = u_src[i01] * (1.0 - sx) + u_src[i11] * sx;
            let v0 = v_src[i00] * (1.0 - sx) + v_src[i10] * sx;
            let v1 = v_src[i01] * (1.0 - sx) + v_src[i11] * sx;

            u_dst[i] = u0 * (1.0 - sy) + u1 * sy;
            v_dst[i] = v0 * (1.0 - sy) + v1 * sy;
        }
    }
}

fn diffuse(w: usize, h: usize, x: &mut [f32], x0: &[f32], diff: f32, dt: f32, iters: usize) {
    let a = diff * dt * (w as f32) * (h as f32) * 0.25;
    let inv_c = 1.0 / (1.0 + 4.0 * a);

    x.copy_from_slice(x0);

    for _ in 0..iters {
        for y in 1..(h - 1) {
            for x_ in 1..(w - 1) {
                let i = idx(w, x_, y);
                let l = idx(w, x_ - 1, y);
                let r = idx(w, x_ + 1, y);
                let d = idx(w, x_, y - 1);
                let u = idx(w, x_, y + 1);

                x[i] = (x0[i] + a * (x[l] + x[r] + x[d] + x[u])) * inv_c;
            }
        }
        boundary_copy_inward(w, h, x);
    }
}

fn project(w: usize, h: usize, u: &mut [f32], v: &mut [f32], p: &mut [f32], div: &mut [f32], iters: usize) {
    for y in 1..(h - 1) {
        for x in 1..(w - 1) {
            let i = idx(w, x, y);
            let l = idx(w, x - 1, y);
            let r = idx(w, x + 1, y);
            let d = idx(w, x, y - 1);
            let uu = idx(w, x, y + 1);

            div[i] = -0.5 * (u[r] - u[l] + v[uu] - v[d]);
            p[i] = 0.0;
        }
    }
    boundary_copy_inward(w, h, div);
    boundary_copy_inward(w, h, p);

    for _ in 0..iters {
        for y in 1..(h - 1) {
            for x in 1..(w - 1) {
                let i = idx(w, x, y);
                let l = idx(w, x - 1, y);
                let r = idx(w, x + 1, y);
                let d = idx(w, x, y - 1);
                let uu = idx(w, x, y + 1);

                p[i] = (div[i] + p[l] + p[r] + p[d] + p[uu]) * 0.25;
            }
        }
        boundary_copy_inward(w, h, p);
    }

    for y in 1..(h - 1) {
        for x in 1..(w - 1) {
            let i = idx(w, x, y);
            let l = idx(w, x - 1, y);
            let r = idx(w, x + 1, y);
            let d = idx(w, x, y - 1);
            let uu = idx(w, x, y + 1);

            u[i] -= 0.5 * (p[r] - p[l]);
            v[i] -= 0.5 * (p[uu] - p[d]);
        }
    }
    boundary_copy_inward(w, h, u);
    boundary_copy_inward(w, h, v);
}

fn vorticity_confinement(w: usize, h: usize, u: &mut [f32], v: &mut [f32], eps: f32) {
    let mut curl = vec![0.0f32; w * h];

    for y in 1..(h - 1) {
        for x in 1..(w - 1) {
            let i = idx(w, x, y);
            let l = idx(w, x - 1, y);
            let r = idx(w, x + 1, y);
            let d = idx(w, x, y - 1);
            let uu = idx(w, x, y + 1);

            let dvdx = (v[r] - v[l]) * 0.5;
            let dudy = (u[uu] - u[d]) * 0.5;
            curl[i] = dvdx - dudy;
        }
    }

    for y in 2..(h - 2) {
        for x in 2..(w - 2) {
            let i = idx(w, x, y);
            let l = idx(w, x - 1, y);
            let r = idx(w, x + 1, y);
            let d = idx(w, x, y - 1);
            let uu = idx(w, x, y + 1);

            let c_l = curl[l].abs();
            let c_r = curl[r].abs();
            let c_d = curl[d].abs();
            let c_u = curl[uu].abs();

            let nx = (c_r - c_l) * 0.5;
            let ny = (c_u - c_d) * 0.5;

            let len = (nx * nx + ny * ny).sqrt() + 1e-6;
            let nxn = nx / len;
            let nyn = ny / len;

            let c = curl[i];

            u[i] += eps * (nyn * c);
            v[i] += eps * (-nxn * c);
        }
    }
}

// -------------------------
// Themes
// -------------------------
#[derive(Clone, Copy)]
struct Theme {
    name: &'static str,
    bg: Color,
    panel_bg: Color,
    panel_fg: Color,
    emitter: Color,
    i0: Color,
    i1: Color,
    i2: Color,
    i3: Color,
    i4: Color,
}

const THEMES: &[Theme] = &[
    Theme {
        name: "Amber",
        bg: Color::Rgb { r: 16, g: 12, b: 0 },
        panel_bg: Color::Rgb { r: 20, g: 16, b: 0 },
        panel_fg: Color::Rgb { r: 255, g: 226, b: 140 },
        emitter: Color::Rgb { r: 255, g: 240, b: 180 },
        i0: Color::Rgb { r: 80, g: 68, b: 0 },
        i1: Color::Rgb { r: 130, g: 110, b: 0 },
        i2: Color::Rgb { r: 180, g: 150, b: 10 },
        i3: Color::Rgb { r: 220, g: 185, b: 40 },
        i4: Color::Rgb { r: 255, g: 220, b: 90 },
    },
    Theme {
        name: "Ice",
        bg: Color::Rgb { r: 0, g: 10, b: 16 },
        panel_bg: Color::Rgb { r: 0, g: 14, b: 22 },
        panel_fg: Color::Rgb { r: 190, g: 230, b: 255 },
        emitter: Color::Rgb { r: 220, g: 250, b: 255 },
        i0: Color::Rgb { r: 20, g: 80, b: 110 },
        i1: Color::Rgb { r: 40, g: 120, b: 160 },
        i2: Color::Rgb { r: 80, g: 170, b: 210 },
        i3: Color::Rgb { r: 140, g: 220, b: 240 },
        i4: Color::Rgb { r: 210, g: 250, b: 255 },
    },
    Theme {
        name: "Mono",
        bg: Color::Black,
        panel_bg: Color::Black,
        panel_fg: Color::White,
        emitter: Color::White,
        i0: Color::DarkGrey,
        i1: Color::Grey,
        i2: Color::White,
        i3: Color::White,
        i4: Color::White,
    },
];

#[derive(Clone)]
struct Field {
    w: usize,
    h: usize,
    u: Vec<f32>,
    v: Vec<f32>,
    u0: Vec<f32>,
    v0: Vec<f32>,
    d: Vec<f32>,
    d0: Vec<f32>,
    p: Vec<f32>,
    div: Vec<f32>,
}

impl Field {
    fn new(w: usize, h: usize) -> Self {
        let n = w * h;
        Self {
            w,
            h,
            u: vec![0.0; n],
            v: vec![0.0; n],
            u0: vec![0.0; n],
            v0: vec![0.0; n],
            d: vec![0.0; n],
            d0: vec![0.0; n],
            p: vec![0.0; n],
            div: vec![0.0; n],
        }
    }

    fn reset(&mut self) {
        for a in [
            &mut self.u,
            &mut self.v,
            &mut self.u0,
            &mut self.v0,
            &mut self.d,
            &mut self.d0,
            &mut self.p,
            &mut self.div,
        ] {
            a.fill(0.0);
        }
    }

    fn add_splat(&mut self, cx: f32, cy: f32, radius: f32, dye: f32, fx: f32, fy: f32) {
        let r2 = radius * radius;
        let xmin = ((cx - radius) as i32).clamp(0, (self.w as i32) - 1) as usize;
        let xmax = ((cx + radius) as i32).clamp(0, (self.w as i32) - 1) as usize;
        let ymin = ((cy - radius) as i32).clamp(0, (self.h as i32) - 1) as usize;
        let ymax = ((cy + radius) as i32).clamp(0, (self.h as i32) - 1) as usize;

        for y in ymin..=ymax {
            for x in xmin..=xmax {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let dist2 = dx * dx + dy * dy;
                if dist2 <= r2 {
                    let t = 1.0 - (dist2 / r2);
                    let k = t * t;
                    let i = idx(self.w, x, y);
                    self.d[i] += dye * k;
                    self.u[i] += fx * k;
                    self.v[i] += fy * k;
                }
            }
        }
    }

    fn step(
        &mut self,
        dt: f32,
        viscosity: f32,
        dye_diff: f32,
        proj_iters: usize,
        diff_iters: usize,
        vorticity: bool,
        vort_eps: f32,
    ) {
        let w = self.w;
        let h = self.h;

        self.u0.copy_from_slice(&self.u);
        self.v0.copy_from_slice(&self.v);
        self.d0.copy_from_slice(&self.d);

        diffuse(w, h, &mut self.u, &self.u0, viscosity, dt, diff_iters);
        diffuse(w, h, &mut self.v, &self.v0, viscosity, dt, diff_iters);

        project(w, h, &mut self.u, &mut self.v, &mut self.p, &mut self.div, proj_iters);

        if vorticity {
            vorticity_confinement(w, h, &mut self.u, &mut self.v, vort_eps);
        }

        self.u0.copy_from_slice(&self.u);
        self.v0.copy_from_slice(&self.v);
        advect_velocity(w, h, &mut self.u, &mut self.v, &self.u0, &self.v0, dt);

        project(w, h, &mut self.u, &mut self.v, &mut self.p, &mut self.div, proj_iters);

        self.d0.copy_from_slice(&self.d);
        diffuse(w, h, &mut self.d, &self.d0, dye_diff, dt, diff_iters);

        self.d0.copy_from_slice(&self.d);
        advect_scalar(w, h, &self.u, &self.v, &mut self.d, &self.d0, dt);

        for vv in &mut self.d {
            *vv *= 0.997;
            if *vv < 0.001 {
                *vv = 0.0;
            }
        }
    }
}

// Convert a 2x4 block to a braille character.
// Braille dot numbering:
// 1 4
// 2 5
// 3 6
// 7 8
// Bits: dot1=0, dot2=1, dot3=2, dot4=3, dot5=4, dot6=5, dot7=6, dot8=7
fn braille_char(mask: u8) -> char {
    char::from_u32(0x2800 + mask as u32).unwrap_or(' ')
}

fn braille_bit(dx: usize, dy: usize) -> u8 {
    // dx: 0..1, dy:0..3
    // Map to braille dot bit.
    match (dx, dy) {
        (0, 0) => 1 << 0, // dot 1
        (0, 1) => 1 << 1, // dot 2
        (0, 2) => 1 << 2, // dot 3
        (0, 3) => 1 << 6, // dot 7
        (1, 0) => 1 << 3, // dot 4
        (1, 1) => 1 << 4, // dot 5
        (1, 2) => 1 << 5, // dot 6
        (1, 3) => 1 << 7, // dot 8
        _ => 0,
    }
}

struct App {
    field: Field,
    paused: bool,
    show_debug: bool,
    vorticity: bool,
    continuous_emit: bool,

    emitter_x: f32, // in hi-res coordinates
    emitter_y: f32,
    emit_dye: f32,
    emit_force: f32,
    radius: f32,

    dt: f32,
    viscosity: f32,
    dye_diff: f32,
    proj_iters: usize,
    diff_iters: usize,
    vort_eps: f32,

    rng: StdRng,

    frames: u64,
    last_fps: Instant,
    fps: f32,

    theme_idx: usize,

    // terminal-cell dimensions (sim area, excluding panel)
    cell_w: usize,
    cell_h: usize,
}

impl App {
    fn new(cell_w: usize, cell_h: usize) -> Self {
        let hi_w = cell_w * BRAILLE_W;
        let hi_h = cell_h * BRAILLE_H;

        let mut app = Self {
            field: Field::new(hi_w, hi_h),
            paused: false,
            show_debug: true,
            vorticity: true,
            continuous_emit: true,

            emitter_x: (hi_w as f32) * 0.5,
            emitter_y: (hi_h as f32) * 0.5,
            emit_dye: 10.0,
            emit_force: 14.0,
            radius: 10.0,

            dt: 0.12,
            viscosity: 0.0007,
            dye_diff: 0.0002,
            proj_iters: 25,
            diff_iters: 10,
            vort_eps: 0.9,

            rng: StdRng::seed_from_u64(0xC0FFEE),
            frames: 0,
            last_fps: Instant::now(),
            fps: 0.0,

            theme_idx: 0,

            cell_w,
            cell_h,
        };

        app.seed();
        app
    }

    fn theme(&self) -> Theme {
        THEMES[self.theme_idx % THEMES.len()]
    }

    fn next_theme(&mut self) {
        self.theme_idx = (self.theme_idx + 1) % THEMES.len();
    }

    fn seed(&mut self) {
        let w = self.field.w as f32;
        let h = self.field.h as f32;
        let cx = w * 0.5;
        let cy = h * 0.5;
        let r = w.min(h) * 0.18;

        for k in 0..12 {
            let a = (k as f32) / 12.0 * std::f32::consts::TAU;
            let x = cx + a.cos() * r;
            let y = cy + a.sin() * r;

            let fx = -a.sin() * 18.0;
            let fy = a.cos() * 18.0;

            self.field.add_splat(x, y, 12.0, 22.0, fx, fy);
        }
    }

    fn resize_cells(term_w: u16, term_h: u16) -> (usize, usize) {
        let w = term_w as usize;
        let h = term_h as usize;

        // Leave room for panel + a spacer column
        let cell_w = w.saturating_sub(PANEL_W + 1).max(12);
        let cell_h = h.max(8);
        (cell_w, cell_h)
    }

    fn clamp_emitter(&mut self) {
        self.emitter_x = self.emitter_x.clamp(2.0, (self.field.w as f32) - 3.0);
        self.emitter_y = self.emitter_y.clamp(2.0, (self.field.h as f32) - 3.0);
    }

    fn inject(&mut self) {
        let angle = self.rng.gen_range(0.0..std::f32::consts::TAU);
        let fx = angle.cos() * self.emit_force;
        let fy = angle.sin() * self.emit_force;
        self.field
            .add_splat(self.emitter_x, self.emitter_y, self.radius, self.emit_dye, fx, fy);
    }

    fn tick(&mut self) {
        if self.paused {
            return;
        }
        if self.continuous_emit {
            self.inject();
        }
        self.field.step(
            self.dt,
            self.viscosity,
            self.dye_diff,
            self.proj_iters,
            self.diff_iters,
            self.vorticity,
            self.vort_eps,
        );
    }

    fn update_fps(&mut self) {
        self.frames += 1;
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_fps);
        if elapsed >= Duration::from_millis(500) {
            self.fps = (self.frames as f32) / elapsed.as_secs_f32();
            self.frames = 0;
            self.last_fps = now;
        }
    }

    fn intensity_color(theme: Theme, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        if t < 0.15 {
            theme.i0
        } else if t < 0.35 {
            theme.i1
        } else if t < 0.55 {
            theme.i2
        } else if t < 0.75 {
            theme.i3
        } else {
            theme.i4
        }
    }

    fn panel_lines(&self, max_d: f32, theme_name: &str) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.cell_h);

        lines.push(Self::pad_panel("+------------------------+"));
        lines.push(Self::pad_panel("|      Fluid Lite        |"));
        lines.push(Self::pad_panel("+------------------------+"));
        lines.push(Self::pad_panel("| q quit                 |"));
        lines.push(Self::pad_panel("| p pause                |"));
        lines.push(Self::pad_panel("| arrows move emitter    |"));
        lines.push(Self::pad_panel("| space splat            |"));
        lines.push(Self::pad_panel("| f toggle emit          |"));
        lines.push(Self::pad_panel("| c toggle vorticity     |"));
        lines.push(Self::pad_panel("| r reset                |"));
        lines.push(Self::pad_panel("| x theme                |"));
        lines.push(Self::pad_panel("+------------------------+"));

        if self.show_debug {
            lines.push(Self::pad_panel(&format!("| FPS: {:>7.2}           |", self.fps)));
            lines.push(Self::pad_panel(&format!("| maxD:{:>7.2}           |", max_d)));
            lines.push(Self::pad_panel(&format!("| dt: {:>7.3}            |", self.dt)));
            lines.push(Self::pad_panel(&format!("| dye: {:>7.2}           |", self.emit_dye)));
            lines.push(Self::pad_panel(&format!("| frc: {:>7.2}           |", self.emit_force)));
            lines.push(Self::pad_panel(&format!("| rad: {:>7.2}           |", self.radius)));
            lines.push(Self::pad_panel(&format!("| thm: {:<14}          |", theme_name)));
        } else {
            lines.push(Self::pad_panel("| d show debug           |"));
        }

        while lines.len() + 1 < self.cell_h {
            lines.push(Self::pad_panel("|                        |"));
        }
        lines.push(Self::pad_panel("+------------------------+"));
        lines
    }

    fn pad_panel(s: &str) -> String {
        let mut t = s.to_string();
        let chars = t.chars().count();
        if chars < PANEL_W {
            t.push_str(&" ".repeat(PANEL_W - chars));
        } else if chars > PANEL_W {
            // safe truncation by chars
            t = t.chars().take(PANEL_W).collect();
        }
        t
    }

    fn handle_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('p') => self.paused = !self.paused,
            KeyCode::Char('d') => self.show_debug = !self.show_debug,
            KeyCode::Char('c') => self.vorticity = !self.vorticity,
            KeyCode::Char('f') => self.continuous_emit = !self.continuous_emit,
            KeyCode::Char('x') => self.next_theme(),
            KeyCode::Char('r') => {
                self.field.reset();
                self.seed();
            }
            KeyCode::Char(' ') => self.inject(),

            // move emitter in hi-res coordinates: 2x4 per keypress gives nice speed
            KeyCode::Up => {
                self.emitter_y -= 4.0;
                self.clamp_emitter();
            }
            KeyCode::Down => {
                self.emitter_y += 4.0;
                self.clamp_emitter();
            }
            KeyCode::Left => {
                self.emitter_x -= 2.0;
                self.clamp_emitter();
            }
            KeyCode::Right => {
                self.emitter_x += 2.0;
                self.clamp_emitter();
            }
            _ => {}
        }
        false
    }

    fn render(&self, stdout: &mut Stdout) -> io::Result<()> {
        let theme = self.theme();

        let mut max_d = 0.0f32;
        for &v in &self.field.d {
            if v.is_finite() && v > max_d {
                max_d = v;
            }
        }
        let exposure = if max_d > 0.0 { 1.0 / max_d } else { 1.0 };

        let panel = self.panel_lines(max_d, theme.name);

        let ex = self.emitter_x.round() as i32;
        let ey = self.emitter_y.round() as i32;

        queue!(stdout, cursor::MoveTo(0, 0))?;

        for cy in 0..self.cell_h {
            // ---- sim region (braille) ----
            for cx in 0..self.cell_w {
                let base_x = cx * BRAILLE_W;
                let base_y = cy * BRAILLE_H;

                let mut mask: u8 = 0;
                let mut avg = 0.0f32;
                let mut count = 0.0f32;

                for dy in 0..BRAILLE_H {
                    for dx in 0..BRAILLE_W {
                        let sx = base_x + dx;
                        let sy = base_y + dy;
                        let i = idx(self.field.w, sx, sy);
                        let mut v = self.field.d[i];
                        if !v.is_finite() {
                            v = 0.0;
                        }
                        v *= exposure;
                        v = (v * 1.4).clamp(0.0, 1.0);

                        avg += v;
                        count += 1.0;

                        // threshold to light a braille dot
                        if v >= 0.10 {
                            mask |= braille_bit(dx, dy);
                        }
                    }
                }

                let avg = if count > 0.0 { avg / count } else { 0.0 };

                // emitter overlay: if emitter is inside this cell, force a visible glyph
                let in_cell = ex >= base_x as i32
                    && ex < (base_x + BRAILLE_W) as i32
                    && ey >= base_y as i32
                    && ey < (base_y + BRAILLE_H) as i32;

                if in_cell {
                    queue!(
                        stdout,
                        SetBackgroundColor(theme.bg),
                        SetForegroundColor(theme.emitter),
                        Print("⣿")
                    )?;
                    continue;
                }

                if mask == 0 {
                    // overwrite to blank for true clearing
                    queue!(stdout, SetBackgroundColor(theme.bg), Print(" "))?;
                } else {
                    let fg = Self::intensity_color(theme, avg);
                    let ch = braille_char(mask);
                    queue!(
                        stdout,
                        SetBackgroundColor(theme.bg),
                        SetForegroundColor(fg),
                        Print(ch)
                    )?;
                }
            }

            // ---- panel region ----
            let line = if cy < panel.len() { &panel[cy] } else { "" };
            queue!(
                stdout,
                SetBackgroundColor(theme.panel_bg),
                SetForegroundColor(theme.panel_fg),
                Print(line)
            )?;

            if cy + 1 < self.cell_h {
                queue!(stdout, ResetColor, Print("\r\n"))?;
            }
        }

        queue!(stdout, ResetColor)?;
        Ok(())
    }
}

fn main() -> io::Result<()> {
    let mut stdout = io::stdout();

    terminal::enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    execute!(stdout, Clear(ClearType::All), cursor::MoveTo(0, 0))?;

    let (tw, th) = terminal::size()?;
    let (cell_w, cell_h) = App::resize_cells(tw, th);
    let mut app = App::new(cell_w, cell_h);

    let target_frame = Duration::from_millis(1000 / FPS_CAP);

    'outer: loop {
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if app.handle_key(k.code) {
                        break 'outer;
                    }
                }
                Event::Resize(nw, nh) => {
                    let (cw, ch) = App::resize_cells(nw, nh);
                    app = App::new(cw, ch);
                    execute!(stdout, Clear(ClearType::All), cursor::MoveTo(0, 0))?;
                }
                _ => {}
            }
        }

        app.tick();
        app.update_fps();

        app.render(&mut stdout)?;
        stdout.flush()?;

        std::thread::sleep(target_frame);
    }

    execute!(stdout, ResetColor, cursor::Show, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    Ok(())
}

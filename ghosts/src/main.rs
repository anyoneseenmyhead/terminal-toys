use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::cmp::Ordering;
use std::env;
use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

const FPS_CAP: u64 = 60;
const DT_CLAMP: f32 = 0.05;
const MIN_GHOSTS: usize = 6;
const MAX_GHOSTS: usize = 80;

// Ghost separation tuning knobs.
const REPULSE_MIN_DIST_BASE: f32 = 0.50;
const REPULSE_MIN_DIST_DEPTH_SCALE: f32 = 0.10;
const REPULSE_STRENGTH: f32 = 0.70;
const REPULSE_DY_WEIGHT: f32 = 1.25;
const REPULSE_DZ_WEIGHT: f32 = 0.85;
const REPULSE_VX_GAIN: f32 = 1.80;
const REPULSE_VY_GAIN: f32 = 0.90;
const REPULSE_VZ_GAIN: f32 = 0.90;

const SUB_X: usize = 2;
const SUB_Y: usize = 4;
const LAYER_BG: u8 = 0;
const LAYER_GHOST_FAR: u8 = 1;
const LAYER_GHOST_NEAR: u8 = 2;
const LAYER_EYE: u8 = 3;
const LAYER_MOON: u8 = 4;
const LAYER_FOREGROUND: u8 = 5;
const LAYER_GROUND: u8 = 6;
const LAYER_GLOW: u8 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cell {
    ch: char,
    fg: Color,
    bg: Color,
}

impl Cell {
    fn blank(bg: Color) -> Self {
        Self {
            ch: ' ',
            fg: Color::Reset,
            bg,
        }
    }
}

struct Renderer {
    w: u16,
    h: u16,
    front: Vec<Cell>,
    back: Vec<Cell>,
    full_redraw: bool,
    last_fg: Color,
    last_bg: Color,
}

impl Renderer {
    fn new(w: u16, h: u16, bg: Color) -> Self {
        let n = (w as usize) * (h as usize);
        Self {
            w,
            h,
            front: vec![Cell::blank(bg); n],
            back: vec![Cell::blank(bg); n],
            full_redraw: true,
            last_fg: Color::Reset,
            last_bg: bg,
        }
    }

    fn resize(&mut self, w: u16, h: u16, bg: Color) {
        self.w = w;
        self.h = h;
        let n = (w as usize) * (h as usize);
        self.front = vec![Cell::blank(bg); n];
        self.back = vec![Cell::blank(bg); n];
        self.full_redraw = true;
        self.last_fg = Color::Reset;
        self.last_bg = bg;
    }

    #[inline]
    fn idx(&self, x: u16, y: u16) -> usize {
        (y as usize) * (self.w as usize) + (x as usize)
    }

    fn clear_back(&mut self, bg: Color) {
        for c in &mut self.back {
            *c = Cell::blank(bg);
        }
    }

    fn put(&mut self, x: i32, y: i32, cell: Cell) {
        if x < 0 || y < 0 {
            return;
        }
        let xu = x as u16;
        let yu = y as u16;
        if xu >= self.w || yu >= self.h {
            return;
        }
        let i = self.idx(xu, yu);
        self.back[i] = cell;
    }

    fn flush(&mut self, out: &mut Stdout) -> io::Result<()> {
        queue!(out, BeginSynchronizedUpdate)?;

        let w = self.w as usize;
        let h = self.h as usize;

        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let b = self.back[i];
                let f = self.front[i];

                if !self.full_redraw && b == f {
                    continue;
                }

                if b.bg != self.last_bg {
                    queue!(out, SetBackgroundColor(b.bg))?;
                    self.last_bg = b.bg;
                }
                if b.fg != self.last_fg {
                    queue!(out, SetForegroundColor(b.fg))?;
                    self.last_fg = b.fg;
                }
                queue!(out, cursor::MoveTo(x as u16, y as u16), Print(b.ch))?;
                self.front[i] = b;
            }
        }

        self.full_redraw = false;
        queue!(out, EndSynchronizedUpdate)?;
        out.flush()
    }
}

#[derive(Clone, Copy)]
struct Theme {
    bg: Color,
    fog_deep: Color,
    fog_low: Color,
    fog_mid: Color,
    fog_hi: Color,
    fog_glow: Color,
    ghost_near_shadow: Color,
    ghost_near_mid: Color,
    ghost_near_hi: Color,
    ghost_far_shadow: Color,
    ghost_far_mid: Color,
    ghost_far_hi: Color,
    glow: Color,
    eye: Color,
    hud: Color,
    moon: Color,
    foreground_dark: Color,
    foreground_bright: Color,
    ground_dark: Color,
    ground_mid: Color,
    ground_hi: Color,
}

fn theme() -> Theme {
    Theme {
        bg: Color::AnsiValue(16),
        fog_deep: Color::AnsiValue(17),
        fog_low: Color::AnsiValue(18),
        fog_mid: Color::AnsiValue(19),
        fog_hi: Color::AnsiValue(25),
        fog_glow: Color::AnsiValue(31),
        ghost_near_shadow: Color::AnsiValue(252),
        ghost_near_mid: Color::AnsiValue(255),
        ghost_near_hi: Color::AnsiValue(231),
        ghost_far_shadow: Color::AnsiValue(246),
        ghost_far_mid: Color::AnsiValue(250),
        ghost_far_hi: Color::AnsiValue(254),
        glow: Color::AnsiValue(117),
        eye: Color::AnsiValue(15),
        hud: Color::AnsiValue(153),
        moon: Color::AnsiValue(223),
        foreground_dark: Color::AnsiValue(65),
        foreground_bright: Color::AnsiValue(108),
        ground_dark: Color::AnsiValue(59),
        ground_mid: Color::AnsiValue(66),
        ground_hi: Color::AnsiValue(72),
    }
}

#[derive(Clone)]
struct Ghost {
    x: f32,
    y: f32,
    z: f32,
    vx: f32,
    vy: f32,
    vz: f32,
    bob_phase: f32,
    bob_rate: f32,
    mood: f32,
    gaze_phase: f32,
    gaze_rate: f32,
    eye_sep: f32,
    eye_scale: f32,
}

struct World {
    rng: StdRng,
    ghosts: Vec<Ghost>,
    t: f32,
    paused: bool,
    show_hud: bool,
    screensaver: bool,
    lightning: f32,
    lightning_timer: f32,
    lightning_cooldown: f32,
    ghost_size: f32,
}

impl World {
    fn new(seed: u64, screensaver: bool) -> Self {
        let mut world = Self {
            rng: StdRng::seed_from_u64(seed),
            ghosts: Vec::new(),
            t: 0.0,
            paused: false,
            show_hud: !screensaver,
            screensaver,
            lightning: 0.0,
            lightning_timer: 0.0,
            lightning_cooldown: 3.0,
            ghost_size: 1.5,
        };

        for _ in 0..12 {
            let ghost = world.spawn_ghost();
            world.ghosts.push(ghost);
        }
        world
    }

    fn spawn_ghost(&mut self) -> Ghost {
        Ghost {
            x: self.rng.gen_range(-1.3..1.3),
            y: self.rng.gen_range(-0.35..0.75),
            z: self.rng.gen_range(0.2..1.35),
            vx: self.rng.gen_range(-0.30..0.30),
            vy: self.rng.gen_range(-0.15..0.15),
            vz: self.rng.gen_range(-0.14..0.14),
            bob_phase: self.rng.gen_range(0.0..std::f32::consts::TAU),
            bob_rate: self.rng.gen_range(0.6..1.8),
            mood: self.rng.gen_range(0.0..1.0),
            gaze_phase: self.rng.gen_range(0.0..std::f32::consts::TAU),
            gaze_rate: self.rng.gen_range(0.8..2.4),
            eye_sep: self.rng.gen_range(0.24..0.39),
            eye_scale: self.rng.gen_range(0.85..1.25),
        }
    }

    fn update(&mut self, dt: f32) {
        if self.paused {
            return;
        }
        self.t += dt;
        self.lightning_cooldown -= dt;

        // Pairwise separation force to reduce clumping/overlap.
        let mut repel = vec![(0.0f32, 0.0f32, 0.0f32); self.ghosts.len()];
        for i in 0..self.ghosts.len() {
            for j in (i + 1)..self.ghosts.len() {
                let gi = &self.ghosts[i];
                let gj = &self.ghosts[j];

                let dx = gi.x - gj.x;
                let dy = (gi.y - gj.y) * REPULSE_DY_WEIGHT;
                let dz = (gi.z - gj.z) * REPULSE_DZ_WEIGHT;
                let d2 = dx * dx + dy * dy + dz * dz;

                let min_d = REPULSE_MIN_DIST_BASE
                    + (1.0 - gi.z.min(gj.z)).max(0.0) * REPULSE_MIN_DIST_DEPTH_SCALE;
                let min_d2 = min_d * min_d;
                if d2 >= min_d2 {
                    continue;
                }

                let d = d2.sqrt().max(1e-4);
                let nx = dx / d;
                let ny = dy / d;
                let nz = dz / d;
                let strength = ((min_d - d) / min_d).clamp(0.0, 1.0) * REPULSE_STRENGTH;

                repel[i].0 += nx * strength;
                repel[i].1 += ny * strength;
                repel[i].2 += nz * strength;
                repel[j].0 -= nx * strength;
                repel[j].1 -= ny * strength;
                repel[j].2 -= nz * strength;
            }
        }

        for i in 0..self.ghosts.len() {
            let g = &mut self.ghosts[i];
            g.bob_phase += dt * g.bob_rate;
            g.gaze_phase += dt * g.gaze_rate;

            let drift = (self.t * (0.55 + g.mood * 0.85) + i as f32 * 0.71).sin() * 0.12;
            g.vx += drift * dt * 0.9;
            g.vy += (self.t * 0.9 + i as f32).cos() * dt * 0.04;
            g.vz += (self.t * 0.6 + i as f32 * 1.7).sin() * dt * 0.05;
            g.vx += repel[i].0 * dt * REPULSE_VX_GAIN;
            g.vy += repel[i].1 * dt * REPULSE_VY_GAIN;
            g.vz += repel[i].2 * dt * REPULSE_VZ_GAIN;

            g.vx = g.vx.clamp(-0.45, 0.45) * 0.995;
            g.vy = g.vy.clamp(-0.22, 0.22) * 0.996;
            g.vz = g.vz.clamp(-0.22, 0.22) * 0.994;

            g.x += g.vx * dt;
            g.y += g.vy * dt;
            g.z += g.vz * dt;

            if self.lightning > 0.1 {
                g.vx += (0.0 - g.x) * dt * 0.22;
                g.vy += (-0.08 - g.y) * dt * 0.18;
            }

            if g.x < -1.8 {
                g.x = -1.8;
                g.vx = g.vx.abs();
            } else if g.x > 1.8 {
                g.x = 1.8;
                g.vx = -g.vx.abs();
            }

            if g.y < -0.55 {
                g.y = -0.55;
                g.vy = g.vy.abs();
            } else if g.y > 0.95 {
                g.y = 0.95;
                g.vy = -g.vy.abs();
            }

            if g.z < 0.05 {
                g.z = 0.05;
                g.vz = g.vz.abs();
            } else if g.z > 1.8 {
                g.z = 1.8;
                g.vz = -g.vz.abs();
            }
        }

        if self.lightning_timer > 0.0 {
            self.lightning_timer -= dt;
            let t = self.lightning_timer;
            self.lightning = if t > 0.16 {
                1.0
            } else if t > 0.11 {
                0.18
            } else if t > 0.07 {
                0.85
            } else if t > 0.0 {
                (t / 0.07).clamp(0.0, 1.0) * 0.35
            } else {
                0.0
            };
        } else {
            self.lightning = 0.0;
            if self.lightning_cooldown <= 0.0 && self.rng.gen_range(0.0..1.0) < dt * 0.11 {
                self.lightning_timer = 0.24;
                self.lightning_cooldown = self.rng.gen_range(7.0..22.0);
            }
        }
    }

    fn add_ghost(&mut self) {
        if self.ghosts.len() < MAX_GHOSTS {
            let ghost = self.spawn_ghost();
            self.ghosts.push(ghost);
        }
    }

    fn remove_ghost(&mut self) {
        if self.ghosts.len() > MIN_GHOSTS {
            self.ghosts.pop();
        }
    }

    fn shuffle(&mut self) {
        let mut new_ghosts = Vec::with_capacity(self.ghosts.len());
        for _ in 0..self.ghosts.len() {
            new_ghosts.push(self.spawn_ghost());
        }
        self.ghosts = new_ghosts;
    }
}

struct BrailleCanvas {
    tw: usize,
    th: usize,
    sw: usize,
    sh: usize,
    val: Vec<u8>,
    layer: Vec<u8>,
}

impl BrailleCanvas {
    fn new(term_w: usize, term_h: usize) -> Self {
        let sw = term_w * SUB_X;
        let sh = term_h * SUB_Y;
        Self {
            tw: term_w,
            th: term_h,
            sw,
            sh,
            val: vec![0u8; sw * sh],
            layer: vec![LAYER_BG; sw * sh],
        }
    }

    fn resize(&mut self, term_w: usize, term_h: usize) {
        self.tw = term_w;
        self.th = term_h;
        self.sw = term_w * SUB_X;
        self.sh = term_h * SUB_Y;
        self.val = vec![0u8; self.sw * self.sh];
        self.layer = vec![LAYER_BG; self.sw * self.sh];
    }

    fn clear(&mut self) {
        self.val.fill(0);
        self.layer.fill(LAYER_BG);
    }

    #[inline]
    fn sidx(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 {
            return None;
        }
        let (x, y) = (x as usize, y as usize);
        if x >= self.sw || y >= self.sh {
            return None;
        }
        Some(y * self.sw + x)
    }

    fn set_if_stronger(&mut self, x: i32, y: i32, v: u8, layer: u8) {
        if let Some(i) = self.sidx(x, y) {
            if self.layer[i] == LAYER_GROUND
                && matches!(layer, LAYER_GHOST_FAR | LAYER_GHOST_NEAR | LAYER_EYE | LAYER_GLOW)
            {
                let mixed = (self.val[i] as f32 * 0.45 + v as f32 * 0.55).round() as u8;
                if mixed >= self.val[i] {
                    self.val[i] = mixed;
                    self.layer[i] = layer;
                }
                return;
            }
            if v > self.val[i] || (v == self.val[i] && layer > self.layer[i]) {
                self.val[i] = v;
                self.layer[i] = layer;
            }
        }
    }

    fn add_bg(&mut self, x: i32, y: i32, v: u8) {
        if let Some(i) = self.sidx(x, y) {
            let sum = self.val[i] as u16 + v as u16;
            self.val[i] = sum.min(255) as u8;
            self.layer[i] = LAYER_BG;
        }
    }

    fn draw_ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, intensity: u8, layer: u8) {
        if rx < 0.5 || ry < 0.5 {
            return;
        }
        let minx = (cx - rx - 1.0).floor() as i32;
        let maxx = (cx + rx + 1.0).ceil() as i32;
        let miny = (cy - ry - 1.0).floor() as i32;
        let maxy = (cy + ry + 1.0).ceil() as i32;

        for y in miny..=maxy {
            for x in minx..=maxx {
                let dx = (x as f32 - cx) / rx.max(1e-3);
                let dy = (y as f32 - cy) / ry.max(1e-3);
                let d = (dx * dx + dy * dy).sqrt();
                if d <= 1.05 {
                    let edge = (1.0 - d).clamp(0.0, 1.0);
                    let v = (intensity as f32 * (0.45 + 0.55 * edge)) as u8;
                    self.set_if_stronger(x, y, v, layer);
                }
            }
        }
    }

    fn draw_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, intensity: u8, layer: u8) {
        let xa = x0.min(x1);
        let xb = x0.max(x1);
        let ya = y0.min(y1);
        let yb = y0.max(y1);
        for y in ya..=yb {
            for x in xa..=xb {
                self.set_if_stronger(x, y, intensity, layer);
            }
        }
    }

    fn paint_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, intensity: u8, layer: u8) {
        let xa = x0.min(x1);
        let xb = x0.max(x1);
        let ya = y0.min(y1);
        let yb = y0.max(y1);
        for y in ya..=yb {
            for x in xa..=xb {
                if let Some(i) = self.sidx(x, y) {
                    self.val[i] = intensity;
                    self.layer[i] = layer;
                }
            }
        }
    }

    fn draw_line(&mut self, ax: f32, ay: f32, bx: f32, by: f32, thickness: f32, intensity: u8, layer: u8) {
        let dx = bx - ax;
        let dy = by - ay;
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let steps = (len * 1.2).clamp(6.0, 320.0) as i32;
        let r = thickness.max(0.6);
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let x = ax + dx * t;
            let y = ay + dy * t;
            self.draw_ellipse(x, y, r, r, intensity, layer);
        }
    }

    fn draw_ghost_body(
        &mut self,
        cx_cell: f32,
        cy_cell: f32,
        scale_cell: f32,
        phase: f32,
        intensity: u8,
        layer: u8,
    ) {
        let cx = cx_cell * SUB_X as f32;
        let cy = cy_cell * SUB_Y as f32;

        let rx = scale_cell * 1.15 * SUB_X as f32;
        let ry = scale_cell * 1.55 * SUB_Y as f32;

        let minx = (cx - rx - 2.0).floor() as i32;
        let maxx = (cx + rx + 2.0).ceil() as i32;
        let miny = (cy - ry - 2.0).floor() as i32;
        let maxy = (cy + ry + 2.0).ceil() as i32;

        for sy in miny..=maxy {
            for sx in minx..=maxx {
                let px = sx as f32;
                let py = sy as f32;
                let lx = (px - cx) / rx.max(1e-3);
                let ly = (py - cy) / ry.max(1e-3);

                let dome = ((lx * 1.03).powi(2) + ((ly + 0.58) * 1.18).powi(2)).sqrt();
                let inside_dome = dome <= 1.0 && ly <= 0.10;

                let side_curve = 1.0 - (lx.abs().powf(1.6) * 1.06);
                let hem_wave = 0.74 + 0.13 * (lx * 12.0 + phase).sin();
                let inside_sheet = ly >= -0.02 && ly <= hem_wave && ly <= side_curve;

                if inside_dome || inside_sheet {
                    let edge_h = (1.0 - lx.abs()).clamp(0.0, 1.0);
                    let edge_v = if inside_dome {
                        (1.0 - dome).clamp(0.0, 1.0)
                    } else {
                        (hem_wave - ly).clamp(0.0, 1.0)
                    };
                    let v = (intensity as f32 * (0.4 + 0.6 * edge_h.min(edge_v))) as u8;
                    self.set_if_stronger(sx, sy, v, layer);
                }
            }
        }
    }

    fn sample_avg(&self, tx: usize, ty: usize) -> u8 {
        let sx0 = tx * SUB_X;
        let sy0 = ty * SUB_Y;
        let mut sum = 0u32;
        for oy in 0..SUB_Y {
            for ox in 0..SUB_X {
                sum += self.val[(sy0 + oy) * self.sw + (sx0 + ox)] as u32;
            }
        }
        (sum / (SUB_X * SUB_Y) as u32) as u8
    }

    fn dominant_layer(&self, tx: usize, ty: usize, threshold: u8) -> u8 {
        let sx0 = tx * SUB_X;
        let sy0 = ty * SUB_Y;
        let mut weights = [0u16; 8];

        for oy in 0..SUB_Y {
            for ox in 0..SUB_X {
                let i = (sy0 + oy) * self.sw + (sx0 + ox);
                let v = self.val[i];
                if v >= threshold {
                    let layer = self.layer[i] as usize;
                    if layer < weights.len() {
                        weights[layer] = weights[layer].saturating_add(v as u16);
                    }
                }
            }
        }

        let mut best = 0usize;
        let mut best_w = 0u16;
        for (i, w) in weights.iter().enumerate() {
            if *w >= best_w {
                best = i;
                best_w = *w;
            }
        }
        best as u8
    }

    fn to_braille_cell(&self, tx: usize, ty: usize, threshold: u8) -> u8 {
        let sx0 = tx * SUB_X;
        let sy0 = ty * SUB_Y;
        let mut mask = 0u8;

        for oy in 0..SUB_Y {
            for ox in 0..SUB_X {
                let v = self.val[(sy0 + oy) * self.sw + (sx0 + ox)];
                if v >= threshold {
                    let bit = match (ox, oy) {
                        (0, 0) => 0x01,
                        (0, 1) => 0x02,
                        (0, 2) => 0x04,
                        (0, 3) => 0x40,
                        (1, 0) => 0x08,
                        (1, 1) => 0x10,
                        (1, 2) => 0x20,
                        (1, 3) => 0x80,
                        _ => 0,
                    };
                    mask |= bit;
                }
            }
        }
        mask
    }
}

fn parse_args() -> bool {
    let mut screensaver = false;
    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--screensaver" => screensaver = true,
            "--help" | "-h" => {
                println!(
                    "ghosts\n\n\
                     Usage:\n\
                     \tghosts [--screensaver]\n\n\
                     Controls:\n\
                     \tQ / Esc   quit\n\
                     \tP         pause/resume\n\
                     \tH         toggle HUD (disabled in screensaver)\n\
                     \t?         toggle help line\n\
                     \t+ / -     add/remove ghosts\n\
                     \tR         reshuffle ghosts\n\
                     \t[ / ]     smaller/larger ghosts\n\
                     \t0         reset ghost size\n"
                );
                std::process::exit(0);
            }
            _ => {}
        }
    }
    screensaver
}

struct CleanupGuard;

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = queue!(
            out,
            EndSynchronizedUpdate,
            cursor::Show,
            ResetColor,
            EnableLineWrap,
            LeaveAlternateScreen
        );
        let _ = out.flush();
        let _ = terminal::disable_raw_mode();
    }
}

fn main() -> io::Result<()> {
    let screensaver = parse_args();
    let mut out = io::stdout();

    terminal::enable_raw_mode()?;
    queue!(out, EnterAlternateScreen, DisableLineWrap, cursor::Hide)?;
    out.flush()?;

    let _guard = CleanupGuard;

    let th = theme();
    let mut last_size = terminal::size()?;
    let mut renderer = Renderer::new(last_size.0, last_size.1, th.bg);
    let mut canvas = BrailleCanvas::new(last_size.0 as usize, last_size.1 as usize);

    let seed = (Instant::now().elapsed().as_nanos() as u64) ^ 0x6C4A_98D1_5510_AE3F;
    let mut world = World::new(seed, screensaver);

    let mut last = Instant::now();
    let mut fps_acc = 0.0f32;
    let mut fps_frames = 0u32;
    let mut fps_est = 0.0f32;
    let mut show_help_line = !screensaver;

    loop {
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Resize(w, h) => {
                    last_size = (w, h);
                    renderer.resize(w, h, th.bg);
                    canvas.resize(w as usize, h as usize);
                }
                Event::Key(KeyEvent {
                    code,
                    kind,
                    ..
                }) => {
                    if kind != KeyEventKind::Press {
                        continue;
                    }
                    match code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(()),
                        KeyCode::Char('p') | KeyCode::Char('P') => world.paused = !world.paused,
                        KeyCode::Char('h') | KeyCode::Char('H') => {
                            if !world.screensaver {
                                world.show_hud = !world.show_hud;
                            }
                        }
                        KeyCode::Char('?') => {
                            if !world.screensaver {
                                show_help_line = !show_help_line;
                            }
                        }
                        KeyCode::Char('+') | KeyCode::Char('=') => world.add_ghost(),
                        KeyCode::Char('-') => world.remove_ghost(),
                        KeyCode::Char('r') | KeyCode::Char('R') => world.shuffle(),
                        KeyCode::Char(']') => world.ghost_size = (world.ghost_size + 0.05).clamp(0.5, 2.4),
                        KeyCode::Char('[') => world.ghost_size = (world.ghost_size - 0.05).clamp(0.5, 2.4),
                        KeyCode::Char('0') => world.ghost_size = 1.0,
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        let now = Instant::now();
        let mut dt = (now - last).as_secs_f32();
        last = now;
        if dt > DT_CLAMP {
            dt = DT_CLAMP;
        }

        fps_acc += dt;
        fps_frames += 1;
        if fps_acc >= 0.5 {
            fps_est = fps_frames as f32 / fps_acc;
            fps_acc = 0.0;
            fps_frames = 0;
        }

        world.update(dt);

        render_frame(&mut renderer, &mut canvas, &world, th, fps_est, show_help_line)?;

        let target = Duration::from_millis(1000 / FPS_CAP.max(1));
        let elapsed = now.elapsed();
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }

        let size_now = terminal::size()?;
        if size_now != last_size {
            last_size = size_now;
            renderer.resize(last_size.0, last_size.1, th.bg);
            canvas.resize(last_size.0 as usize, last_size.1 as usize);
        }
    }
}

fn render_frame(
    renderer: &mut Renderer,
    canvas: &mut BrailleCanvas,
    world: &World,
    th: Theme,
    fps_est: f32,
    show_help_line: bool,
) -> io::Result<()> {
    renderer.clear_back(th.bg);
    canvas.clear();

    draw_background(canvas, world.t, world.lightning);
    let ground_y = ground_level_sub(canvas);
    draw_ground(canvas, world.lightning, ground_y);

    let mut order: Vec<(usize, f32)> = world
        .ghosts
        .iter()
        .enumerate()
        .map(|(i, g)| (i, g.z))
        .collect();
    order.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));

    for (idx, _) in order {
        draw_ghost(
            canvas,
            &world.ghosts[idx],
            world.t,
            idx,
            world.lightning,
            world.ghost_size,
        );
    }
    draw_foreground(canvas, world.t, world.lightning, ground_y);

    let w = renderer.w as usize;
    let h = renderer.h as usize;

    for ty in 0..h {
        for tx in 0..w {
            let avg = canvas.sample_avg(tx, ty);
            let threshold = if avg > 130 {
                68
            } else if avg > 90 {
                56
            } else {
                48
            };

            let mask = canvas.to_braille_cell(tx, ty, threshold);
            let layer = canvas.dominant_layer(tx, ty, threshold);

            let fg = match layer {
                LAYER_MOON => th.moon,
                LAYER_EYE => th.eye,
                LAYER_GLOW => th.glow,
                LAYER_GHOST_NEAR => {
                    if avg > 190 {
                        th.ghost_near_hi
                    } else if avg > 132 {
                        th.ghost_near_mid
                    } else {
                        th.ghost_near_shadow
                    }
                }
                LAYER_GHOST_FAR => {
                    if avg > 168 {
                        th.ghost_far_hi
                    } else if avg > 118 {
                        th.ghost_far_mid
                    } else {
                        th.ghost_far_shadow
                    }
                }
                LAYER_FOREGROUND => {
                    if avg > 118 {
                        th.foreground_bright
                    } else {
                        th.foreground_dark
                    }
                }
                LAYER_GROUND => {
                    if avg > 122 {
                        th.ground_hi
                    } else if avg > 82 {
                        th.ground_mid
                    } else {
                        th.ground_dark
                    }
                }
                _ => {
                    if avg < 34 {
                        th.fog_deep
                    } else if avg < 62 {
                        th.fog_low
                    } else if avg < 108 {
                        th.fog_mid
                    } else if avg < 152 {
                        th.fog_hi
                    } else {
                        th.fog_glow
                    }
                }
            };

            let ch = if mask == 0 {
                ' '
            } else {
                char::from_u32(0x2800 + mask as u32).unwrap_or(' ')
            };

            let fg = if world.lightning > 0.01 {
                if matches!(layer, LAYER_GHOST_NEAR | LAYER_GHOST_FAR | LAYER_EYE) {
                    th.moon
                } else {
                    fg
                }
            } else {
                fg
            };

            renderer.put(
                tx as i32,
                ty as i32,
                Cell {
                    ch,
                    fg,
                    bg: th.bg,
                },
            );
        }
    }

    if world.show_hud && !world.screensaver {
        draw_hud(renderer, world, fps_est, th, show_help_line);
    }

    let mut out = io::stdout();
    renderer.flush(&mut out)
}

fn draw_background(canvas: &mut BrailleCanvas, t: f32, lightning: f32) {
    let sw = canvas.sw as i32;
    let sh = canvas.sh as i32;

    for y in 0..sh {
        let yn = y as f32 / (sh.max(1) as f32);
        for x in 0..sw {
            let xn = x as f32 / (sw.max(1) as f32);
            let fog = 20.0
                + 16.0 * (xn * 8.0 + t * 0.6).sin()
                + 12.0 * (yn * 12.0 - t * 0.35).cos()
                + 10.0 * ((xn + yn) * 18.0 + t * 0.4).sin();
            let mut v = fog.max(0.0) as u8;
            v = v.saturating_add((lightning * 85.0) as u8);

            if yn > 0.65 {
                let ground_haze = ((yn - 0.65) * 90.0) as u8;
                v = v.saturating_add(ground_haze);
            }
            canvas.add_bg(x, y, v.min(95));
        }
    }

    let mx = canvas.sw as f32 * 0.82;
    let my = canvas.sh as f32 * 0.14;
    let moon_intensity = (180.0 + lightning * 50.0).min(255.0) as u8;
    canvas.draw_ellipse(mx, my, 6.0, 6.8, moon_intensity, LAYER_MOON);
    canvas.draw_ellipse(mx + 2.0, my - 0.6, 5.6, 6.2, 10, LAYER_BG);
}

fn ground_level_sub(canvas: &BrailleCanvas) -> f32 {
    canvas.sh as f32 * 0.84
}

fn draw_ground(canvas: &mut BrailleCanvas, lightning: f32, ground_y: f32) {
    let sw = canvas.sw as f32;
    let sh = canvas.sh as f32;
    let gy = ground_y as i32;
    for y in gy..(sh as i32) {
        let depth = (y - gy) as f32 / (sh - ground_y).max(1.0);
        let undulate = ((y as f32 * 0.11).sin() * 8.0 + (y as f32 * 0.037).cos() * 6.0).max(0.0);
        let base = 56.0 + depth * 58.0 + undulate + lightning * 36.0;
        canvas.paint_rect(0, y, sw as i32 - 1, y, base.min(255.0) as u8, LAYER_GROUND);
    }
}

fn draw_foreground(canvas: &mut BrailleCanvas, t: f32, lightning: f32, ground_y: f32) {
    let sw = canvas.sw as f32;
    let sh = canvas.sh as f32;

    let fence_step = 14.0;
    let drift = (t * 0.45).sin() * 8.0;
    let rail_intensity = (85.0 + lightning * 70.0) as u8;
    let post_intensity = (110.0 + lightning * 70.0) as u8;

    let mut x = -fence_step + drift;
    while x < sw + fence_step {
        let post_h = 7.0 + ((x * 0.05 + t * 0.8).sin() * 1.8).abs();
        canvas.draw_line(x, ground_y - post_h, x, ground_y + 2.0, 0.9, post_intensity, LAYER_FOREGROUND);
        x += fence_step;
    }
    canvas.draw_line(-6.0 + drift, ground_y - 2.0, sw + 6.0 + drift, ground_y - 2.0, 0.8, rail_intensity, LAYER_FOREGROUND);
    canvas.draw_line(-6.0 + drift, ground_y - 5.0, sw + 6.0 + drift, ground_y - 5.0, 0.8, rail_intensity, LAYER_FOREGROUND);

    let stone_intensity = (95.0 + lightning * 80.0) as u8;
    for i in 0..5 {
        let nx = 0.10 + i as f32 * 0.19 + (t * (0.03 + i as f32 * 0.01)).sin() * 0.01;
        let cx = nx * sw;
        let top = ground_y - (8.0 + (i % 3) as f32 * 2.0);
        canvas.draw_rect((cx - 2.0) as i32, top as i32, (cx + 2.0) as i32, ground_y as i32, stone_intensity, LAYER_FOREGROUND);
        canvas.draw_ellipse(cx, top, 2.1, 1.6, stone_intensity.saturating_add(12), LAYER_FOREGROUND);
    }

    let branch_intensity = (72.0 + lightning * 90.0) as u8;
    let bx = sw * 0.18 + (t * 0.32).sin() * 5.0;
    let by = sh * 0.32;
    canvas.draw_line(bx, by, bx - 10.0, by + 8.0, 0.8, branch_intensity, LAYER_FOREGROUND);
    canvas.draw_line(bx - 5.0, by + 4.0, bx - 13.0, by + 2.0, 0.7, branch_intensity, LAYER_FOREGROUND);
    canvas.draw_line(bx - 7.0, by + 6.0, bx - 14.0, by + 10.0, 0.7, branch_intensity, LAYER_FOREGROUND);
}

fn draw_ghost(
    canvas: &mut BrailleCanvas,
    g: &Ghost,
    t: f32,
    idx: usize,
    lightning: f32,
    size_mul: f32,
) {
    let w = canvas.tw as f32;
    let h = canvas.th as f32;

    let z = g.z + 0.45;
    let fov = (w.min(h) * 0.70).max(20.0);

    let bob = (g.bob_phase + t * g.bob_rate).sin() * 0.07;
    let sway = (t * (0.7 + g.mood) + idx as f32).sin() * 0.06;

    let sx = (w * 0.5) + ((g.x + sway) / z) * fov;
    let sy = (h * 0.62) + ((g.y + bob) / z) * fov * 0.54;
    let scale = ((2.9 / z) * size_mul).clamp(0.45, 6.0);

    let shadow_rx = (scale * 0.95) * SUB_X as f32;
    let shadow_ry = (scale * 0.24) * SUB_Y as f32;
    let shadow_x = sx * SUB_X as f32;
    let shadow_y = (h * 0.88 - (1.0 / z) * 2.0) * SUB_Y as f32;
    canvas.draw_ellipse(shadow_x, shadow_y, shadow_rx, shadow_ry, 45, LAYER_BG);

    let glow_r = scale * (1.1 + g.mood * 0.5);
    let glow_intensity = (42.0 + (1.0 - g.z.min(1.0)) * 30.0 + lightning * 48.0).min(120.0) as u8;
    canvas.draw_ellipse(
        sx * SUB_X as f32,
        (sy + scale * 0.05) * SUB_Y as f32,
        glow_r * SUB_X as f32,
        glow_r * 1.12 * SUB_Y as f32,
        glow_intensity,
        LAYER_GLOW,
    );

    let layer = if g.z < 0.62 {
        LAYER_GHOST_NEAR
    } else {
        LAYER_GHOST_FAR
    };
    let base_intensity = if layer == LAYER_GHOST_NEAR { 238.0 } else { 198.0 };
    let intensity = (base_intensity + lightning * 45.0).min(255.0) as u8;
    canvas.draw_ghost_body(sx, sy, scale, t * 6.0 + idx as f32 * 0.9, intensity, layer);

    let gaze_x = (g.gaze_phase + t * 0.5 + g.mood * 3.2).sin() * scale * 0.08;
    let gaze_y = (g.gaze_phase * 0.73 + t * 0.35).cos() * scale * 0.05;
    let eye_y = sy - scale * 0.42 + gaze_y;
    let eye_dx = scale * g.eye_sep;

    let blink = ((t * (1.8 + g.gaze_rate * 1.2) + idx as f32 * 1.37).sin() + 1.0) * 0.5;
    let eye_ry = if blink < 0.08 {
        0.08 * SUB_Y as f32
    } else {
        (0.16 + 0.08 * blink) * SUB_Y as f32
    } * g.eye_scale;
    let eye_rx = (0.09 + 0.04 * scale.min(2.0)) * SUB_X as f32 * g.eye_scale;
    let eye_intensity = (245.0 + lightning * 10.0).min(255.0) as u8;

    let eye_sub_y = eye_y * SUB_Y as f32;
    canvas.draw_ellipse(
        (sx - eye_dx + gaze_x) * SUB_X as f32,
        eye_sub_y,
        eye_rx,
        eye_ry,
        eye_intensity,
        LAYER_EYE,
    );
    canvas.draw_ellipse(
        (sx + eye_dx + gaze_x) * SUB_X as f32,
        eye_sub_y,
        eye_rx,
        eye_ry,
        eye_intensity,
        LAYER_EYE,
    );
}

fn draw_hud(renderer: &mut Renderer, world: &World, fps_est: f32, th: Theme, show_help_line: bool) {
    let w = renderer.w as usize;
    if renderer.h == 0 {
        return;
    }

    let line1 = format!(
        "  Ghosts   | count: {}  | size: {:.2}x  | mode: {}  | flash: {}  | {:.0} fps  ",
        world.ghosts.len(),
        world.ghost_size,
        if world.paused { "paused" } else { "drifting" },
        if world.lightning > 0.01 { "yes" } else { "no" },
        fps_est
    );
    for (i, ch) in line1.chars().take(w).enumerate() {
        renderer.put(
            i as i32,
            0,
            Cell {
                ch,
                fg: th.hud,
                bg: th.bg,
            },
        );
    }

    if renderer.h > 1 && show_help_line {
        let line2 = "  keys: Q/Esc quit  P pause  H hud  ? hints  +/- ghost count  R reshuffle  [ / ] size  0 reset size  ";
        for (i, ch) in line2.chars().take(w).enumerate() {
            renderer.put(
                i as i32,
                1,
                Cell {
                    ch,
                    fg: th.hud,
                    bg: th.bg,
                },
            );
        }
    }
}

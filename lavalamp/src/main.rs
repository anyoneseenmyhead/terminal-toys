// src/main.rs
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use rand::{rngs::StdRng, Rng, SeedableRng};

#[derive(Clone, Copy, Debug)]
struct Vec2 {
    x: f32,
    y: f32,
}
impl Vec2 {
    fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    fn add(self, o: Vec2) -> Self {
        Self::new(self.x + o.x, self.y + o.y)
    }
    fn sub(self, o: Vec2) -> Self {
        Self::new(self.x - o.x, self.y - o.y)
    }
    fn mul(self, k: f32) -> Self {
        Self::new(self.x * k, self.y * k)
    }
    fn len2(self) -> f32 {
        self.x * self.x + self.y * self.y
    }
    fn len(self) -> f32 {
        self.len2().sqrt()
    }
    fn norm(self) -> Self {
        let l = self.len();
        if l <= 1e-6 {
            Self::new(0.0, 0.0)
        } else {
            self.mul(1.0 / l)
        }
    }
}

#[derive(Clone, Copy)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}
impl Rgb {
    fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let lerp1 = |x: u8, y: u8| -> u8 {
            (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8
        };
        Rgb {
            r: lerp1(a.r, b.r),
            g: lerp1(a.g, b.g),
            b: lerp1(a.b, b.b),
        }
    }
    fn scale(self, k: f32) -> Rgb {
        let k = k.max(0.0);
        let s = |v: u8| -> u8 { ((v as f32) * k).round().clamp(0.0, 255.0) as u8 };
        Rgb {
            r: s(self.r),
            g: s(self.g),
            b: s(self.b),
        }
    }
    fn to_color(self) -> Color {
        Color::Rgb {
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }
}

#[derive(Clone, Copy)]
struct Theme {
    bg_top: Rgb,
    bg_mid: Rgb,
    bg_bot: Rgb,
    wax_cool: Rgb,
    wax_hot: Rgb,
    glass_edge: Rgb,
    bg_global: Rgb,
    hud_fg: Rgb,
    hud_fg_dim: Rgb,
    hud_bg: Rgb,
}

const THEMES: [Theme; 4] = [
    Theme {
        bg_top: Rgb { r: 10, g: 12, b: 18 },
        bg_mid: Rgb { r: 12, g: 18, b: 30 },
        bg_bot: Rgb { r: 18, g: 26, b: 40 },
        wax_cool: Rgb { r: 170, g: 90, b: 230 },
        wax_hot: Rgb { r: 255, g: 115, b: 160 },
        glass_edge: Rgb { r: 70, g: 90, b: 130 },
        bg_global: Rgb { r: 6, g: 7, b: 11 },
        hud_fg: Rgb { r: 210, g: 220, b: 245 },
        hud_fg_dim: Rgb { r: 170, g: 185, b: 210 },
        hud_bg: Rgb { r: 0, g: 0, b: 0 },
    },
    Theme {
        bg_top: Rgb { r: 8, g: 16, b: 12 },
        bg_mid: Rgb { r: 10, g: 24, b: 16 },
        bg_bot: Rgb { r: 14, g: 34, b: 22 },
        wax_cool: Rgb { r: 90, g: 200, b: 170 },
        wax_hot: Rgb { r: 240, g: 200, b: 90 },
        glass_edge: Rgb { r: 80, g: 110, b: 95 },
        bg_global: Rgb { r: 5, g: 10, b: 7 },
        hud_fg: Rgb { r: 210, g: 230, b: 220 },
        hud_fg_dim: Rgb { r: 160, g: 190, b: 175 },
        hud_bg: Rgb { r: 0, g: 0, b: 0 },
    },
    Theme {
        bg_top: Rgb { r: 14, g: 10, b: 10 },
        bg_mid: Rgb { r: 22, g: 12, b: 16 },
        bg_bot: Rgb { r: 30, g: 14, b: 20 },
        wax_cool: Rgb { r: 220, g: 90, b: 90 },
        wax_hot: Rgb { r: 255, g: 190, b: 90 },
        glass_edge: Rgb { r: 120, g: 90, b: 90 },
        bg_global: Rgb { r: 8, g: 6, b: 6 },
        hud_fg: Rgb { r: 235, g: 215, b: 200 },
        hud_fg_dim: Rgb { r: 190, g: 170, b: 155 },
        hud_bg: Rgb { r: 0, g: 0, b: 0 },
    },
    Theme {
        bg_top: Rgb { r: 8, g: 8, b: 16 },
        bg_mid: Rgb { r: 12, g: 10, b: 26 },
        bg_bot: Rgb { r: 18, g: 14, b: 36 },
        wax_cool: Rgb { r: 120, g: 120, b: 255 },
        wax_hot: Rgb { r: 120, g: 210, b: 255 },
        glass_edge: Rgb { r: 90, g: 110, b: 150 },
        bg_global: Rgb { r: 5, g: 5, b: 12 },
        hud_fg: Rgb { r: 210, g: 225, b: 255 },
        hud_fg_dim: Rgb { r: 165, g: 185, b: 215 },
        hud_bg: Rgb { r: 0, g: 0, b: 0 },
    },
];

fn theme_for(idx: usize) -> Theme {
    THEMES[idx % THEMES.len()]
}

#[derive(Clone)]
struct Cell {
    ch: char,
    fg: Rgb,
    bg: Rgb,
}

struct Diff {
    w: u16,
    h: u16,
    prev: Vec<Cell>,
    next: Vec<Cell>,
}
impl Diff {
    fn new(w: u16, h: u16) -> Self {
        let blank = Cell {
            ch: ' ',
            fg: Rgb { r: 255, g: 255, b: 255 },
            bg: Rgb { r: 0, g: 0, b: 0 },
        };
        let n = w as usize * h as usize;
        Self {
            w,
            h,
            prev: vec![blank.clone(); n],
            next: vec![blank; n],
        }
    }
    fn resize(&mut self, w: u16, h: u16) {
        if self.w == w && self.h == h {
            return;
        }
        *self = Self::new(w, h);
    }
    fn idx(&self, x: u16, y: u16) -> usize {
        y as usize * self.w as usize + x as usize
    }
    fn clear_next(&mut self, bg: Rgb) {
        for c in &mut self.next {
            c.ch = ' ';
            c.fg = Rgb { r: 255, g: 255, b: 255 };
            c.bg = bg;
        }
    }
    fn set_next(&mut self, x: u16, y: u16, cell: Cell) {
        if x >= self.w || y >= self.h {
            return;
        }
        let i = self.idx(x, y);
        self.next[i] = cell;
    }
    fn flush<W: Write>(&mut self, out: &mut W) -> io::Result<()> {
        let mut last_fg: Option<Rgb> = None;
        let mut last_bg: Option<Rgb> = None;

        for y in 0..self.h {
            for x in 0..self.w {
                let i = self.idx(x, y);
                let a = &self.prev[i];
                let b = &self.next[i];
                if a.ch == b.ch && a.fg.r == b.fg.r && a.fg.g == b.fg.g && a.fg.b == b.fg.b
                    && a.bg.r == b.bg.r && a.bg.g == b.bg.g && a.bg.b == b.bg.b
                {
                    continue;
                }

                queue!(out, cursor::MoveTo(x, y))?;

                if last_bg.map(|c| (c.r, c.g, c.b)) != Some((b.bg.r, b.bg.g, b.bg.b)) {
                    queue!(out, SetBackgroundColor(b.bg.to_color()))?;
                    last_bg = Some(b.bg);
                }
                if last_fg.map(|c| (c.r, c.g, c.b)) != Some((b.fg.r, b.fg.g, b.fg.b)) {
                    queue!(out, SetForegroundColor(b.fg.to_color()))?;
                    last_fg = Some(b.fg);
                }

                queue!(out, Print(b.ch))?;
            }
        }

        std::mem::swap(&mut self.prev, &mut self.next);
        Ok(())
    }
}

// Braille mapping: 2x4 subpixels per terminal cell.
// Dots are numbered:
// 1 4
// 2 5
// 3 6
// 7 8
fn braille_from_bits(bits: u8) -> char {
    // bits: bit0=dot1, bit1=dot2, ... bit7=dot8
    let codepoint = 0x2800u32 + bits as u32;
    char::from_u32(codepoint).unwrap_or(' ')
}

#[derive(Clone)]
struct Blob {
    p: Vec2,
    v: Vec2,
    r: f32,
    heat_bias: f32,
    temp: f32,
    split_cd: f32,
}

struct Sim {
    rng: StdRng,
    blobs: Vec<Blob>,
    t: f32,
    max_blobs: usize,
    theme_idx: usize,

    heat: f32,      // 0..1
    viscosity: f32, // 0..1
    wobble: f32,    // 0..1
    threshold: f32, // iso-surface
    paused: bool,
    show_hud: bool,
}

impl Sim {
    fn new(seed: u64) -> Self {
        let mut s = Self {
            rng: StdRng::seed_from_u64(seed),
            blobs: vec![],
            t: 0.0,
            max_blobs: 16,
            theme_idx: 0,
            heat: 0.50,
            viscosity: 0.45,
            wobble: 0.55,
            threshold: 1.22,
            paused: false,
            show_hud: true,
        };
        s.reset_blobs(9);
        s
    }

    fn reset_blobs(&mut self, n: usize) {
        self.blobs.clear();
        for _ in 0..n {
            let r = self.rng.gen_range(0.055f32..0.12f32);
            let p = Vec2::new(self.rng.gen_range(0.25..0.75), self.rng.gen_range(0.15..0.85));
            let v = Vec2::new(self.rng.gen_range(-0.02..0.02), self.rng.gen_range(-0.02..0.02));
            let heat_bias = self.rng.gen_range(-0.12..0.12);
            self.blobs.push(Blob { p, v, r, heat_bias, temp: self.heat, split_cd: 0.0 });
        }
    }

    fn set_blob_count(&mut self, n: usize) {
        let n = n.clamp(3, 18);
        if n == self.blobs.len() {
            return;
        }
        if n < self.blobs.len() {
            self.blobs.truncate(n);
            return;
        }
        let add = n - self.blobs.len();
        for _ in 0..add {
            let r = self.rng.gen_range(0.05f32..0.10f32);
            let p = Vec2::new(self.rng.gen_range(0.25..0.75), self.rng.gen_range(0.65..0.92));
            let v = Vec2::new(self.rng.gen_range(-0.02..0.02), self.rng.gen_range(-0.01..0.01));
            let heat_bias = self.rng.gen_range(-0.10..0.10);
            self.blobs.push(Blob { p, v, r, heat_bias, temp: self.heat, split_cd: 0.0 });
        }
    }

    fn step(&mut self, dt: f32) {
        if self.paused {
            return;
        }
        self.t += dt;
        for b in &mut self.blobs {
            b.split_cd = (b.split_cd - dt).max(0.0);
        }

        // Heater near bottom: soften the gradient so heat isn't only at the very bottom.
        let base_buoy = 0.16 + 0.60 * self.heat;
        let cool_sink = 0.12 + 0.30 * (1.0 - self.heat);

        // Viscosity affects damping and how strongly blobs "stick" together.
        let visc = self.viscosity.clamp(0.0, 1.0);
        let damping = (0.35 + 3.0 * visc) * dt;

        // Slight global swirl
        let swirl = 0.06 * self.wobble * (0.6 + 0.4 * (self.t * 0.25).sin());

        // Pairwise interactions: gentle attraction at mid-range, repulsion when too close.
        let k_attr = 0.02 + 0.08 * visc;
        let k_rep = 0.14 + 0.35 * (1.0 - visc);

        let n = self.blobs.len();
        let mut dv = vec![Vec2::new(0.0, 0.0); n];

        for i in 0..n {
            for j in (i + 1)..n {
                let pi = self.blobs[i].p;
                let pj = self.blobs[j].p;
                let d = pj.sub(pi);
                let d2 = d.len2() + 1e-6;
                let dist = d2.sqrt();

                let target = (self.blobs[i].r + self.blobs[j].r) * 1.65;
                let dir = d.mul(1.0 / dist);

                let cd = self.blobs[i].split_cd.max(self.blobs[j].split_cd);
                if dist < target {
                    // repulsion
                    let push = (target - dist) / target;
                    let f = k_rep * push.powf(3.0);
                    dv[i] = dv[i].sub(dir.mul(f));
                    dv[j] = dv[j].add(dir.mul(f));
                } else if cd > 0.0 {
                    if dist < target * 2.6 {
                        let push = 1.0 - (dist / (target * 2.6));
                        let f = 0.35 * push * push;
                        dv[i] = dv[i].sub(dir.mul(f));
                        dv[j] = dv[j].add(dir.mul(f));
                    }
                } else if dist < target * 1.8 {
                    // weak attraction
                    let pull = (dist - target) / (target * 1.6);
                    let f = k_attr * (1.0 - pull).clamp(0.0, 1.0) * (0.6 + 0.4 * visc);
                    dv[i] = dv[i].add(dir.mul(f));
                    dv[j] = dv[j].sub(dir.mul(f));
                }
            }
        }

        for (i, b) in self.blobs.iter_mut().enumerate() {
            // Buoyancy depends on y (hotter at bottom) + per-blob bias.
            let hot01 = (1.0 - b.p.y).clamp(0.0, 1.0); // 1 at bottom, 0 at top
            let local_heat = (0.25 + 0.75 * hot01.powf(1.1)) * self.heat + 0.05;
            b.temp += (local_heat - b.temp) * (0.8 * dt);
            b.temp = b.temp.clamp(0.0, 1.0);

            let buoy = base_buoy
                * (0.25 + 0.75 * hot01.powf(1.1))
                * (0.6 + 0.9 * b.temp)
                * (1.0 + b.heat_bias);
            let sink = cool_sink * (0.35 + 0.65 * (1.0 - b.temp).powf(1.2));
            let cool01 = 1.0 - hot01;
            let pad = 0.06;

            let mut a = Vec2::new(0.0, 0.0);
            a.y -= buoy;
            a.y += sink;
            // Cooling near the top makes blobs heavier so they can fall back down.
            let fall = 0.12 * cool01 * (0.6 + 0.4 * (1.0 - b.temp));
            a.y += fall;
            // Gentle pull toward mid-height to avoid sticking at extremes.
            let center_pull = 0.08 * (0.6 + 0.4 * self.wobble);
            a.y += center_pull * (0.5 - b.p.y);
            // Anti-stiction kicks near the glass to keep blobs mobile.
            if b.p.y < pad + 0.02 {
                a.y += 0.35;
                if self.rng.gen::<f32>() < 0.6 * dt {
                    b.v.y += self.rng.gen_range(0.10..0.22);
                }
            } else if b.p.y > 1.0 - pad - 0.02 {
                a.y -= 0.35;
                if self.rng.gen::<f32>() < 0.6 * dt {
                    b.v.y -= self.rng.gen_range(0.10..0.22);
                }
            }

            // Swirl and wobble
            a.x += swirl * (b.p.y * 2.0 - 1.0);
            a.y += 0.05 * self.wobble * (self.t * 0.9 + (b.p.x * 6.0)).sin();
            // Vertical convection cell that flips over time to drive up/down cycles.
            let convection = 0.55 * (0.5 + 0.5 * self.wobble);
            a.y += convection * (0.5 - b.p.y) * (self.t * 0.55).sin();
            let shear = 0.12 * (0.3 + 0.7 * self.wobble) * (0.4 + 0.6 * self.heat);
            a.x += shear * (b.p.y * 6.0 + self.t * 0.8).sin();

            // Add pairwise forces
            a = a.add(dv[i]);

            // Integrate
            b.v = b.v.add(a.mul(dt));

            // Damping
            b.v = b.v.mul((1.0 - damping).clamp(0.0, 1.0));

            // Clamp max speed for stability
            let sp2 = b.v.len2();
            let max_sp = 0.55;
            if sp2 > max_sp * max_sp {
                b.v = b.v.mul(max_sp / sp2.sqrt());
            }

            b.p = b.p.add(b.v.mul(dt));

            // Soft boundary inside "glass"
            if b.p.x < pad {
                b.p.x = pad;
                b.v.x *= -0.55;
            }
            if b.p.x > 1.0 - pad {
                b.p.x = 1.0 - pad;
                b.v.x *= -0.55;
            }
            if b.p.y < pad {
                b.p.y = pad;
                b.v.y *= -0.55;
            }
            if b.p.y > 1.0 - pad {
                b.p.y = 1.0 - pad;
                b.v.y *= -0.55;
            }

            // Cooling shrink near the top encourages splits.
            let shrink = 1.0 - 0.24 * cool01 * (0.6 + 0.4 * (1.0 - b.temp));
            b.r = (b.r * shrink).clamp(0.045, 0.135);
        }

        // Very slow radius breathing to suggest merging/splitting without topological ops.
        for b in &mut self.blobs {
            let w = 0.8 + 0.2 * self.wobble;
            let s = (self.t * (0.22 + 0.18 * w) + b.p.x * 6.0).sin();
            let base = b.r;
            b.r = (base * (1.0 + 0.10 * w * s)).clamp(0.045, 0.135);
        }

        // Split big, fast, cooling blobs.
        let mut to_spawn: Vec<Blob> = Vec::new();
        for b in &mut self.blobs {
            let speed = b.v.len();
            let cooling_zone = b.p.y < 0.25;
            let big = b.r > 0.095;
            let split_chance = (0.015 + 0.03 * (1.0 - self.heat)) * dt;

            if big && (speed > 0.22 || cooling_zone) && self.rng.gen::<f32>() < split_chance {
                let dir = Vec2::new(
                    self.rng.gen_range(-1.0..1.0),
                    self.rng.gen_range(-1.0..1.0),
                )
                .norm();
                let child_r = (b.r * 0.72).clamp(0.045, 0.12);
                b.r = child_r;

                let offset = dir.mul(child_r * 2.4);
                b.p = b.p.sub(offset);
                b.v = b.v.sub(dir.mul(0.18));
                b.split_cd = 1.2;

                let child = Blob {
                    p: b.p.add(offset.mul(2.0)),
                    v: b.v.add(dir.mul(0.36)),
                    r: child_r,
                    heat_bias: b.heat_bias + self.rng.gen_range(-0.05..0.05),
                    temp: b.temp,
                    split_cd: 1.2,
                };
                to_spawn.push(child);
            }
        }

        if !to_spawn.is_empty() {
            self.blobs.extend(to_spawn);
            if self.blobs.len() > self.max_blobs {
                self.blobs.sort_by(|a, b| a.r.partial_cmp(&b.r).unwrap_or(std::cmp::Ordering::Equal));
                let keep_from = self.blobs.len().saturating_sub(self.max_blobs);
                self.blobs.drain(0..keep_from);
            }
        }
    }

    // Scalar field: sum of blob influences.
    fn field(&self, p: Vec2) -> f32 {
        let mut v = 0.0;
        for b in &self.blobs {
            let d = p.sub(b.p);
            let d2 = d.len2() + 1e-6;
            // metaball kernel
            v += (b.r * b.r) / d2;
        }
        v
    }
}

fn quantize_bg(y01: f32, theme: Theme) -> Rgb {
    // Liquid inside glass: deep navy -> bluish
    let top = theme.bg_top;
    let mid = theme.bg_mid;
    let bot = theme.bg_bot;

    let t = y01.clamp(0.0, 1.0);
    if t < 0.6 {
        Rgb::lerp(top, mid, t / 0.6)
    } else {
        Rgb::lerp(mid, bot, (t - 0.6) / 0.4)
    }
}

fn wax_color(heat: f32, y01: f32, inside: f32, theme: Theme) -> Rgb {
    // Wax palette shifts with heat: cooler = purple, hotter = orange-pink.
    let cool = theme.wax_cool;
    let hot = theme.wax_hot;
    let base = Rgb::lerp(cool, hot, heat);

    // Darken slightly toward top; brighten at bottom
    let shade = (0.92 + 0.18 * (1.0 - y01)).clamp(0.75, 1.15);

    // Inside factor (how strongly inside the iso-surface): boost
    base.scale(shade * (0.65 + 0.55 * inside.clamp(0.0, 1.0)))
}

fn main() -> io::Result<()> {
    let mut out = io::stdout();

    execute!(out, EnterAlternateScreen, DisableLineWrap, cursor::Hide)?;
    terminal::enable_raw_mode()?;

    let mut sim = Sim::new(7);
    let mut last = Instant::now();
    let mut acc = 0.0f32;
    let dt_fixed = 1.0 / 120.0;

    let mut last_fps = Instant::now();
    let mut fps_smoothed = 60.0f32;
    let mut frames = 0u32;

    let mut size = terminal::size()?;
    let mut diff = Diff::new(size.0, size.1);

    let mut quit = false;

    while !quit {
        // Input
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => quit = true,
                    KeyCode::Char(' ') => sim.paused = !sim.paused,
                    KeyCode::Char('h') => sim.show_hud = !sim.show_hud,
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        sim.theme_idx = sim.theme_idx.wrapping_add(1);
                    }
                    KeyCode::Char('r') => sim.reset_blobs(sim.blobs.len()),
                    KeyCode::Up => sim.heat = (sim.heat + 0.03).clamp(0.0, 1.0),
                    KeyCode::Down => sim.heat = (sim.heat - 0.03).clamp(0.0, 1.0),
                    KeyCode::Left => sim.viscosity = (sim.viscosity - 0.03).clamp(0.0, 1.0),
                    KeyCode::Right => sim.viscosity = (sim.viscosity + 0.03).clamp(0.0, 1.0),
                    KeyCode::Char('[') => {
                        let n = sim.blobs.len().saturating_sub(1);
                        sim.set_blob_count(n);
                    }
                    KeyCode::Char(']') => {
                        let n = sim.blobs.len() + 1;
                        sim.set_blob_count(n);
                    }
                    KeyCode::Char(',') => sim.wobble = (sim.wobble - 0.03).clamp(0.0, 1.0),
                    KeyCode::Char('.') => sim.wobble = (sim.wobble + 0.03).clamp(0.0, 1.0),
                    KeyCode::Char('-') => sim.threshold = (sim.threshold + 0.03).clamp(0.7, 2.0),
                    KeyCode::Char('=') | KeyCode::Char('+') => {
                        sim.threshold = (sim.threshold - 0.03).clamp(0.7, 2.0)
                    }
                    _ => {}
                },
                Event::Resize(w, h) => {
                    size = (w, h);
                    diff.resize(w, h);
                }
                _ => {}
            }
        }

        // Time
        let now = Instant::now();
        let frame_dt = (now - last).as_secs_f32().min(0.05);
        last = now;

        if !sim.paused {
            acc += frame_dt;
            let mut steps = 0;
            while acc >= dt_fixed && steps < 8 {
                sim.step(dt_fixed);
                acc -= dt_fixed;
                steps += 1;
            }
        }

        // FPS
        frames += 1;
        let fps_window = (now - last_fps).as_secs_f32();
        if fps_window >= 0.33 {
            let fps = frames as f32 / fps_window.max(1e-6);
            fps_smoothed = fps_smoothed * 0.85 + fps * 0.15;
            frames = 0;
            last_fps = now;
        }

        // Render
        let (w, h) = (size.0, size.1);
        let theme = theme_for(sim.theme_idx);
        let bg_global = theme.bg_global;
        diff.clear_next(bg_global);

        let glass_pad_x = 2u16;
        let glass_pad_y = 1u16;
        let inner_w = w.saturating_sub(glass_pad_x * 2);
        let inner_h = h.saturating_sub(glass_pad_y * 2);

        // Draw glass outline
        let edge = theme.glass_edge;
        for x in 0..w {
            diff.set_next(
                x,
                0,
                Cell {
                    ch: if x == 0 || x == w - 1 { '╭' } else { '─' },
                    fg: edge,
                    bg: bg_global,
                },
            );
            diff.set_next(
                x,
                h.saturating_sub(1),
                Cell {
                    ch: if x == 0 || x == w - 1 { '╰' } else { '─' },
                    fg: edge,
                    bg: bg_global,
                },
            );
        }
        for y in 0..h {
            diff.set_next(
                0,
                y,
                Cell {
                    ch: if y == 0 || y == h - 1 { '│' } else { '│' },
                    fg: edge,
                    bg: bg_global,
                },
            );
            diff.set_next(
                w.saturating_sub(1),
                y,
                Cell {
                    ch: if y == 0 || y == h - 1 { '│' } else { '│' },
                    fg: edge,
                    bg: bg_global,
                },
            );
        }
        // Fix corners
        if w >= 2 && h >= 2 {
            diff.set_next(
                0,
                0,
                Cell {
                    ch: '╭',
                    fg: edge,
                    bg: bg_global,
                },
            );
            diff.set_next(
                w - 1,
                0,
                Cell {
                    ch: '╮',
                    fg: edge,
                    bg: bg_global,
                },
            );
            diff.set_next(
                0,
                h - 1,
                Cell {
                    ch: '╰',
                    fg: edge,
                    bg: bg_global,
                },
            );
            diff.set_next(
                w - 1,
                h - 1,
                Cell {
                    ch: '╯',
                    fg: edge,
                    bg: bg_global,
                },
            );
        }

        // Lava lamp field rendered with braille: each terminal cell corresponds to 2x4 subpixels.
        // Map inner region to normalized [0,1]^2 space.
        let sx = 2.0;
        let sy = 4.0;

        let ramp_bg = |yy: u16| -> Rgb {
            let y01 = if inner_h <= 1 {
                0.5
            } else {
                (yy as f32) / ((inner_h - 1) as f32)
            };
            quantize_bg(y01, theme)
        };

        let mut y = 0u16;
        while y < inner_h {
            let bg_row = ramp_bg(y);
            let y01_cell = if inner_h <= 1 {
                0.5
            } else {
                (y as f32) / ((inner_h - 1) as f32)
            };

            for x in 0..inner_w {
                // Compute braille dots by sampling the field at 2x4 positions.
                let mut bits: u8 = 0;
                let mut cov = 0.0f32; // coverage 0..1
                let mut v_acc = 0.0f32;

                for py in 0..4 {
                    for px in 0..2 {
                        let fx = (x as f32 + (px as f32 + 0.5) / sx) / (inner_w.max(1) as f32);
                        let fy = (y as f32 + (py as f32 + 0.5) / sy) / (inner_h.max(1) as f32);
                        let p = Vec2::new(fx, fy);

                        let v = sim.field(p);
                        v_acc += v;

                        // Soft threshold to reduce speckle
                        let soft = sim.threshold;
                        let inside = (v - soft) * 3.2; // scale
                        let on = inside > 0.0;

                        if on {
                            cov += 1.0;
                            let dot_index = match (px, py) {
                                (0, 0) => 0, // 1
                                (0, 1) => 1, // 2
                                (0, 2) => 2, // 3
                                (0, 3) => 6, // 7
                                (1, 0) => 3, // 4
                                (1, 1) => 4, // 5
                                (1, 2) => 5, // 6
                                (1, 3) => 7, // 8
                                _ => 0,
                            };
                            bits |= 1u8 << dot_index;
                        }
                    }
                }

                cov /= 8.0;
                let v_mean = v_acc / 8.0;

                // Color: blend wax vs liquid. Use cov as the primary mix.
                let wax = wax_color(sim.heat, y01_cell, (v_mean - sim.threshold).max(0.0), theme);
                let bg = bg_row;

                let fg = if bits == 0 {
                    // faint particulate shimmer in liquid
                    let shimmer = 0.02 + 0.05 * (sim.wobble) * (sim.t * 0.7 + (x as f32) * 0.07).sin().abs();
                    Rgb::lerp(bg, Rgb { r: 130, g: 170, b: 235 }, shimmer)
                } else {
                    // wax foreground
                    Rgb::lerp(bg, wax, (0.55 + 0.45 * cov).clamp(0.0, 1.0))
                };

                let ch = if bits == 0 { ' ' } else { braille_from_bits(bits) };

                diff.set_next(
                    x + glass_pad_x,
                    y + glass_pad_y,
                    Cell { ch, fg, bg },
                );
            }
            y += 1;
        }

        // HUD
        if sim.show_hud && h >= 3 {
            let hud_bg = theme.hud_bg;
            let hud_fg = theme.hud_fg;

            let line1 = format!(
                "Lava Lamp  heat:{:>3}%  visc:{:>3}%  wobble:{:>3}%  blobs:{}  iso:{:.2}  {:>5.0} fps{}",
                (sim.heat * 100.0).round() as i32,
                (sim.viscosity * 100.0).round() as i32,
                (sim.wobble * 100.0).round() as i32,
                sim.blobs.len(),
                sim.threshold,
                fps_smoothed,
                if sim.paused { "  [PAUSED]" } else { "" }
            );
            let line2 =
                "Keys: ↑/↓ heat  ←/→ viscosity  ,/. wobble  [-]/[+] iso  [ / ] blobs  C theme  Space pause  R reseed  H hud  Q quit";

            let y0 = 0u16;
            for (i, ch) in line1.chars().take(w as usize).enumerate() {
                diff.set_next(
                    i as u16,
                    y0,
                    Cell { ch, fg: hud_fg, bg: hud_bg },
                );
            }
            if h >= 2 {
                for (i, ch) in line2.chars().take(w as usize).enumerate() {
                    diff.set_next(
                        i as u16,
                        1,
                        Cell { ch, fg: theme.hud_fg_dim, bg: hud_bg },
                    );
                }
            }
        }

        // Flush with flicker mitigation
        queue!(out, BeginSynchronizedUpdate)?;
        diff.flush(&mut out)?;
        queue!(out, ResetColor, EndSynchronizedUpdate)?;
        out.flush()?;

        // Cap frame rate
        std::thread::sleep(Duration::from_millis(8));
    }

    // Cleanup
    terminal::disable_raw_mode()?;
    execute!(
        out,
        ResetColor,
        cursor::Show,
        EnableLineWrap,
        LeaveAlternateScreen
    )?;
    Ok(())
}

use std::cmp::{max, min};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use rand::{rngs::StdRng, Rng, SeedableRng};

type CrosstermResult<T> = io::Result<T>;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

#[derive(Clone, Copy)]
struct Theme {
    name: &'static str,
    bg: Rgb,
    fg: Rgb,
    glow: Rgb,
    water: Rgb,
    stone: Rgb,
}

fn theme_by_name(name: &str) -> Theme {
    match name {
        "amber" => Theme {
            name: "amber",
            bg: Rgb { r: 7, g: 6, b: 3 },
            fg: Rgb { r: 255, g: 200, b: 120 },
            glow: Rgb { r: 255, g: 150, b: 70 },
            water: Rgb { r: 70, g: 90, b: 100 },
            stone: Rgb { r: 70, g: 60, b: 40 },
        },
        "ice" => Theme {
            name: "ice",
            bg: Rgb { r: 5, g: 7, b: 10 },
            fg: Rgb { r: 170, g: 220, b: 255 },
            glow: Rgb { r: 120, g: 180, b: 255 },
            water: Rgb { r: 40, g: 70, b: 110 },
            stone: Rgb { r: 60, g: 70, b: 80 },
        },
        "purple" => Theme {
            name: "purple",
            bg: Rgb { r: 10, g: 5, b: 15 },
            fg: Rgb { r: 210, g: 165, b: 255 },
            glow: Rgb { r: 185, g: 95, b: 255 },
            water: Rgb { r: 55, g: 60, b: 120 },
            stone: Rgb { r: 70, g: 55, b: 90 },
        },
        "mono" => Theme {
            name: "mono",
            bg: Rgb { r: 6, g: 6, b: 8 },
            fg: Rgb { r: 235, g: 235, b: 235 },
            glow: Rgb { r: 255, g: 255, b: 255 },
            water: Rgb { r: 70, g: 70, b: 80 },
            stone: Rgb { r: 85, g: 85, b: 90 },
        },
        _ => Theme {
            name: "mint",
            bg: Rgb { r: 5, g: 7, b: 10 },
            fg: Rgb { r: 165, g: 255, b: 220 },
            glow: Rgb { r: 110, g: 230, b: 205 },
            water: Rgb { r: 40, g: 75, b: 90 },
            stone: Rgb { r: 55, g: 60, b: 68 },
        },
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Cell {
    ch: char,
    fg: Rgb,
    bg: Rgb,
}
impl Cell {
    fn blank(bg: Rgb) -> Self {
        Self {
            ch: ' ',
            fg: bg,
            bg,
        }
    }
}

#[derive(Clone)]
struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    kind: ParticleKind,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ParticleKind {
    Droplet,
    Mist,
    Splash,
}

struct Waves {
    // 1D water surface across basin width (sim-resolution columns)
    h: Vec<f32>,
    v: Vec<f32>,
}
impl Waves {
    fn new(n: usize) -> Self {
        Self {
            h: vec![0.0; n],
            v: vec![0.0; n],
        }
    }
    fn resize(&mut self, n: usize) {
        self.h.resize(n, 0.0);
        self.v.resize(n, 0.0);
    }
    fn splash(&mut self, idx: usize, amp: f32) {
        if self.h.is_empty() {
            return;
        }
        let i = idx.min(self.h.len() - 1);
        self.v[i] += amp;
        if i > 0 {
            self.v[i - 1] += amp * 0.35;
        }
        if i + 1 < self.v.len() {
            self.v[i + 1] += amp * 0.35;
        }
    }
    fn step(&mut self, dt: f32, tension: f32, damping: f32, spread: f32) {
        if self.h.len() < 3 {
            return;
        }
        // wave equation-ish
        for i in 0..self.h.len() {
            self.v[i] -= self.h[i] * tension * dt;
            self.v[i] *= (1.0 - damping * dt).max(0.0);
        }
        for i in 0..self.h.len() {
            self.h[i] += self.v[i] * dt;
        }
        // neighbor spread
        let mut dh = vec![0.0f32; self.h.len()];
        for i in 1..self.h.len() - 1 {
            let lap = self.h[i - 1] + self.h[i + 1] - 2.0 * self.h[i];
            dh[i] = lap * spread;
        }
        for i in 1..self.h.len() - 1 {
            self.v[i] += dh[i] * dt;
        }
    }
}

// Braille helper: 2x4 pixels -> Unicode braille.
// Dots: (x,y) -> dot bit
// (0,0)=1, (0,1)=2, (0,2)=4, (1,0)=8, (1,1)=16, (1,2)=32, (0,3)=64, (1,3)=128
fn braille_bit(x: usize, y: usize) -> u8 {
    match (x, y) {
        (0, 0) => 0x01,
        (0, 1) => 0x02,
        (0, 2) => 0x04,
        (1, 0) => 0x08,
        (1, 1) => 0x10,
        (1, 2) => 0x20,
        (0, 3) => 0x40,
        (1, 3) => 0x80,
        _ => 0,
    }
}

fn clamp01(x: f32) -> f32 {
    x.max(0.0).min(1.0)
}
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = clamp01(t);
    let u = 1.0 - t;
    Rgb {
        r: (a.r as f32 * u + b.r as f32 * t) as u8,
        g: (a.g as f32 * u + b.g as f32 * t) as u8,
        b: (a.b as f32 * u + b.b as f32 * t) as u8,
    }
}

struct Args {
    theme: String,
    fps: u32,
    flow: f32,
    spread: f32,
    mist: f32,
}
fn parse_args() -> Args {
    let mut out = Args {
        theme: "mint".to_string(),
        fps: 60,
        flow: 1.0,
        spread: 0.65,
        mist: 0.55,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--theme" => {
                if let Some(v) = it.next() {
                    out.theme = v;
                }
            }
            "--fps" => {
                if let Some(v) = it.next() {
                    out.fps = v.parse().unwrap_or(out.fps);
                }
            }
            "--flow" => {
                if let Some(v) = it.next() {
                    out.flow = v.parse().unwrap_or(out.flow);
                }
            }
            "--spread" => {
                if let Some(v) = it.next() {
                    out.spread = v.parse().unwrap_or(out.spread);
                }
            }
            "--mist" => {
                if let Some(v) = it.next() {
                    out.mist = v.parse().unwrap_or(out.mist);
                }
            }
            "--help" | "-h" => {
                println!(
                    "cli_fountain\n\
                     \n\
                     USAGE:\n\
                     \tcli_fountain [--theme mint|amber|ice|purple|mono] [--fps 30..240]\n\
                     \t            [--flow 0.0..2.5] [--spread 0.0..1.5] [--mist 0.0..1.5]\n\
                     \n\
                     KEYS:\n\
                     \tQ/Esc quit | Space pause | C theme | +/- flow | [ and ] spread | M mist\n\
                     \tH help overlay | R reset\n"
                );
                std::process::exit(0);
            }
            _ => {}
        }
    }
    out.fps = out.fps.clamp(15, 240);
    out
}

struct Renderer {
    cols: u16,
    rows: u16,
    back: Vec<Cell>,
    front: Vec<Cell>,
}
impl Renderer {
    fn new(cols: u16, rows: u16, bg: Rgb) -> Self {
        let n = cols as usize * rows as usize;
        Self {
            cols,
            rows,
            back: vec![Cell::blank(bg); n],
            front: vec![Cell::blank(bg); n],
        }
    }
    fn resize(&mut self, cols: u16, rows: u16, bg: Rgb) {
        self.cols = cols;
        self.rows = rows;
        let n = cols as usize * rows as usize;
        self.back.resize(n, Cell::blank(bg));
        self.front.resize(n, Cell::blank(bg));
        self.back.fill(Cell::blank(bg));
        self.front.fill(Cell::blank(bg));
    }
    fn clear_back(&mut self, bg: Rgb) {
        self.back.fill(Cell::blank(bg));
    }
    fn set(&mut self, x: i32, y: i32, ch: char, fg: Rgb, bg: Rgb) {
        if x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as u16, y as u16);
        if x >= self.cols || y >= self.rows {
            return;
        }
        let i = y as usize * self.cols as usize + x as usize;
        self.back[i] = Cell { ch, fg, bg };
    }

    fn flush(&mut self, out: &mut io::Stdout) -> CrosstermResult<()> {
        queue!(out, BeginSynchronizedUpdate)?;
        let mut last_fg: Option<Rgb> = None;
        let mut last_bg: Option<Rgb> = None;

        let cols = self.cols as usize;
        let rows = self.rows as usize;

        for y in 0..rows {
            let mut x = 0usize;
            while x < cols {
                let i = y * cols + x;
                if self.back[i] == self.front[i] {
                    x += 1;
                    continue;
                }
                // find run
                let mut x2 = x + 1;
                while x2 < cols {
                    let j = y * cols + x2;
                    if self.back[j] == self.front[j] {
                        break;
                    }
                    x2 += 1;
                }

                queue!(out, cursor::MoveTo(x as u16, y as u16))?;
                for xx in x..x2 {
                    let k = y * cols + xx;
                    let c = self.back[k];
                    if last_bg != Some(c.bg) {
                        queue!(out, SetBackgroundColor(Color::Rgb {
                            r: c.bg.r,
                            g: c.bg.g,
                            b: c.bg.b
                        }))?;
                        last_bg = Some(c.bg);
                    }
                    if last_fg != Some(c.fg) {
                        queue!(out, SetForegroundColor(Color::Rgb {
                            r: c.fg.r,
                            g: c.fg.g,
                            b: c.fg.b
                        }))?;
                        last_fg = Some(c.fg);
                    }
                    queue!(out, Print(c.ch))?;
                }

                // commit to front
                self.front[i..(y * cols + x2)].copy_from_slice(&self.back[i..(y * cols + x2)]);
                x = x2;
            }
        }

        queue!(out, ResetColor, EndSynchronizedUpdate)?;
        out.flush()?;
        Ok(())
    }
}

fn main() -> CrosstermResult<()> {
    let mut args = parse_args();
    let mut theme = theme_by_name(&args.theme);

    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, cursor::Hide, DisableLineWrap)?;
    terminal::enable_raw_mode()?;

    let mut rng = StdRng::seed_from_u64(0xF0_17_A1_1A_u64);

    let mut paused = false;
    let mut show_help = false;

    // Controls state
    let mut flow = args.flow.clamp(0.0, 2.5);
    let mut spray = args.spread.clamp(0.0, 1.5);
    let mut mist_amt = args.mist.clamp(0.0, 1.5);

    // Simulation state
    let mut particles: VecDeque<Particle> = VecDeque::new();
    let mut t = 0.0f32;

    // Terminal sizing
    let (mut cols, mut rows) = terminal::size()?;
    cols = cols.max(60);
    rows = rows.max(24);

    let mut r = Renderer::new(cols, rows, theme.bg);

    // Braille sim resolution
    // 1 terminal cell = 2x4 "subpixels"
    let mut sim_w = cols as usize * 2;
    let mut sim_h = rows as usize * 4;

    // Fountain geometry in sim coordinates
    let mut basin_w = (sim_w as f32 * 0.56) as i32;
    let mut basin_h = (sim_h as f32 * 0.19) as i32;
    let mut basin_x0 = (sim_w as i32 - basin_w) / 2;
    let mut basin_y0 = (sim_h as i32 - basin_h) - (sim_h as i32 / 16);
    let mut water_y = basin_y0 + (basin_h as i32 / 2);

    // Waves across basin interior
    let mut waves = Waves::new(max(8, basin_w as usize));

    let target_dt = Duration::from_secs_f64(1.0 / args.fps as f64);
    let mut last = Instant::now();
    let mut acc = Duration::ZERO;

    // FPS meter (smoothed)
    let mut fps_hist: VecDeque<f32> = VecDeque::new();
    let mut fps_smoothed = 60.0f32;

    // Minor shimmer noise for glow
    let mut shimmer_phase = 0.0f32;

    let mut reset_seed = 1u64;

    loop {
        let now = Instant::now();
        let frame_dt = now - last;
        last = now;
        acc += frame_dt;

        // Input
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Resize(c, rr) => {
                    cols = c.max(60);
                    rows = rr.max(24);
                    r.resize(cols, rows, theme.bg);

                    sim_w = cols as usize * 2;
                    sim_h = rows as usize * 4;

                    basin_w = (sim_w as f32 * 0.56) as i32;
                    basin_h = (sim_h as f32 * 0.19) as i32;
                    basin_x0 = (sim_w as i32 - basin_w) / 2;
                    basin_y0 = (sim_h as i32 - basin_h) - (sim_h as i32 / 16);
                    water_y = basin_y0 + (basin_h as i32 / 2);

                    waves.resize(max(8, basin_w as usize));
                }
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    match k.code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                            cleanup(&mut out)?;
                            return Ok(());
                        }
                        KeyCode::Char(' ') => paused = !paused,
                        KeyCode::Char('h') | KeyCode::Char('H') => show_help = !show_help,
                        KeyCode::Char('c') | KeyCode::Char('C') => {
                            theme = cycle_theme(theme);
                            args.theme = theme.name.to_string();
                        }
                        KeyCode::Char('r') | KeyCode::Char('R') => {
                            reset_seed = reset_seed.wrapping_mul(1664525).wrapping_add(1013904223);
                            rng = StdRng::seed_from_u64(0xF0_17_A1_1A_u64 ^ reset_seed);
                            particles.clear();
                            waves.h.fill(0.0);
                            waves.v.fill(0.0);
                            t = 0.0;
                        }
                        KeyCode::Up => flow = (flow + 0.08).min(2.5),
                        KeyCode::Down => flow = (flow - 0.08).max(0.0),
                        KeyCode::Left => spray = (spray - 0.06).max(0.0),
                        KeyCode::Right => spray = (spray + 0.06).min(1.5),
                        KeyCode::Char('m') | KeyCode::Char('M') => {
                            mist_amt = if mist_amt > 0.01 { 0.0 } else { 0.55 };
                        }
                        KeyCode::Char('0') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                            flow = 1.0;
                            spray = 0.65;
                            mist_amt = 0.55;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // Fixed-ish timestep update (cap catch-up)
        let mut steps = 0;
        while acc >= target_dt && steps < 4 {
            acc -= target_dt;
            steps += 1;

            if !paused {
                let dt = target_dt.as_secs_f32();
                t += dt;
                shimmer_phase += dt * 1.4;

                // Geometry updates (in case theme changed bg)
                basin_w = (sim_w as f32 * 0.56) as i32;
                basin_h = (sim_h as f32 * 0.19) as i32;
                basin_x0 = (sim_w as i32 - basin_w) / 2;
                basin_y0 = (sim_h as i32 - basin_h) - (sim_h as i32 / 16);
                water_y = basin_y0 + (basin_h as i32 / 2);

                if waves.h.len() != max(8, basin_w as usize) {
                    waves.resize(max(8, basin_w as usize));
                }

                // Emit droplets
                let nozzle_x = (sim_w as f32 * 0.50) + (rng.gen::<f32>() - 0.5) * 1.2;
                let nozzle_y = (basin_y0 as f32) - (sim_h as f32 * 0.18);
                let base_up = 88.0 * (0.55 + flow * 0.55);
                let count = (2.0 + flow * 6.0) as usize;

                for _ in 0..count {
                    let ang = (rng.gen::<f32>() - 0.5) * 0.55 * spray;
                    let sp = base_up * (0.85 + rng.gen::<f32>() * 0.35);
                    let vx = ang.sin() * sp * 0.32 + (rng.gen::<f32>() - 0.5) * 6.0 * spray;
                    let vy = -sp * (0.90 + rng.gen::<f32>() * 0.12);
                    particles.push_back(Particle {
                        x: nozzle_x + (rng.gen::<f32>() - 0.5) * 1.0,
                        y: nozzle_y + (rng.gen::<f32>() - 0.5) * 1.0,
                        vx,
                        vy,
                        life: 1.6 + rng.gen::<f32>() * 0.9,
                        kind: ParticleKind::Droplet,
                    });
                }

                // Mist around nozzle
                if mist_amt > 0.001 {
                    let mcount = (1.0 + mist_amt * 6.0) as usize;
                    for _ in 0..mcount {
                        particles.push_back(Particle {
                            x: nozzle_x + (rng.gen::<f32>() - 0.5) * (10.0 + 22.0 * spray),
                            y: nozzle_y + (rng.gen::<f32>() - 0.5) * (8.0 + 20.0 * spray),
                            vx: (rng.gen::<f32>() - 0.5) * 16.0,
                            vy: -10.0 - rng.gen::<f32>() * 30.0,
                            life: 0.45 + rng.gen::<f32>() * 0.35,
                            kind: ParticleKind::Mist,
                        });
                    }
                }

                // Wave step
                waves.step(dt, 32.0, 2.8, 120.0);

                // Particle integrate
                let g = 130.0;
                let drag = 0.06;

                let basin_left = basin_x0 as f32;
                let basin_right = (basin_x0 + basin_w) as f32;

                let mut spawned = Vec::new();
                for p in particles.iter_mut() {
                    p.life -= dt;
                    match p.kind {
                        ParticleKind::Droplet | ParticleKind::Splash => {
                            p.vy += g * dt;
                            p.vx *= (1.0 - drag * dt).max(0.0);
                            p.vy *= (1.0 - drag * dt).max(0.0);
                        }
                        ParticleKind::Mist => {
                            p.vx *= (1.0 - 2.2 * dt).max(0.0);
                            p.vy *= (1.0 - 2.2 * dt).max(0.0);
                            p.vy += g * 0.08 * dt;
                        }
                    }

                    // slight turbulence (cheap)
                    let w = (p.y * 0.07 + t * 2.1).sin() * 0.6 + (p.x * 0.05 - t * 1.7).cos() * 0.4;
                    p.vx += w * dt * 18.0 * (0.25 + spray);

                    p.x += p.vx * dt;
                    p.y += p.vy * dt;

                    // bounds
                    if p.x < 1.0 {
                        p.x = 1.0;
                        p.vx *= -0.35;
                    }
                    if p.x > (sim_w as f32 - 2.0) {
                        p.x = sim_w as f32 - 2.0;
                        p.vx *= -0.35;
                    }

                    // Water hit
                    if (p.kind == ParticleKind::Droplet || p.kind == ParticleKind::Splash)
                        && p.y >= water_y as f32
                        && p.x >= basin_left
                        && p.x <= basin_right
                        && p.vy > 0.0
                    {
                        // splash into wave line
                        let xi = ((p.x - basin_left) / (basin_right - basin_left + 1.0)
                            * (waves.h.len() as f32 - 1.0)) as usize;
                        let amp = (p.vy.abs() * 0.0025).min(1.6) * (0.8 + flow * 0.5);
                        waves.splash(xi, amp);

                        // spawn a few splash beads
                        let scount = 1 + (amp * 6.0) as usize;
                        for _ in 0..scount {
                            let a = (rng.gen::<f32>() - 0.5) * 2.2;
                            let sp = 35.0 + rng.gen::<f32>() * 90.0 * amp;
                            spawned.push(Particle {
                                x: p.x + (rng.gen::<f32>() - 0.5) * 2.0,
                                y: water_y as f32 - 1.0,
                                vx: a.sin() * sp * 0.35,
                                vy: -sp * (0.4 + rng.gen::<f32>() * 0.6),
                                life: 0.35 + rng.gen::<f32>() * 0.55,
                                kind: ParticleKind::Splash,
                            });
                        }

                        // kill the impacting droplet
                        p.life = 0.0;
                    }
                }

                for particle in spawned {
                    particles.push_back(particle);
                }

                // cull
                particles.retain(|p| p.life > 0.0 && p.y < sim_h as f32 + 12.0);
                while particles.len() > 5500 {
                    particles.pop_front();
                }
            }
        }

        // FPS smoothing (visual only)
        let inst_fps = if frame_dt.as_secs_f32() > 0.0001 {
            1.0 / frame_dt.as_secs_f32()
        } else {
            999.0
        };
        fps_hist.push_back(inst_fps);
        if fps_hist.len() > 24 {
            fps_hist.pop_front();
        }
        fps_smoothed = fps_hist.iter().copied().sum::<f32>() / fps_hist.len() as f32;

        // Render
        r.clear_back(theme.bg);

        // Offscreen subpixel buffers at braille resolution
        let term_w = cols as usize;
        let term_h = rows as usize;

        // Per-braille-cell bitmask and intensity
        let mut mask = vec![0u8; term_w * term_h];
        let mut inten = vec![0f32; term_w * term_h];
        let mut kindmix = vec![0f32; term_w * term_h]; // 0=stone, 0.5=water, 1=glow
        let mut foam = vec![0f32; term_w * term_h];

        let to_cell = |sx: f32, sy: f32, term_w: usize, term_h: usize| -> Option<(usize, usize, usize, usize)> {
            if sx < 0.0 || sy < 0.0 {
                return None;
            }
            let bx = (sx as i32) / 2;
            let by = (sy as i32) / 4;
            if bx < 0 || by < 0 {
                return None;
            }
            let bxu = bx as usize;
            let byu = by as usize;
            if bxu >= term_w || byu >= term_h {
                return None;
            }
            let subx = (sx as i32 - (bx as i32) * 2) as usize;
            let suby = (sy as i32 - (by as i32) * 4) as usize;
            Some((bxu, byu, subx, suby))
        };

        // Draw basin stone and frame in sim space (as dense braille)
        // Outline and interior shading: add subpixel bits along edges.
        let edge_thick = 2;
        let inner_pad = 4;

        for sy in max(0, basin_y0 - 12)..min(sim_h as i32, basin_y0 + basin_h + 12) {
            for sx in max(0, basin_x0 - 12)..min(sim_w as i32, basin_x0 + basin_w + 12) {
                let inside = sx >= basin_x0
                    && sx < basin_x0 + basin_w
                    && sy >= basin_y0
                    && sy < basin_y0 + basin_h;

                let border = sx >= basin_x0 - edge_thick
                    && sx < basin_x0 + basin_w + edge_thick
                    && sy >= basin_y0 - edge_thick
                    && sy < basin_y0 + basin_h + edge_thick
                    && !inside;

                let xdist = min((sx - basin_x0).abs(), (sx - (basin_x0 + basin_w)).abs());
                let ydist = min((sy - basin_y0).abs(), (sy - (basin_y0 + basin_h)).abs());

                let near_edge = inside
                    && (sx < basin_x0 + inner_pad
                        || sx > basin_x0 + basin_w - inner_pad
                        || sy < basin_y0 + inner_pad
                        || sy > basin_y0 + basin_h - inner_pad);

                let base_alpha = if border { 0.85 } else if near_edge { 0.45 } else { 0.0 };
                if base_alpha <= 0.001 {
                    continue;
                }

                if let Some((bx, by, subx, suby)) =
                    to_cell(sx as f32, sy as f32, term_w, term_h)
                {
                    let i = by * term_w + bx;
                    mask[i] |= braille_bit(subx, suby);
                    let wob = (shimmer_phase * 0.7 + (sx as f32) * 0.035 + (sy as f32) * 0.022).sin();
                    let a = base_alpha + 0.10 * wob;
                    inten[i] = inten[i].max(a);
                    kindmix[i] = kindmix[i].max(0.10 + 0.15 * (xdist as f32 + ydist as f32).min(10.0) / 10.0);
                }
            }
        }

        // Draw water surface and fill
        let wlen = waves.h.len();
        let basin_left = basin_x0 as f32;
        let basin_right = (basin_x0 + basin_w) as f32;

        for sx in basin_x0..(basin_x0 + basin_w) {
            let t01 = (sx - basin_x0) as f32 / (basin_w.max(1) as f32);
            let wi = (t01 * (wlen as f32 - 1.0)).clamp(0.0, (wlen - 1) as f32) as usize;
            let surf = water_y as f32 + waves.h[wi] * 8.0;
            // fill water downwards
            let y0 = surf as i32;
            for sy in y0..(basin_y0 + basin_h) {
                if let Some((bx, by, subx, suby)) =
                    to_cell(sx as f32, sy as f32, term_w, term_h)
                {
                    let i = by * term_w + bx;
                    mask[i] |= braille_bit(subx, suby);
                    let depth = (sy - y0) as f32 / (basin_h.max(1) as f32);
                    let a = 0.18 + 0.35 * (1.0 - depth).powf(0.35);
                    inten[i] = inten[i].max(a);
                    kindmix[i] = kindmix[i].max(0.55);
                }
            }
            // highlight surface line
            let sy = y0;
            if let Some((bx, by, subx, suby)) = to_cell(sx as f32, sy as f32, term_w, term_h) {
                let i = by * term_w + bx;
                mask[i] |= braille_bit(subx, suby);
                inten[i] = inten[i].max(0.55);
                kindmix[i] = kindmix[i].max(0.72);
            }
        }

        // Draw pump/nozzle column (stone pedestal)
        let nozzle_x = (sim_w as f32 * 0.50) as i32;
        let nozzle_y = basin_y0 - (sim_h as i32 / 6);
        let col_w = 8;
        let col_h = (basin_y0 - nozzle_y).max(10);
        for sy in nozzle_y..basin_y0 {
            for sx in (nozzle_x - col_w / 2)..(nozzle_x + col_w / 2) {
                if let Some((bx, by, subx, suby)) =
                    to_cell(sx as f32, sy as f32, term_w, term_h)
                {
                    let i = by * term_w + bx;
                    mask[i] |= braille_bit(subx, suby);
                    let tcol = (sy - nozzle_y) as f32 / (col_h as f32);
                    let a = 0.22 + 0.38 * (1.0 - tcol).powf(0.4);
                    inten[i] = inten[i].max(a);
                    kindmix[i] = kindmix[i].max(0.12);
                }
            }
        }

        // Nozzle head glow
        for dy in -6..=6 {
            for dx in -12..=12 {
                let sx = nozzle_x + dx;
                let sy = nozzle_y + dy;
                let d2 = (dx * dx + dy * dy) as f32;
                if d2 > 12.0 * 12.0 {
                    continue;
                }
                if let Some((bx, by, subx, suby)) =
                    to_cell(sx as f32, sy as f32, term_w, term_h)
                {
                    let i = by * term_w + bx;
                    mask[i] |= braille_bit(subx, suby);
                    let a = 0.30 * (1.0 - d2 / (12.0 * 12.0));
                    inten[i] = inten[i].max(a);
                    kindmix[i] = kindmix[i].max(0.95);
                }
            }
        }

        // Draw particles
        for p in particles.iter() {
            let (a, km, radius) = match p.kind {
                ParticleKind::Droplet => (0.85, 0.95, 1.0),
                ParticleKind::Splash => (0.75, 0.92, 1.0),
                ParticleKind::Mist => (0.12, 0.88, 1.6),
            };
            let speed = (p.vx * p.vx + p.vy * p.vy).sqrt();
            let speed01 = clamp01((speed - 5.0) / 50.0);
            let rr = radius * (1.0 + 0.6 * spray);
            let x0 = (p.x - rr).floor() as i32;
            let x1 = (p.x + rr).ceil() as i32;
            let y0 = (p.y - rr).floor() as i32;
            let y1 = (p.y + rr).ceil() as i32;

            for sy in y0..=y1 {
                for sx in x0..=x1 {
                    let dx = sx as f32 - p.x;
                    let dy = sy as f32 - p.y;
                    let d2 = dx * dx + dy * dy;
                    if d2 > rr * rr {
                        continue;
                    }
                    if let Some((bx, by, subx, suby)) =
                        to_cell(sx as f32, sy as f32, term_w, term_h)
                    {
                        let i = by * term_w + bx;
                        mask[i] |= braille_bit(subx, suby);
                        let rr2 = rr * rr + 1e-6;
                        let falloff = (1.0 - d2 / rr2).max(0.0);
                        let life01 = clamp01(p.life / 1.8);
                        //aa = base alpha, modulated by life and falloff
                        let aa = a * falloff * (0.25 + 0.75 * life01);
                        inten[i] = inten[i].max(aa);
                        kindmix[i] = kindmix[i].max(km);
                        foam[i] = foam[i].max(speed01 * aa);
                        if matches!(p.kind, ParticleKind::Mist) {
                            foam[i] = foam[i].max(aa * 0.92);
                        }
                    }
                }
            }
        }

        // Convert braille buffers to terminal cells with theme colors
        for by in 0..term_h {
            for bx in 0..term_w {
                let i = by * term_w + bx;
                let m = mask[i];
                if m == 0 {
                    continue;
                }
                let ch = char::from_u32(0x2800 + m as u32).unwrap_or(' ');
                let a = inten[i].min(1.0);

                let base = if kindmix[i] < 0.35 {
                    theme.stone
                } else if kindmix[i] < 0.80 {
                    theme.water
                } else {
                    theme.glow
                };

                // glow-y mix: brighter pixels bias toward glow
                let glow_bias = clamp01((kindmix[i] - 0.65) * 1.5) * (0.35 + 0.65 * a);
                let fg = mix(theme.fg, mix(base, theme.glow, glow_bias), 0.55 + 0.35 * a);
                let fg = mix(fg, Rgb { r: 255, g: 255, b: 255 }, foam[i] * 0.85);

                // Slight background tint for water, mostly bg
                let bg = if kindmix[i] > 0.45 && kindmix[i] < 0.85 {
                    mix(theme.bg, theme.water, 0.08 + 0.14 * a)
                } else {
                    theme.bg
                };

                r.set(bx as i32, by as i32, ch, fg, bg);
            }
        }

        // HUD
        draw_hud(
            &mut r,
            theme,
            paused,
            show_help,
            flow,
            spray,
            mist_amt,
            fps_smoothed,
        );

        r.flush(&mut out)?;
    }
}

fn cycle_theme(cur: Theme) -> Theme {
    let order = ["mint", "amber", "ice", "purple", "mono"];
    let mut idx = 0usize;
    for (i, &n) in order.iter().enumerate() {
        if n == cur.name {
            idx = i;
            break;
        }
    }
    theme_by_name(order[(idx + 1) % order.len()])
}

fn draw_text(r: &mut Renderer, x: i32, y: i32, s: &str, fg: Rgb, bg: Rgb) {
    for (i, ch) in s.chars().enumerate() {
        r.set(x + i as i32, y, ch, fg, bg);
    }
}

fn draw_hud(
    r: &mut Renderer,
    theme: Theme,
    paused: bool,
    show_help: bool,
    flow: f32,
    spray: f32,
    mist: f32,
    fps: f32,
) {
    let fg = mix(theme.fg, theme.glow, 0.25);
    let bg = mix(theme.bg, theme.bg, 0.0);

    let top = 0;
    let line = format!(
        " CLI Fountain  |  theme={}  |  height={:.2}  width={:.2}  mist={}  |  {:.0} fps {}",
        theme.name,
        flow,
        spray,
        if mist > 0.01 { "on" } else { "off" },
        fps,
        if paused { "(paused)" } else { "" }
    );
    draw_text(r, 1, top, &line, fg, bg);

    if show_help {
        let y0 = 2;
        let help = [
            "Keys:",
            "  Q / Esc    quit",
            "  Space      pause",
            "  C          cycle theme",
            "  Up/Down    height up/down",
            "  Left/Right width narrower/wider",
            "  M          toggle mist",
            "  R          reset (new seed)",
            "  H          toggle this help",
            "Tip: resize the terminal to change the fountain scale",
        ];
        for (i, s) in help.iter().enumerate() {
            draw_text(r, 1, y0 + i as i32, s, mix(theme.fg, theme.glow, 0.15), bg);
        }
    }
}

fn cleanup(out: &mut io::Stdout) -> CrosstermResult<()> {
    terminal::disable_raw_mode()?;
    execute!(
        out,
        EndSynchronizedUpdate,
        ResetColor,
        EnableLineWrap,
        cursor::Show,
        LeaveAlternateScreen
    )?;
    Ok(())
}

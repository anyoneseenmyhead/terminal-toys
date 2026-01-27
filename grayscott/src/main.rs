// src/main.rs
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::cmp::min;
use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
struct Params {
    du: f32,
    dv: f32,
    feed: f32,
    kill: f32,
    dt: f32,
}

#[derive(Clone, Copy)]
struct Preset {
    name: &'static str,
    p: Params,
    steps_per_frame: usize,
}

const PRESETS: &[Preset] = &[
    // Commonly used "nice" Gray–Scott regimes (qualitative names).
    Preset {
        name: "Mitosis",
        p: Params {
            du: 0.16,
            dv: 0.08,
            feed: 0.0220,
            kill: 0.0510,
            dt: 1.0,
        },
        steps_per_frame: 10,
    },
    Preset {
        name: "Worms",
        p: Params {
            du: 0.16,
            dv: 0.08,
            feed: 0.0285,
            kill: 0.0590,
            dt: 1.0,
        },
        steps_per_frame: 8,
    },
    Preset {
        name: "Solitons",
        p: Params {
            du: 0.16,
            dv: 0.08,
            feed: 0.0350,
            kill: 0.0595,
            dt: 1.0,
        },
        steps_per_frame: 10,
    },
    Preset {
        name: "Spots",
        p: Params {
            du: 0.16,
            dv: 0.08,
            feed: 0.0270,
            kill: 0.0545,
            dt: 1.0,
        },
        steps_per_frame: 10,
    },
    Preset {
        name: "Stripes",
        p: Params {
            du: 0.16,
            dv: 0.08,
            feed: 0.022,
            kill: 0.051,
            dt: 1.0,
        },
        steps_per_frame: 12,
    },
];

#[derive(Clone, Copy)]
struct Theme {
    name: &'static str,
    map: fn(v: f32, u: f32, b: f32, params: &Params) -> (u8, u8, u8),
}

const THEMES: &[Theme] = &[
    Theme {
        name: "Mono",
        map: theme_mono,
    },
    Theme {
        name: "Heat",
        map: theme_heat,
    },
    Theme {
        name: "Ocean",
        map: theme_ocean,
    },
    Theme {
        name: "Aurora",
        map: theme_aurora,
    },
    Theme {
        name: "Param",
        map: theme_parametric,
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
struct Cell {
    ch: char,
    fg: Color,
}

impl Cell {
    fn blank() -> Self {
        Self {
            ch: ' ',
            fg: Color::White,
        }
    }
}

struct TermGuard {
    out: Stdout,
}

impl TermGuard {
    fn new() -> io::Result<Self> {
        let mut out = io::stdout();
        terminal::enable_raw_mode()?;
        execute!(
            out,
            EnterAlternateScreen,
            DisableLineWrap,
            cursor::Hide,
            cursor::MoveTo(0, 0)
        )?;
        Ok(Self { out })
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        let _ = execute!(
            self.out,
            EndSynchronizedUpdate,
            ResetColor,
            cursor::Show,
            EnableLineWrap,
            LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

struct Sim {
    w: usize,
    h: usize,
    u: Vec<f32>,
    v: Vec<f32>,
    u2: Vec<f32>,
    v2: Vec<f32>,
    params: Params,
}

impl Sim {
    fn new(w: usize, h: usize, params: Params) -> Self {
        let n = w * h;
        Self {
            w,
            h,
            u: vec![1.0; n],
            v: vec![0.0; n],
            u2: vec![1.0; n],
            v2: vec![0.0; n],
            params,
        }
    }

    fn resize(&mut self, w: usize, h: usize) {
        self.w = w;
        self.h = h;
        let n = w * h;
        self.u.resize(n, 1.0);
        self.v.resize(n, 0.0);
        self.u2.resize(n, 1.0);
        self.v2.resize(n, 0.0);
    }

    fn reset(&mut self, seed: u64) {
        let mut rng = StdRng::seed_from_u64(seed);
        let n = self.w * self.h;
        self.u.fill(1.0);
        self.v.fill(0.0);

        // Seed a handful of V "droplets".
        let droplets = 10usize;
        for _ in 0..droplets {
            let cx = rng.gen_range(0..self.w);
            let cy = rng.gen_range(0..self.h);
            let r = rng.gen_range(6..20) as isize;

            for dy in -r..=r {
                for dx in -r..=r {
                    if dx * dx + dy * dy > r * r {
                        continue;
                    }
                    let x = ((cx as isize + dx).rem_euclid(self.w as isize)) as usize;
                    let y = ((cy as isize + dy).rem_euclid(self.h as isize)) as usize;
                    let i = y * self.w + x;
                    self.v[i] = 1.0;
                    self.u[i] = 0.0;
                }
            }
        }

        // Tiny noise to break symmetry.
        for i in 0..n {
            let j: f32 = rng.gen_range(-0.005..0.005);
            self.u[i] = (self.u[i] + j).clamp(0.0, 1.0);
            self.v[i] = (self.v[i] - j).clamp(0.0, 1.0);
        }
    }

    #[inline]
    fn idx(&self, x: usize, y: usize) -> usize {
        y * self.w + x
    }

    #[inline]
    fn wrap(&self, x: isize, y: isize) -> usize {
        let xx = x.rem_euclid(self.w as isize) as usize;
        let yy = y.rem_euclid(self.h as isize) as usize;
        self.idx(xx, yy)
    }

    fn step(&mut self) {
        let Params {
            du,
            dv,
            feed,
            kill,
            dt,
        } = self.params;

        // 9-point Laplacian stencil weights (sum to 0).
        const W_C: f32 = -1.0;
        const W_N: f32 = 0.2;
        const W_D: f32 = 0.05;

        for y in 0..self.h {
            for x in 0..self.w {
                let i = self.idx(x, y);

                let u = self.u[i];
                let v = self.v[i];

                let xm = x as isize - 1;
                let xp = x as isize + 1;
                let ym = y as isize - 1;
                let yp = y as isize + 1;
                let xc = x as isize;
                let yc = y as isize;

                let i_c = self.wrap(xc, yc);
                let i_l = self.wrap(xm, yc);
                let i_r = self.wrap(xp, yc);
                let i_u = self.wrap(xc, ym);
                let i_d = self.wrap(xc, yp);

                let i_ul = self.wrap(xm, ym);
                let i_ur = self.wrap(xp, ym);
                let i_dl = self.wrap(xm, yp);
                let i_dr = self.wrap(xp, yp);

                let lap_u = W_C * self.u[i_c]
                    + W_N * (self.u[i_l] + self.u[i_r] + self.u[i_u] + self.u[i_d])
                    + W_D * (self.u[i_ul] + self.u[i_ur] + self.u[i_dl] + self.u[i_dr]);

                let lap_v = W_C * self.v[i_c]
                    + W_N * (self.v[i_l] + self.v[i_r] + self.v[i_u] + self.v[i_d])
                    + W_D * (self.v[i_ul] + self.v[i_ur] + self.v[i_dl] + self.v[i_dr]);

                let reaction = u * v * v;

                let du_dt = du * lap_u - reaction + feed * (1.0 - u);
                let dv_dt = dv * lap_v + reaction - (feed + kill) * v;

                let nu = (u + dt * du_dt).clamp(0.0, 1.2);
                let nv = (v + dt * dv_dt).clamp(0.0, 1.2);

                self.u2[i] = nu;
                self.v2[i] = nv;
            }
        }

        std::mem::swap(&mut self.u, &mut self.u2);
        std::mem::swap(&mut self.v, &mut self.v2);
    }

    // Paint a blob of V at normalized coordinates [0,1]x[0,1] in sim space
    fn paint_v(&mut self, nx: f32, ny: f32, radius: usize, amount: f32) {
        let cx = (nx * self.w as f32).clamp(0.0, (self.w - 1) as f32) as isize;
        let cy = (ny * self.h as f32).clamp(0.0, (self.h - 1) as f32) as isize;
        let r = radius as isize;

        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let i = self.wrap(cx + dx, cy + dy);
                self.v[i] = (self.v[i] + amount).clamp(0.0, 1.2);
                self.u[i] = (self.u[i] - 0.5 * amount).clamp(0.0, 1.2);
            }
        }
    }
}

// Braille dot bit positions used by Unicode braille.
const BRAILLE_BASE: u32 = 0x2800;
const DOT1: u8 = 1 << 0;
const DOT2: u8 = 1 << 1;
const DOT3: u8 = 1 << 2;
const DOT4: u8 = 1 << 3;
const DOT5: u8 = 1 << 4;
const DOT6: u8 = 1 << 5;
const DOT7: u8 = 1 << 6;
const DOT8: u8 = 1 << 7;

// 0..8-dot "ramp" patterns with a pleasant fill order.
// This approximates brightness by adding dots progressively.
const RAMP: [u8; 9] = [
    0,
    DOT1,
    DOT1 | DOT4,
    DOT1 | DOT2 | DOT4,
    DOT1 | DOT2 | DOT4 | DOT5,
    DOT1 | DOT2 | DOT3 | DOT4 | DOT5,
    DOT1 | DOT2 | DOT3 | DOT4 | DOT5 | DOT6,
    DOT1 | DOT2 | DOT3 | DOT4 | DOT5 | DOT6 | DOT7,
    DOT1 | DOT2 | DOT3 | DOT4 | DOT5 | DOT6 | DOT7 | DOT8,
];

fn ramp_braille(level_0_to_8: usize) -> char {
    let m = RAMP[level_0_to_8.min(8)] as u32;
    char::from_u32(BRAILLE_BASE + m).unwrap_or(' ')
}

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_color(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    (lerp(a.0, b.0, t), lerp(a.1, b.1, t), lerp(a.2, b.2, t))
}

fn tri_gradient(t: f32, c0: (f32, f32, f32), c1: (f32, f32, f32), c2: (f32, f32, f32)) -> (f32, f32, f32) {
    let tt = clamp01(t);
    if tt <= 0.5 {
        lerp_color(c0, c1, tt * 2.0)
    } else {
        lerp_color(c1, c2, (tt - 0.5) * 2.0)
    }
}

fn to_rgb_u8(c: (f32, f32, f32)) -> (u8, u8, u8) {
    let r = (clamp01(c.0) * 255.0).round() as u8;
    let g = (clamp01(c.1) * 255.0).round() as u8;
    let b = (clamp01(c.2) * 255.0).round() as u8;
    (r, g, b)
}

fn theme_mono(_v: f32, _u: f32, b: f32, _params: &Params) -> (u8, u8, u8) {
    let g = (clamp01(b) * 255.0).round() as u8;
    (g, g, g)
}

fn theme_heat(v: f32, _u: f32, b: f32, _params: &Params) -> (u8, u8, u8) {
    let t = clamp01(v * 0.7 + b * 0.6);
    let c = tri_gradient(t, (0.02, 0.0, 0.0), (0.9, 0.2, 0.0), (1.0, 0.95, 0.6));
    to_rgb_u8(c)
}

fn theme_ocean(v: f32, u: f32, b: f32, _params: &Params) -> (u8, u8, u8) {
    let t = clamp01(v * 0.8 + b * 0.4);
    let depth = clamp01(u * 0.8 + 0.1);
    let base = tri_gradient(t, (0.0, 0.05, 0.15), (0.0, 0.55, 0.65), (0.8, 0.95, 1.0));
    let c = lerp_color(base, (0.0, 0.2, 0.35), depth * 0.35);
    to_rgb_u8(c)
}

fn theme_aurora(v: f32, u: f32, b: f32, _params: &Params) -> (u8, u8, u8) {
    let t = clamp01(v * 0.9 + b * 0.5);
    let band = clamp01((u * 1.3).fract());
    let c0 = (0.05, 0.05, 0.1);
    let c1 = lerp_color((0.0, 0.7, 0.4), (0.2, 0.9, 0.95), band);
    let c2 = (0.95, 0.95, 0.9);
    let c = tri_gradient(t, c0, c1, c2);
    to_rgb_u8(c)
}

fn theme_parametric(v: f32, u: f32, b: f32, params: &Params) -> (u8, u8, u8) {
    let t = clamp01(v * 0.85 + b * 0.45);
    let k = clamp01((params.feed + params.kill) * 8.0);
    let warm = (0.75, 0.25, 0.05);
    let cool = (0.1, 0.3, 0.8);
    let mid_a = (0.1, 0.85, 0.5);
    let mid_b = (0.9, 0.2, 0.7);
    let bright_a = (0.95, 0.95, 0.85);
    let bright_b = (1.0, 0.85, 0.5);

    let c0 = lerp_color(cool, warm, k);
    let c1 = lerp_color(mid_a, mid_b, k);
    let c2 = lerp_color(bright_a, bright_b, k);
    let c = tri_gradient(t + (u * 0.1), c0, c1, c2);
    to_rgb_u8(c)
}

fn main() -> io::Result<()> {
    let mut tg = TermGuard::new()?;
    let out = &mut tg.out;

    let mut rng = StdRng::seed_from_u64(0xC0FFEE_u64);

    let mut preset_idx: usize = 0;
    let mut paused = false;

    let mut cols_rows = terminal::size()?;
    let mut cols = cols_rows.0 as usize;
    let mut rows = cols_rows.1 as usize;

    let hud_rows = 3usize;
    let render_rows = rows.saturating_sub(hud_rows);
    let sim_w = cols * 2;
    let sim_h = render_rows * 4;

    let mut sim = Sim::new(sim_w.max(2), sim_h.max(2), PRESETS[preset_idx].p);
    sim.reset(rng.gen());

    let mut last_frame: Vec<Cell> = vec![Cell::blank(); cols * render_rows];

    let mut last_present = Instant::now();
    let mut fps_timer = Instant::now();
    let mut frames: u32 = 0;
    let mut fps: f32 = 0.0;

    // Render parameters
    let mut contrast: f32 = 3.8; // increases local differences in V
    let mut v_mid: f32 = 0.13; // center for mapping V -> brightness
    let mut paint_radius_cells: usize = 10;
    let mut steps_per_frame: usize = PRESETS[preset_idx].steps_per_frame;
    let mut theme_idx: usize = 0;

    loop {
        // Handle terminal resize
        let now_cols_rows = terminal::size()?;
        if now_cols_rows != cols_rows {
            cols_rows = now_cols_rows;
            cols = cols_rows.0 as usize;
            rows = cols_rows.1 as usize;

            let render_rows_new = rows.saturating_sub(hud_rows);
            let sim_w_new = cols * 2;
            let sim_h_new = render_rows_new * 4;

            sim.resize(sim_w_new.max(2), sim_h_new.max(2));
            sim.reset(rng.gen());

            last_frame = vec![Cell::blank(); cols * render_rows_new];
            execute!(out, terminal::Clear(terminal::ClearType::All), cursor::MoveTo(0, 0))?;
        }

        // Input
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    match (k.code, k.modifiers) {
                        (KeyCode::Char('q') | KeyCode::Char('Q'), _) => return Ok(()),
                        (KeyCode::Char(' '), _) => paused = !paused,
                        (KeyCode::Char('r') | KeyCode::Char('R'), _) => sim.reset(rng.gen()),
                        (KeyCode::Char('c') | KeyCode::Char('C'), _) => {
                            contrast = (contrast + 0.2).min(5.0)
                        }
                        (KeyCode::Char('x') | KeyCode::Char('X'), _) => {
                            contrast = (contrast - 0.2).max(0.4)
                        }
                        (KeyCode::Char('['), _) => v_mid = (v_mid - 0.02).clamp(0.05, 0.95),
                        (KeyCode::Char(']'), _) => v_mid = (v_mid + 0.02).clamp(0.05, 0.95),

                        (KeyCode::Char('-'), _) => steps_per_frame = steps_per_frame.saturating_sub(1).max(1),
                        (KeyCode::Char('='), _) => steps_per_frame = min(steps_per_frame + 1, 50),

                        (KeyCode::Char('p') | KeyCode::Char('P'), _) => {
                            preset_idx = (preset_idx + 1) % PRESETS.len();
                            sim.params = PRESETS[preset_idx].p;
                            steps_per_frame = PRESETS[preset_idx].steps_per_frame;
                            sim.reset(rng.gen());
                        }
                        (KeyCode::Char('t') | KeyCode::Char('T'), _) => {
                            theme_idx = (theme_idx + 1) % THEMES.len()
                        }

                        // Fine parameter tweaks
                        (KeyCode::Up, _) => sim.params.feed = (sim.params.feed + 0.0005).clamp(0.0, 0.1),
                        (KeyCode::Down, _) => sim.params.feed = (sim.params.feed - 0.0005).clamp(0.0, 0.1),
                        (KeyCode::Right, _) => sim.params.kill = (sim.params.kill + 0.0005).clamp(0.0, 0.1),
                        (KeyCode::Left, _) => sim.params.kill = (sim.params.kill - 0.0005).clamp(0.0, 0.1),

                        // Bigger steps with shift
                        (KeyCode::Up, KeyModifiers::SHIFT) => {
                            sim.params.feed = (sim.params.feed + 0.002).clamp(0.0, 0.1)
                        }
                        (KeyCode::Down, KeyModifiers::SHIFT) => {
                            sim.params.feed = (sim.params.feed - 0.002).clamp(0.0, 0.1)
                        }
                        (KeyCode::Right, KeyModifiers::SHIFT) => {
                            sim.params.kill = (sim.params.kill + 0.002).clamp(0.0, 0.1)
                        }
                        (KeyCode::Left, KeyModifiers::SHIFT) => {
                            sim.params.kill = (sim.params.kill - 0.002).clamp(0.0, 0.1)
                        }

                        // Brush size
                        (KeyCode::Char(','), _) => paint_radius_cells = paint_radius_cells.saturating_sub(1).max(2),
                        (KeyCode::Char('.'), _) => paint_radius_cells = min(paint_radius_cells + 1, 80),

                        _ => {}
                    }
                }

                Event::Mouse(m) => {
                    // Optional: paint with left mouse button drag/press
                    use crossterm::event::{MouseButton, MouseEventKind};
                    match m.kind {
                        MouseEventKind::Down(MouseButton::Left)
                        | MouseEventKind::Drag(MouseButton::Left) => {
                            // Map mouse in render area only (ignore HUD)
                            let mx = m.column as i32;
                            let my = m.row as i32;
                            let render_rows_cur = (terminal::size()?.1 as usize).saturating_sub(hud_rows);

                            if my >= hud_rows as i32 && my < (hud_rows + render_rows_cur) as i32 {
                                let nx = (mx as f32 + 0.5) / (cols.max(1) as f32);
                                let ny = ((my as usize - hud_rows) as f32 + 0.5)
                                    / (render_rows_cur.max(1) as f32);

                                // Convert brush radius in terminal cells to sim pixels
                                let r_sim = paint_radius_cells * 4;
                                sim.paint_v(nx, ny, r_sim, 0.35);
                            }
                        }
                        _ => {}
                    }
                }

                _ => {}
            }
        }

        if !paused {
            for _ in 0..steps_per_frame {
                sim.step();
            }
        }

        let cols_cur = terminal::size()?.0 as usize;
        let rows_cur = terminal::size()?.1 as usize;
        let render_rows_cur = rows_cur.saturating_sub(hud_rows);
        let w = cols_cur;
        let h = render_rows_cur;

        // Start synchronized draw
        queue!(out, BeginSynchronizedUpdate)?;

        // Render field to braille cells using last_frame diff.
        // Map V to brightness using a smooth curve centered on v_mid.
        let sim_w_cur = sim.w;
        let sim_h_cur = sim.h;

        // Helpers for sampling sim at integer coordinates (wrap already implicit in sim arrays layout).
        let sample = |x: usize, y: usize, v: &Vec<f32>, w: usize| -> f32 { v[y * w + x] };

        let theme = THEMES[theme_idx];
        let mut cur_fg = Color::White;
        let mut v_min = 1.0f32;
        let mut v_max = 0.0f32;
        let mut v_sum = 0.0f32;
        let mut b_sum = 0.0f32;
        let mut cell_count: usize = 0;

        for ty in 0..h {
            let y0 = ty * 4;
            if y0 + 3 >= sim_h_cur {
                break;
            }

            for tx in 0..w {
                let x0 = tx * 2;
                if x0 + 1 >= sim_w_cur {
                    break;
                }

                // 2x4 samples of V and U
                let mut v00 = sample(x0, y0 + 0, &sim.v, sim_w_cur);
                let mut v10 = sample(x0 + 1, y0 + 0, &sim.v, sim_w_cur);
                let mut v01 = sample(x0, y0 + 1, &sim.v, sim_w_cur);
                let mut v11 = sample(x0 + 1, y0 + 1, &sim.v, sim_w_cur);
                let mut v02 = sample(x0, y0 + 2, &sim.v, sim_w_cur);
                let mut v12 = sample(x0 + 1, y0 + 2, &sim.v, sim_w_cur);
                let mut v03 = sample(x0, y0 + 3, &sim.v, sim_w_cur);
                let mut v13 = sample(x0 + 1, y0 + 3, &sim.v, sim_w_cur);
                let u00 = sample(x0, y0 + 0, &sim.u, sim_w_cur);
                let u10 = sample(x0 + 1, y0 + 0, &sim.u, sim_w_cur);
                let u01 = sample(x0, y0 + 1, &sim.u, sim_w_cur);
                let u11 = sample(x0 + 1, y0 + 1, &sim.u, sim_w_cur);
                let u02 = sample(x0, y0 + 2, &sim.u, sim_w_cur);
                let u12 = sample(x0 + 1, y0 + 2, &sim.u, sim_w_cur);
                let u03 = sample(x0, y0 + 3, &sim.u, sim_w_cur);
                let u13 = sample(x0 + 1, y0 + 3, &sim.u, sim_w_cur);

                // Contrast curve around v_mid:
                //   b = sigmoid-ish via tanh, then map to 0..1
                //   also include slight local enhancement using difference from local mean
                let mean = (v00 + v10 + v01 + v11 + v02 + v12 + v03 + v13) * 0.125;
                let mean_u = (u00 + u10 + u01 + u11 + u02 + u12 + u03 + u13) * 0.125;
                let enhance = 0.6;
                v00 = (v00 + enhance * (v00 - mean)).clamp(0.0, 1.2);
                v10 = (v10 + enhance * (v10 - mean)).clamp(0.0, 1.2);
                v01 = (v01 + enhance * (v01 - mean)).clamp(0.0, 1.2);
                v11 = (v11 + enhance * (v11 - mean)).clamp(0.0, 1.2);
                v02 = (v02 + enhance * (v02 - mean)).clamp(0.0, 1.2);
                v12 = (v12 + enhance * (v12 - mean)).clamp(0.0, 1.2);
                v03 = (v03 + enhance * (v03 - mean)).clamp(0.0, 1.2);
                v13 = (v13 + enhance * (v13 - mean)).clamp(0.0, 1.2);

                let to_b = |vv: f32| -> f32 {
                    let x = (vv - v_mid) * contrast * 6.0;
                    // tanh-like curve without needing libm
                    // approx: x / (1 + |x|) mapped to 0..1
                    let t = x / (1.0 + x.abs());
                    (0.5 + 0.5 * t).clamp(0.0, 1.0)
                };

                let b00 = to_b(v00);
                let b10 = to_b(v10);
                let b01 = to_b(v01);
                let b11 = to_b(v11);
                let b02 = to_b(v02);
                let b12 = to_b(v12);
                let b03 = to_b(v03);
                let b13 = to_b(v13);

                // Convert to a single char by choosing dot count based on average brightness.
                // Also pick a foreground intensity to match.
                let b_avg = (b00 + b10 + b01 + b11 + b02 + b12 + b03 + b13) * 0.125;
                let dots = (b_avg * 8.0).round().clamp(0.0, 8.0) as usize;
                let ch = ramp_braille(dots);
                let (r, g, b) = (theme.map)(mean, mean_u, b_avg, &sim.params);
                let fg = Color::Rgb { r, g, b };

                v_min = v_min.min(mean);
                v_max = v_max.max(mean);
                v_sum += mean;
                b_sum += b_avg;
                cell_count += 1;

                let fi = ty * w + tx;
                let new_cell = Cell { ch, fg };

                if fi < last_frame.len() && last_frame[fi] != new_cell {
                    let screen_y = (ty + hud_rows) as u16;
                    let screen_x = tx as u16;

                    queue!(out, cursor::MoveTo(screen_x, screen_y))?;
                    if fg != cur_fg {
                        cur_fg = fg;
                        queue!(out, SetForegroundColor(cur_fg))?;
                    }
                    queue!(out, Print(ch))?;
                    last_frame[fi] = new_cell;
                }
            }
        }

        // HUD (always redraw, cheap)
        let v_avg = if cell_count > 0 {
            v_sum / cell_count as f32
        } else {
            0.0
        };
        let b_avg_frame = if cell_count > 0 {
            b_sum / cell_count as f32
        } else {
            0.0
        };
        queue!(out, cursor::MoveTo(0, 0), ResetColor)?;
        let preset_name = PRESETS[preset_idx].name;
        let line1 = format!(
            "Gray–Scott  preset:{}  theme:{}  paused:{}  steps/frame:{}  fps:{:>5.1}",
            preset_name,
            theme.name,
            if paused { "yes" } else { "no " },
            steps_per_frame,
            fps
        );
        let line2 = format!(
            "F:{:.4}  k:{:.4}  Du:{:.2}  Dv:{:.2}  contrast:{:.1}  v_mid:{:.2}  brush:{:>2}  keys: Q quit  SPACE pause  P preset  T theme  R reset  arrows tweak  +/- speed  C/X contrast  [ ] mid  , . brush",
            sim.params.feed,
            sim.params.kill,
            sim.params.du,
            sim.params.dv,
            contrast,
            v_mid,
            paint_radius_cells
        );
        let line3 = format!(
            "v[min:{:.3} max:{:.3} avg:{:.3}]  b_avg:{:.3}  sim:{}x{}  term:{}x{}",
            v_min,
            v_max,
            v_avg,
            b_avg_frame,
            sim_w_cur,
            sim_h_cur,
            w,
            h
        );

        // Clear HUD lines (pad to width)
        let pad1 = if line1.len() < w { " ".repeat(w - line1.len()) } else { String::new() };
        let pad2 = if line2.len() < w { " ".repeat(w - line2.len()) } else { String::new() };
        let pad3 = if line3.len() < w { " ".repeat(w - line3.len()) } else { String::new() };
        queue!(out, Print(line1), Print(pad1))?;
        queue!(out, cursor::MoveTo(0, 1), Print(line2), Print(pad2))?;
        queue!(out, cursor::MoveTo(0, 2), Print(line3), Print(pad3))?;

        queue!(out, ResetColor, EndSynchronizedUpdate)?;
        out.flush()?;

        // FPS estimate
        frames += 1;
        if fps_timer.elapsed() >= Duration::from_millis(500) {
            let secs = fps_timer.elapsed().as_secs_f32();
            fps = frames as f32 / secs;
            fps_timer = Instant::now();
            frames = 0;
        }

        // Frame cap (about 30 fps)
        let target = Duration::from_millis(33);
        let elapsed = last_present.elapsed();
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
        last_present = Instant::now();
    }
}

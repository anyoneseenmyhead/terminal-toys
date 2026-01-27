// src/main.rs
use clap::Parser;
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
use std::cmp::min;
use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

#[derive(Parser, Debug, Clone)]
#[command(name = "liquid_motion_toy")]
#[command(about = "CLI liquid motion desk-toy simulation (headless-friendly)", long_about = None)]
struct Args {
    /// FPS cap (render rate). Physics runs at fixed dt.
    #[arg(long, default_value_t = 60)]
    fps: u64,

    /// Quality preset: 0=low, 1=med, 2=high
    #[arg(long, default_value_t = 1)]
    quality: u8,

    /// Hide HUD (still shows a minimal hint line)
    #[arg(long, default_value_t = false)]
    no_hud: bool,

    /// Start in paused state
    #[arg(long, default_value_t = false)]
    paused: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cell {
    ch: char,
    fg: Color,
    bg: Color,
}

impl Cell {
    fn new(ch: char, fg: Color, bg: Color) -> Self {
        Self { ch, fg, bg }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

impl Rgb {
    fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
    fn lerp(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let r = (self.r as f32 + (other.r as f32 - self.r as f32) * t).round() as i32;
        let g = (self.g as f32 + (other.g as f32 - self.g as f32) * t).round() as i32;
        let b = (self.b as f32 + (other.b as f32 - self.b as f32) * t).round() as i32;
        Rgb::new(r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8)
    }
    fn to_color(self) -> Color {
        Color::Rgb {
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }
    fn scale(self, s: f32) -> Rgb {
        let s = s.clamp(0.0, 2.0);
        let r = (self.r as f32 * s).round() as i32;
        let g = (self.g as f32 * s).round() as i32;
        let b = (self.b as f32 * s).round() as i32;
        Rgb::new(r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8)
    }
}

#[inline]
fn clamp_i32(v: i32, a: i32, b: i32) -> i32 {
    v.max(a).min(b)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;
    execute!(
        stdout,
        EnterAlternateScreen,
        DisableLineWrap,
        cursor::Hide
    )?;

    let res = run(&mut stdout, args);

    execute!(
        stdout,
        EndSynchronizedUpdate,
        ResetColor,
        cursor::Show,
        EnableLineWrap,
        LeaveAlternateScreen
    )?;
    terminal::disable_raw_mode()?;

    res
}

fn run(stdout: &mut Stdout, args: Args) -> io::Result<()> {
    // Terminal size
    let (tw, th) = terminal::size()?;
    if tw < 60 || th < 18 {
        queue!(
            stdout,
            BeginSynchronizedUpdate,
            cursor::MoveTo(0, 0),
            Print("Terminal too small. Need at least 60x18.\r\n"),
            EndSynchronizedUpdate
        )?;
        stdout.flush()?;
        std::thread::sleep(Duration::from_millis(1200));
        return Ok(());
    }

    let hud_rows: u16 = if args.no_hud { 1 } else { 3 };
    let play_rows = th.saturating_sub(hud_rows).max(8);
    let play_cols = tw;

    // Braille "pixel" resolution: 2x4 subpixels per cell.
    let px_w = (play_cols as usize) * 2;
    let px_h = (play_rows as usize) * 4;

    let mut sim = Sim::new(px_w, px_h, args.quality);

    let mut renderer = Renderer::new(play_cols as usize, play_rows as usize, hud_rows as usize);
    renderer.clear_all(stdout)?;

    let dt_sim = 1.0 / 120.0;
    let mut acc = 0.0f32;
    let mut last = Instant::now();

    let mut paused = args.paused;

    let fps_cap = args.fps.clamp(10, 240);
    let frame_dt = Duration::from_millis((1000.0 / fps_cap as f32).round() as u64);
    let mut next_frame = Instant::now();

    // Input state
    let mut tilt_target: f32 = 0.0; // radians, positive tilts right
    let tilt_max = 0.55;
    let mut left_down = false;
    let mut right_down = false;

    // Simple FPS estimate
    let mut fps_smoothed = 60.0f32;
    let mut frame_counter = 0u32;
    let mut fps_stamp = Instant::now();

    loop {
        // ----- input (non-blocking) -----
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) => match k.kind {
                    KeyEventKind::Press | KeyEventKind::Repeat => match k.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(()),
                        KeyCode::Char(' ') => paused = !paused,
                        KeyCode::Char('r') | KeyCode::Char('R') => sim.reset(),
                        KeyCode::Char('f') | KeyCode::Char('F') => sim.flip(),
                        KeyCode::Char('c') | KeyCode::Char('C') => sim.jostle(),
                        KeyCode::Left => {
                            left_down = true;
                            right_down = false;
                        }
                        KeyCode::Right => {
                            right_down = true;
                            left_down = false;
                        }
                        KeyCode::Down => tilt_target *= 0.85,
                        KeyCode::Up => tilt_target *= 0.95,
                        KeyCode::Char('0') => sim.set_quality(0),
                        KeyCode::Char('1') => sim.set_quality(1),
                        KeyCode::Char('2') => sim.set_quality(2),
                        _ => {}
                    },
                    KeyEventKind::Release => match k.code {
                        KeyCode::Left => left_down = false,
                        KeyCode::Right => right_down = false,
                        _ => {}
                    },
                    _ => {}
                },
                Event::Resize(_, _) => {
                    // For simplicity, exit with a message. Keeps "ready to ship" predictable.
                    renderer.clear_all(stdout)?;
                    queue!(
                        stdout,
                        BeginSynchronizedUpdate,
                        cursor::MoveTo(0, 0),
                        Print("Resize detected. Please restart the program.\r\n"),
                        EndSynchronizedUpdate
                    )?;
                    stdout.flush()?;
                    return Ok(());
                }
                _ => {}
            }
        }

        // ----- timing -----
        let now = Instant::now();
        let mut dt = (now - last).as_secs_f32();
        last = now;

        // Clamp large dt spikes (SSH hiccups)
        dt = dt.clamp(0.0, 0.05);

        let input = (right_down as i32 - left_down as i32) as f32;
        let tilt_rate = 1.6;
        tilt_target = (tilt_target + input * tilt_rate * dt).clamp(-tilt_max, tilt_max);

        if !paused {
            acc += dt;
        }

        // ----- simulate at fixed dt -----
        while acc >= dt_sim {
            sim.set_tilt_target(tilt_target);
            sim.step(dt_sim);
            acc -= dt_sim;
        }

        // ----- render at fps cap -----
        if Instant::now() < next_frame {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }
        next_frame += frame_dt;

        // Build braille cells from sim pixel buffer
        let frame = sim.render_to_cells(play_cols as usize, play_rows as usize);

        // HUD
        let hud = if args.no_hud {
            vec![format!(
                "Q quit | Space pause | ←/→ tilt | F flip | C jostle | R reset | 0/1/2 quality | tilt:{:+.2}",
                sim.tilt
            )]
        } else {
            vec![
                format!(
                    "Liquid Motion Toy  |  Space: {}  |  Q quit  |  R reset  |  F flip  |  C jostle",
                    if paused { "resume" } else { "pause " }
                ),
                format!(
                    "Tilt: {:+.2}  Target: {:+.2}  Quality: {}  Interface: waves+slosh  |  ←/→ tilt, ↓ soften",
                    sim.tilt, tilt_target, sim.quality
                ),
                format!(
                    "FPS: {:>5.1}  Grid: {}x{} px  (terminal {}x{}; braille 2x4)",
                    fps_smoothed, sim.w, sim.h, play_cols, play_rows
                ),
            ]
        };

        renderer.draw(stdout, &hud, &frame)?;

        // FPS estimate
        frame_counter += 1;
        let elapsed = fps_stamp.elapsed().as_secs_f32();
        if elapsed >= 0.5 {
            let inst = frame_counter as f32 / elapsed;
            fps_smoothed = lerp(fps_smoothed, inst, 0.35);
            frame_counter = 0;
            fps_stamp = Instant::now();
        }
    }
}

// ---------------- Simulation ----------------

struct Sim {
    w: usize,
    h: usize,
    quality: u8,
    boat_scale: f32,

    // Interface height per x (in pixel coordinates, 0..h-1, where y increases downward)
    iface: Vec<f32>,
    iface_v: Vec<f32>,

    // Forcing and tilt
    tilt: f32,
    tilt_target: f32,
    tilt_v: f32,
    flipped: bool,

    // Boat state (in pixel coords)
    boat_x: f32,
    boat_y: f32,
    boat_vx: f32,
    boat_vy: f32,
    boat_ang: f32,
    boat_av: f32,

    // Render buffers
    // per-pixel intensity [0..1], and per-pixel color
    inten: Vec<f32>,
    col: Vec<Rgb>,

    // Jostle timer
    jostle_t: f32,

    // Frame time
    t: f32,
}

impl Sim {
    fn new(w: usize, h: usize, quality: u8) -> Self {
        let mut s = Self {
            w,
            h,
            quality: quality.min(2),
            boat_scale: 1.45,
            iface: vec![0.0; w],
            iface_v: vec![0.0; w],
            tilt: 0.0,
            tilt_target: 0.0,
            tilt_v: 0.0,
            flipped: false,
            boat_x: (w as f32) * 0.65,
            boat_y: (h as f32) * 0.45,
            boat_vx: 0.0,
            boat_vy: 0.0,
            boat_ang: 0.0,
            boat_av: 0.0,
            inten: vec![0.0; w * h],
            col: vec![Rgb::new(0, 0, 0); w * h],
            jostle_t: 0.0,
            t: 0.0,
        };
        s.reset();
        s
    }

    fn set_quality(&mut self, q: u8) {
        self.quality = q.min(2);
    }

    fn reset(&mut self) {
        self.t = 0.0;
        self.flipped = false;
        self.tilt = 0.0;
        self.tilt_target = 0.0;
        self.tilt_v = 0.0;
        self.jostle_t = 0.0;

        let base = (self.h as f32) * 0.52;
        for x in 0..self.w {
            let xf = x as f32 / (self.w as f32 - 1.0);
            let ripple = (xf * 6.0).sin() * 0.6 + (xf * 13.0).sin() * 0.25;
            self.iface[x] = base + ripple;
            self.iface_v[x] = 0.0;
        }

        self.boat_x = (self.w as f32) * 0.70;
        self.boat_vx = 0.0;
        self.boat_vy = 0.0;
        let iy = self.sample_iface(self.boat_x);
        let draft = 4.8 * self.boat_scale.powf(0.85);
        self.boat_y = iy - draft;
        self.boat_ang = 0.0;
        self.boat_av = 0.0;
    }

    fn flip(&mut self) {
        self.flipped = !self.flipped;
        // Flip interface about midline, invert velocities for a nice "release"
        let mid = (self.h as f32) * 0.52;
        for x in 0..self.w {
            let d = self.iface[x] - mid;
            self.iface[x] = mid - d;
            self.iface_v[x] = -self.iface_v[x] * 0.8;
        }
        // Nudge the boat to the new interface
        let iy = self.sample_iface(self.boat_x);
        let draft = 4.8 * self.boat_scale.powf(0.85);
        self.boat_y = iy - draft;
        self.boat_vy *= -0.4;
    }

    fn jostle(&mut self) {
        self.jostle_t = 0.55;
        // Add a quick shake into interface velocity
        for x in 0..self.w {
            let xf = x as f32 / (self.w as f32 - 1.0);
            let kick = ((xf * 29.0).sin() * 0.9 + (xf * 7.0).cos() * 0.6) * 3.0;
            self.iface_v[x] += kick;
        }
        self.boat_vx += 30.0 * (self.frand(self.t) - 0.5);
        self.boat_vy += 18.0 * (self.frand(self.t + 10.0) - 0.5);
        self.boat_av += 1.2 * (self.frand(self.t + 20.0) - 0.5);
    }

    fn set_tilt_target(&mut self, target: f32) {
        self.tilt_target = target.clamp(-0.60, 0.60);
    }

    fn step(&mut self, dt: f32) {
        self.t += dt;

        // Smooth tilt (second-order response)
        let k = 55.0;
        let d = 14.0;
        let a = k * (self.tilt_target - self.tilt) - d * self.tilt_v;
        self.tilt_v += a * dt;
        self.tilt += self.tilt_v * dt;

        // "Gravity" direction: flip changes which side feels heavy.
        let g = if self.flipped { -1.0 } else { 1.0 };

        // Interface dynamics: damped wave + spring toward tilted equilibrium plane
        let base = (self.h as f32) * 0.52;
        let center = (self.w as f32 - 1.0) * 0.5;

        let (c2, spring, visc) = match self.quality {
            0 => (85.0, 12.0, 3.2),
            1 => (120.0, 15.0, 2.6),
            _ => (155.0, 18.0, 2.2),
        };

        let slope_scale = (self.h as f32) * 0.16 * 1.15;
        let jostle = if self.jostle_t > 0.0 {
            self.jostle_t = (self.jostle_t - dt).max(0.0);
            smoothstep(0.0, 0.55, self.jostle_t) * 1.0
        } else {
            0.0
        };

        // Update interface
        for x in 0..self.w {
            let xm = if x == 0 { 0 } else { x - 1 };
            let xp = if x + 1 >= self.w { self.w - 1 } else { x + 1 };
            let lap = self.iface[xm] - 2.0 * self.iface[x] + self.iface[xp];

            let dx = x as f32 - center;
            let target = base + g * dx * self.tilt * (slope_scale / center.max(1.0));

            // small procedural slosh detail
            let slosh = (self.t * 1.7 + x as f32 * 0.07).sin() * 0.12
                + (self.t * 0.9 - x as f32 * 0.045).cos() * 0.09;

            let force = c2 * lap - spring * (self.iface[x] - target) - visc * self.iface_v[x] + slosh * 0.8;
            self.iface_v[x] += force * dt;

            // Jostle adds micro energy
            if jostle > 0.0 {
                let n = (self.t * 21.0 + x as f32 * 0.31).sin() * 0.9 + (self.t * 17.0 - x as f32 * 0.21).cos();
                self.iface_v[x] += n * 2.4 * jostle;
            }

            let vmax = match self.quality {
                0 => 70.0,
                1 => 90.0,
                _ => 110.0,
            };
            self.iface_v[x] = self.iface_v[x].clamp(-vmax, vmax);
        }
        for x in 0..self.w {
            self.iface[x] += self.iface_v[x] * dt;
            self.iface[x] = self.iface[x].clamp((self.h as f32) * 0.18, (self.h as f32) * 0.86);
        }

        // Boat: rides interface with spring, drifts laterally with tilt + local slope
        let boat_w = 20.0 * self.boat_scale;
        let boat_h = 7.0 * self.boat_scale;

        // Keep inside container margins
        let left = 4.0 + boat_w * 0.5;
        let right = (self.w as f32) - 5.0 - boat_w * 0.5;

        let offsets = [-boat_w * 0.25, 0.0, boat_w * 0.25];
        let mut iy = 0.0;
        let mut slope = 0.0;
        for off in offsets {
            iy += self.sample_iface(self.boat_x + off);
            slope += self.sample_iface_slope(self.boat_x + off);
        }
        iy /= offsets.len() as f32;
        slope /= offsets.len() as f32;

        // Draft: boat sits slightly above interface, but on the denser side
        let draft = 4.8 * self.boat_scale.powf(0.85);
        let target_y = iy - draft;

        let ky = 45.0;
        let dy = if self.boat_scale > 1.3 { 12.0 } else { 10.5 };
        let ay = ky * (target_y - self.boat_y) - dy * self.boat_vy;
        self.boat_vy += ay * dt;
        self.boat_y += self.boat_vy * dt;

        // Slide along the interface under tilt and slope
        let g_parallel = (self.tilt * 140.0) * g;
        let slope_pull = (-slope * 75.0) * g;

        let kx = 0.0;
        let drag = 3.2;
        let ax = kx * 0.0 + g_parallel + slope_pull - drag * self.boat_vx;
        self.boat_vx += ax * dt;
        self.boat_x += self.boat_vx * dt;

        if self.boat_x < left {
            self.boat_x = left;
            self.boat_vx *= -0.25;
        } else if self.boat_x > right {
            self.boat_x = right;
            self.boat_vx *= -0.25;
        }

        // Angle follows local slope + tilt, with inertia
        let target_ang = (slope * 0.9 + self.tilt * 0.45).clamp(-0.55, 0.55);
        let ka = 18.0;
        let da = 5.6;
        let aa = ka * (target_ang - self.boat_ang) - da * self.boat_av;
        self.boat_av += aa * dt;
        self.boat_ang += self.boat_av * dt;

        // Keep boat near interface bounds visually
        self.boat_y = self.boat_y.clamp((self.h as f32) * 0.12, (self.h as f32) * 0.90);

        // Mild energy bleed
        self.boat_vx *= (1.0 - dt * 0.02).max(0.0);
        self.boat_vy *= (1.0 - dt * 0.02).max(0.0);
        self.iface_v.iter_mut().for_each(|v| *v *= (1.0 - dt * 0.004).max(0.0));
    }

    fn sample_iface(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, (self.w - 1) as f32);
        let x0 = x.floor() as usize;
        let x1 = min(x0 + 1, self.w - 1);
        let t = x - x0 as f32;
        self.iface[x0] * (1.0 - t) + self.iface[x1] * t
    }

    fn sample_iface_slope(&self, x: f32) -> f32 {
        let x = x.clamp(1.0, (self.w - 2) as f32);
        let x0 = x.floor() as usize;
        let xm = x0.saturating_sub(1);
        let xp = min(x0 + 1, self.w - 1);
        (self.iface[xp] - self.iface[xm]) * 0.5
    }

    fn frand(&self, seed: f32) -> f32 {
        // Deterministic-ish hash noise (0..1)
        let s = (seed * 12.9898 + 78.233).sin() * 43758.5453;
        s.fract().abs()
    }

    fn render_to_cells(&mut self, cols: usize, rows: usize) -> Vec<Cell> {
        // Render into pixel buffers (self.inten, self.col)
        self.draw_scene();

        // Downsample pixels to braille cells
        // rows/cols are in terminal cells for the playfield
        let mut out = vec![Cell::new(' ', Color::Reset, Color::Reset); cols * rows];

        let bg = Rgb::new(7, 8, 12); // container/void
        let glass = Rgb::new(120, 150, 170);
        let glass2 = Rgb::new(60, 80, 90);

        for cy in 0..rows {
            for cx in 0..cols {
                let px0 = cx * 2;
                let py0 = cy * 4;

                // Collect 2x4 pixels
                let mut bits: u8 = 0;
                let mut sum = 0.0f32;
                let mut cnt = 0.0f32;
                let mut rsum = 0.0f32;
                let mut gsum = 0.0f32;
                let mut bsum = 0.0f32;

                for sy in 0..4 {
                    for sx in 0..2 {
                        let x = px0 + sx;
                        let y = py0 + sy;
                        if x >= self.w || y >= self.h {
                            continue;
                        }
                        let i = y * self.w + x;
                        let a = self.inten[i].clamp(0.0, 1.0);
                        let c = self.col[i];
                        sum += a;
                        cnt += 1.0;
                        rsum += (c.r as f32) * a;
                        gsum += (c.g as f32) * a;
                        bsum += (c.b as f32) * a;

                        // Braille bit mapping:
                        // (sx,sy) -> dot index
                        // left column: 1,2,3,7 ; right column: 4,5,6,8
                        // sy 0..3
                        let dot = match (sx, sy) {
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

                        // Per-subpixel thresholding, slightly biased to keep interface crisp
                        let thr = 0.46;
                        if a > thr {
                            bits |= 1 << dot;
                        }
                    }
                }

                let mean = if cnt > 0.0 { sum / cnt } else { 0.0 };
                let fill_frac = (bits.count_ones() as f32) / 8.0;

                // Compute an "avg" color weighted by intensity
                let (mut avg, mut alpha) = (Rgb::new(0, 0, 0), mean);
                if mean > 0.0001 {
                    avg = Rgb::new(
                        (rsum / sum).round().clamp(0.0, 255.0) as u8,
                        (gsum / sum).round().clamp(0.0, 255.0) as u8,
                        (bsum / sum).round().clamp(0.0, 255.0) as u8,
                    );
                } else {
                    alpha = 0.0;
                }

                // Add subtle "glass" edge hint using background tint near borders
                let is_border = cx == 0 || cy == 0 || cx + 1 == cols || cy + 1 == rows;
                let bg_col = if is_border {
                    let t = 0.55;
                    bg.lerp(glass2, t)
                } else {
                    bg
                };

                let idx = cy * cols + cx;

                if fill_frac < 0.06 || alpha < 0.06 {
                    // Empty
                    let ch = if is_border { ' ' } else { ' ' };
                    out[idx] = Cell::new(ch, Color::Reset, bg_col.to_color());
                    continue;
                }

                if fill_frac > 0.92 {
                    // Full cell: use background color fill for smoothness
                    let mut bcol = avg;
                    if is_border {
                        bcol = bcol.lerp(glass, 0.18);
                    }
                    out[idx] = Cell::new(' ', Color::Reset, bcol.to_color());
                    continue;
                }

                let ch = char::from_u32(0x2800 + bits as u32).unwrap_or('⣿');
                let mut fg = avg;
                if is_border {
                    fg = fg.lerp(glass, 0.22);
                }
                out[idx] = Cell::new(ch, fg.to_color(), bg_col.to_color());
            }
        }

        out
    }

    fn draw_scene(&mut self) {
        let w = self.w;
        let h = self.h;

        // Palette
        let bg = Rgb::new(7, 8, 12);

        let top0 = Rgb::new(20, 80, 160);
        let top1 = Rgb::new(60, 150, 220);

        let bot0 = Rgb::new(10, 35, 95);
        let bot1 = Rgb::new(25, 70, 140);

        let iface_glow = Rgb::new(160, 220, 255);
        let foam = Rgb::new(210, 245, 255);

        // Clear buffers
        for i in 0..(w * h) {
            self.inten[i] = 0.0;
            self.col[i] = bg;
        }

        // Container margins in pixels
        let mx = 2usize;
        let my = 2usize;
        let left = mx;
        let right = w.saturating_sub(mx + 1);
        let top = my;
        let bottom = h.saturating_sub(my + 1);

        // Draw border glass
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let border = x <= left || x >= right || y <= top || y >= bottom;
                if border {
                    let d = (x.min(w - 1 - x) + y.min(h - 1 - y)) as f32;
                    let t = (0.15 + 0.06 * (d / 6.0).min(1.0)).clamp(0.0, 0.35);
                    self.inten[i] = 0.22 + t;
                    self.col[i] = Rgb::new(90, 120, 140).lerp(Rgb::new(140, 170, 190), t);
                }
            }
        }

        // Liquids + interface band
        for x in left..=right {
            let hx = self.iface[x].clamp(top as f32 + 2.0, bottom as f32 - 2.0);

            for y in top..=bottom {
                let i = y * w + x;

                // Procedural "depth shading"
                let yn = y as f32 / (h as f32 - 1.0);
                let xn = x as f32 / (w as f32 - 1.0);
                let vign = 1.0 - (xn - 0.5).abs() * 0.9 - (yn - 0.5).abs() * 0.7;
                let vign = vign.clamp(0.2, 1.0);

                // Determine which fluid
                let dy = (y as f32) - hx;
                let band = (-dy.abs() + 2.2).clamp(0.0, 2.2) / 2.2; // 0..1 near interface
                let wob = (self.t * 0.7 + xn * 9.0).sin() * 0.015 + (self.t * 1.1 - yn * 8.0).cos() * 0.012;

                let in_top = dy < 0.0;
                let (c0, c1) = if in_top { (top0, top1) } else { (bot0, bot1) };

                let depth = if in_top {
                    smoothstep(0.0, 1.0, yn * 1.05)
                } else {
                    smoothstep(0.0, 1.0, (1.0 - yn) * 1.05)
                };

                let mut c = c0.lerp(c1, depth);
                c = c.scale((0.92 + wob).clamp(0.75, 1.1));
                c = c.scale(vign);

                let mut a = 0.55;

                // Add interface glow and foam specks
                if band > 0.01 {
                    let glow = iface_glow.lerp(foam, (self.frand(self.t * 2.3 + x as f32 * 0.17 + y as f32 * 0.11)));
                    c = c.lerp(glow, band * 0.55);
                    a = lerp(a, 0.92, band * 0.65);

                    // sparse foam dots, but stable-ish
                    let n = (self.frand(self.t * 0.3 + x as f32 * 0.31 + y as f32 * 0.19));
                    if n > 0.985 {
                        c = foam;
                        a = 0.95;
                    }
                }

                // Slight darkening near walls inside container
                let wall_d = (x - left).min(right - x).min((y - top).min(bottom - y)) as f32;
                let wall_t = (1.0 - (wall_d / 8.0).clamp(0.0, 1.0)) * 0.18;
                c = c.lerp(Rgb::new(8, 10, 14), wall_t);

                self.col[i] = c;
                self.inten[i] = a;
            }
        }

        // Draw a small "rock/iceberg" hint on the left, like the photo
        self.draw_iceberg(left as i32 + 10, top as i32 + 7);

        // Draw the boat on top of interface
        self.draw_boat();

        // Subtle highlight on glass border
        for x in left..=right {
            let y = top;
            let i = y * w + x;
            self.col[i] = self.col[i].lerp(Rgb::new(170, 200, 220), 0.08);
            self.inten[i] = (self.inten[i] + 0.08).clamp(0.0, 1.0);
        }
    }

    fn draw_iceberg(&mut self, x0: i32, y0: i32) {
        let w = self.w as i32;
        let h = self.h as i32;

        for y in 0..18 {
            for x in 0..18 {
                let xx = x0 + x;
                let yy = y0 + y;
                if xx < 0 || yy < 0 || xx >= w || yy >= h {
                    continue;
                }
                let xf = x as f32 / 18.0;
                let yf = y as f32 / 18.0;
                let tri = (1.0 - (xf - 0.5).abs() * 2.2) * (1.0 - yf * 1.2);
                if tri <= 0.0 {
                    continue;
                }
                let n = (self.frand(xx as f32 * 0.21 + yy as f32 * 0.37) - 0.5) * 0.12;
                let shade = (0.65 + tri * 0.55 + n).clamp(0.45, 1.0);
                let col = Rgb::new(190, 210, 225).lerp(Rgb::new(230, 245, 255), tri * 0.45).scale(shade);
                let i = (yy as usize) * self.w + (xx as usize);
                self.col[i] = self.col[i].lerp(col, 0.55);
                self.inten[i] = (self.inten[i] + tri * 0.22).clamp(0.0, 1.0);
            }
        }
    }

    fn draw_boat(&mut self) {
        let w = self.w as i32;
        let h = self.h as i32;

        let boat_w = 22.0 * self.boat_scale;
        let boat_h = 8.0 * self.boat_scale;

        let cx = self.boat_x;
        let cy = self.boat_y;

        let ang = self.boat_ang;
        let ca = ang.cos();
        let sa = ang.sin();

        // boat colors
        let hull = Rgb::new(18, 22, 28);
        let deck = Rgb::new(32, 38, 45);
        let trim = Rgb::new(220, 230, 240);

        let smokestack = Rgb::new(12, 14, 18);

        // helper: transform local -> world
        let to_world = |lx: f32, ly: f32| -> (f32, f32) {
            let x = lx * ca - ly * sa + cx;
            let y = lx * sa + ly * ca + cy;
            (x, y)
        };

        // Fill hull: a rounded-ish capsule
        let sx = (30.0 * self.boat_scale).round().max(12.0) as i32;
        let sy = (12.0 * self.boat_scale).round().max(6.0) as i32;
        for py in -sy..=sy {
            for px in -sx..=sx {
                let lx = (px as f32) * (boat_w / (sx as f32)) * 0.5;
                let ly = (py as f32) * (boat_h / (sy as f32)) * 0.5;

                // Hull shape in local space
                let xnorm = lx / (boat_w * 0.5);
                let ynorm = ly / (boat_h * 0.5);

                // Base capsule
                let cap = (xnorm * xnorm) / 1.05 + (ynorm * ynorm) / 0.85;
                if cap > 1.0 {
                    continue;
                }

                // Carve top to make hull line
                let deck_cut = ynorm < -0.20;
                let (wx, wy) = to_world(lx, ly);
                let ix = wx.round() as i32;
                let iy = wy.round() as i32;
                if ix < 0 || iy < 0 || ix >= w || iy >= h {
                    continue;
                }

                let i = (iy as usize) * self.w + (ix as usize);

                let mut c = if deck_cut { deck } else { hull };
                // highlight rim
                let rim = (1.0 - cap).clamp(0.0, 1.0);
                c = c.lerp(trim, rim * 0.12);

                // Blend on top of fluid
                self.col[i] = self.col[i].lerp(c, 0.88);
                self.inten[i] = (self.inten[i] + 0.30).clamp(0.0, 1.0);
            }
        }

        // Draw smokestacks
        for s in 0..3 {
            let sx = (-6.0 + s as f32 * 4.2) * self.boat_scale;
            let sy = -5.5 * self.boat_scale;
            let stack_h = (6.0 * self.boat_scale).round().max(2.0) as i32;
            let stack_w = (2.0 * self.boat_scale).round().max(2.0) as i32;
            for py in 0..stack_h {
                for px in 0..stack_w {
                    let lx = sx + px as f32;
                    let ly = sy - py as f32;
                    let (wx, wy) = to_world(lx, ly);
                    let ix = wx.round() as i32;
                    let iy = wy.round() as i32;
                    if ix < 0 || iy < 0 || ix >= w || iy >= h {
                        continue;
                    }
                    let i = (iy as usize) * self.w + (ix as usize);
                    self.col[i] = self.col[i].lerp(smokestack, 0.90);
                    self.inten[i] = (self.inten[i] + 0.25).clamp(0.0, 1.0);

                    // faint smoke
                    if s == 1 && py > 3 {
                        let puff = (self.frand(self.t * 2.0 + ix as f32 * 0.17 + iy as f32 * 0.11));
                        if puff > 0.78 {
                            self.col[i] = self.col[i].lerp(Rgb::new(210, 220, 230), 0.25);
                            self.inten[i] = (self.inten[i] + 0.10).clamp(0.0, 1.0);
                        }
                    }
                }
            }
        }
    }
}

// ---------------- Renderer (diff-based, flicker-resistant) ----------------

struct Renderer {
    term_w: usize,
    term_h_total: usize,
    hud_rows: usize,
    play_rows: usize,

    prev: Vec<Cell>,
}

impl Renderer {
    fn new(term_w: usize, play_rows: usize, hud_rows: usize) -> Self {
        let term_h_total = play_rows + hud_rows;
        let prev = vec![Cell::new('\0', Color::Reset, Color::Reset); term_w * play_rows];
        Self {
            term_w,
            term_h_total,
            hud_rows,
            play_rows,
            prev,
        }
    }

    fn clear_all(&mut self, stdout: &mut Stdout) -> io::Result<()> {
        queue!(stdout, BeginSynchronizedUpdate)?;
        // Clear whole screen by painting with background; avoid terminal::Clear flashes.
        for y in 0..(self.term_h_total as u16) {
            queue!(
                stdout,
                cursor::MoveTo(0, y),
                SetBackgroundColor(Color::Rgb { r: 7, g: 8, b: 12 }),
                SetForegroundColor(Color::Rgb { r: 200, g: 210, b: 220 }),
                Print(" ".repeat(self.term_w)),
                ResetColor
            )?;
        }
        queue!(stdout, EndSynchronizedUpdate)?;
        stdout.flush()?;
        // Reset prev so next draw writes everything
        for c in self.prev.iter_mut() {
            c.ch = '\0';
        }
        Ok(())
    }

    fn draw(&mut self, stdout: &mut Stdout, hud_lines: &[String], frame: &[Cell]) -> io::Result<()> {
        queue!(stdout, BeginSynchronizedUpdate)?;

        // Draw HUD (simple full-line prints; small, does not cause major flicker)
        self.draw_hud(stdout, hud_lines)?;

        // Draw playfield diff
        let w = self.term_w;
        let h = self.play_rows;

        for y in 0..h {
            let row_off = y * w;
            let screen_y = (self.hud_rows + y) as u16;

            // We still do per-cell diff, but we group runs by color to reduce queue volume.
            let mut x = 0usize;
            while x < w {
                let i = row_off + x;
                let cur = frame[i];

                if cur == self.prev[i] {
                    x += 1;
                    continue;
                }

                // Start run
                let run_fg = cur.fg;
                let run_bg = cur.bg;

                let mut end = x + 1;
                while end < w {
                    let j = row_off + end;
                    let cj = frame[j];
                    if cj == self.prev[j] {
                        break;
                    }
                    if cj.fg != run_fg || cj.bg != run_bg {
                        break;
                    }
                    end += 1;
                }

                // Write run
                queue!(
                    stdout,
                    cursor::MoveTo(x as u16, screen_y),
                    SetForegroundColor(run_fg),
                    SetBackgroundColor(run_bg),
                )?;

                for xx in x..end {
                    let j = row_off + xx;
                    let ch = frame[j].ch;
                    queue!(stdout, Print(ch))?;
                    self.prev[j] = frame[j];
                }

                x = end;
            }
        }

        queue!(stdout, ResetColor, EndSynchronizedUpdate)?;
        stdout.flush()?;
        Ok(())
    }

    fn draw_hud(&self, stdout: &mut Stdout, lines: &[String]) -> io::Result<()> {
        let bg = Color::Rgb { r: 7, g: 8, b: 12 };
        let fg = Color::Rgb { r: 200, g: 210, b: 220 };
        let dim = Color::Rgb { r: 150, g: 160, b: 170 };

        for y in 0..self.hud_rows {
            let s = lines.get(y).map(|x| x.as_str()).unwrap_or("");
            let mut line = s.to_string();
            if line.len() > self.term_w {
                line.truncate(self.term_w);
            } else if line.len() < self.term_w {
                line.push_str(&" ".repeat(self.term_w - line.len()));
            }

            let color = if y == 0 { fg } else { dim };
            queue!(
                stdout,
                cursor::MoveTo(0, y as u16),
                SetBackgroundColor(bg),
                SetForegroundColor(color),
                Print(line),
                ResetColor
            )?;
        }
        Ok(())
    }
}

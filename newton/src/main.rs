// src/main.rs
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use std::f32::consts::PI;
use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default)]
struct Vec2 {
    x: f32,
    y: f32,
}
impl Vec2 {
    fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y
    }
    fn len(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    fn norm(self) -> Self {
        let l = self.len();
        if l > 1e-8 {
            Self::new(self.x / l, self.y / l)
        } else {
            Self::new(1.0, 0.0)
        }
    }
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y)
    }
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y)
    }
    fn mul(self, k: f32) -> Self {
        Self::new(self.x * k, self.y * k)
    }
}

#[derive(Clone, Copy, Debug)]
struct Ball {
    pivot: Vec2, // fixed point
    pos: Vec2,   // current
    prev: Vec2,  // previous (for velocity derivation)
    vel: Vec2,   // derived each step
    r: f32,      // radius
}

struct Sim {
    balls: Vec<Ball>,
    length: f32,
    gravity: f32,
    air_drag: f32,  // exponential per-second
    rest: f32,      // coefficient of restitution (ball-ball)
    slop: f32,      // collision slop
    iters: usize,   // solver iterations
    dt_fixed: f32,  // fixed step
    paused: bool,
    grab: Option<usize>,
    // view
    theme: usize,
}

const MIN_BALLS: usize = 2;
const MAX_BALLS: usize = 9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cell {
    ch: char,
    fg: Color,
}

fn braille_bit(dx: u32, dy: u32) -> u8 {
    // Braille dot numbering:
    // (0,0)->1, (0,1)->2, (0,2)->3, (0,3)->7
    // (1,0)->4, (1,1)->5, (1,2)->6, (1,3)->8
    match (dx, dy) {
        (0, 0) => 1 << 0,
        (0, 1) => 1 << 1,
        (0, 2) => 1 << 2,
        (0, 3) => 1 << 6,
        (1, 0) => 1 << 3,
        (1, 1) => 1 << 4,
        (1, 2) => 1 << 5,
        (1, 3) => 1 << 7,
        _ => 0,
    }
}

fn clamp(v: f32, a: f32, b: f32) -> f32 {
    v.max(a).min(b)
}

impl Sim {
    fn new(n: usize) -> Self {
        // World units: arbitrary.
        // We use a PBD-like loop:
        // 1) integrate (gravity)
        // 2) project constraints to pivots (fixed rod length)
        // 3) resolve ball-ball collisions
        // 4) derive velocities from (pos - prev)/dt
        let length = 10.0;
        let r = 1.05;
        let gap = 0.01; // tiny gap helps stability and avoids constant resting overlap
        let g = 45.0;

        let mut balls = Vec::with_capacity(n);
        let total_w = (n as f32 - 1.0) * (2.0 * r + gap);
        let x0 = -total_w * 0.5;
        let pivot_y = 0.0;

        for i in 0..n {
            let px = x0 + i as f32 * (2.0 * r + gap);
            let pivot = Vec2::new(px, pivot_y);
            let pos = Vec2::new(px, pivot_y + length);
            balls.push(Ball {
                pivot,
                pos,
                prev: pos,
                vel: Vec2::default(),
                r,
            });
        }

        // Give the left-most ball a starting pull as a demo
        if let Some(b0) = balls.first_mut() {
            // swing out by setting an initial angle
            let ang = (-40.0_f32).to_radians();
            let x = b0.pivot.x + length * ang.sin();
            let y = b0.pivot.y + length * ang.cos();
            b0.pos = Vec2::new(x, y);
            b0.prev = b0.pos;
        }

        Self {
            balls,
            length,
            gravity: g,
            air_drag: 0.015,
            rest: 0.999,
            slop: 0.001,
            iters: 30,
            dt_fixed: 1.0 / 940.0,
            paused: false,
            grab: None,
            theme: 0,
        }
    }

    fn rebuild(&mut self, n: usize) {
        let mut next = Sim::new(n);
        next.air_drag = self.air_drag;
        next.rest = self.rest;
        next.slop = self.slop;
        next.iters = self.iters;
        next.dt_fixed = self.dt_fixed;
        next.paused = self.paused;
        next.theme = self.theme;
        *self = next;
    }

    fn reset(&mut self) {
        let n = self.balls.len();
        let theme = self.theme;
        *self = Sim::new(n);
        self.theme = theme;
    }

    fn add_ball(&mut self) {
        let n = (self.balls.len() + 1).min(MAX_BALLS);
        self.rebuild(n);
    }

    fn remove_ball(&mut self) {
        let n = self.balls.len().saturating_sub(1).max(MIN_BALLS);
        self.rebuild(n);
    }

    fn pluck(&mut self, idx: usize, degrees: f32) {
        if idx >= self.balls.len() {
            return;
        }
        let ang = degrees.to_radians();
        let (len, p) = (self.length, self.balls[idx].pivot);
        let x = p.x + len * ang.sin();
        let y = p.y + len * ang.cos();
        self.balls[idx].pos = Vec2::new(x, y);
        self.balls[idx].prev = self.balls[idx].pos; // no initial velocity
        self.balls[idx].vel = Vec2::default();
    }

    fn step_fixed(&mut self, dt: f32) {
        if self.paused {
            return;
        }

        // if grabbed, pin that ball to mouse-like position in world space (we fake: keep it swung out)
        if let Some(i) = self.grab {
            // just hold it at a fixed offset angle for controllable "release"
            let degrees = -55.0;
            self.pluck(i, degrees);
        }

        // pre-step: compute velocities from prev
        for b in &mut self.balls {
            b.vel = b.pos.sub(b.prev).mul(1.0 / dt);
        }

        // integrate with gravity (semi-implicit in position form)
        let drag = (-self.air_drag * dt).exp();
        for b in &mut self.balls {
            b.prev = b.pos;
            b.vel.y += self.gravity * dt;
            b.vel.x *= drag;
            b.vel.y *= drag;
            b.pos = b.pos.add(b.vel.mul(dt));
        }

        // iterative constraint solve (PBD style)
        for _ in 0..self.iters {
            self.solve_pivot_constraints();
            self.solve_collisions(dt);
        }

        // post-step: update velocities from corrected positions
        for b in &mut self.balls {
            b.vel = b.pos.sub(b.prev).mul(1.0 / dt);
        }
    }

    fn solve_pivot_constraints(&mut self) {
        // enforce |pos - pivot| = length
        for b in &mut self.balls {
            let d = b.pos.sub(b.pivot);
            let dist = d.len();
            if dist < 1e-6 {
                continue;
            }
            let err = dist - self.length;
            // project back onto circle around pivot
            b.pos = b.pos.sub(d.mul(err / dist));
        }
    }

    fn solve_collisions(&mut self, dt: f32) {
        // ball-ball collisions with impulse applied by modifying positions AND velocities indirectly.
        // We do:
        // - positional separation (prevents sinking / resting overlap)
        // - velocity-level impulse along collision normal (encoded as position tweak over dt)
        let n = self.balls.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let (ri, rj) = (self.balls[i].r, self.balls[j].r);
                let min_d = ri + rj;

                let delta = self.balls[j].pos.sub(self.balls[i].pos);
                let dist = delta.len();
                if dist <= 1e-6 {
                    continue;
                }

                let overlap = min_d - dist;
                if overlap > self.slop {
                    let nrm = delta.mul(1.0 / dist);

                    // Positional correction: split separation
                    let corr = nrm.mul((overlap - self.slop) * 0.5);
                    self.balls[i].pos = self.balls[i].pos.sub(corr);
                    self.balls[j].pos = self.balls[j].pos.add(corr);

                    // Velocity impulse along normal:
                    // approximate masses equal; apply 1D Newton impact along normal.
                    let vi = self.balls[i].pos.sub(self.balls[i].prev).mul(1.0 / dt);
                    let vj = self.balls[j].pos.sub(self.balls[j].prev).mul(1.0 / dt);
                    let rel = vj.sub(vi);
                    let reln = rel.dot(nrm);

                    // only if closing
                    if reln < 0.0 {
                        let e = self.rest;
                        let jimp = -(1.0 + e) * reln * 0.5; // equal masses => divide by 2

                        // Encode impulse by adjusting positions via prev (equivalent to changing velocity).
                        // v' = v + impulse*nrm, so pos' = prev + v'*dt
                        // We keep current pos, adjust prev to produce v'
                        let dv = nrm.mul(jimp);
                        // vi' = vi - dv; vj' = vj + dv
                        // prev = pos - v'*dt
                        let vi_p = vi.sub(dv);
                        let vj_p = vj.add(dv);

                        self.balls[i].prev = self.balls[i].pos.sub(vi_p.mul(dt));
                        self.balls[j].prev = self.balls[j].pos.sub(vj_p.mul(dt));
                    }
                }
            }
        }
    }

    fn world_bounds(&self) -> (f32, f32, f32, f32) {
        // camera framing: fixed box around pivots
        let n = self.balls.len();
        let left = self.balls.first().map(|b| b.pivot.x).unwrap_or(0.0) - 16.0;
        let right = self.balls.last().map(|b| b.pivot.x).unwrap_or(0.0) + 16.0;
        let top = -6.0;
        let bottom = self.length + 6.0;
        // widen slightly based on count
        let widen = (n as f32).sqrt() * 0.6;
        (left - widen, right + widen, top, bottom)
    }
}

fn theme(sim: &Sim) -> (Color, Color) {
    // (foreground main, accent)
    match sim.theme % 5 {
        0 => (Color::Cyan, Color::White),
        1 => (Color::Green, Color::White),
        2 => (Color::Yellow, Color::White),
        3 => (Color::Magenta, Color::White),
        _ => (Color::White, Color::Grey),
    }
}

fn main() -> io::Result<()> {
    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, DisableLineWrap, cursor::Hide)?;

    let res = run(&mut stdout);

    execute!(stdout, cursor::Show, EnableLineWrap, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    res
}

fn run(stdout: &mut Stdout) -> io::Result<()> {
    let mut sim = Sim::new(5);

    // double buffer of terminal cells for diff-draw
    let mut last: Vec<Cell> = Vec::new();
    let mut now: Vec<Cell> = Vec::new();

    let mut last_tick = Instant::now();
    let mut acc = 0.0_f32;

    loop {
        // input (non-blocking)
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    match k.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(()),
                        KeyCode::Char(' ') => sim.paused = !sim.paused,
                        KeyCode::Char('r') | KeyCode::Char('R') => sim.reset(),
                        KeyCode::Char('c') | KeyCode::Char('C') => sim.theme = sim.theme.wrapping_add(1),
                        KeyCode::Char('a') | KeyCode::Char('A') => sim.pluck(0, -55.0),
                        KeyCode::Char('d') | KeyCode::Char('D') => {
                            let last = sim.balls.len().saturating_sub(1);
                            sim.pluck(last, 55.0);
                        }
                        KeyCode::Char('+') | KeyCode::Char('=') => sim.add_ball(),
                        KeyCode::Char('-') => sim.remove_ball(),
                        KeyCode::Char('g') | KeyCode::Char('G') => {
                            // toggle "grab" leftmost; press again to release
                            sim.grab = if sim.grab.is_some() { None } else { Some(0) };
                            if sim.grab.is_none() {
                                // on release, keep current pose (no velocity)
                                // leaving as-is gives a clean drop from rest
                            }
                        }
                        KeyCode::Up => sim.rest = clamp(sim.rest + 0.002, 0.90, 0.999),
                        KeyCode::Down => sim.rest = clamp(sim.rest - 0.002, 0.90, 0.999),
                        KeyCode::Left => sim.air_drag = clamp(sim.air_drag - 0.01, 0.0, 0.9),
                        KeyCode::Right => sim.air_drag = clamp(sim.air_drag + 0.01, 0.0, 0.9),
                        _ => {}
                    }
                }
                Event::Resize(_, _) => {
                    // force full redraw
                    last.clear();
                }
                _ => {}
            }
        }

        // time step
        let t = Instant::now();
        let dt = (t - last_tick).as_secs_f32();
        last_tick = t;

        // cap dt to avoid exploding on terminal stalls
        let dt = dt.min(1.0 / 20.0);
        acc += dt;

        // fixed-step integration for stable collisions/constraints
        while acc >= sim.dt_fixed {
            sim.step_fixed(sim.dt_fixed);
            acc -= sim.dt_fixed;
        }

        // render
        draw(stdout, &sim, &mut last, &mut now)?;
        // limit frame rate (also reduces ssh/terminal tearing)
        std::thread::sleep(Duration::from_millis(8));
    }
}

fn draw(stdout: &mut Stdout, sim: &Sim, last: &mut Vec<Cell>, now: &mut Vec<Cell>) -> io::Result<()> {
    let (tw, th) = terminal::size()?;
    let w = tw as usize;
    let h = th as usize;

    // Braille grid: each cell covers 2x4 subpixels.
    let px_w = w * 2;
    let px_h = h * 4;

    // allocate
    if now.len() != w * h {
        now.resize(
            w * h,
            Cell {
                ch: ' ',
                fg: Color::Reset,
            },
        );
    }
    for c in now.iter_mut() {
        c.ch = ' ';
        c.fg = Color::Reset;
    }

    // a tiny offscreen "subpixel" mask for braille
    let mut mask = vec![0u8; w * h]; // braille bits ORed into each terminal cell
    let mut col = vec![Color::Reset; w * h];

    let (fg, accent) = theme(sim);

    // map world -> subpixel coords
    let (wx0, wx1, wy0, wy1) = sim.world_bounds();
    let sx = (px_w as f32 - 1.0) / (wx1 - wx0);
    let sy = (px_h as f32 - 1.0) / (wy1 - wy0);

    let world_to_px = |p: Vec2| -> (i32, i32) {
        let x = ((p.x - wx0) * sx).round() as i32;
        let y = ((p.y - wy0) * sy).round() as i32;
        (x, y)
    };

    // draw line (rope) into subpixels (simple DDA)
    let mut plot = |x: i32, y: i32, color: Color| {
        if x < 0 || y < 0 || x >= px_w as i32 || y >= px_h as i32 {
            return;
        }
        let cx = (x as usize) / 2;
        let cy = (y as usize) / 4;
        let dx = (x as u32) % 2;
        let dy = (y as u32) % 4;
        let i = cy * w + cx;
        let bit = braille_bit(dx, dy);
        mask[i] |= bit;
        // color priority: accent overrides base
        if col[i] == Color::Reset || color == accent {
            col[i] = color;
        }
    };

    // ropes
    for b in &sim.balls {
        let (x0, y0) = world_to_px(b.pivot);
        let (x1, y1) = world_to_px(b.pos);
        let dx = x1 - x0;
        let dy = y1 - y0;
        let steps = dx.abs().max(dy.abs()).max(1);
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            let x = (x0 as f32 + dx as f32 * t).round() as i32;
            let y = (y0 as f32 + dy as f32 * t).round() as i32;
            plot(x, y, fg);
        }
    }

    // top bar (pivot rail)
    {
        let y = ((0.0 - wy0) * sy).round() as i32;
        for x in 0..(px_w as i32) {
            if x % 2 == 0 {
                plot(x, y, accent);
            }
        }
    }

    // balls (filled discs in subpixel space)
    for (i, b) in sim.balls.iter().enumerate() {
        let (cx, cy) = world_to_px(b.pos);
        let pr = (b.r * 1.10 * ((sx + sy) * 0.5)).max(4.0);
        let rr = pr * pr;
        let color = if Some(i) == sim.grab { accent } else { fg };

        let minx = (cx as f32 - pr).floor() as i32;
        let maxx = (cx as f32 + pr).ceil() as i32;
        let miny = (cy as f32 - pr).floor() as i32;
        let maxy = (cy as f32 + pr).ceil() as i32;

        for y in miny..=maxy {
            for x in minx..=maxx {
                let dx = x as f32 - cx as f32;
                let dy = y as f32 - cy as f32;
                if dx * dx + dy * dy <= rr {
                    plot(x, y, color);
                }
            }
        }

        // small highlight spec
        plot(cx - 2, cy - 2, accent);
        plot(cx - 1, cy - 2, accent);
    }

    // write braille cells into now[]
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let bits = mask[i];
            if bits != 0 {
                let ch = char::from_u32(0x2800 + bits as u32).unwrap_or('⣿');
                now[i] = Cell { ch, fg: col[i] };
            }
        }
    }

    // HUD (simple text along top row)
    let hud = format!(
        "Newton's Cradle | Space pause | A left pluck | D right pluck | +/- balls | G toggle hold | C theme | R reset | Q quit | e={:.3} drag={:.2}",
        sim.rest, sim.air_drag
    );
    let hud_y = 0usize;
    for (k, ch) in hud.chars().take(w).enumerate() {
        now[hud_y * w + k] = Cell { ch, fg: Color::White };
    }

    // diff draw
    queue!(stdout, BeginSynchronizedUpdate)?;
    if last.len() != now.len() {
        last.resize(
            now.len(),
            Cell {
                ch: '\0',
                fg: Color::Reset,
            },
        );
    }

    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let a = last[i];
            let b = now[i];
            if a.ch != b.ch || a.fg != b.fg {
                queue!(
                    stdout,
                    cursor::MoveTo(x as u16, y as u16),
                    SetForegroundColor(b.fg),
                    Print(b.ch)
                )?;
                last[i] = b;
            }
        }
    }

    queue!(stdout, ResetColor, EndSynchronizedUpdate)?;
    stdout.flush()?;
    Ok(())
}

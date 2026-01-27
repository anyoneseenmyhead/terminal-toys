use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::f32::consts::PI;
use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

const FPS_CAP: u64 = 60;
const DT_CLAMP: f32 = 0.05;

// Braille: each terminal cell is 2x4 subpixels.
const SUB_X: usize = 2;
const SUB_Y: usize = 4;

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

#[derive(Clone, Copy, Debug)]
struct Vec2 {
    x: f32,
    y: f32,
}

impl Vec2 {
    fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    fn len(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    fn norm(self) -> Self {
        let l = self.len();
        if l <= 1e-6 {
            Self::new(0.0, 0.0)
        } else {
            Self::new(self.x / l, self.y / l)
        }
    }
    fn clamp_len(self, max_len: f32) -> Self {
        let l = self.len();
        if l > max_len {
            self.norm() * max_len
        } else {
            self
        }
    }
}

use std::ops::{Add, AddAssign, Mul, Sub};

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x + rhs.x, self.y + rhs.y)
    }
}
impl AddAssign for Vec2 {
    fn add_assign(&mut self, rhs: Vec2) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}
impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x - rhs.x, self.y - rhs.y)
    }
}
impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, rhs: f32) -> Vec2 {
        Vec2::new(self.x * rhs, self.y * rhs)
    }
}

#[derive(Clone, Copy)]
struct Theme {
    name: &'static str,
    bg: Color,
    water_low: Color,
    water_mid: Color,
    water_hi: Color,
    plant: Color,
    rock: Color,
    hud: Color,
    fish_a: Color,
    fish_b: Color,
    bubble: Color,
}

fn themes() -> [Theme; 5] {
    [
        Theme {
            name: "Mint",
            bg: Color::AnsiValue(16),
            water_low: Color::AnsiValue(23),
            water_mid: Color::AnsiValue(37),
            water_hi: Color::AnsiValue(51),
            plant: Color::AnsiValue(41),
            rock: Color::AnsiValue(102),
            hud: Color::AnsiValue(159),
            fish_a: Color::AnsiValue(229),
            fish_b: Color::AnsiValue(215),
            bubble: Color::AnsiValue(159),
        },
        Theme {
            name: "Amber",
            bg: Color::AnsiValue(16),
            water_low: Color::AnsiValue(94),
            water_mid: Color::AnsiValue(136),
            water_hi: Color::AnsiValue(179),
            plant: Color::AnsiValue(100),
            rock: Color::AnsiValue(137),
            hud: Color::AnsiValue(222),
            fish_a: Color::AnsiValue(220),
            fish_b: Color::AnsiValue(214),
            bubble: Color::AnsiValue(223),
        },
        Theme {
            name: "Ice",
            bg: Color::AnsiValue(16),
            water_low: Color::AnsiValue(19),
            water_mid: Color::AnsiValue(33),
            water_hi: Color::AnsiValue(45),
            plant: Color::AnsiValue(37),
            rock: Color::AnsiValue(103),
            hud: Color::AnsiValue(153),
            fish_a: Color::AnsiValue(255),
            fish_b: Color::AnsiValue(195),
            bubble: Color::AnsiValue(159),
        },
        Theme {
            name: "Violet",
            bg: Color::AnsiValue(16),
            water_low: Color::AnsiValue(17),
            water_mid: Color::AnsiValue(55),
            water_hi: Color::AnsiValue(93),
            plant: Color::AnsiValue(71),
            rock: Color::AnsiValue(60),
            hud: Color::AnsiValue(183),
            fish_a: Color::AnsiValue(225),
            fish_b: Color::AnsiValue(219),
            bubble: Color::AnsiValue(189),
        },
        Theme {
            name: "Mono",
            bg: Color::AnsiValue(16),
            water_low: Color::AnsiValue(236),
            water_mid: Color::AnsiValue(242),
            water_hi: Color::AnsiValue(250),
            plant: Color::AnsiValue(244),
            rock: Color::AnsiValue(240),
            hud: Color::AnsiValue(252),
            fish_a: Color::AnsiValue(254),
            fish_b: Color::AnsiValue(253),
            bubble: Color::AnsiValue(252),
        },
    ]
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

    fn put(&mut self, x: u16, y: u16, cell: Cell) {
        if x >= self.w || y >= self.h {
            return;
        }
        let i = self.idx(x, y);
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

        queue!(out, ResetColor, EndSynchronizedUpdate)?;
        out.flush()?;
        Ok(())
    }
}

#[derive(Clone)]
struct Fish {
    pos: Vec2,   // 0..1
    vel: Vec2,   // in normalized space per second
    size: f32,   // relative
    phase: f32,  // swim phase
    tint: u8,    // 0 or 1 for fish color swap
    species: u8, // 0=minnow, 1=goldfish, 2=angelfish
    wander: f32, // target change timer
}

#[derive(Clone)]
struct Bubble {
    pos: Vec2,
    vel: Vec2,
    r: f32,
    wobble: f32,
}

struct Aquarium {
    rng: StdRng,
    t: f32,
    fish: Vec<Fish>,
    bubbles: Vec<Bubble>,
    enable_bubbles: bool,
    current: f32,
    paused: bool,
    show_hud: bool,
    show_help: bool,
    theme_ix: usize,
    feed_impulse: Option<(Vec2, f32)>, // (point, remaining seconds)
}

impl Aquarium {
    fn new(seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut a = Self {
            rng,
            t: 0.0,
            fish: Vec::new(),
            bubbles: Vec::new(),
            enable_bubbles: true,
            current: 0.12,
            paused: false,
            show_hud: true,
            show_help: false,
            theme_ix: 0,
            feed_impulse: None,
        };
        for _ in 0..9 {
            a.spawn_fish();
        }
        for _ in 0..28 {
            a.spawn_bubble();
        }
        a
    }

    fn theme(&self) -> Theme {
        themes()[self.theme_ix % themes().len()]
    }

    fn spawn_fish(&mut self) {
        let pos = Vec2::new(self.rng.gen::<f32>(), self.rng.gen::<f32>() * 0.75 + 0.12);
        let dir = if self.rng.gen_bool(0.5) { 1.0 } else { -1.0 };
        let vel = Vec2::new(dir * (0.10 + self.rng.gen::<f32>() * 0.18), (self.rng.gen::<f32>() - 0.5) * 0.05);
        let size = 0.022 + self.rng.gen::<f32>() * 0.030;
        let phase = self.rng.gen::<f32>() * 2.0 * PI;
        let tint = if self.rng.gen_bool(0.5) { 0 } else { 1 };
        let species = self.rng.gen_range(0..3);
        self.fish.push(Fish {
            pos,
            vel,
            size,
            phase,
            tint,
            species,
            wander: self.rng.gen::<f32>() * 2.0,
        });
    }

    fn spawn_bubble(&mut self) {
        let pos = Vec2::new(self.rng.gen::<f32>(), 1.02 + self.rng.gen::<f32>() * 0.6);
        let vel = Vec2::new((self.rng.gen::<f32>() - 0.5) * 0.02, -(0.06 + self.rng.gen::<f32>() * 0.12));
        let r = 0.006 + self.rng.gen::<f32>() * 0.010;
        let wobble = self.rng.gen::<f32>() * 2.0 * PI;
        self.bubbles.push(Bubble { pos, vel, r, wobble });
    }

    fn add_fish(&mut self) {
        if self.fish.len() < 40 {
            self.spawn_fish();
        }
    }

    fn remove_fish(&mut self) {
        if !self.fish.is_empty() {
            self.fish.pop();
        }
    }

    fn toggle_theme(&mut self) {
        self.theme_ix = (self.theme_ix + 1) % themes().len();
    }

    fn feed(&mut self) {
        let p = Vec2::new(0.25 + self.rng.gen::<f32>() * 0.50, 0.25 + self.rng.gen::<f32>() * 0.45);
        self.feed_impulse = Some((p, 1.1));
    }

    fn update(&mut self, dt: f32) {
        if self.paused {
            return;
        }

        self.t += dt;

        // Feeding impulse decays quickly.
        if let Some((_p, ref mut rem)) = self.feed_impulse {
            *rem -= dt;
            if *rem <= 0.0 {
                self.feed_impulse = None;
            }
        }

        // Fish
        for f in &mut self.fish {
            f.phase += dt * (2.0 + 2.0 * (f.vel.len() * 2.0).min(1.0));
            f.wander -= dt;

            let mut acc = Vec2::new(0.0, 0.0);

            // Mild vertical bob.
            acc.y += (f.phase * 0.7).sin() * 0.015;

            // Current pushes horizontally with a slight vertical curl.
            acc.x += self.current * 0.25;
            acc.y += (f.pos.x * 2.0 * PI + self.t * 0.6).sin() * self.current * 0.08;

            // Wander: occasional direction changes.
            if f.wander <= 0.0 {
                f.wander = 0.9 + self.rng.gen::<f32>() * 1.8;
                let turn = (self.rng.gen::<f32>() - 0.5) * 0.25;
                f.vel.y = (f.vel.y + turn).clamp(-0.22, 0.22);
                if self.rng.gen_bool(0.12) {
                    f.vel.x *= -1.0;
                }
            }

            // Feed attraction.
            if let Some((p, rem)) = self.feed_impulse {
                let toward = (p - f.pos).norm();
                let strength = (rem / 1.1).clamp(0.0, 1.0);
                acc += toward * (0.55 * strength);
            }

            // Soft boundary forces.
            let margin = 0.06;
            if f.pos.x < margin {
                acc.x += (margin - f.pos.x) * 2.6;
            }
            if f.pos.x > 1.0 - margin {
                acc.x -= (f.pos.x - (1.0 - margin)) * 2.6;
            }
            if f.pos.y < 0.10 {
                acc.y += (0.10 - f.pos.y) * 2.8;
            }
            if f.pos.y > 0.92 {
                acc.y -= (f.pos.y - 0.92) * 2.8;
            }

            // Integrate.
            f.vel += acc * dt;
            f.vel = f.vel.clamp_len(0.35);
            f.pos += f.vel * dt;

            // Wrap a little to keep motion natural.
            if f.pos.x < -0.12 {
                f.pos.x = 1.12;
            } else if f.pos.x > 1.12 {
                f.pos.x = -0.12;
            }
            f.pos.y = f.pos.y.clamp(0.08, 0.96);
        }

        // Bubbles
        if self.enable_bubbles {
            for b in &mut self.bubbles {
                b.wobble += dt * (1.4 + self.rng.gen::<f32>() * 0.4);
                let wob = (b.wobble).sin() * 0.015;
                b.pos.x += (b.vel.x + wob) * dt;
                b.pos.y += b.vel.y * dt;

                // Slight drift to center.
                b.pos.x += (0.5 - b.pos.x) * 0.006 * dt;

                if b.pos.y < -0.08 {
                    b.pos = Vec2::new(self.rng.gen::<f32>(), 1.06 + self.rng.gen::<f32>() * 0.6);
                }
                if b.pos.x < -0.1 {
                    b.pos.x = 1.1;
                }
                if b.pos.x > 1.1 {
                    b.pos.x = -0.1;
                }
            }
        }
    }
}

fn clamp_u8(x: i32) -> u8 {
    x.clamp(0, 255) as u8
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// Tiny hash noise for water shimmer.
fn hash2(mut x: i32, mut y: i32, seed: u32) -> u32 {
    x ^= (seed as i32).wrapping_mul(374761393);
    y ^= (seed as i32).wrapping_mul(668265263);
    let mut n = (x as u32).wrapping_mul(2654435761) ^ (y as u32).wrapping_mul(2246822519);
    n ^= n >> 13;
    n = n.wrapping_mul(3266489917);
    n ^= n >> 16;
    n
}

struct BrailleCanvas {
    tw: usize,
    th: usize,
    sw: usize,
    sh: usize,
    sub: Vec<u8>, // intensity 0..255 per subpixel
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
            sub: vec![0u8; sw * sh],
        }
    }

    fn resize(&mut self, term_w: usize, term_h: usize) {
        self.tw = term_w;
        self.th = term_h;
        self.sw = term_w * SUB_X;
        self.sh = term_h * SUB_Y;
        self.sub = vec![0u8; self.sw * self.sh];
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

    fn clear(&mut self) {
        self.sub.fill(0);
    }

    fn add(&mut self, x: i32, y: i32, v: u8) {
        if let Some(i) = self.sidx(x, y) {
            let cur = self.sub[i] as i32;
            self.sub[i] = clamp_u8(cur + v as i32);
        }
    }

    fn set_max(&mut self, x: i32, y: i32, v: u8) {
        if let Some(i) = self.sidx(x, y) {
            self.sub[i] = self.sub[i].max(v);
        }
    }

    fn add_disc(&mut self, cx: f32, cy: f32, r: f32, strength: u8) {
        let minx = ((cx - r) * self.sw as f32) as i32 - 2;
        let maxx = ((cx + r) * self.sw as f32) as i32 + 2;
        let miny = ((cy - r) * self.sh as f32) as i32 - 2;
        let maxy = ((cy + r) * self.sh as f32) as i32 + 2;

        let cxs = cx * self.sw as f32;
        let cys = cy * self.sh as f32;
        let rs = r * (self.sw.min(self.sh) as f32);

        for y in miny..=maxy {
            for x in minx..=maxx {
                let dx = x as f32 - cxs;
                let dy = y as f32 - cys;
                let d = (dx * dx + dy * dy).sqrt();
                if d <= rs {
                    let fall = 1.0 - (d / rs).clamp(0.0, 1.0);
                    let v = (strength as f32 * (0.35 + 0.65 * fall)) as u8;
                    self.add(x, y, v);
                }
            }
        }
    }

    fn add_line(&mut self, ax: f32, ay: f32, bx: f32, by: f32, thickness: f32, strength: u8) {
        let a = Vec2::new(ax, ay);
        let b = Vec2::new(bx, by);
        let ab = b - a;
        let len = ab.len().max(1e-6);
        let steps = (len * (self.sw.min(self.sh) as f32) * 1.1).clamp(6.0, 200.0) as i32;

        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let p = a + ab * t;
            self.add_disc(p.x, p.y, thickness, strength);
        }
    }

    fn sample_avg(&self, tx: usize, ty: usize) -> u8 {
        let sx0 = tx * SUB_X;
        let sy0 = ty * SUB_Y;
        let mut sum = 0u32;
        for oy in 0..SUB_Y {
            for ox in 0..SUB_X {
                sum += self.sub[(sy0 + oy) * self.sw + (sx0 + ox)] as u32;
            }
        }
        (sum / (SUB_X * SUB_Y) as u32) as u8
    }

    fn to_braille_cell(&self, tx: usize, ty: usize, threshold: u8) -> u8 {
        // Unicode braille dot numbering:
        // (x=0,y=0)->dot1, (0,1)->dot2, (0,2)->dot3, (0,3)->dot7
        // (x=1,y=0)->dot4, (1,1)->dot5, (1,2)->dot6, (1,3)->dot8
        let sx0 = tx * SUB_X;
        let sy0 = ty * SUB_Y;

        let mut mask = 0u8;
        for oy in 0..SUB_Y {
            for ox in 0..SUB_X {
                let v = self.sub[(sy0 + oy) * self.sw + (sx0 + ox)];
                if v >= threshold {
                    let bit = match (ox, oy) {
                        (0, 0) => 0x01, // dot 1
                        (0, 1) => 0x02, // dot 2
                        (0, 2) => 0x04, // dot 3
                        (0, 3) => 0x40, // dot 7
                        (1, 0) => 0x08, // dot 4
                        (1, 1) => 0x10, // dot 5
                        (1, 2) => 0x20, // dot 6
                        (1, 3) => 0x80, // dot 8
                        _ => 0,
                    };
                    mask |= bit;
                }
            }
        }
        mask
    }
}

fn water_intensity(nx: f32, ny: f32, t: f32, seed: u32) -> f32 {
    // nx, ny in 0..1
    let wave1 = (nx * 8.0 + t * 0.85).sin() * 0.45;
    let wave2 = (ny * 10.0 - t * 0.65).sin() * 0.35;
    let wave3 = ((nx + ny) * 6.5 + t * 0.40).sin() * 0.25;

    let ix = (nx * 120.0) as i32;
    let iy = (ny * 80.0) as i32;
    let h = hash2(ix, iy, seed) & 1023;
    let n = (h as f32) / 1023.0; // 0..1

    let shimmer = (t * 2.2 + nx * 14.0 + ny * 9.0).sin() * 0.15;
    let base = 0.34 + 0.14 * ny;

    (base + 0.12 * wave1 + 0.08 * wave2 + 0.06 * wave3 + 0.07 * (n - 0.5) + shimmer)
        .clamp(0.0, 1.0)
}

fn main() -> io::Result<()> {
    let mut out = io::stdout();

    terminal::enable_raw_mode()?;
    queue!(out, EnterAlternateScreen, DisableLineWrap, cursor::Hide)?;
    out.flush()?;

    let mut cleanup = CleanupGuard {};
    let mut last_size = terminal::size()?;
    let mut renderer = Renderer::new(last_size.0, last_size.1, themes()[0].bg);
    let mut canvas = BrailleCanvas::new(last_size.0 as usize, last_size.1 as usize);

    let seed = (Instant::now().elapsed().as_nanos() as u64) ^ 0xA11CE_u64;
    let mut aq = Aquarium::new(seed);

    let mut last = Instant::now();
    let mut fps_acc = 0.0f32;
    let mut fps_frames = 0u32;
    let mut fps_est = 0.0f32;

    loop {
        // Input
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Resize(w, h) => {
                    last_size = (w, h);
                    let th = aq.theme();
                    renderer.resize(w, h, th.bg);
                    canvas.resize(w as usize, h as usize);
                }
                Event::Key(KeyEvent { code, modifiers, kind, .. }) => {
                    if kind != KeyEventKind::Press {
                        continue;
                    }
                    match code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(()),
                        KeyCode::Char('c') | KeyCode::Char('C') => {
                            aq.toggle_theme();
                            let th = aq.theme();
                            renderer.full_redraw = true;
                            renderer.last_bg = th.bg;
                        }
                        KeyCode::Char('p') | KeyCode::Char('P') => aq.paused = !aq.paused,
                        KeyCode::Char('h') | KeyCode::Char('H') => aq.show_hud = !aq.show_hud,
                        KeyCode::Char('?') => aq.show_help = !aq.show_help,
                        KeyCode::Char('b') | KeyCode::Char('B') => aq.enable_bubbles = !aq.enable_bubbles,
                        KeyCode::Char('f') | KeyCode::Char('F') => aq.feed(),
                        KeyCode::Char('+') | KeyCode::Char('=') => aq.add_fish(),
                        KeyCode::Char('-') => aq.remove_fish(),
                        KeyCode::Left => aq.current = (aq.current - 0.04).clamp(-0.35, 0.35),
                        KeyCode::Right => aq.current = (aq.current + 0.04).clamp(-0.35, 0.35),
                        KeyCode::Char('0') | KeyCode::Insert => aq.current = 0.0,
                        KeyCode::Char('l') if modifiers.contains(KeyModifiers::CONTROL) => {
                            renderer.full_redraw = true;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // Timing
        let now = Instant::now();
        let mut dt = (now - last).as_secs_f32();
        last = now;
        if dt > DT_CLAMP {
            dt = DT_CLAMP;
        }

        // FPS estimate
        fps_acc += dt;
        fps_frames += 1;
        if fps_acc >= 0.5 {
            fps_est = fps_frames as f32 / fps_acc;
            fps_acc = 0.0;
            fps_frames = 0;
        }

        aq.update(dt);

        // Render
        render_frame(&mut renderer, &mut canvas, &aq, fps_est)?;

        // Cap FPS
        let frame_time = Duration::from_secs_f32(dt);
        let target = Duration::from_millis(1000 / FPS_CAP.max(1));
        if frame_time < target {
            std::thread::sleep(target - frame_time);
        }
    }
}

fn render_frame(renderer: &mut Renderer, canvas: &mut BrailleCanvas, aq: &Aquarium, fps_est: f32) -> io::Result<()> {
    let th = aq.theme();
    renderer.clear_back(th.bg);
    canvas.clear();

    let w = renderer.w as usize;
    let h = renderer.h as usize;

    // Water background as subpixel intensity with caustics shimmer.
    let seed = (aq.rng.clone().next_u32()) ^ 0x51A7_1234;
    for sy in 0..canvas.sh {
        let ny = sy as f32 / (canvas.sh.max(1) as f32);
        for sx in 0..canvas.sw {
            let nx = sx as f32 / (canvas.sw.max(1) as f32);
            let v = water_intensity(nx, ny, aq.t, seed);
            let caust = ((nx * 9.0 - aq.t * 1.4).sin() * (ny * 7.5 + aq.t * 1.1).cos()).abs();
            let c = smoothstep(0.55, 0.92, caust) * 0.22;

            let intensity = ((v + c) * 150.0).clamp(0.0, 170.0) as u8;
            canvas.sub[sy * canvas.sw + sx] = intensity;
        }
    }

    // Subtle surface line.
    let surface_y = (0.08 * canvas.sh as f32) as i32;
    for x in 0..canvas.sw as i32 {
        let shimmer = ((x as f32) * 0.09 + aq.t * 2.2).sin();
        let v = (120.0 + shimmer * 30.0) as i32;
        canvas.set_max(x, surface_y, clamp_u8(v));
    }

    // Rocks and plants (simple silhouettes).
    draw_rocks(canvas, aq.t);
    draw_plants(canvas, aq.t);

    // Bubbles
    if aq.enable_bubbles {
        for b in &aq.bubbles {
            canvas.add_disc(b.pos.x, b.pos.y, b.r, 140);
            // highlight
            canvas.add_disc(b.pos.x - b.r * 0.35, b.pos.y - b.r * 0.35, b.r * 0.35, 70);
        }
    }

    // Fish
    for f in &aq.fish {
        draw_fish(canvas, aq.t, f);
    }

    // Convert subpixels to terminal cells.
    // Thresholds tuned so background stays smooth and silhouettes pop.
    let base_thresh = 68u8;

    for ty in 0..h {
        for tx in 0..w {
            let avg = canvas.sample_avg(tx, ty);

            // Choose a water shade based on average intensity.
            let fg = if avg < 60 {
                th.water_low
            } else if avg < 105 {
                th.water_mid
            } else {
                th.water_hi
            };

            // Higher threshold in the brightest water to avoid overfilling.
            let thresh = if avg > 140 { base_thresh + 18 } else { base_thresh };
            let mask = canvas.to_braille_cell(tx, ty, thresh);

            let ch = if mask == 0 { ' ' } else { char::from_u32(0x2800 + mask as u32).unwrap_or(' ') };

            renderer.put(
                tx as u16,
                ty as u16,
                Cell {
                    ch,
                    fg,
                    bg: th.bg,
                },
            );
        }
    }

    // HUD
    if aq.show_hud && h >= 2 {
        draw_hud(renderer, aq, fps_est);
    }

    // Help overlay
    if aq.show_help {
        draw_help(renderer, aq);
    }

    // Flush
    let mut out = io::stdout();
    renderer.flush(&mut out)
}

fn draw_hud(renderer: &mut Renderer, aq: &Aquarium, fps_est: f32) {
    let th = aq.theme();
    let w = renderer.w as usize;

    let line = format!(
        "  Aquarium  | theme: {}  | fish: {}  | bubbles: {}  | current: {:+.2}  | {}  | {:.0} fps  ",
        th.name,
        aq.fish.len(),
        if aq.enable_bubbles { "on" } else { "off" },
        aq.current,
        if aq.paused { "paused" } else { "running" },
        fps_est
    );

    let hint = "  keys: Q quit  C theme  P pause  H hud  ? help  +/- fish  B bubbles  F feed  ←/→ current  0 reset  ";

    for x in 0..w {
        renderer.put(
            x as u16,
            0,
            Cell {
                ch: ' ',
                fg: th.hud,
                bg: th.bg,
            },
        );
        if renderer.h > 1 {
            renderer.put(
                x as u16,
                1,
                Cell {
                    ch: ' ',
                    fg: th.hud,
                    bg: th.bg,
                },
            );
        }
    }

    for (i, ch) in line.chars().take(w).enumerate() {
        renderer.put(
            i as u16,
            0,
            Cell {
                ch,
                fg: th.hud,
                bg: th.bg,
            },
        );
    }
    if renderer.h > 1 {
        for (i, ch) in hint.chars().take(w).enumerate() {
            renderer.put(
                i as u16,
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

fn draw_help(renderer: &mut Renderer, aq: &Aquarium) {
    let th = aq.theme();
    let w = renderer.w as i32;
    let h = renderer.h as i32;

    let box_w = (w as f32 * 0.72).clamp(34.0, (w - 2) as f32) as i32;
    let box_h = 11i32.min(h - 2).max(8);

    let x0 = (w - box_w) / 2;
    let y0 = (h - box_h) / 2;

    let lines = [
        "Braille Aquarium",
        "",
        "Q: quit",
        "C: cycle theme",
        "P: pause/resume",
        "H: toggle HUD",
        "?: toggle this help",
        "+/-: add/remove fish",
        "B: toggle bubbles",
        "F: feed (fish dart toward food)",
        "Left/Right: adjust current, 0 resets",
    ];

    // Dim background behind overlay slightly by repainting a rectangle.
    for y in 0..h {
        for x in 0..w {
            if x >= x0 - 1 && x <= x0 + box_w && y >= y0 - 1 && y <= y0 + box_h {
                continue;
            }
            // light tint: replace fg with mid water for readability.
            let i = renderer.idx(x as u16, y as u16);
            let mut c = renderer.back[i];
            c.fg = th.water_mid;
            renderer.back[i] = c;
        }
    }

    // Box
    for y in 0..box_h {
        for x in 0..box_w {
            let px = x0 + x;
            let py = y0 + y;
            let is_border = x == 0 || x == box_w - 1 || y == 0 || y == box_h - 1;
            let ch = if is_border {
                if y == 0 && x == 0 {
                    '┌'
                } else if y == 0 && x == box_w - 1 {
                    '┐'
                } else if y == box_h - 1 && x == 0 {
                    '└'
                } else if y == box_h - 1 && x == box_w - 1 {
                    '┘'
                } else if y == 0 || y == box_h - 1 {
                    '─'
                } else {
                    '│'
                }
            } else {
                ' '
            };

            renderer.put(
                px as u16,
                py as u16,
                Cell {
                    ch,
                    fg: th.hud,
                    bg: th.bg,
                },
            );
        }
    }

    // Text
    let mut row = 1;
    for s in lines.iter() {
        if row >= box_h - 1 {
            break;
        }
        for (i, ch) in s.chars().take((box_w - 4) as usize).enumerate() {
            renderer.put(
                (x0 + 2 + i as i32) as u16,
                (y0 + row) as u16,
                Cell {
                    ch,
                    fg: th.hud,
                    bg: th.bg,
                },
            );
        }
        row += 1;
    }

    // Footer note
    let footer = if aq.paused { "paused" } else { "running" };
    let footer_line = format!("status: {}   theme: {}", footer, th.name);
    let fy = y0 + box_h - 2;
    for (i, ch) in footer_line.chars().take((box_w - 4) as usize).enumerate() {
        renderer.put(
            (x0 + 2 + i as i32) as u16,
            fy as u16,
            Cell {
                ch,
                fg: th.hud,
                bg: th.bg,
            },
        );
    }
}

fn draw_rocks(canvas: &mut BrailleCanvas, t: f32) {
    // A couple of soft mounds at the bottom.
    let rocks = [
        (0.18f32, 0.92f32, 0.12f32),
        (0.42f32, 0.94f32, 0.16f32),
        (0.72f32, 0.93f32, 0.14f32),
        (0.90f32, 0.95f32, 0.10f32),
    ];
    for &(x, y, r) in &rocks {
        let wob = (t * 0.4 + x * 7.0).sin() * 0.0015;
        canvas.add_disc(x, y + wob, r, 85);
        canvas.add_disc(x - r * 0.25, y - r * 0.18 + wob, r * 0.22, 45);
    }
}

fn draw_plants(canvas: &mut BrailleCanvas, t: f32) {
    let stems = [
        (0.08f32, 0.98f32, 0.20f32, 0.010f32),
        (0.14f32, 0.98f32, 0.24f32, 0.010f32),
        (0.86f32, 0.98f32, 0.26f32, 0.012f32),
        (0.92f32, 0.98f32, 0.22f32, 0.010f32),
    ];

    for &(x, yb, h, thick) in &stems {
        let sway = (t * 0.9 + x * 10.0).sin() * 0.04;
        let steps = 14;
        let mut last = Vec2::new(x, yb);
        for i in 1..=steps {
            let u = i as f32 / steps as f32;
            let bend = sway * u * u;
            let p = Vec2::new(x + bend, yb - h * u);
            canvas.add_line(last.x, last.y, p.x, p.y, thick * (1.0 - u * 0.3), 70);
            last = p;

            // leaf flick
            if i % 4 == 0 {
                let dir = if (i / 4) % 2 == 0 { 1.0 } else { -1.0 };
                let leaf_len = 0.04 + 0.02 * (t * 1.2 + x * 6.0 + u * 8.0).sin().abs();
                let lp = Vec2::new(p.x + dir * leaf_len * (0.5 + u), p.y - leaf_len * 0.25);
                canvas.add_line(p.x, p.y, lp.x, lp.y, thick * 0.75, 55);
            }
        }
    }
}

fn draw_fish(canvas: &mut BrailleCanvas, t: f32, f: &Fish) {
    let mut forward = f.vel.norm();
    if forward.len() <= 1e-6 {
        forward = Vec2::new(1.0, 0.0);
    }
    let side = Vec2::new(-forward.y, forward.x);

    let (len_scale, height_scale, tail_ratio, tail_flare, wiggle_k, wiggle_amp, dorsal_scale, belly_bias) =
        match f.species {
            0 => (4.6, 1.15, 0.22, 0.60, 8.0, 0.38, 0.35, 0.05), // minnow
            1 => (3.4, 1.65, 0.30, 1.10, 5.2, 0.28, 0.45, 0.22), // goldfish
            _ => (3.8, 2.05, 0.25, 0.85, 6.4, 0.32, 0.90, 0.10), // angelfish
        };

    let body_len = f.size * len_scale;
    let body_height = f.size * height_scale;
    let base_r = body_height * 0.5;
    let body_half = body_len * 0.5;
    let head_front = body_half;
    let tail_base = -body_half;
    let tail_len = body_len * tail_ratio;
    let tail_tip = tail_base - tail_len;

    let max_extent = body_len * 0.65 + tail_len + body_height * 1.8;
    let minx = ((f.pos.x - max_extent) * canvas.sw as f32) as i32 - 2;
    let maxx = ((f.pos.x + max_extent) * canvas.sw as f32) as i32 + 2;
    let miny = ((f.pos.y - max_extent) * canvas.sh as f32) as i32 - 2;
    let maxy = ((f.pos.y + max_extent) * canvas.sh as f32) as i32 + 2;

    let inv_sw = 1.0 / canvas.sw as f32;
    let inv_sh = 1.0 / canvas.sh as f32;

    let wave_amp = body_height * wiggle_amp;
    let edge_band = body_height * 0.08;
    let shadow_band = body_height * 0.10;

    for y in miny..=maxy {
        for x in minx..=maxx {
            let p = Vec2::new((x as f32 + 0.5) * inv_sw, (y as f32 + 0.5) * inv_sh);
            let d = p - f.pos;
            let u = d.x * forward.x + d.y * forward.y;
            let v0 = d.x * side.x + d.y * side.y;

            let tail_weight = ((head_front - u) / (head_front - tail_tip)).clamp(0.0, 1.0);
            let wave = (f.phase + u / body_len * wiggle_k).sin();
            let spine_offset = wave * wave_amp * tail_weight;
            let mut v = v0 - spine_offset;

            let in_body_span = u >= tail_base && u <= head_front + body_len * 0.03;
            if in_body_span {
                let mut r = base_r * (0.25 + 0.75 * smoothstep(tail_base, head_front, u));
                let head_cap = smoothstep(head_front - body_len * 0.18, head_front, u);
                let snout = smoothstep(head_front - body_len * 0.06, head_front + body_len * 0.02, u);
                r *= 1.0 + 0.25 * head_cap;
                r *= 1.0 - 0.35 * snout;

                if v > 0.0 {
                    r *= 1.0 + belly_bias;
                }

                let av = v.abs();
                if av <= r {
                    let mut intensity = {
                        let dist = (av / r).clamp(0.0, 1.0);
                        let mut val = 210.0 + (150.0 - 210.0) * dist;
                        if dist < 0.35 {
                            val += 18.0 * (1.0 - dist / 0.35);
                        }
                        val
                    };

                    let mut notch = false;
                    if u > head_front - body_len * 0.06 && u < head_front {
                        if v > 0.0 && v < r * 0.7 {
                            let wedge = (u - (head_front - body_len * 0.06)) / (body_len * 0.06);
                            if v > r * (0.15 + 0.55 * wedge) {
                                notch = true;
                            }
                        }
                    }
                    if notch {
                        intensity *= 0.35;
                    }

                    canvas.add(x, y, intensity.clamp(0.0, 255.0) as u8);
                } else if av > r && av < r + edge_band {
                    canvas.add(x, y, 38);
                } else if v > r + edge_band && v < r + edge_band + shadow_band {
                    canvas.add(x, y, 22);
                }
            }

            if u < tail_base && u >= tail_tip {
                let tail_t = (tail_base - u) / (tail_base - tail_tip);
                let wag = (f.phase + tail_t * 5.0).sin() * 0.25;
                let fin_half = base_r * (0.20 + tail_flare * tail_t) * (1.0 + wag);
                let gap = base_r * 0.08;
                let av = v.abs();
                if av < fin_half && av > gap {
                    let intensity = 135.0 - 35.0 * tail_t;
                    canvas.add(x, y, intensity.clamp(0.0, 255.0) as u8);
                }
            }

            let dorsal_u0 = body_len * 0.10;
            let dorsal_u1 = body_len * 0.30;
            if u > dorsal_u0 && u < dorsal_u1 {
                let t = (u - dorsal_u0) / (dorsal_u1 - dorsal_u0);
                let peak = 1.0 - (t - 0.5).abs() * 2.0;
                let dorsal_h = body_height * dorsal_scale * peak;
                let r = base_r * (0.25 + 0.75 * smoothstep(tail_base, head_front, u));
                if v < -r && v > -r - dorsal_h {
                    canvas.add(x, y, 105);
                }
            }

            let pec_u0 = head_front - body_len * 0.42;
            let pec_u1 = head_front - body_len * 0.22;
            if u > pec_u0 && u < pec_u1 {
                let span = (u - pec_u0) / (pec_u1 - pec_u0);
                let slope = 0.15 + 0.65 * span;
                let r = base_r * (0.25 + 0.75 * smoothstep(tail_base, head_front, u));
                if v > r * 0.10 && v < r * 0.55 && v < r * (0.10 + slope) {
                    canvas.add(x, y, 70);
                }
            }
        }
    }

    let eye_u = head_front - body_len * 0.25;
    let eye_v = -0.15 * body_height;
    let eye_tail = ((head_front - eye_u) / (head_front - tail_tip)).clamp(0.0, 1.0);
    let eye_wiggle = (f.phase + eye_u / body_len * wiggle_k).sin() * wave_amp * eye_tail;
    let eye_pos = f.pos + forward * eye_u + side * (eye_v + eye_wiggle);
    let ex = (eye_pos.x * canvas.sw as f32) as i32;
    let ey = (eye_pos.y * canvas.sh as f32) as i32;
    canvas.add_disc(eye_pos.x, eye_pos.y, body_height * 0.08, 40);
    canvas.set_max(ex, ey, 255);
}

struct CleanupGuard;

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = queue!(
            out,
            EndSynchronizedUpdate,
            ResetColor,
            cursor::Show,
            EnableLineWrap,
            LeaveAlternateScreen
        );
        let _ = out.flush();
        let _ = terminal::disable_raw_mode();
    }
}

// Provide next_u32 for StdRng clone usage.
trait NextU32 {
    fn next_u32(&mut self) -> u32;
}
impl NextU32 for StdRng {
    fn next_u32(&mut self) -> u32 {
        self.gen::<u32>()
    }
}

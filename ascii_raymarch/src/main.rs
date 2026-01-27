use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{
        self, Clear, ClearType, DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::{
    io::{self, Stdout, Write},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
struct Vec2 {
    x: f64,
    y: f64,
}
impl Vec2 {
    fn add(self, o: Vec2) -> Vec2 {
        Vec2 {
            x: self.x + o.x,
            y: self.y + o.y,
        }
    }
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2 {
            x: self.x - o.x,
            y: self.y - o.y,
        }
    }
    fn mul(self, k: f64) -> Vec2 {
        Vec2 {
            x: self.x * k,
            y: self.y * k,
        }
    }
    fn len2(self) -> f64 {
        self.x * self.x + self.y * self.y
    }
}

#[derive(Clone, Debug)]
struct Body {
    name: &'static str,
    pos: Vec2,
    vel: Vec2,
    mass: f64,
    glyph: char,
    color: Color,
    trail_glyph: char,
}

#[derive(Clone)]
struct Theme {
    name: &'static str,
    star: Color,
    hud: Color,
    trail_dim: Color,
}

fn themes() -> Vec<Theme> {
    vec![
        Theme {
            name: "Mint",
            star: Color::Green,
            hud: Color::White,
            trail_dim: Color::DarkGreen,
        },
        Theme {
            name: "Amber",
            star: Color::Yellow,
            hud: Color::White,
            trail_dim: Color::DarkYellow,
        },
        Theme {
            name: "Ice",
            star: Color::Cyan,
            hud: Color::White,
            trail_dim: Color::DarkCyan,
        },
        Theme {
            name: "Purple",
            star: Color::Magenta,
            hud: Color::White,
            trail_dim: Color::DarkMagenta,
        },
        Theme {
            name: "Mono",
            star: Color::Grey,
            hud: Color::White,
            trail_dim: Color::DarkGrey,
        },
    ]
}

fn spawn_orbiter(
    rng: &mut StdRng,
    g: f64,
    central_mass: f64,
    name: &'static str,
    r: f64,
    mass: f64,
    glyph: char,
    color: Color,
    trail_glyph: char,
    phase: f64,
    ecc_kick: f64,
) -> Body {
    let x = r * phase.cos();
    let y = r * phase.sin();

    // circular orbit speed v = sqrt(G*M/r)
    let v = (g * central_mass / r).sqrt();

    // tangent direction
    let tx = -phase.sin();
    let ty = phase.cos();

    // small random tweak to make each orbit feel distinct
    let kick = ecc_kick * rng.gen_range(-1.0..1.0);
    let vel = Vec2 {
        x: tx * v * (1.0 + kick),
        y: ty * v * (1.0 - kick),
    };

    Body {
        name,
        pos: Vec2 { x, y },
        vel,
        mass,
        glyph,
        color,
        trail_glyph,
    }
}

#[derive(Clone)]
struct Sim {
    bodies: Vec<Body>,
    central_mass: f64,
    g: f64,
    softening: f64,
    dt_scale: f64,
    zoom: f64,
    paused: bool,
    trails: bool,
    nbody: bool,
    theme_idx: usize,
    seed: u64,
}

impl Sim {
    fn new(seed: u64) -> Self {
        let mut s = Self {
            bodies: vec![],
            central_mass: 2000.0,
            g: 1.0,
            softening: 0.55,
            dt_scale: 1.0,
            zoom: 1.0,
            paused: false,
            trails: true,
            nbody: false,
            theme_idx: 0,
            seed,
        };
        s.reset();
        s
    }

    fn reset(&mut self) {
        let mut rng = StdRng::seed_from_u64(self.seed);

        let mut bodies: Vec<Body> = Vec::new();

        let palette = [
            Color::Green,
            Color::Cyan,
            Color::Yellow,
            Color::Magenta,
            Color::Blue,
            Color::White,
        ];

        // 6 orbiters
        for i in 0..6 {
            let r = 8.0 + i as f64 * 4.0 + rng.gen_range(-0.8..0.8);
            let m = 1.0 + (i as f64).powf(1.25) * 0.35;
            let phase = rng.gen_range(0.0..std::f64::consts::TAU);
            let col = palette[i % palette.len()];
            let glyph = ['o', 'O', '0', '*', '●', '•'][i % 6];
            let trail = ['.', '·', ':', ',', '`', '\''][i % 6];

            bodies.push(spawn_orbiter(
                &mut rng,
                self.g,
                self.central_mass,
                match i {
                    0 => "I",
                    1 => "II",
                    2 => "III",
                    3 => "IV",
                    4 => "V",
                    _ => "VI",
                },
                r,
                m,
                glyph,
                col,
                trail,
                phase,
                0.06,
            ));
        }

        // A heavier "rogue"
        let r = 26.0 + rng.gen_range(-1.0..1.0);
        let phase = rng.gen_range(0.0..std::f64::consts::TAU);
        bodies.push(spawn_orbiter(
            &mut rng,
            self.g,
            self.central_mass,
            "Rogue",
            r,
            8.0,
            '@',
            Color::Red,
            '.',
            phase,
            0.10,
        ));

        self.bodies = bodies;
    }

    fn step(&mut self, dt_real: f64) {
        if self.paused {
            return;
        }
        let dt = dt_real * self.dt_scale;

        let mut acc = vec![Vec2 { x: 0.0, y: 0.0 }; self.bodies.len()];

        // Central gravity (origin)
        for (i, b) in self.bodies.iter().enumerate() {
            let r = b.pos;
            let d2 = r.len2() + self.softening * self.softening;
            let inv_d = 1.0 / d2.sqrt();
            let inv_d3 = inv_d * inv_d * inv_d;
            let a = r.mul(-self.g * self.central_mass * inv_d3);
            acc[i] = acc[i].add(a);
        }

        // Optional mutual gravity
        if self.nbody {
            for i in 0..self.bodies.len() {
                for j in (i + 1)..self.bodies.len() {
                    let pi = self.bodies[i].pos;
                    let pj = self.bodies[j].pos;
                    let d = pj.sub(pi);

                    let d2 = d.len2() + self.softening * self.softening;
                    let inv_d = 1.0 / d2.sqrt();
                    let inv_d3 = inv_d * inv_d * inv_d;

                    let ai = d.mul(self.g * self.bodies[j].mass * inv_d3);
                    let aj = d.mul(-self.g * self.bodies[i].mass * inv_d3);

                    acc[i] = acc[i].add(ai);
                    acc[j] = acc[j].add(aj);
                }
            }
        }

        // Semi-implicit Euler
        for i in 0..self.bodies.len() {
            self.bodies[i].vel = self.bodies[i].vel.add(acc[i].mul(dt));
            self.bodies[i].pos = self.bodies[i].pos.add(self.bodies[i].vel.mul(dt));
        }
    }
}

struct ScreenBuf {
    w: u16,
    h: u16,
    ch: Vec<char>,
    col: Vec<Color>,
    dim: Vec<bool>,
}

impl ScreenBuf {
    fn new(w: u16, h: u16) -> Self {
        let n = (w as usize) * (h as usize);
        Self {
            w,
            h,
            ch: vec![' '; n],
            col: vec![Color::Reset; n],
            dim: vec![false; n],
        }
    }
    fn resize(&mut self, w: u16, h: u16) {
        self.w = w;
        self.h = h;
        let n = (w as usize) * (h as usize);
        self.ch.resize(n, ' ');
        self.col.resize(n, Color::Reset);
        self.dim.resize(n, false);
        self.clear();
    }
    fn clear(&mut self) {
        for i in 0..self.ch.len() {
            self.ch[i] = ' ';
            self.col[i] = Color::Reset;
            self.dim[i] = false;
        }
    }
    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 {
            return None;
        }
        let (x, y) = (x as u16, y as u16);
        if x >= self.w || y >= self.h {
            return None;
        }
        Some((y as usize) * (self.w as usize) + (x as usize))
    }
    fn set(&mut self, x: i32, y: i32, c: char, color: Color, dim: bool) {
        if let Some(i) = self.idx(x, y) {
            self.ch[i] = c;
            self.col[i] = color;
            self.dim[i] = dim;
        }
    }
}

fn world_to_screen(p: Vec2, w: u16, h: u16, zoom: f64) -> (i32, i32) {
    let cx = (w as f64 - 1.0) * 0.5;
    let cy = (h as f64 - 1.0) * 0.5;

    // X squish so circles look less stretched in terminal fonts
    let x = cx + p.x * zoom * 1.8;
    let y = cy + p.y * zoom * 1.0;

    (x.round() as i32, y.round() as i32)
}

fn render(stdout: &mut Stdout, buf: &ScreenBuf) -> io::Result<()> {
    queue!(stdout, cursor::MoveTo(0, 0))?;

    let mut last_color = Color::Reset;

    for y in 0..buf.h {
        for x in 0..buf.w {
            let i = (y as usize) * (buf.w as usize) + (x as usize);
            let c = buf.ch[i];
            let col = buf.col[i];

            if col != last_color {
                queue!(stdout, SetForegroundColor(col))?;
                last_color = col;
            }

            queue!(stdout, Print(c))?;
        }
        if y + 1 < buf.h {
            queue!(stdout, Print('\n'))?;
        }
    }

    queue!(stdout, ResetColor)?;
    stdout.flush()?;
    Ok(())
}

fn main() -> io::Result<()> {
    let mut stdout = io::stdout();

    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    terminal::enable_raw_mode()?;
    execute!(stdout, DisableLineWrap)?;

    let mut sim = Sim::new(0xC0FFEE_u64);
    let theme_list = themes();

    let (mut w, mut h) = terminal::size()?;
    let mut buf = ScreenBuf::new(w, h);

    // Trail buffer: chars persist between frames
    let mut trail = vec![' '; (w as usize) * (h as usize)];

    let mut last = Instant::now();
    let mut fps_timer = 0.0_f64;
    let mut fps_count = 0_u32;
    let mut fps = 0_u32;

    'outer: loop {
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => break 'outer,
                    KeyCode::Char(' ') => sim.paused = !sim.paused,
                    KeyCode::Char('r') => {
                        sim.seed = sim.seed.wrapping_add(1);
                        sim.reset();
                        trail.fill(' ');
                    }
                    KeyCode::Char('t') => sim.trails = !sim.trails,
                    KeyCode::Char('n') => sim.nbody = !sim.nbody,
                    KeyCode::Char('c') => sim.theme_idx = (sim.theme_idx + 1) % theme_list.len(),
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        sim.dt_scale = (sim.dt_scale * 1.25).min(20.0)
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') => {
                        sim.dt_scale = (sim.dt_scale / 1.25).max(0.02)
                    }
                    KeyCode::Char(']') => sim.zoom = (sim.zoom * 1.15).min(8.0),
                    KeyCode::Char('[') => sim.zoom = (sim.zoom / 1.15).max(0.10),
                    _ => {}
                },
                Event::Resize(nw, nh) => {
                    w = nw;
                    h = nh;
                    buf.resize(w, h);
                    trail = vec![' '; (w as usize) * (h as usize)];
                }
                _ => {}
            }
        }

        let now = Instant::now();
        let dt = (now - last).as_secs_f64().min(0.050);
        last = now;

        fps_timer += dt;
        fps_count += 1;
        if fps_timer >= 0.5 {
            fps = (fps_count as f64 / fps_timer).round() as u32;
            fps_timer = 0.0;
            fps_count = 0;
        }

        sim.step(dt);

        let theme = &theme_list[sim.theme_idx];

        if sim.trails {
            buf.clear();
            for y in 0..h {
                for x in 0..w {
                    let i = (y as usize) * (w as usize) + (x as usize);
                    let c = trail[i];
                    if c != ' ' {
                        buf.ch[i] = c;
                        buf.col[i] = theme.trail_dim;
                        buf.dim[i] = true;
                    }
                }
            }
        } else {
            buf.clear();
            trail.fill(' ');
        }

        // star at origin
        let (sx, sy) = world_to_screen(Vec2 { x: 0.0, y: 0.0 }, w, h, sim.zoom);
        buf.set(sx, sy, '✶', theme.star, false);

        // bodies + trails
        for b in &sim.bodies {
            let (x, y) = world_to_screen(b.pos, w, h, sim.zoom);

            if sim.trails {
                if let Some(i) = buf.idx(x, y) {
                    trail[i] = b.trail_glyph;
                }
            }

            buf.set(x, y, b.glyph, b.color, false);
        }

        // HUD
        let hud = format!(
            "Orbit CLI | FPS {:>3} | {} | {} | dt x{:.2} | zoom {:.2} | trails {} | keys: q quit  space pause  r reset  n nbody  t trails  +/- speed  [] zoom  c theme",
            fps,
            if sim.nbody { "N-BODY" } else { "CENTRAL" },
            theme.name,
            sim.dt_scale,
            sim.zoom,
            if sim.trails { "on" } else { "off" },
        );
        for (i, ch) in hud.chars().take(w as usize).enumerate() {
            buf.set(i as i32, 0, ch, theme.hud, false);
        }

        execute!(stdout, Clear(ClearType::All))?;
        render(&mut stdout, &buf)?;

        std::thread::sleep(Duration::from_millis(16));
    }

    execute!(stdout, EnableLineWrap)?;
    terminal::disable_raw_mode()?;
    execute!(stdout, LeaveAlternateScreen, cursor::Show)?;
    Ok(())
}

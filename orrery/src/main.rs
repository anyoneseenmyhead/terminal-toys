use chrono::{DateTime, Local, TimeZone, Utc};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, ClearType, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::{
    f32::consts::PI,
    io::{self, Stdout, Write},
    time::{Duration, Instant},
};

const FPS_CAP: u64 = 30;
const ASPECT_X: f32 = 0.65;

// -------------------- Shared math --------------------
#[derive(Clone, Copy)]
struct Vec2 {
    x: f32,
    y: f32,
}
impl Vec2 {
    fn add(self, o: Vec2) -> Vec2 {
        Vec2 { x: self.x + o.x, y: self.y + o.y }
    }
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2 { x: self.x - o.x, y: self.y - o.y }
    }
    fn len(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
}

fn clamp01(x: f32) -> f32 {
    x.max(0.0).min(1.0)
}
fn clamp(x: f32, a: f32, b: f32) -> f32 {
    x.max(a).min(b)
}
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let aa = a as f32;
    let bb = b as f32;
    clamp(aa + (bb - aa) * t, 0.0, 255.0).round() as u8
}

fn deg(x: f32) -> f32 {
    x * PI / 180.0
}
fn normalize_angle(mut a: f32) -> f32 {
    while a < -PI {
        a += 2.0 * PI;
    }
    while a > PI {
        a -= 2.0 * PI;
    }
    a
}

fn v3_norm(x: f32, y: f32, z: f32) -> (f32, f32, f32) {
    let l = (x * x + y * y + z * z).sqrt().max(1e-6);
    (x / l, y / l, z / l)
}
fn v3_dot(ax: f32, ay: f32, az: f32, bx: f32, by: f32, bz: f32) -> f32 {
    ax * bx + ay * by + az * bz
}
fn v3_rot_y(x: f32, y: f32, z: f32, ang: f32) -> (f32, f32, f32) {
    let (s, c) = ang.sin_cos();
    (c * x + s * z, y, -s * x + c * z)
}
fn rot2(v: Vec2, ang: f32) -> Vec2 {
    let (s, c) = ang.sin_cos();
    Vec2 { x: v.x * c - v.y * s, y: v.x * s + v.y * c }
}

// -------------------- Braille helpers (2x4) --------------------
fn bayer_2x4_threshold(ix: usize, iy: usize) -> f32 {
    const M: [[u8; 2]; 4] = [[0, 4], [6, 2], [1, 5], [7, 3]];
    let v = M[iy & 3][ix & 1] as f32;
    (v + 0.5) / 8.0
}

fn braille_from_2x4(bits: [[bool; 2]; 4]) -> char {
    let mut mask = 0u16;
    if bits[0][0] { mask |= 1 << 0; } // 1
    if bits[1][0] { mask |= 1 << 1; } // 2
    if bits[2][0] { mask |= 1 << 2; } // 3
    if bits[0][1] { mask |= 1 << 3; } // 4
    if bits[1][1] { mask |= 1 << 4; } // 5
    if bits[2][1] { mask |= 1 << 5; } // 6
    if bits[3][0] { mask |= 1 << 6; } // 7
    if bits[3][1] { mask |= 1 << 7; } // 8
    std::char::from_u32(0x2800 + mask as u32).unwrap_or(' ')
}

// -------------------- Procedural noise (value noise + fbm) --------------------
fn hash_u32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}
fn hash3(ix: i32, iy: i32, iz: i32, seed: u32) -> u32 {
    let mut h = seed ^ 0x9e37_79b9;
    h ^= (ix as u32).wrapping_mul(0x85eb_ca6b);
    h = hash_u32(h);
    h ^= (iy as u32).wrapping_mul(0xc2b2_ae35);
    h = hash_u32(h);
    h ^= (iz as u32).wrapping_mul(0x27d4_eb2f);
    hash_u32(h)
}
fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}
fn value_noise_3d(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let ix0 = x.floor() as i32;
    let iy0 = y.floor() as i32;
    let iz0 = z.floor() as i32;
    let fx = x - ix0 as f32;
    let fy = y - iy0 as f32;
    let fz = z - iz0 as f32;

    let sx = smoothstep(fx);
    let sy = smoothstep(fy);
    let sz = smoothstep(fz);

    let v = |dx: i32, dy: i32, dz: i32| -> f32 {
        let h = hash3(ix0 + dx, iy0 + dy, iz0 + dz, seed);
        (h as f32) / (u32::MAX as f32)
    };

    let c000 = v(0, 0, 0);
    let c100 = v(1, 0, 0);
    let c010 = v(0, 1, 0);
    let c110 = v(1, 1, 0);
    let c001 = v(0, 0, 1);
    let c101 = v(1, 0, 1);
    let c011 = v(0, 1, 1);
    let c111 = v(1, 1, 1);

    let x00 = lerp(c000, c100, sx);
    let x10 = lerp(c010, c110, sx);
    let x01 = lerp(c001, c101, sx);
    let x11 = lerp(c011, c111, sx);

    let y0 = lerp(x00, x10, sy);
    let y1 = lerp(x01, x11, sy);

    lerp(y0, y1, sz)
}
fn fbm_3d(x: f32, y: f32, z: f32, seed: u32, octaves: usize) -> f32 {
    let mut amp = 0.55;
    let mut freq = 1.0;
    let mut sum = 0.0;
    let mut norm = 0.0;

    for o in 0..octaves {
        let s = seed.wrapping_add((o as u32).wrapping_mul(0x9e37_79b9));
        let n = value_noise_3d(x * freq, y * freq, z * freq, s);
        sum += (n * 2.0 - 1.0) * amp;
        norm += amp;
        amp *= 0.52;
        freq *= 2.03;
    }

    clamp01(0.5 + 0.5 * (sum / norm.max(1e-6)))
}

// -------------------- UI Cell buffer + diff render --------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cell {
    ch: char,
    fg: Color,
    bg: Color,
}
impl Cell {
    fn blank(bg: Color) -> Self {
        Self { ch: ' ', fg: Color::Reset, bg }
    }
}

fn put_cell(buf: &mut [Cell], w: u16, h: u16, x: u16, y: u16, c: Cell) {
    let ww = w as usize;
    let hh = h as usize;
    let xi = x as usize;
    let yi = y as usize;
    if xi >= ww || yi >= hh {
        return;
    }
    buf[yi * ww + xi] = c;
}

fn box_draw(buf: &mut [Cell], w: u16, h: u16, x0: u16, y0: u16, bw: u16, bh: u16, fg: Color, bg: Color) {
    if bw < 2 || bh < 2 {
        return;
    }
    let x1 = x0.saturating_add(bw - 1);
    let y1 = y0.saturating_add(bh - 1);

    for x in x0 + 1..x1 {
        put_cell(buf, w, h, x, y0, Cell { ch: '─', fg, bg });
        put_cell(buf, w, h, x, y1, Cell { ch: '─', fg, bg });
    }
    for y in y0 + 1..y1 {
        put_cell(buf, w, h, x0, y, Cell { ch: '│', fg, bg });
        put_cell(buf, w, h, x1, y, Cell { ch: '│', fg, bg });
    }
    put_cell(buf, w, h, x0, y0, Cell { ch: '┌', fg, bg });
    put_cell(buf, w, h, x1, y0, Cell { ch: '┐', fg, bg });
    put_cell(buf, w, h, x0, y1, Cell { ch: '└', fg, bg });
    put_cell(buf, w, h, x1, y1, Cell { ch: '┘', fg, bg });
}

fn write_str(buf: &mut [Cell], w: u16, h: u16, x: u16, y: u16, s: &str, fg: Color, bg: Color) {
    let ww = w as usize;
    let hh = h as usize;
    let yi = y as usize;
    if yi >= hh {
        return;
    }
    let mut xi = x as usize;
    for ch in s.chars() {
        if xi >= ww {
            break;
        }
        buf[yi * ww + xi] = Cell { ch, fg, bg };
        xi += 1;
    }
}

fn write_wrapped(
    buf: &mut [Cell],
    w: u16,
    h: u16,
    x: u16,
    y: u16,
    max_w: u16,
    s: &str,
    fg: Color,
    bg: Color,
) -> u16 {
    if max_w == 0 {
        return 0;
    }
    let mut line = String::new();
    let mut row = y;
    for word in s.split_whitespace() {
        if word.len() > max_w as usize {
            if !line.is_empty() {
                write_str(buf, w, h, x, row, &line, fg, bg);
                row = row.saturating_add(1);
                line.clear();
            }
            let mut start = 0;
            let bytes = word.as_bytes();
            while start < bytes.len() {
                let end = (start + max_w as usize).min(bytes.len());
                let chunk = std::str::from_utf8(&bytes[start..end]).unwrap_or("");
                write_str(buf, w, h, x, row, chunk, fg, bg);
                row = row.saturating_add(1);
                start = end;
            }
            continue;
        }
        let need = if line.is_empty() { word.len() } else { line.len() + 1 + word.len() };
        if need > max_w as usize {
            if !line.is_empty() {
                write_str(buf, w, h, x, row, &line, fg, bg);
                row = row.saturating_add(1);
                line.clear();
            }
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        write_str(buf, w, h, x, row, &line, fg, bg);
    }
    row.saturating_sub(y).saturating_add(1)
}

fn render_diff(out: &mut Stdout, w: u16, h: u16, prev: &mut [Cell], cur: &[Cell]) -> io::Result<()> {
    let mut cur_fg = Color::Reset;
    let mut cur_bg = Color::Reset;

    for y in 0..h as usize {
        for x in 0..w as usize {
            let i = y * (w as usize) + x;
            if prev[i] == cur[i] {
                continue;
            }
            prev[i] = cur[i];

            let c = cur[i];
            queue!(out, cursor::MoveTo(x as u16, y as u16))?;

            if c.bg != cur_bg {
                cur_bg = c.bg;
                queue!(out, SetBackgroundColor(cur_bg))?;
            }
            if c.fg != cur_fg {
                cur_fg = c.fg;
                queue!(out, SetForegroundColor(cur_fg))?;
            }
            queue!(out, Print(c.ch))?;
        }
    }
    Ok(())
}

// -------------------- Orrery physics (Kepler) --------------------
#[derive(Clone, Copy)]
struct OrbitalElements {
    a: f32,        // AU
    e: f32,
    i: f32,        // rad
    omega: f32,    // arg periapsis
    big_omega: f32,// asc node
    m0: f32,       // mean anomaly at epoch
    n: f32,        // rad/day
    period_days: f32,
}

#[derive(Clone, Copy)]
struct Body {
    name: &'static str,
    color: Color,
    el: OrbitalElements,
}

fn solve_kepler(m: f32, e: f32) -> f32 {
    let mut e_anom = m;
    for _ in 0..9 {
        let f = e_anom - e * e_anom.sin() - m;
        let fp = 1.0 - e * e_anom.cos();
        e_anom -= f / fp;
    }
    e_anom
}

fn heliocentric_pos(el: OrbitalElements, days_since_epoch: f32) -> Vec2 {
    let m = normalize_angle(el.m0 + el.n * days_since_epoch);
    let e_anom = solve_kepler(m, el.e);

    let cos_e = e_anom.cos();
    let sin_e = e_anom.sin();
    let r = el.a * (1.0 - el.e * cos_e);
    let nu = ((1.0 - el.e * el.e).sqrt() * sin_e).atan2(cos_e - el.e);

    let x_op = r * nu.cos();
    let y_op = r * nu.sin();

    let cos_om = el.big_omega.cos();
    let sin_om = el.big_omega.sin();
    let cos_w = el.omega.cos();
    let sin_w = el.omega.sin();
    let cos_i = el.i.cos();

    let x = (cos_om * cos_w - sin_om * sin_w * cos_i) * x_op
        + (-cos_om * sin_w - sin_om * cos_w * cos_i) * y_op;
    let y = (sin_om * cos_w + cos_om * sin_w * cos_i) * x_op
        + (-sin_om * sin_w + cos_om * cos_w * cos_i) * y_op;

    Vec2 { x, y }
}

// -------------------- Planet style for detail view --------------------
#[derive(Clone, Copy)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}
impl Rgb {
    fn to_color(self) -> Color {
        Color::Rgb { r: self.r, g: self.g, b: self.b }
    }
}

#[derive(Clone, Copy)]
struct PlanetStyle {
    name: &'static str,
    base: Rgb,
    accent: Rgb,
    ocean: Rgb,
    atmosphere: Rgb,
    rings: bool,
    seed: u32,
    roughness: f32,
    bands: f32,
    clouds: f32,
    ice: f32,
}

#[derive(Clone, Copy)]
struct PlanetFacts {
    first_observed: &'static str,
    discovered_by: &'static str,
    atmosphere: &'static str,
    trivia: &'static str,
}

fn mix_rgb(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = clamp01(t);
    Rgb {
        r: lerp_u8(a.r, b.r, t),
        g: lerp_u8(a.g, b.g, t),
        b: lerp_u8(a.b, b.b, t),
    }
}
fn scale_rgb(a: Rgb, t: f32) -> Rgb {
    let t = clamp01(t);
    Rgb {
        r: clamp((a.r as f32) * t, 0.0, 255.0) as u8,
        g: clamp((a.g as f32) * t, 0.0, 255.0) as u8,
        b: clamp((a.b as f32) * t, 0.0, 255.0) as u8,
    }
}
fn color_to_rgb(c: Color) -> Rgb {
    match c {
        Color::Rgb { r, g, b } => Rgb { r, g, b },
        Color::Grey => Rgb { r: 170, g: 170, b: 170 },
        Color::DarkGrey => Rgb { r: 110, g: 110, b: 110 },
        Color::Yellow => Rgb { r: 255, g: 220, b: 90 },
        Color::Cyan => Rgb { r: 80, g: 210, b: 255 },
        Color::Red => Rgb { r: 255, g: 90, b: 80 },
        Color::Blue => Rgb { r: 90, g: 140, b: 255 },
        _ => Rgb { r: 200, g: 200, b: 200 },
    }
}

// -------------------- Main --------------------
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Orrery,
    PlanetDetail,
}

#[derive(Clone, Copy)]
struct OrbitView {
    cam_pan: Vec2,
    cam_zoom: f32,
    cam_rot: f32,
    show_labels: bool,
    show_orbits: bool,
    show_trails: bool,
    show_axes: bool,
}

#[derive(Clone, Copy)]
struct Star {
    x: u16,
    y: u16,
    phase: f32,
    depth: f32,
}

fn build_stars(w: u16, h: u16, count: usize, seed: u64) -> Vec<Star> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut stars = Vec::with_capacity(count);
    if w == 0 || h == 0 {
        return stars;
    }
    for _ in 0..count {
        stars.push(Star {
            x: rng.gen_range(0..w),
            y: rng.gen_range(0..h),
            phase: rng.gen_range(0.0..(PI * 2.0)),
            depth: rng.gen_range(0.35..1.0),
        });
    }
    stars
}

fn main() -> io::Result<()> {
    let mut out = io::stdout();
    terminal::enable_raw_mode()?;
    execute!(out, EnterAlternateScreen, cursor::Hide, DisableLineWrap)?;
    let res = run(&mut out);
    execute!(out, EndSynchronizedUpdate, ResetColor, cursor::Show, EnableLineWrap, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    res
}

fn run(out: &mut Stdout) -> io::Result<()> {
    let bodies = default_bodies();
    let styles = default_styles();
    let facts = default_facts();

    let epoch = Utc.with_ymd_and_hms(2000, 1, 1, 12, 0, 0).unwrap();
    let mut sim_time_utc: DateTime<Utc> = Utc::now();
    let mut warp_days_per_sec: f32 = 1.0;
    let mut paused = false;

    let mut mode = Mode::Orrery;

    // selection and follow
    let mut selected: usize = 3; // Earth (Sun = 0)
    let mut follow: Option<usize> = Some(selected); // follow selection by default

    // detail view params
    let mut detail_rot_speed: f32 = 0.55;
    let mut detail_tilt: f32 = 0.25;
    let mut detail_rot: f32 = 0.0;

    // orbit view params
    let mut orbit_view = OrbitView {
        cam_pan: Vec2 { x: 0.0, y: 0.0 },
        cam_zoom: 1.0,
        cam_rot: 0.0,
        show_labels: true,
        show_orbits: true,
        show_trails: true,
        show_axes: true,
    };
    let mut trails: Vec<Vec<Vec2>> = vec![Vec::new(); bodies.len()];
    let trail_len: usize = 120;

    // buffers
    let mut prev_w: u16 = 0;
    let mut prev_h: u16 = 0;
    let mut prev_buf: Vec<Cell> = Vec::new();
    let mut cur_buf: Vec<Cell> = Vec::new();

    // rng (stars/grain)
    let mut rng = StdRng::seed_from_u64(0xA11CE_0BEEF);
    let mut stars: Vec<Star> = Vec::new();

    let mut last_frame = Instant::now();
    let start_time = Instant::now();
    let frame_dt = Duration::from_millis(1000 / FPS_CAP);

    loop {
        // input
        while event::poll(Duration::from_millis(0))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    match k.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(()),

                        KeyCode::Char('p') | KeyCode::Char('P') => paused = !paused,

                        KeyCode::Char('i') | KeyCode::Char('I') => {
                            mode = if mode == Mode::Orrery { Mode::PlanetDetail } else { Mode::Orrery };
                            execute!(out, terminal::Clear(ClearType::All))?;
                            for c in prev_buf.iter_mut() {
                                *c = Cell::blank(Color::Black);
                            }
                        }

                        KeyCode::Char('0') => {
                            selected = 0;
                            follow = None;
                            orbit_view.cam_pan = Vec2 { x: 0.0, y: 0.0 };
                        }
                        KeyCode::Char('1') => { selected = 1; follow = Some(selected); }
                        KeyCode::Char('2') => { selected = 2; follow = Some(selected); }
                        KeyCode::Char('3') => { selected = 3; follow = Some(selected); }
                        KeyCode::Char('4') => { selected = 4; follow = Some(selected); }
                        KeyCode::Char('5') => { selected = 5; follow = Some(selected); }
                        KeyCode::Char('6') => { selected = 6; follow = Some(selected); }
                        KeyCode::Char('7') => { selected = 7; follow = Some(selected); }
                        KeyCode::Char('8') => { selected = 8; follow = Some(selected); }
                        KeyCode::Char('9') => { selected = 9; follow = Some(selected); }

                        // mode-specific navigation
                        _ => {
                            if mode == Mode::PlanetDetail {
                                match k.code {
                                    KeyCode::Left => { selected = selected.saturating_sub(1); follow = Some(selected); }
                                    KeyCode::Right => { selected = (selected + 1).min(bodies.len() - 1); follow = Some(selected); }
                                    KeyCode::Up => detail_tilt = (detail_tilt + 0.06).min(0.85),
                                    KeyCode::Down => detail_tilt = (detail_tilt - 0.06).max(-0.85),
                                    KeyCode::Char('[') => detail_rot_speed -= 0.08,
                                    KeyCode::Char(']') => detail_rot_speed += 0.08,
                                    KeyCode::Char('r') | KeyCode::Char('R') => {
                                        detail_rot_speed = 0.55;
                                        detail_tilt = 0.25;
                                        detail_rot = 0.0;
                                    }
                                    _ => {}
                                }
                            } else {
                                match k.code {
                                    KeyCode::Left => orbit_view.cam_pan.x -= 0.35 / orbit_view.cam_zoom,
                                    KeyCode::Right => orbit_view.cam_pan.x += 0.35 / orbit_view.cam_zoom,
                                    KeyCode::Up => orbit_view.cam_pan.y -= 0.35 / orbit_view.cam_zoom,
                                    KeyCode::Down => orbit_view.cam_pan.y += 0.35 / orbit_view.cam_zoom,
                                    KeyCode::Char('w') | KeyCode::Char('W') => {
                                        orbit_view.cam_zoom = (orbit_view.cam_zoom * 1.10).min(6.0);
                                    }
                                    KeyCode::Char('s') | KeyCode::Char('S') => {
                                        orbit_view.cam_zoom = (orbit_view.cam_zoom / 1.10).max(0.25);
                                    }
                                    KeyCode::Char('a') | KeyCode::Char('A') => orbit_view.cam_rot -= 0.08,
                                    KeyCode::Char('d') | KeyCode::Char('D') => orbit_view.cam_rot += 0.08,
                                    KeyCode::Char('r') | KeyCode::Char('R') => {
                                        orbit_view.cam_pan = Vec2 { x: 0.0, y: 0.0 };
                                        orbit_view.cam_zoom = 1.0;
                                        orbit_view.cam_rot = 0.0;
                                    }
                                    KeyCode::Char('l') | KeyCode::Char('L') => {
                                        orbit_view.show_labels = !orbit_view.show_labels;
                                    }
                                    KeyCode::Char('o') | KeyCode::Char('O') => {
                                        orbit_view.show_orbits = !orbit_view.show_orbits;
                                    }
                                    KeyCode::Char('t') | KeyCode::Char('T') => {
                                        orbit_view.show_trails = !orbit_view.show_trails;
                                    }
                                    KeyCode::Char('x') | KeyCode::Char('X') => {
                                        orbit_view.show_axes = !orbit_view.show_axes;
                                    }
                                    KeyCode::Char('f') | KeyCode::Char('F') => {
                                        follow = if follow.is_some() { None } else { Some(selected) };
                                    }
                                    KeyCode::Char('n') | KeyCode::Char('N') => sim_time_utc = Utc::now(),
                                    KeyCode::Char('e') | KeyCode::Char('E') => sim_time_utc = epoch,
                                    KeyCode::Char('=') | KeyCode::Char('+') => warp_days_per_sec *= 2.0,
                                    KeyCode::Char('-') => warp_days_per_sec *= 0.5,
                                    KeyCode::Char(']') => warp_days_per_sec *= 1.25,
                                    KeyCode::Char('[') => warp_days_per_sec *= 0.8,
                                    KeyCode::Char(',') => {
                                        if paused {
                                            sim_time_utc = sim_time_utc - chrono::Duration::hours(6);
                                        }
                                    }
                                    KeyCode::Char('.') => {
                                        if paused {
                                            sim_time_utc = sim_time_utc + chrono::Duration::hours(6);
                                        }
                                    }
                                    KeyCode::Char('j') | KeyCode::Char('J') => {
                                        if paused {
                                            sim_time_utc = sim_time_utc - chrono::Duration::days(1);
                                        }
                                    }
                                    KeyCode::Char('k') | KeyCode::Char('K') => {
                                        if paused {
                                            sim_time_utc = sim_time_utc + chrono::Duration::days(1);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        // resize and (re)alloc buffers
        let (w, h) = terminal::size()?;
        if w != prev_w || h != prev_h {
            prev_w = w.max(70);
            prev_h = h.max(24);
            prev_buf = vec![Cell::blank(Color::Black); (prev_w as usize) * (prev_h as usize)];
            cur_buf = vec![Cell::blank(Color::Black); (prev_w as usize) * (prev_h as usize)];
            execute!(out, terminal::Clear(ClearType::All))?;
            let hud_w = 32u16.min(prev_w / 2);
            let main_w = prev_w.saturating_sub(hud_w);
            let area = (main_w as usize).saturating_mul(prev_h as usize);
            let count = (area / 70).clamp(60, 240);
            let seed = 0x5A17_5A17u64 ^ ((prev_w as u64) << 32) ^ (prev_h as u64);
            stars = build_stars(main_w, prev_h, count, seed);
        }

        let now = Instant::now();
        let dt = (now - last_frame).as_secs_f32().min(0.05);
        last_frame = now;

        if !paused && warp_days_per_sec != 0.0 {
            let add_days = (warp_days_per_sec * dt) as f64;
            sim_time_utc = sim_time_utc + chrono::Duration::milliseconds((add_days * 86_400_000.0) as i64);
        }
        if !paused {
            detail_rot += dt * detail_rot_speed;
        }

        // compute positions
        let days_since_epoch = (sim_time_utc - epoch).num_seconds() as f32 / 86_400.0;
        let mut pos: Vec<Vec2> = Vec::with_capacity(bodies.len());
        for b in &bodies {
            pos.push(heliocentric_pos(b.el, days_since_epoch));
        }
        if !paused {
            for (i, p) in pos.iter().enumerate() {
                if i == 0 {
                    continue;
                }
                let trail = &mut trails[i];
                trail.push(*p);
                if trail.len() > trail_len {
                    let overflow = trail.len() - trail_len;
                    trail.drain(0..overflow);
                }
            }
        }

        // clear cur buffer
        for c in cur_buf.iter_mut() {
            *c = Cell::blank(Color::Black);
        }

        // render mode
        match mode {
            Mode::Orrery => {
                render_orrery(
                    &mut cur_buf,
                    prev_w,
                    prev_h,
                    &bodies,
                    &pos,
                    &trails,
                    &stars,
                    start_time.elapsed().as_secs_f32(),
                    sim_time_utc.with_timezone(&Local),
                    warp_days_per_sec,
                    paused,
                    follow,
                    selected,
                    &orbit_view,
                );
            }
            Mode::PlanetDetail => {
                render_planet_detail(
                    &mut cur_buf,
                    prev_w,
                    prev_h,
                    &bodies,
                    &pos,
                    &styles,
                    &facts,
                    selected,
                    sim_time_utc.with_timezone(&Local),
                    warp_days_per_sec,
                    paused,
                    detail_rot,
                    detail_tilt,
                    &mut rng,
                );
            }
        }

        // flush diff
        execute!(out, BeginSynchronizedUpdate)?;
        render_diff(out, prev_w, prev_h, &mut prev_buf, &cur_buf)?;
        execute!(out, EndSynchronizedUpdate)?;
        out.flush()?;

        // cap fps
        let elapsed = Instant::now() - now;
        if elapsed < frame_dt {
            std::thread::sleep(frame_dt - elapsed);
        }
    }
}

// -------------------- Orrery renderer (simple, cell-based) --------------------
fn render_orrery(
    buf: &mut [Cell],
    w: u16,
    h: u16,
    bodies: &[Body],
    pos: &[Vec2],
    trails: &[Vec<Vec2>],
    stars: &[Star],
    t_real: f32,
    sim_local: DateTime<Local>,
    warp_days_per_sec: f32,
    paused: bool,
    follow: Option<usize>,
    selected: usize,
    view: &OrbitView,
) {
    let bg = Color::Black;
    let fg = Color::Rgb { r: 220, g: 220, b: 220 };
    let dim = Color::Rgb { r: 120, g: 120, b: 120 };
    let edge = Color::Rgb { r: 80, g: 95, b: 120 };

    // layout: main + right HUD
    let hud_w = 32u16.min(w / 2);
    let main_w = w.saturating_sub(hud_w);

    // divider
    for y in 0..h {
        put_cell(buf, w, h, main_w, y, Cell { ch: '│', fg: edge, bg });
    }

    // HUD boxes
    let top_h = 9u16.min(h.saturating_sub(8).max(6));
    let bottom_h = h.saturating_sub(top_h);
    box_draw(buf, w, h, main_w, 0, hud_w, top_h, edge, bg);
    box_draw(buf, w, h, main_w, top_h, hud_w, bottom_h, edge, bg);

    let panel_x = main_w + 2;
    let panel_w = hud_w.saturating_sub(4);
    let mut ty = 1u16;
    let top_limit = top_h.saturating_sub(2);
    write_str(buf, w, h, panel_x, ty, "Orrery", fg, bg);
    ty = ty.saturating_add(1);
    if ty <= top_limit {
        ty = ty.saturating_add(write_wrapped(
            buf,
            w,
            h,
            panel_x,
            ty,
            panel_w,
            &format!("Time: {}", sim_local.format("%Y-%m-%d %H:%M:%S")),
            dim,
            bg,
        ));
    }
    if ty <= top_limit {
        ty = ty.saturating_add(write_wrapped(
            buf,
            w,
            h,
            panel_x,
            ty,
            panel_w,
            &format!("Warp: {:.2} d/s", warp_days_per_sec),
            dim,
            bg,
        ));
    }
    if ty <= top_limit {
        ty = ty.saturating_add(write_wrapped(
            buf,
            w,
            h,
            panel_x,
            ty,
            panel_w,
            &format!("State: {}", if paused { "paused" } else { "running" }),
            dim,
            bg,
        ));
    }
    if ty <= top_limit {
        let follow_name = match follow {
            None => "Sun",
            Some(i) if i < bodies.len() => bodies[i].name,
            _ => "Unknown",
        };
        ty = ty.saturating_add(write_wrapped(
            buf,
            w,
            h,
            panel_x,
            ty,
            panel_w,
            &format!("Follow: {}  Selected: {}", follow_name, bodies[selected].name),
            dim,
            bg,
        ));
    }

    // draw a simple top-down orbit map (points + dotted rings)
    let cx = (main_w as f32) * 0.50;
    let cy = (h as f32) * 0.52;
    let base_scale = (main_w as f32).min(h as f32) * 0.075; // AU to cells (cinematic)
    let scale = base_scale * view.cam_zoom;
    let follow_pos = follow.map(|i| pos[i]).unwrap_or(Vec2 { x: 0.0, y: 0.0 });
    let au_per_cell = 1.0 / scale.max(1e-6);
    let d_follow = follow
        .map(|i| pos[selected].sub(pos[i]).len())
        .unwrap_or(pos[selected].len());
    if ty <= top_limit {
        ty = ty.saturating_add(write_wrapped(
            buf,
            w,
            h,
            panel_x,
            ty,
            panel_w,
            &format!("r(sun): {:.3} AU", pos[selected].len()),
            dim,
            bg,
        ));
    }

    let mut by = top_h.saturating_add(1);
    let bottom_limit = h.saturating_sub(2);
    if by <= bottom_limit {
        write_str(buf, w, h, panel_x, by, "Controls", fg, bg);
        by = by.saturating_add(1);
    }
    for line in [
        "Select: 0 Sun | 1-9 planets | F follow",
        "Move: arrows pan | W/S zoom | A/D rotate",
        "Time: +/- coarse | [/] fine",
        "Scrub: ,/. +/-6h | J/K +/-1d (paused)",
        "Jump: N now | E epoch | R reset view",
        "Modes: P pause | I detail | Q quit",
    ] {
        if by > bottom_limit {
            break;
        }
        let used = write_wrapped(buf, w, h, panel_x, by, panel_w, line, dim, bg);
        by = by.saturating_add(used);
    }
    if by <= bottom_limit {
        by = by.saturating_add(1);
        write_str(buf, w, h, panel_x, by, "View", fg, bg);
        by = by.saturating_add(1);
    }
    for line in [
        format!("Zoom: {:.2}x  Rot: {:.0}°", view.cam_zoom, view.cam_rot.to_degrees()),
        format!("Scale: 1 cell = {:.3} AU", au_per_cell),
        format!(
            "Toggles: labels {} | orbits {} | trails {} | axes {}",
            if view.show_labels { "on" } else { "off" },
            if view.show_orbits { "on" } else { "off" },
            if view.show_trails { "on" } else { "off" },
            if view.show_axes { "on" } else { "off" }
        ),
        format!("d(follow): {:.3} AU", d_follow),
    ] {
        if by > bottom_limit {
            break;
        }
        let used = write_wrapped(buf, w, h, panel_x, by, panel_w, &line, dim, bg);
        by = by.saturating_add(used);
    }

    if view.show_axes {
        for x in 1..main_w.saturating_sub(1) {
            put_cell(buf, w, h, x, cy as u16, Cell { ch: '·', fg: Color::Rgb { r: 70, g: 80, b: 90 }, bg });
        }
        for y in 1..h.saturating_sub(1) {
            put_cell(buf, w, h, cx as u16, y, Cell { ch: '·', fg: Color::Rgb { r: 70, g: 80, b: 90 }, bg });
        }
    }

    for s in stars {
        if s.x >= main_w {
            continue;
        }
        let tw = (t_real * 0.65 + s.phase).sin() * 0.5 + 0.5;
        let b = lerp(0.2, 1.0, tw * s.depth);
        let c = clamp(40.0 + b * 180.0, 0.0, 255.0) as u8;
        let ch = if b > 0.82 { '✦' } else if b > 0.62 { '•' } else { '·' };
        put_cell(
            buf,
            w,
            h,
            s.x,
            s.y,
            Cell {
                ch,
                fg: Color::Rgb { r: c, g: c, b: (c as u16 + 25).min(255) as u8 },
                bg,
            },
        );
    }

    if view.show_axes {
        let mut bar_au = 1.0;
        if scale * bar_au < 8.0 {
            bar_au = 2.0;
        } else if scale * bar_au > 18.0 {
            bar_au = 0.5;
        }
        let bar_len = (bar_au * scale).round().max(3.0) as u16;
        let bar_x = 2u16;
        let bar_y = h.saturating_sub(2);
        for x in bar_x..bar_x.saturating_add(bar_len).min(main_w.saturating_sub(2)) {
            put_cell(buf, w, h, x, bar_y, Cell { ch: '─', fg: dim, bg });
        }
        write_str(buf, w, h, bar_x, bar_y.saturating_sub(1), &format!("{:.1} AU", bar_au), dim, bg);
    }

    // Sun
    let sun_screen = rot2(Vec2 { x: 0.0, y: 0.0 }.sub(follow_pos).add(view.cam_pan), view.cam_rot);
    let sun_x = cx + sun_screen.x * scale;
    let sun_y = cy + sun_screen.y * scale * 0.92;
    if sun_x >= 1.0 && sun_y >= 1.0 && sun_x < (main_w - 1) as f32 && sun_y < (h - 1) as f32 {
        put_cell(
            buf,
            w,
            h,
            sun_x as u16,
            sun_y as u16,
            Cell { ch: '●', fg: Color::Rgb { r: 255, g: 220, b: 120 }, bg },
        );
    }

    if view.show_orbits {
        let rings = [0.39, 0.72, 1.00, 1.52, 5.20, 9.58, 19.19, 30.07, 39.48];
        for (ri, r_au) in rings.iter().enumerate() {
            let rr = *r_au;
            let steps = ((rr * scale) * 6.0).max(30.0) as i32;
            for s in 0..steps {
                let a = 2.0 * PI * (s as f32 / steps as f32);
                let p = Vec2 { x: rr * a.cos(), y: rr * a.sin() };
                let v = rot2(p.sub(follow_pos).add(view.cam_pan), view.cam_rot);
                let x = cx + v.x * scale;
                let y = cy + v.y * scale * 0.92;
                if x >= 1.0 && y >= 1.0 && x < (main_w - 1) as f32 && y < (h - 1) as f32 {
                    if (s + ri as i32) % 3 == 0 {
                        put_cell(buf, w, h, x as u16, y as u16, Cell { ch: '·', fg: edge, bg });
                    }
                }
            }
        }
    }

    if view.show_trails {
        for (i, trail) in trails.iter().enumerate() {
            if i == 0 {
                continue;
            }
            let rgb = color_to_rgb(bodies[i].color);
            for (ti, p) in trail.iter().enumerate() {
                let fade = lerp(0.15, 0.90, (ti as f32) / (trail.len().max(1) as f32));
                let col = scale_rgb(rgb, fade);
                let v = rot2(p.sub(follow_pos).add(view.cam_pan), view.cam_rot);
                let x = cx + v.x * scale;
                let y = cy + v.y * scale * 0.92;
                if x >= 1.0 && y >= 1.0 && x < (main_w - 1) as f32 && y < (h - 1) as f32 {
                    put_cell(buf, w, h, x as u16, y as u16, Cell { ch: '·', fg: col.to_color(), bg });
                }
            }
        }
    }

    // planets as points
    for (i, b) in bodies.iter().enumerate() {
        let p = pos[i];
        let v = rot2(p.sub(follow_pos).add(view.cam_pan), view.cam_rot);
        let x = cx + v.x * scale;
        let y = cy + v.y * scale * 0.92;
        if x >= 1.0 && y >= 1.0 && x < (main_w - 1) as f32 && y < (h - 1) as f32 {
            let ch = if i == 0 { '●' } else if i == selected { '◆' } else { '●' };
            let fg = if i == 0 {
                Color::Rgb { r: 255, g: 220, b: 140 }
            } else {
                b.color
            };
            put_cell(buf, w, h, x as u16, y as u16, Cell { ch, fg, bg });
            if view.show_labels {
                let label_fg = if i == selected { fg } else { dim };
                write_str(buf, w, h, x as u16 + 2, y as u16, b.name, label_fg, bg);
            }
        }
    }

}

// -------------------- Planet detail view renderer --------------------
fn render_planet_detail(
    buf: &mut [Cell],
    w: u16,
    h: u16,
    bodies: &[Body],
    pos: &[Vec2],
    styles: &[PlanetStyle],
    facts: &[PlanetFacts],
    selected: usize,
    sim_local: DateTime<Local>,
    warp_days_per_sec: f32,
    paused: bool,
    rot: f32,
    tilt: f32,
    rng: &mut StdRng,
) {
    let bg = Color::Black;
    let edge = Color::Rgb { r: 90, g: 100, b: 120 };
    let fg = Color::Rgb { r: 220, g: 230, b: 245 };
    let dim = Color::Rgb { r: 140, g: 155, b: 175 };

    // layout
    let panel_w = 34u16.min(w / 2);
    let left_w = w.saturating_sub(panel_w);

    // starfield backdrop (left)
    for _ in 0..140 {
        let x = rng.gen_range(0..left_w.saturating_sub(1).max(1));
        let y = rng.gen_range(0..h.max(1));
        if (x + y) % 7 == 0 {
            put_cell(buf, w, h, x, y, Cell { ch: '·', fg: Color::Rgb { r: 90, g: 95, b: 110 }, bg });
        }
    }

    // divider
    for y in 0..h {
        put_cell(buf, w, h, left_w, y, Cell { ch: '│', fg: edge, bg });
    }

    // right panels
    box_draw(buf, w, h, left_w, 0, panel_w, 9, edge, bg);
    box_draw(buf, w, h, left_w, 9, panel_w, h.saturating_sub(9), edge, bg);

    let b = bodies[selected];
    let style = styles[selected];
    let info = facts[selected];

    write_str(buf, w, h, left_w + 2, 1, "Planet Detail", fg, bg);
    write_str(buf, w, h, left_w + 2, 2, &format!("Selected: {}", b.name), style.accent.to_color(), bg);
    write_str(buf, w, h, left_w + 2, 3, &format!("Time: {}", sim_local.format("%Y-%m-%d %H:%M:%S")), dim, bg);
    write_str(buf, w, h, left_w + 2, 4, &format!("Warp: {:.2} d/s", warp_days_per_sec), dim, bg);
    write_str(buf, w, h, left_w + 2, 5, &format!("State: {}", if paused { "paused" } else { "running" }), dim, bg);
    write_str(buf, w, h, left_w + 2, 6, "Keys: I back | ←/→ change | ↑/↓ tilt | [/] rot | R reset", dim, bg);

    // compute some info from current position
    let p = pos[selected];
    let r_au = p.len();

    // rough speed estimate using mean motion n (AU/day converted to km/s approximately)
    // 1 AU/day ≈ 1731.456 km/s
    let v_au_day = b.el.n * b.el.a; // coarse
    let v_kms = v_au_day.abs() * 1731.456;

    let y0 = 10;
    write_str(buf, w, h, left_w + 2, y0, "Orbital", fg, bg);
    write_str(buf, w, h, left_w + 2, y0 + 1, &format!("a: {:.3} AU", b.el.a), dim, bg);
    write_str(buf, w, h, left_w + 2, y0 + 2, &format!("e: {:.4}", b.el.e), dim, bg);
    write_str(buf, w, h, left_w + 2, y0 + 3, &format!("i: {:.2}°", b.el.i * 180.0 / PI), dim, bg);
    write_str(buf, w, h, left_w + 2, y0 + 4, &format!("Period: {:.1} d", b.el.period_days), dim, bg);
    write_str(buf, w, h, left_w + 2, y0 + 6, "Now", fg, bg);
    write_str(buf, w, h, left_w + 2, y0 + 7, &format!("r: {:.3} AU", r_au), dim, bg);
    write_str(buf, w, h, left_w + 2, y0 + 8, &format!("v: {:.1} km/s (approx)", v_kms), dim, bg);

    let facts_y0 = y0 + 10;
    write_str(buf, w, h, left_w + 2, facts_y0, "Facts", fg, bg);
    let facts_w = panel_w.saturating_sub(4);
    let mut fy = facts_y0 + 1;
    for line in [
        format!("First observed: {}", info.first_observed),
        format!("Discovered by: {}", info.discovered_by),
        format!("Atmosphere: {}", info.atmosphere),
        format!("Note: {}", info.trivia),
    ] {
        let used = write_wrapped(buf, w, h, left_w + 2, fy, facts_w, &line, dim, bg);
        fy = fy.saturating_add(used);
        if fy >= h.saturating_sub(2) {
            break;
        }
    }

    // left render area for planet
    let x0 = 1u16;
    let y0v = 1u16;
    let vw = left_w.saturating_sub(2);
    let vh = h.saturating_sub(2);

    let cx = (vw as f32) * 0.52;
    let cy = (vh as f32) * 0.52;
    let radius = (vw as f32).min(vh as f32) * 0.40;

    if selected == 0 {
        render_sun_braille(
            buf,
            w,
            h,
            x0,
            y0v,
            vw,
            vh,
            cx,
            cy,
            radius,
            rot,
            style,
        );
    } else {
        render_procedural_planet_braille(
            buf,
            w,
            h,
            x0,
            y0v,
            vw,
            vh,
            cx,
            cy,
            radius,
            rot,
            tilt,
            style,
        );
        if style.rings {
            render_rings(
                buf,
                w,
                h,
                x0,
                y0v,
                vw,
                vh,
                cx,
                cy,
                radius,
                rot,
                tilt,
                style,
            );
        }
    }

    // add a small label under the planet
    let label_y = (y0v as f32 + cy + radius + 1.5).min((h - 2) as f32) as u16;
    write_str(
        buf,
        w,
        h,
        2,
        label_y,
        &format!("{}  (procedural surface preview)", style.name),
        style.accent.to_color(),
        bg,
    );
}

fn render_procedural_planet_braille(
    buf: &mut [Cell],
    w: u16,
    h: u16,
    x0: u16,
    y0: u16,
    vw: u16,
    vh: u16,
    cx: f32,
    cy: f32,
    r: f32,
    rot: f32,
    tilt: f32,
    style: PlanetStyle,
) {
    let ww = w as usize;
    let hh = h as usize;

    // light direction (fixed)
    let (lx, ly, lz) = v3_norm(-0.34, 0.38, 0.86);

    for y in 0..vh as usize {
        for x in 0..vw as usize {
            let gx = x0 as usize + x;
            let gy = y0 as usize + y;
            if gx >= ww || gy >= hh {
                continue;
            }

            let mut bits = [[false; 2]; 4];
            let mut avg_i = 0.0f32;
            let mut avg_a = 0.0f32;

            let mut col_sum_r: u32 = 0;
            let mut col_sum_g: u32 = 0;
            let mut col_sum_b: u32 = 0;
            let mut col_count: u32 = 0;

            let mut any = false;
            let mut covered = false;

            for sy in 0..4usize {
                for sx in 0..2usize {
                    let px = ((x as f32 + (sx as f32 + 0.5) / 2.0) - cx) * ASPECT_X;
                    let py = (y as f32 + (sy as f32 + 0.5) / 4.0) - cy;

                    let nx = px / r;
                    let ny = py / r;

                    let d2 = nx * nx + ny * ny;
                    if d2 <= 1.0 {
                        covered = true;
                    }

                    // atmosphere glow outside
                    if d2 > 1.0 {
                        let d = d2.sqrt();
                        let glow = clamp01(1.0 - (d - 1.0) / 0.14);
                        if glow > 0.03 {
                            let th = bayer_2x4_threshold(x * 2 + sx, y * 4 + sy);
                            if glow * 0.65 > th {
                                bits[sy][sx] = true;
                                any = true;
                                avg_i += glow * 0.25;
                                avg_a += glow;
                            }
                        }
                        continue;
                    }

                    let nz = (1.0 - d2).sqrt();

                    // tilt around x
                    let (ts, tc) = tilt.sin_cos();
                    let y1 = tc * ny - ts * nz;
                    let z1 = ts * ny + tc * nz;

                    // rotate around y
                    let (x2, y2, z2) = v3_rot_y(nx, y1, z1, rot);

                    let ndotl = v3_dot(x2, y2, z2, lx, ly, lz).max(0.0);
                    let shade = ndotl.powf(1.25);
                    let rim = clamp01((1.0 - ndotl).powf(2.2)) * 0.20;

                    // spherical coords
                    let lat = y2.asin();
                    let lon = x2.atan2(z2);

                    // bands and terrain
                    let bands = (lat * (3.0 + style.bands * 9.0) + (rot * 0.35)).sin();
                    let banding = 0.5 + 0.5 * bands;

                    let n0 = fbm_3d(
                        (lon.cos() * 2.0 + 0.7) * 1.3,
                        (lat.sin() * 2.0 + 0.2) * 1.3,
                        (lon.sin() * 2.0 - 0.4) * 1.3,
                        style.seed,
                        5,
                    );
                    let n1 = fbm_3d(
                        lon * 0.55 + 7.1,
                        lat * 0.85 - 3.4,
                        rot * 0.25 + 1.7,
                        style.seed.wrapping_add(0xBADC0FFE),
                        4,
                    );

                    let rough = lerp(n0, n1, style.roughness);
                    let land = clamp01((rough - 0.48) * 2.2);

                    // clouds
                    let cnoise = fbm_3d(
                        lon * 1.10 + rot * 0.25,
                        lat * 1.55 - rot * 0.10,
                        rot * 0.35 + 2.0,
                        style.seed.wrapping_add(0x13579BDF),
                        5,
                    );
                    let clouds = clamp01((cnoise - 0.56) * 2.7) * style.clouds;

                    // ice caps
                    let cap = clamp01((lat.abs() - (0.86 - style.ice * 0.22)) * 9.0) * style.ice;

                    // albedo
                    let mut col = if land > 0.45 {
                        let t = clamp01(0.25 + 0.75 * land) * (0.65 + 0.35 * banding);
                        mix_rgb(style.base, style.accent, t)
                    } else {
                        let t = clamp01(0.35 + 0.65 * (0.60 * banding + 0.40 * (1.0 - rough)));
                        mix_rgb(style.ocean, style.base, t * 0.25)
                    };
                    if cap > 0.01 {
                        col = mix_rgb(col, style.accent, clamp01(cap * 0.85));
                    }

                    col_sum_r += col.r as u32;
                    col_sum_g += col.g as u32;
                    col_sum_b += col.b as u32;
                    col_count += 1;

                    // intensity
                    let mut intensity = clamp01(shade + rim);
                    intensity = clamp01(intensity + clouds * 0.35);

                    // soft terminator
                    let terminator = clamp01((ndotl - 0.02) * 5.0);
                    intensity *= 0.45 + 0.95 * terminator;

                    // dither
                    let th = bayer_2x4_threshold(x * 2 + sx, y * 4 + sy);
                    let on = intensity > th;
                    bits[sy][sx] = on;
                    any |= on;

                    avg_i += intensity;
                    avg_a += 1.0;
                }
            }

            if !any {
                if covered {
                    let i = gy * ww + gx;
                    buf[i] = Cell { ch: ' ', fg: Color::Reset, bg: Color::Black };
                }
                continue;
            }

            let avg_i = avg_i / avg_a.max(1e-6);
            let col = if col_count > 0 {
                Rgb {
                    r: (col_sum_r / col_count) as u8,
                    g: (col_sum_g / col_count) as u8,
                    b: (col_sum_b / col_count) as u8,
                }
            } else {
                style.base
            };

            // rim tint toward atmosphere
            let px = ((x as f32 + 0.5) - cx) * ASPECT_X;
            let py = (y as f32 + 0.5) - cy;
            let d = ((px / r) * (px / r) + (py / r) * (py / r)).sqrt();
            let rim_t = clamp01((d - 0.86) / 0.18);

            let lit = mix_rgb(col, style.accent, clamp01((avg_i - 0.45) * 0.9));
            let tinted = mix_rgb(lit, style.atmosphere, rim_t * 0.55);

            let dark = clamp01(1.0 - avg_i * 1.35);
            let final_col = mix_rgb(tinted, Rgb { r: 8, g: 10, b: 14 }, dark * 0.55);

            let ch = braille_from_2x4(bits);
            let i = gy * ww + gx;
            buf[i] = Cell { ch, fg: final_col.to_color(), bg: Color::Black };
        }
    }
}

fn render_sun_braille(
    buf: &mut [Cell],
    w: u16,
    h: u16,
    x0: u16,
    y0: u16,
    vw: u16,
    vh: u16,
    cx: f32,
    cy: f32,
    r: f32,
    rot: f32,
    style: PlanetStyle,
) {
    let ww = w as usize;
    let hh = h as usize;

    for y in 0..vh as usize {
        for x in 0..vw as usize {
            let gx = x0 as usize + x;
            let gy = y0 as usize + y;
            if gx >= ww || gy >= hh {
                continue;
            }

            let mut bits = [[false; 2]; 4];
            let mut avg_i = 0.0f32;
            let mut avg_a = 0.0f32;

            let mut col_sum_r: u32 = 0;
            let mut col_sum_g: u32 = 0;
            let mut col_sum_b: u32 = 0;
            let mut col_count: u32 = 0;

            let mut any = false;
            let mut covered = false;

            for sy in 0..4usize {
                for sx in 0..2usize {
                    let px = ((x as f32 + (sx as f32 + 0.5) / 2.0) - cx) * ASPECT_X;
                    let py = (y as f32 + (sy as f32 + 0.5) / 4.0) - cy;

                    let nx = px / r;
                    let ny = py / r;

                    let d2 = nx * nx + ny * ny;
                    if d2 <= 1.0 {
                        covered = true;
                    }

                    // glow outside
                    if d2 > 1.0 {
                        let d = d2.sqrt();
                        let glow = clamp01(1.0 - (d - 1.0) / 0.30);
                        if glow > 0.03 {
                            let th = bayer_2x4_threshold(x * 2 + sx, y * 4 + sy);
                            if glow * 0.85 > th {
                                bits[sy][sx] = true;
                                any = true;
                                avg_i += glow * 0.6;
                                avg_a += glow;
                            }
                        }
                        continue;
                    }

                    let n = fbm_3d(nx * 2.2, ny * 2.2, rot * 0.15, style.seed, 4);
                    let heat = clamp01(0.35 + 0.65 * n);
                    let col = mix_rgb(style.base, style.accent, heat);

                    col_sum_r += col.r as u32;
                    col_sum_g += col.g as u32;
                    col_sum_b += col.b as u32;
                    col_count += 1;

                    let intensity = 0.98;
                    let th = bayer_2x4_threshold(x * 2 + sx, y * 4 + sy);
                    let on = intensity > th;
                    bits[sy][sx] = on;
                    any |= on;

                    avg_i += intensity;
                    avg_a += 1.0;
                }
            }

            if !any {
                if covered {
                    let i = gy * ww + gx;
                    buf[i] = Cell { ch: ' ', fg: Color::Reset, bg: Color::Black };
                }
                continue;
            }

            let col = if col_count > 0 {
                Rgb {
                    r: (col_sum_r / col_count) as u8,
                    g: (col_sum_g / col_count) as u8,
                    b: (col_sum_b / col_count) as u8,
                }
            } else {
                style.base
            };

            let lit = mix_rgb(col, style.accent, 0.35);
            let final_col = scale_rgb(lit, 1.0);

            let ch = braille_from_2x4(bits);
            let i = gy * ww + gx;
            buf[i] = Cell { ch, fg: final_col.to_color(), bg: Color::Black };
        }
    }
}

fn render_rings(
    buf: &mut [Cell],
    w: u16,
    h: u16,
    x0: u16,
    y0: u16,
    vw: u16,
    vh: u16,
    cx: f32,
    cy: f32,
    r: f32,
    rot: f32,
    tilt: f32,
    style: PlanetStyle,
) {
    let ww = w as usize;
    let hh = h as usize;

    let ring_r0 = r * 1.15;
    let ring_r1 = r * 1.62;
    let squash = (0.35 + 0.30 * (tilt.abs())).min(0.70);
    let (rs, rc) = (rot * 0.55).sin_cos();

    for y in 0..vh as usize {
        for x in 0..vw as usize {
            let gx = x0 as usize + x;
            let gy = y0 as usize + y;
            if gx >= ww || gy >= hh {
                continue;
            }

            let px = ((x as f32 + 0.5) - cx) * ASPECT_X;
            let py = (y as f32 + 0.5) - cy;

            let rx = rc * px - rs * py;
            let ry = rs * px + rc * py;

            let ex = rx;
            let ey = ry / squash.max(0.12);
            let d = (ex * ex + ey * ey).sqrt();

            if d < ring_r0 || d > ring_r1 {
                continue;
            }

            let nx = px / r;
            let ny = py / r;
            if nx * nx + ny * ny <= 1.0 {
                continue;
            }

            let band = 0.5 + 0.5 * ((d / r) * 10.0 + rot * 0.8).sin();
            let alpha = clamp01(0.30 + 0.45 * band);

            let th = ((x + y) & 7) as f32 / 8.0;
            if alpha < th {
                continue;
            }

            let col = mix_rgb(style.base, style.accent, 0.35 + 0.45 * band);
            let i = gy * ww + gx;
            if buf[i].ch == ' ' || buf[i].ch == '·' {
                buf[i] = Cell {
                    ch: if band > 0.6 { '─' } else { '╌' },
                    fg: col.to_color(),
                    bg: Color::Black,
                };
            }
        }
    }
}

// -------------------- Data --------------------
fn default_bodies() -> Vec<Body> {
    vec![
        Body {
            name: "Sun",
            color: Color::Rgb { r: 255, g: 220, b: 140 },
            el: OrbitalElements {
                a: 0.0, e: 0.0, i: 0.0,
                big_omega: 0.0, omega: 0.0,
                m0: 0.0,
                period_days: 0.0,
                n: 0.0,
            },
        },
        Body {
            name: "Mercury",
            color: Color::Grey,
            el: OrbitalElements {
                a: 0.387, e: 0.2056, i: deg(7.0),
                big_omega: deg(48.3), omega: deg(29.1),
                m0: deg(174.8),
                period_days: 87.969,
                n: 2.0 * PI / 87.969,
            },
        },
        Body {
            name: "Venus",
            color: Color::Yellow,
            el: OrbitalElements {
                a: 0.723, e: 0.0068, i: deg(3.4),
                big_omega: deg(76.7), omega: deg(54.9),
                m0: deg(50.4),
                period_days: 224.701,
                n: 2.0 * PI / 224.701,
            },
        },
        Body {
            name: "Earth",
            color: Color::Cyan,
            el: OrbitalElements {
                a: 1.000, e: 0.0167, i: deg(0.0),
                big_omega: deg(0.0), omega: deg(102.9),
                m0: deg(357.5),
                period_days: 365.256,
                n: 2.0 * PI / 365.256,
            },
        },
        Body {
            name: "Mars",
            color: Color::Red,
            el: OrbitalElements {
                a: 1.524, e: 0.0934, i: deg(1.85),
                big_omega: deg(49.6), omega: deg(286.5),
                m0: deg(19.4),
                period_days: 686.980,
                n: 2.0 * PI / 686.980,
            },
        },
        Body {
            name: "Jupiter",
            color: Color::Rgb { r: 255, g: 200, b: 160 },
            el: OrbitalElements {
                a: 5.203, e: 0.0484, i: deg(1.30),
                big_omega: deg(100.6), omega: deg(273.9),
                m0: deg(20.0),
                period_days: 4332.589,
                n: 2.0 * PI / 4332.589,
            },
        },
        Body {
            name: "Saturn",
            color: Color::Rgb { r: 230, g: 200, b: 150 },
            el: OrbitalElements {
                a: 9.537, e: 0.0542, i: deg(2.49),
                big_omega: deg(113.7), omega: deg(339.4),
                m0: deg(317.0),
                period_days: 10759.22,
                n: 2.0 * PI / 10759.22,
            },
        },
        Body {
            name: "Uranus",
            color: Color::Rgb { r: 160, g: 220, b: 220 },
            el: OrbitalElements {
                a: 19.191, e: 0.0472, i: deg(0.77),
                big_omega: deg(74.0), omega: deg(96.7),
                m0: deg(142.2),
                period_days: 30685.4,
                n: 2.0 * PI / 30685.4,
            },
        },
        Body {
            name: "Neptune",
            color: Color::Blue,
            el: OrbitalElements {
                a: 30.07, e: 0.0086, i: deg(1.77),
                big_omega: deg(131.8), omega: deg(265.6),
                m0: deg(256.2),
                period_days: 60189.0,
                n: 2.0 * PI / 60189.0,
            },
        },
        Body {
            name: "Pluto",
            color: Color::DarkGrey,
            el: OrbitalElements {
                a: 39.48, e: 0.2488, i: deg(17.16),
                big_omega: deg(110.3), omega: deg(113.8),
                m0: deg(14.5),
                period_days: 90560.0,
                n: 2.0 * PI / 90560.0,
            },
        },
    ]
}

fn default_styles() -> Vec<PlanetStyle> {
    vec![
        PlanetStyle {
            name: "Sun",
            base: Rgb { r: 255, g: 190, b: 90 },
            accent: Rgb { r: 255, g: 240, b: 170 },
            ocean: Rgb { r: 0, g: 0, b: 0 },
            atmosphere: Rgb { r: 255, g: 200, b: 120 },
            rings: false,
            seed: 0x51A7_0B57,
            roughness: 0.0,
            bands: 0.0,
            clouds: 0.0,
            ice: 0.0,
        },
        PlanetStyle {
            name: "Mercury",
            base: Rgb { r: 140, g: 140, b: 150 },
            accent: Rgb { r: 220, g: 220, b: 235 },
            ocean: Rgb { r: 20, g: 20, b: 24 },
            atmosphere: Rgb { r: 120, g: 120, b: 140 },
            rings: false,
            seed: 0xA1B2C3D4,
            roughness: 0.92,
            bands: 0.05,
            clouds: 0.02,
            ice: 0.0,
        },
        PlanetStyle {
            name: "Venus",
            base: Rgb { r: 235, g: 180, b: 90 },
            accent: Rgb { r: 255, g: 235, b: 170 },
            ocean: Rgb { r: 50, g: 25, b: 12 },
            atmosphere: Rgb { r: 255, g: 200, b: 110 },
            rings: false,
            seed: 0x11223344,
            roughness: 0.55,
            bands: 0.85,
            clouds: 0.78,
            ice: 0.0,
        },
        PlanetStyle {
            name: "Earth",
            base: Rgb { r: 65, g: 170, b: 90 },
            accent: Rgb { r: 170, g: 220, b: 255 },
            ocean: Rgb { r: 10, g: 35, b: 55 },
            atmosphere: Rgb { r: 120, g: 200, b: 255 },
            rings: false,
            seed: 0x1337BEEF,
            roughness: 0.70,
            bands: 0.15,
            clouds: 0.65,
            ice: 0.25,
        },
        PlanetStyle {
            name: "Mars",
            base: Rgb { r: 210, g: 70, b: 35 },
            accent: Rgb { r: 255, g: 160, b: 90 },
            ocean: Rgb { r: 40, g: 15, b: 10 },
            atmosphere: Rgb { r: 255, g: 120, b: 70 },
            rings: false,
            seed: 0xD0C0B0A0,
            roughness: 0.86,
            bands: 0.10,
            clouds: 0.08,
            ice: 0.12,
        },
        PlanetStyle {
            name: "Jupiter",
            base: Rgb { r: 190, g: 140, b: 95 },
            accent: Rgb { r: 255, g: 220, b: 180 },
            ocean: Rgb { r: 40, g: 25, b: 18 },
            atmosphere: Rgb { r: 255, g: 210, b: 160 },
            rings: false,
            seed: 0xCAFEBABE,
            roughness: 0.45,
            bands: 0.98,
            clouds: 0.35,
            ice: 0.0,
        },
        PlanetStyle {
            name: "Saturn",
            base: Rgb { r: 200, g: 170, b: 120 },
            accent: Rgb { r: 255, g: 230, b: 180 },
            ocean: Rgb { r: 45, g: 30, b: 20 },
            atmosphere: Rgb { r: 255, g: 220, b: 170 },
            rings: true,
            seed: 0xB16B_00B5,
            roughness: 0.35,
            bands: 0.90,
            clouds: 0.30,
            ice: 0.0,
        },
        PlanetStyle {
            name: "Uranus",
            base: Rgb { r: 120, g: 200, b: 210 },
            accent: Rgb { r: 200, g: 250, b: 245 },
            ocean: Rgb { r: 20, g: 40, b: 55 },
            atmosphere: Rgb { r: 170, g: 230, b: 230 },
            rings: true,
            seed: 0x55AA_11EE,
            roughness: 0.30,
            bands: 0.25,
            clouds: 0.18,
            ice: 0.10,
        },
        PlanetStyle {
            name: "Neptune",
            base: Rgb { r: 70, g: 120, b: 200 },
            accent: Rgb { r: 160, g: 200, b: 255 },
            ocean: Rgb { r: 10, g: 20, b: 40 },
            atmosphere: Rgb { r: 130, g: 170, b: 255 },
            rings: true,
            seed: 0x3C5A_9DFF,
            roughness: 0.40,
            bands: 0.20,
            clouds: 0.25,
            ice: 0.05,
        },
        PlanetStyle {
            name: "Pluto",
            base: Rgb { r: 140, g: 130, b: 120 },
            accent: Rgb { r: 210, g: 200, b: 190 },
            ocean: Rgb { r: 20, g: 18, b: 16 },
            atmosphere: Rgb { r: 120, g: 140, b: 160 },
            rings: false,
            seed: 0x0B1D_5EED,
            roughness: 0.65,
            bands: 0.12,
            clouds: 0.05,
            ice: 0.20,
        },
    ]
}

fn default_facts() -> Vec<PlanetFacts> {
    vec![
        PlanetFacts {
            first_observed: "Known to ancient observers",
            discovered_by: "N/A",
            atmosphere: "Hydrogen and helium plasma",
            trivia: "G2V star; powers the solar system.",
        },
        PlanetFacts {
            first_observed: "Known to ancient observers",
            discovered_by: "N/A",
            atmosphere: "None (trace sodium, oxygen, hydrogen)",
            trivia: "Day longer than its year; extreme temperature swings.",
        },
        PlanetFacts {
            first_observed: "Known to ancient observers",
            discovered_by: "N/A",
            atmosphere: "CO2 ~96%, N2 ~3.5%, sulfuric clouds",
            trivia: "Hottest planet; retrograde rotation.",
        },
        PlanetFacts {
            first_observed: "Known to ancient observers",
            discovered_by: "N/A",
            atmosphere: "N2 ~78%, O2 ~21%, argon + trace gases",
            trivia: "Only world with confirmed surface liquid water.",
        },
        PlanetFacts {
            first_observed: "Known to ancient observers",
            discovered_by: "N/A",
            atmosphere: "CO2 ~95%, N2 ~2.6%, argon ~1.9%",
            trivia: "Home to Olympus Mons, the largest volcano.",
        },
        PlanetFacts {
            first_observed: "Known to ancient observers",
            discovered_by: "N/A",
            atmosphere: "H2 ~90%, He ~10%, methane/ammonia traces",
            trivia: "Great Red Spot is a long-lived storm.",
        },
        PlanetFacts {
            first_observed: "Known to ancient observers",
            discovered_by: "N/A",
            atmosphere: "H2 ~96%, He ~3%, methane traces",
            trivia: "Spectacular rings; lowest density of planets.",
        },
        PlanetFacts {
            first_observed: "1781 (William Herschel)",
            discovered_by: "William Herschel",
            atmosphere: "H2 ~83%, He ~15%, methane ~2%",
            trivia: "Extreme axial tilt; rotates on its side.",
        },
        PlanetFacts {
            first_observed: "1846 (predicted; observed by Galle)",
            discovered_by: "U. Le Verrier, J. Adams; J. G. Galle",
            atmosphere: "H2 ~80%, He ~19%, methane ~1.5%",
            trivia: "Strong winds; dark spot storms appear.",
        },
        PlanetFacts {
            first_observed: "1930 (Clyde Tombaugh)",
            discovered_by: "Clyde Tombaugh",
            atmosphere: "Thin N2 with methane and CO (seasonal)",
            trivia: "Dwarf planet with a complex, icy surface.",
        },
    ]
}

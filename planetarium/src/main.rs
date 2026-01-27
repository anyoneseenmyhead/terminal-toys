use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute, queue,
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

const ASPECT_X: f32 = 0.65;
// const ASPECT_X: f32 = 0.95;

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
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

impl Rgb {
    fn to_color(self) -> Color {
        Color::Rgb {
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }
}

#[derive(Clone, Copy, Debug)]
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
fn mix_rgb(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = clamp01(t);
    Rgb {
        r: lerp_u8(a.r, b.r, t),
        g: lerp_u8(a.g, b.g, t),
        b: lerp_u8(a.b, b.b, t),
    }
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
        // 0..1
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

    // map to 0..1
    clamp01(0.5 + 0.5 * (sum / norm.max(1e-6)))
}

fn bayer_2x4_threshold(ix: usize, iy: usize) -> f32 {
    // 2x4 ordered dither. Values 0..7 -> 0..1
    // pattern chosen to look okay with braille.
    const M: [[u8; 2]; 4] = [
        [0, 4],
        [6, 2],
        [1, 5],
        [7, 3],
    ];
    let v = M[iy & 3][ix & 1] as f32;
    (v + 0.5) / 8.0
}

fn braille_from_2x4(bits: [[bool; 2]; 4]) -> char {
    // braille dots:
    // left column: 1,2,3,7
    // right column:4,5,6,8
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

fn alien_glyph_map() -> Vec<char> {
    // Mix of uncommon but widely supported Unicode blocks.
    // Keep to single-width characters.
    let mut g = Vec::new();
    for c in "ᚠᚢᚦᚨᚱᚲᚷᚹᚺᚾᛁᛃᛇᛈᛉᛋᛏᛒᛖᛗᛚᛜᛞᛟᛞᛠᛡᛢᛣᛤᛥᛦᛧᛨᛩᛪ᛫᛬".chars() {
        g.push(c);
    }
    for c in "ꙮ꙰꙳ꚙꚚꚛꚜꚝꚞꚟꚠꚡꚢꚣꚤꚥꚦꚧꚨꚩ".chars() {
        g.push(c);
    }
    for c in "⟟⟊⟒⟐⟄⟅⟆⟇⟈⟉⟌⟍⟠⟡⟣⟤⟥⟦⟧⟪⟫".chars() {
        g.push(c);
    }
    g
}

fn alienize(s: &str, glyphs: &[char]) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_whitespace() {
            out.push(' ');
            continue;
        }
        let idx = (ch as u32).wrapping_mul(2654435761) as usize;
        out.push(glyphs[idx % glyphs.len()]);
    }
    out
}

fn fmt_alien_num(n: f32, glyphs: &[char]) -> String {
    // Render numeric-ish sequence but fully alien.
    let raw = format!("{:.3}", n);
    alienize(&raw, glyphs)
}

fn box_draw(buf: &mut [Cell], w: u16, h: u16, x0: u16, y0: u16, bw: u16, bh: u16, fg: Color, bg: Color) {
    let x1 = x0.saturating_add(bw.saturating_sub(1));
    let y1 = y0.saturating_add(bh.saturating_sub(1));

    let put = |buf: &mut [Cell], x: u16, y: u16, ch: char, fg: Color, bg: Color| {
        let xi = x as usize;
        let yi = y as usize;
        let ww = w as usize;
        let hh = h as usize;
        if xi >= ww || yi >= hh {
            return;
        }
        buf[yi * ww + xi] = Cell { ch, fg, bg };
    };

    for x in x0 + 1..x1 {
        put(buf, x, y0, '─', fg, bg);
        put(buf, x, y1, '─', fg, bg);
    }
    for y in y0 + 1..y1 {
        put(buf, x0, y, '│', fg, bg);
        put(buf, x1, y, '│', fg, bg);
    }
    put(buf, x0, y0, '┌', fg, bg);
    put(buf, x1, y0, '┐', fg, bg);
    put(buf, x0, y1, '└', fg, bg);
    put(buf, x1, y1, '┘', fg, bg);
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
        if xi < ww && yi < hh {
            buf[yi * ww + xi] = Cell { ch, fg, bg };
        }
        xi += 1;
    }
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

fn main() -> io::Result<()> {
    let mut out = io::stdout();

    let styles = [
        PlanetStyle {
            name: "VIRIDIAN STORM",
            base: Rgb { r: 52, g: 210, b: 140 },
            accent: Rgb { r: 150, g: 255, b: 220 },
            ocean: Rgb { r: 10, g: 35, b: 40 },
            atmosphere: Rgb { r: 90, g: 255, b: 210 },
            rings: false,
            seed: 0xA1B2C3D4,
            roughness: 0.85,
            bands: 0.25,
            clouds: 0.78,
            ice: 0.0,
        },
        PlanetStyle {
            name: "AURELIA DUNEWORLD",
            base: Rgb { r: 235, g: 180, b: 90 },
            accent: Rgb { r: 255, g: 235, b: 170 },
            ocean: Rgb { r: 50, g: 25, b: 12 },
            atmosphere: Rgb { r: 255, g: 200, b: 110 },
            rings: true,
            seed: 0x11223344,
            roughness: 0.55,
            bands: 0.92,
            clouds: 0.12,
            ice: 0.0,
        },
        PlanetStyle {
            name: "CRYOST GLACIER",
            base: Rgb { r: 95, g: 170, b: 255 },
            accent: Rgb { r: 215, g: 245, b: 255 },
            ocean: Rgb { r: 15, g: 25, b: 60 },
            atmosphere: Rgb { r: 140, g: 210, b: 255 },
            rings: false,
            seed: 0x55AA77CC,
            roughness: 0.62,
            bands: 0.18,
            clouds: 0.22,
            ice: 0.85,
        },
        PlanetStyle {
            name: "EMBER BASALT",
            base: Rgb { r: 210, g: 70, b: 35 },
            accent: Rgb { r: 255, g: 160, b: 90 },
            ocean: Rgb { r: 40, g: 15, b: 10 },
            atmosphere: Rgb { r: 255, g: 120, b: 70 },
            rings: false,
            seed: 0xD0C0B0A0,
            roughness: 0.9,
            bands: 0.15,
            clouds: 0.05,
            ice: 0.0,
        },
        PlanetStyle {
            name: "AZURE STRATA",
            base: Rgb { r: 55, g: 120, b: 210 },
            accent: Rgb { r: 170, g: 220, b: 255 },
            ocean: Rgb { r: 10, g: 20, b: 45 },
            atmosphere: Rgb { r: 120, g: 200, b: 255 },
            rings: true,
            seed: 0x0A1B2C3D,
            roughness: 0.45,
            bands: 0.95,
            clouds: 0.55,
            ice: 0.1,
        },
        PlanetStyle {
            name: "VERDANT DELTA",
            base: Rgb { r: 65, g: 170, b: 90 },
            accent: Rgb { r: 160, g: 230, b: 150 },
            ocean: Rgb { r: 12, g: 45, b: 40 },
            atmosphere: Rgb { r: 120, g: 220, b: 170 },
            rings: false,
            seed: 0x1337BEEF,
            roughness: 0.7,
            bands: 0.4,
            clouds: 0.3,
            ice: 0.15,
        },
        PlanetStyle {
            name: "UMBER SHADOW",
            base: Rgb { r: 120, g: 85, b: 65 },
            accent: Rgb { r: 200, g: 165, b: 130 },
            ocean: Rgb { r: 20, g: 15, b: 25 },
            atmosphere: Rgb { r: 160, g: 130, b: 110 },
            rings: true,
            seed: 0xCAFEBABE,
            roughness: 0.82,
            bands: 0.2,
            clouds: 0.18,
            ice: 0.35,
        },
        PlanetStyle {
            name: "OPALINE VEIL",
            base: Rgb { r: 185, g: 200, b: 210 },
            accent: Rgb { r: 245, g: 245, b: 255 },
            ocean: Rgb { r: 30, g: 35, b: 55 },
            atmosphere: Rgb { r: 210, g: 235, b: 255 },
            rings: false,
            seed: 0x9090A5B5,
            roughness: 0.5,
            bands: 0.3,
            clouds: 0.85,
            ice: 0.6,
        },
        PlanetStyle {
            name: "THALASSA DEEP",
            base: Rgb { r: 40, g: 110, b: 160 },
            accent: Rgb { r: 120, g: 210, b: 230 },
            ocean: Rgb { r: 5, g: 20, b: 45 },
            atmosphere: Rgb { r: 90, g: 180, b: 210 },
            rings: false,
            seed: 0x1F2E3D4C,
            roughness: 0.58,
            bands: 0.6,
            clouds: 0.4,
            ice: 0.05,
        },
    ];

    execute!(
        out,
        EnterAlternateScreen,
        DisableLineWrap,
        cursor::Hide,
        terminal::Clear(terminal::ClearType::All)
    )?;
    terminal::enable_raw_mode()?;

    let res = run(&mut out, &styles);

    execute!(
        out,
        EndSynchronizedUpdate,
        ResetColor,
        cursor::Show,
        EnableLineWrap,
        LeaveAlternateScreen
    )?;
    terminal::disable_raw_mode().ok();

    res
}

fn run(out: &mut Stdout, styles: &[PlanetStyle]) -> io::Result<()> {
    let glyphs = alien_glyph_map();
    let mut rng = StdRng::seed_from_u64(0xC0FFEE_1234);

    let mut planet_idx: usize = 0;
    let mut rot_speed: f32 = 0.55;
    let mut tilt: f32 = 0.28;
    let mut paused = false;
    let mut alien_mode = true;

    // stars in normalized space, with depth for twinkle
    let mut stars: Vec<(f32, f32, f32, f32)> = (0..520)
        .map(|_| (rng.gen::<f32>(), rng.gen::<f32>(), rng.gen::<f32>(), rng.gen::<f32>() * 10.0))
        .collect();

    let mut last = Instant::now();
    let mut t: f32 = 0.0;

    let mut prev_w: u16 = 0;
    let mut prev_h: u16 = 0;
    let mut prev_buf: Vec<Cell> = Vec::new();
    let mut cur_buf: Vec<Cell> = Vec::new();

    loop {
        // input
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(KeyEvent { code, kind, modifiers, .. }) if kind == KeyEventKind::Press => {
                    match code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(()),
                        KeyCode::Char('c') | KeyCode::Char('C') => {
                            planet_idx = (planet_idx + 1) % styles.len();
                        }
                        KeyCode::Char('1') => planet_idx = 0.min(styles.len().saturating_sub(1)),
                        KeyCode::Char('2') => planet_idx = 1.min(styles.len().saturating_sub(1)),
                        KeyCode::Char('3') => planet_idx = 2.min(styles.len().saturating_sub(1)),
                        KeyCode::Char('4') => planet_idx = 3.min(styles.len().saturating_sub(1)),
                        KeyCode::Char('5') => planet_idx = 4.min(styles.len().saturating_sub(1)),
                        KeyCode::Char('6') => planet_idx = 5.min(styles.len().saturating_sub(1)),
                        KeyCode::Char('7') => planet_idx = 6.min(styles.len().saturating_sub(1)),
                        KeyCode::Char('8') => planet_idx = 7.min(styles.len().saturating_sub(1)),
                        KeyCode::Char('9') => planet_idx = 8.min(styles.len().saturating_sub(1)),
                        KeyCode::Char('0') => alien_mode = !alien_mode,
                        KeyCode::Char(' ') => paused = !paused,
                        KeyCode::Left => rot_speed -= 0.08,
                        KeyCode::Right => rot_speed += 0.08,
                        KeyCode::Up => tilt = (tilt + 0.06).min(0.85),
                        KeyCode::Down => tilt = (tilt - 0.06).max(-0.85),
                        KeyCode::Char('r') | KeyCode::Char('R') if modifiers.contains(KeyModifiers::CONTROL) => {
                            stars = (0..520)
                                .map(|_| {
                                    (
                                        rng.gen::<f32>(),
                                        rng.gen::<f32>(),
                                        rng.gen::<f32>(),
                                        rng.gen::<f32>() * 10.0,
                                    )
                                })
                                .collect();
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        let now = Instant::now();
        let dt = (now - last).as_secs_f32().min(0.05);
        last = now;
        if !paused {
            t += dt;
        }

        // terminal size
        let (w, h) = terminal::size()?;
        if w < 70 || h < 24 {
            // minimal fallback
            execute!(out, BeginSynchronizedUpdate)?;
            execute!(out, terminal::Clear(terminal::ClearType::All))?;
            queue!(out, cursor::MoveTo(0, 0), Print("Terminal too small (need ~70x24)."))?;
            execute!(out, EndSynchronizedUpdate)?;
            out.flush()?;
            std::thread::sleep(Duration::from_millis(60));
            continue;
        }

        if w != prev_w || h != prev_h {
            prev_w = w;
            prev_h = h;
            prev_buf = vec![Cell::blank(Color::Black); (w as usize) * (h as usize)];
            cur_buf = vec![Cell::blank(Color::Black); (w as usize) * (h as usize)];
            execute!(out, terminal::Clear(terminal::ClearType::All))?;
        } else {
            // clear cur
            for c in cur_buf.iter_mut() {
                *c = Cell::blank(Color::Black);
            }
        }

        let style = styles[planet_idx];

        // layout
        let panel_w: u16 = 30.min(w / 2);
        let left_w = w - panel_w;
        let planet_w = left_w.saturating_sub(2);
        let planet_h = h.saturating_sub(2);

        let planet_cx = (planet_w as f32) * 0.52;
        let planet_cy = (planet_h as f32) * 0.52;
        let radius = (planet_h as f32).min(planet_w as f32) * 0.38;

        // background stars
        paint_stars(&mut cur_buf, w, h, left_w, &stars, t);

        // subtle scanline + vignette in left view
        paint_scan_vignette(&mut cur_buf, w, h, left_w, t);

        // planet
        let rot = t * rot_speed;
        render_planet_braille(
            &mut cur_buf,
            w,
            h,
            1,
            1,
            planet_w,
            planet_h,
            planet_cx,
            planet_cy,
            radius,
            rot,
            tilt,
            style,
        );

        // optional rings
        if style.rings {
            render_rings(&mut cur_buf, w, h, 1, 1, planet_w, planet_h, planet_cx, planet_cy, radius, rot, tilt, style);
        }

        // UI panels (right side)
        render_panels(
            &mut cur_buf,
            w,
            h,
            left_w,
            panel_w,
            style,
            rot_speed,
            tilt,
            paused,
            alien_mode,
            &glyphs,
        );

        // blit with synchronized update + diff
        execute!(out, BeginSynchronizedUpdate)?;
        render_diff(out, w, h, &mut prev_buf, &cur_buf)?;
        execute!(out, EndSynchronizedUpdate)?;
        out.flush()?;

        // frame cap
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn paint_stars(buf: &mut [Cell], w: u16, h: u16, left_w: u16, stars: &[(f32, f32, f32, f32)], t: f32) {
    let ww = w as usize;
    let hh = h as usize;

    for &(sx, sy, sz, ph) in stars.iter() {
        let x = (sx * (left_w as f32 - 1.0)).floor() as i32;
        let y = (sy * (h as f32 - 1.0)).floor() as i32;
        if x < 0 || y < 0 || x >= left_w as i32 || y >= h as i32 {
            continue;
        }
        let tw = 0.55 + 0.45 * (t * (0.9 + sz * 1.2) + ph).sin();
        let a = clamp01(0.08 + tw * (0.18 + 0.45 * (1.0 - sz)));
        let brightness = a;

        let ch = if brightness > 0.62 {
            '✦'
        } else if brightness > 0.42 {
            '✧'
        } else if brightness > 0.24 {
            '·'
        } else {
            ' '
        };

        let c = (180.0 + 70.0 * (1.0 - sz)) as u8;
        let fg = Color::Rgb { r: c, g: c, b: (c as f32 * 1.05).min(255.0) as u8 };

        let i = (y as usize) * ww + (x as usize);
        // do not overwrite non-blank aggressively; keep it subtle
        if buf[i].ch == ' ' && ch != ' ' {
            buf[i] = Cell { ch, fg, bg: Color::Black };
        }
    }
}

fn paint_scan_vignette(buf: &mut [Cell], w: u16, h: u16, left_w: u16, t: f32) {
    let ww = w as usize;
    let hh = h as usize;

    for y in 0..hh {
        let yy = y as f32 / (h as f32 - 1.0);
        let scan = 0.06 * (t * 2.2 + yy * 80.0).sin();
        for x in 0..(left_w as usize) {
            let xx = x as f32 / (left_w as f32 - 1.0);
            let dx = xx - 0.52;
            let dy = yy - 0.52;
            let v = (dx * dx + dy * dy).sqrt();
            let vign = clamp01(1.0 - v * 1.25);

            let i = y * ww + x;
            if buf[i].ch == ' ' {
                // add faint grain
                let g = clamp01(0.015 + 0.02 * vign + scan.abs() * 0.02);
                if g > 0.03 && ((x + y) & 7) == 0 {
                    let c = (12.0 + g * 70.0) as u8;
                    buf[i] = Cell {
                        ch: '·',
                        fg: Color::Rgb { r: c, g: c, b: (c as f32 * 1.1).min(255.0) as u8 },
                        bg: Color::Black,
                    };
                }
            }
        }
    }
}

fn render_planet_braille(
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

    let (lx, ly, lz) = v3_norm(-0.34, 0.38, 0.86);
    let gamma = 1.25;

    for y in 0..vh as usize {
        for x in 0..vw as usize {
            let gx = x0 as usize + x;
            let gy = y0 as usize + y;
            if gx >= ww || gy >= hh {
                continue;
            }

            // build 2x4 braille by sampling subpixels
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
                    // subpixel center
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
                                avg_i += glow * 0.35;
                                avg_a += glow;
                            }
                        }
                        continue;
                    }

                    // sphere z, with tilt (rotate around x)
                    let nz = (1.0 - d2).sqrt();

                    // apply tilt: rotate around x axis
                    let (ts, tc) = tilt.sin_cos();
                    let y1 = tc * ny - ts * nz;
                    let z1 = ts * ny + tc * nz;

                    // rotate planet around Y
                    let (x2, y2, z2) = v3_rot_y(nx, y1, z1, rot);

                    // normal is (x2,y2,z2)
                    let ndotl = v3_dot(x2, y2, z2, lx, ly, lz).max(0.0);
                    let shade = ndotl.powf(gamma);

                    // rim light
                    let rim = clamp01((1.0 - ndotl).powf(2.2)) * 0.20;

                    // lat/lon
                    let lat = y2.asin(); // -pi/2..pi/2
                    let lon = x2.atan2(z2); // -pi..pi

                    // procedural surface
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

                    // land mask
                    let rough = lerp(n0, n1, style.roughness);
                    let land = clamp01((rough - 0.48) * 2.2);

                    // clouds layer
                    let cnoise = fbm_3d(
                        lon * 1.10 + rot * 0.25,
                        lat * 1.55 - rot * 0.10,
                        rot * 0.35 + 2.0,
                        style.seed.wrapping_add(0x13579BDF),
                        5,
                    );
                    let clouds = clamp01((cnoise - 0.56) * 2.7) * style.clouds;

                    // ice caps / cracks vibe
                    let cap = clamp01((lat.abs() - (0.86 - style.ice * 0.22)) * 9.0) * style.ice;
                    let cracks = if style.ice > 0.01 {
                        let cn = fbm_3d(lon * 3.2, lat * 2.6, rot * 0.6, style.seed ^ 0xDEAD_BEEF, 3);
                        clamp01((cn - 0.53) * 8.0) * style.ice
                    } else {
                        0.0
                    };

                    // albedo selection
                    let mut col = if land > 0.45 {
                        // land uses base+accent, modulated by banding
                        let t = clamp01(0.25 + 0.75 * land) * (0.65 + 0.35 * banding);
                        mix_rgb(style.base, style.accent, t)
                    } else {
                        // ocean
                        let t = clamp01(0.35 + 0.65 * (0.60 * banding + 0.40 * (1.0 - rough)));
                        mix_rgb(style.ocean, style.base, t * 0.25)
                    };

                    // add ice
                    if cap > 0.01 {
                        col = mix_rgb(col, style.accent, clamp01(cap * 0.85));
                    }
                    if cracks > 0.01 {
                        col = mix_rgb(col, Rgb { r: 235, g: 250, b: 255 }, clamp01(cracks * 0.65));
                    }
                    col_sum_r += col.r as u32;
                    col_sum_g += col.g as u32;
                    col_sum_b += col.b as u32;
                    col_count += 1;

                    // lighting
                    let mut intensity = shade;
                    intensity = clamp01(intensity + rim);

                    // clouds brighten
                    intensity = clamp01(intensity + clouds * 0.35);

                    // terminator softening
                    let terminator = clamp01((ndotl - 0.02) * 5.0);
                    intensity = intensity * (0.45 + 0.95 * terminator);
                    // let terminator = clamp01((ndotl - 0.02) * 3.0);
                    // intensity = intensity * (0.45 + 0.55 * terminator)


                    // emission in night side speckle
                    let city = if ndotl < 0.12 && land > 0.55 {
                        let sp = fbm_3d(lon * 6.0, lat * 6.0, 1.7, style.seed ^ 0xCAFEBABE, 2);
                        clamp01((sp - 0.72) * 8.0) * 0.22
                    } else {
                        0.0
                    };

                    // convert to braille on/off using ordered dither
                    let micro = intensity + city;
                    let th = bayer_2x4_threshold(x * 2 + sx, y * 4 + sy);
                    let on = micro > th;

                    bits[sy][sx] = on;
                    any |= on;
                    avg_i += micro;
                    avg_a += 1.0;
                }
            }

            if !any {
                if covered {
                    let i = gy * ww + gx;
                    buf[i] = Cell {
                        ch: ' ',
                        fg: Color::Black,
                        bg: Color::Black,
                    };
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

            // color grade: mix toward atmosphere on rim
            let px = ((x as f32 + 0.5) - cx) * ASPECT_X;
            let py = (y as f32 + 0.5) - cy;
            let d = ((px / r) * (px / r) + (py / r) * (py / r)).sqrt();
            let rim_t = clamp01((d - 0.86) / 0.18);

            let lit = mix_rgb(col, style.accent, clamp01((avg_i - 0.45) * 0.9));
            let tinted = mix_rgb(lit, style.atmosphere, rim_t * 0.55);

            // compress color in dark areas
            let dark = clamp01(1.0 - avg_i * 1.35);
            let final_col = mix_rgb(tinted, Rgb { r: 8, g: 10, b: 14 }, dark * 0.55);

            let ch = braille_from_2x4(bits);
            let i = gy * ww + gx;
            buf[i] = Cell {
                ch,
                fg: final_col.to_color(),
                bg: Color::Black,
            };
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

    // ring ellipse params
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

            // rotate ring in screen plane
            let rx = rc * px - rs * py;
            let ry = rs * px + rc * py;

            // ellipse distance
            let ex = rx;
            let ey = ry / squash.max(0.12);
            let d = (ex * ex + ey * ey).sqrt();

            if d < ring_r0 || d > ring_r1 {
                continue;
            }

            // avoid drawing ring fully in front of planet: mask with sphere depth cue
            // approximate: if pixel inside planet disc, skip (planet occludes)
            let nx = px / r;
            let ny = py / r;
            if nx * nx + ny * ny <= 1.0 {
                continue;
            }

            let band = 0.5 + 0.5 * ((d / r) * 10.0 + rot * 0.8).sin();
            let alpha = clamp01(0.30 + 0.45 * band);

            // ordered dither on alpha
            let th = ((x + y) & 7) as f32 / 8.0;
            if alpha < th {
                continue;
            }

            let col = mix_rgb(style.base, style.accent, 0.35 + 0.45 * band);
            let i = gy * ww + gx;

            // blend with existing char lightly: only overwrite if background-ish
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

fn render_panels(
    buf: &mut [Cell],
    w: u16,
    h: u16,
    left_w: u16,
    panel_w: u16,
    style: PlanetStyle,
    rot_speed: f32,
    tilt: f32,
    paused: bool,
    alien_mode: bool,
    glyphs: &[char],
) {
    let x0 = left_w;
    let bg = Color::Rgb { r: 7, g: 8, b: 12 };
    let fg = Color::Rgb { r: 210, g: 220, b: 235 };
    let dim = Color::Rgb { r: 140, g: 150, b: 165 };
    let edge = Color::Rgb { r: 90, g: 100, b: 120 };
    let accent = style.accent.to_color();
    let base = style.base.to_color();
    let t = |s: &str| if alien_mode { alienize(s, glyphs) } else { s.to_string() };
    let n = |v: f32| if alien_mode { fmt_alien_num(v, glyphs) } else { format!("{:.3}", v) };

    // panel background fill
    for y in 0..h as usize {
        for x in x0 as usize..w as usize {
            buf[y * (w as usize) + x] = Cell { ch: ' ', fg, bg };
        }
    }

    // frames
    let top_h = 8.min(h.saturating_sub(1));
    let mid_h = 10.min(h.saturating_sub(top_h + 2));
    let bot_h = h.saturating_sub(top_h + mid_h + 2).max(6);

    box_draw(buf, w, h, x0, 0, panel_w, top_h, edge, bg);
    box_draw(buf, w, h, x0, top_h, panel_w, mid_h, edge, bg);
    box_draw(buf, w, h, x0, top_h + mid_h, panel_w, bot_h, edge, bg);

    // header
    let title = format!(" {} ", style.name);
    write_str(buf, w, h, x0 + 2, 1, &title, accent, bg);

    let sub = t("ORBITAL SURVEY ARRAY");
    write_str(buf, w, h, x0 + 2, 2, &sub, dim, bg);

    let status = if paused { t("STATUS: PAUSED") } else { t("STATUS: LIVE") };
    write_str(buf, w, h, x0 + 2, 4, &status, fg, bg);

    let hint = t("C cycle   1-9 select   0 language   arrows adjust   Q quit");
    let mut hint_line = hint;
    let hint_max = panel_w.saturating_sub(4) as usize;
    if hint_line.chars().count() > hint_max {
        hint_line = hint_line.chars().take(hint_max).collect();
    }
    write_str(buf, w, h, x0 + 2, 6, &hint_line, dim, bg);

    // telemetry (middle)
    write_str(buf, w, h, x0 + 2, top_h + 1, &t("TELEMETRY"), base, bg);

    let a1 = format!("{} {}", t("ROT"), n(rot_speed));
    let a2 = format!("{} {}", t("TILT"), n(tilt));
    let base_hex = format!("{:02X}{:02X}{:02X}", style.base.r, style.base.g, style.base.b);
    let seed_hex = format!("{:08X}", style.seed);
    let a3 = format!("{} {}", t("ALB"), if alien_mode { alienize(&base_hex, glyphs) } else { base_hex });
    let a4 = format!("{} {}", t("SEED"), if alien_mode { alienize(&seed_hex, glyphs) } else { seed_hex });
    write_str(buf, w, h, x0 + 2, top_h + 3, &fit_line(&a1, panel_w), fg, bg);
    write_str(buf, w, h, x0 + 2, top_h + 4, &fit_line(&a2, panel_w), fg, bg);
    write_str(buf, w, h, x0 + 2, top_h + 5, &fit_line(&a3, panel_w), fg, bg);
    write_str(buf, w, h, x0 + 2, top_h + 6, &fit_line(&a4, panel_w), fg, bg);

    // dossier (bottom)
    let yb = top_h + mid_h;
    write_str(buf, w, h, x0 + 2, yb + 1, &t("PLANET DOSSIER"), base, bg);

    // Make a few convincing-looking blocks of alien “text”
    let mut lines = vec![
        t("Spectral return indicates layered aerosols with nontrivial dielectric variance."),
        t("Surface shows coherent band structures and turbulent shear signatures."),
        t("Subsurface echo: void lattices probable. Magnetic flux harmonics stable."),
        t("Advisory: maintain standoff range; photonic shimmer may induce sensor drift."),
    ];

    if style.rings {
        lines.insert(1, t("Ring system: particulate silicates with resonant micro-arc luminescence."));
    }
    if style.ice > 0.2 {
        lines.insert(2, t("Cryo-regions: fracture webs detected; albedo spikes at polar latitudes."));
    }
    if style.clouds > 0.5 {
        lines.insert(3, t("Cloud deck: high-altitude scatter layers, convective plumes on trailing hemisphere."));
    }

    let max_lines = h.saturating_sub(yb + 3) as usize;
    let mut wrapped: Vec<String> = Vec::new();
    for line in lines {
        let s = wrap_text(&line, panel_w.saturating_sub(4) as usize);
        wrapped.extend(s.lines().map(|l| l.to_string()));
        if wrapped.len() >= max_lines {
            break;
        }
    }

    for (i, line) in wrapped.into_iter().take(max_lines).enumerate() {
        write_str(buf, w, h, x0 + 2, yb + 3 + i as u16, &line, dim, bg);
    }
}

fn fit_line(s: &str, panel_w: u16) -> String {
    let max = panel_w.saturating_sub(4) as usize;
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

fn wrap_text(s: &str, width: usize) -> String {
    if width < 8 {
        return s.to_string();
    }
    let mut out = String::new();
    let mut col = 0usize;
    for word in s.split_whitespace() {
        let mut rest = word;
        loop {
            let wlen = rest.chars().count();
            if wlen == 0 {
                break;
            }
            if wlen > width {
                if col > 0 {
                    out.push('\n');
                    col = 0;
                }
                let take = width.saturating_sub(1).max(1);
                let (head, tail) = split_at_char(rest, take);
                out.push_str(head);
                if !tail.is_empty() && width > 1 {
                    out.push('-');
                }
                if tail.is_empty() {
                    col = head.chars().count();
                    break;
                }
                out.push('\n');
                col = 0;
                rest = tail;
                continue;
            }

            if col > 0 {
                if col + 1 + wlen > width {
                    out.push('\n');
                    col = 0;
                } else {
                    out.push(' ');
                    col += 1;
                }
            }
            out.push_str(rest);
            col += wlen;
            break;
        }
    }
    out
}

fn split_at_char(s: &str, n: usize) -> (&str, &str) {
    if n == 0 {
        return ("", s);
    }
    let mut idx = s.len();
    let mut count = 0usize;
    for (i, _) in s.char_indices() {
        if count == n {
            idx = i;
            break;
        }
        count += 1;
    }
    (&s[..idx], &s[idx..])
}

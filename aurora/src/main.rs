use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, Clear, ClearType, DisableLineWrap, EnableLineWrap,
        EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::{
    cmp::max,
    io::{stdout, Write},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}
impl Rgb {
    fn lerp(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let f = |a: u8, b: u8| -> u8 {
            ((a as f32) + (b as f32 - a as f32) * t)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        Rgb {
            r: f(self.r, other.r),
            g: f(self.g, other.g),
            b: f(self.b, other.b),
        }
    }
    fn scale(self, k: f32) -> Rgb {
        let k = k.max(0.0);
        let f = |a: u8| -> u8 { ((a as f32) * k).round().clamp(0.0, 255.0) as u8 };
        Rgb {
            r: f(self.r),
            g: f(self.g),
            b: f(self.b),
        }
    }
}

#[derive(Clone)]
struct Config {
    fps_cap: u32,
    gamma: f32,

    activity: f32,  // intensity
    wind: f32,      // horizontal drift
    curtains: f32,  // striation strength
    height: f32,    // how low the aurora reaches
    hue_shift: f32, // palette rotation

    stars: bool,
    show_help: bool,
    paused: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            fps_cap: 50,
            gamma: 1.15,
            activity: 0.95,
            wind: 0.70,
            curtains: 0.85,
            height: 0.62,
            hue_shift: 0.0,
            stars: true,
            show_help: true,
            paused: false,
        }
    }
}

// ------------------------------
// Small deterministic noise
// ------------------------------

fn hash_u32(mut x: u32) -> u32 {
    // xorshift-ish
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^= x >> 16;
    x
}

fn hash2(x: i32, y: i32, seed: u32) -> u32 {
    let mut h = seed ^ (x as u32).wrapping_mul(0x9e3779b1) ^ (y as u32).wrapping_mul(0x85ebca6b);
    h = hash_u32(h);
    h
}

fn rand01_from_hash(h: u32) -> f32 {
    // 24-bit mantissa style
    ((h & 0x00FF_FFFF) as f32) / 16_777_215.0
}

fn fade(t: f32) -> f32 {
    // smoothstep-ish
    t * t * (3.0 - 2.0 * t)
}

fn value_noise2(x: f32, y: f32, seed: u32) -> f32 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let xf = x - xi as f32;
    let yf = y - yi as f32;

    let h00 = rand01_from_hash(hash2(xi, yi, seed));
    let h10 = rand01_from_hash(hash2(xi + 1, yi, seed));
    let h01 = rand01_from_hash(hash2(xi, yi + 1, seed));
    let h11 = rand01_from_hash(hash2(xi + 1, yi + 1, seed));

    let u = fade(xf);
    let v = fade(yf);

    let x0 = h00 + (h10 - h00) * u;
    let x1 = h01 + (h11 - h01) * u;
    x0 + (x1 - x0) * v
}

fn fbm2(mut x: f32, mut y: f32, seed: u32, octaves: usize, lacunarity: f32, gain: f32) -> f32 {
    let mut amp = 0.5;
    let mut sum = 0.0;
    let mut norm = 0.0;

    for i in 0..octaves {
        let s = seed.wrapping_add((i as u32).wrapping_mul(1013));
        let n = value_noise2(x, y, s) * 2.0 - 1.0; // [-1,1]
        sum += n * amp;
        norm += amp;
        x *= lacunarity;
        y *= lacunarity;
        amp *= gain;
    }

    (sum / norm) * 0.5 + 0.5 // -> [0,1] roughly
}

// ------------------------------
// Braille rendering helpers
// ------------------------------

// Dots mapping for braille char bits (Unicode braille patterns).
// Subpixel coords: sx in [0..1], sy in [0..3] where (0,0) is top-left.
// Standard dot numbering:
// 1 4
// 2 5
// 3 6
// 7 8
fn braille_bit(sx: usize, sy: usize) -> u8 {
    match (sx, sy) {
        (0, 0) => 0x01, // dot 1
        (0, 1) => 0x02, // dot 2
        (0, 2) => 0x04, // dot 3
        (0, 3) => 0x40, // dot 7
        (1, 0) => 0x08, // dot 4
        (1, 1) => 0x10, // dot 5
        (1, 2) => 0x20, // dot 6
        (1, 3) => 0x80, // dot 8
        _ => 0,
    }
}

fn braille_char(bits: u8) -> char {
    // Unicode braille patterns start at U+2800
    char::from_u32(0x2800 + bits as u32).unwrap_or(' ')
}

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// Palette inspired by common aurora hues, blended by height + intensity.
fn aurora_palette(height01: f32, intensity01: f32, hue_shift: f32) -> Rgb {
    // Base anchors
    let deep = Rgb {
        r: 30,
        g: 210,
        b: 120,
    }; // green
    let mid = Rgb {
        r: 90,
        g: 255,
        b: 200,
    }; // mint
    let high = Rgb {
        r: 160,
        g: 170,
        b: 255,
    }; // blue
    let hot = Rgb {
        r: 220,
        g: 120,
        b: 255,
    }; // violet

    let h = clamp01(height01 + hue_shift * 0.06); // small palette rotation effect
    let t1 = smoothstep(0.00, 0.55, h);
    let t2 = smoothstep(0.45, 1.00, h);

    let a = deep.lerp(mid, t1);
    let b = high.lerp(hot, t2);
    let mut c = a.lerp(b, t2);

    // Brightness color push
    let glow = Rgb {
        r: 255,
        g: 255,
        b: 255,
    };
    c = c.lerp(glow, (intensity01 * intensity01).clamp(0.0, 0.55));
    c
}

fn main() -> std::io::Result<()> {
    let mut stdout = stdout();
    terminal::enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, DisableLineWrap, cursor::Hide)?;

    // Best-effort: reduce tearing in terminals that support it.
    let _ = execute!(stdout, BeginSynchronizedUpdate);

    let res = run(&mut stdout);

    // Cleanup
    let _ = execute!(stdout, EndSynchronizedUpdate);
    execute!(
        stdout,
        ResetColor,
        EnableLineWrap,
        cursor::Show,
        LeaveAlternateScreen
    )?;
    terminal::disable_raw_mode()?;

    res
}

fn run(stdout: &mut std::io::Stdout) -> std::io::Result<()> {
    let mut cfg = Config::default();
    let mut rng = StdRng::seed_from_u64(u64::from_le_bytes(*b"AUR0RA!!") ^ 0x5EED_1234);

    // Buffers for diff-based rendering
    let mut prev_chars: Vec<char> = Vec::new();
    let mut prev_rgb: Vec<Rgb> = Vec::new();

    // Star field
    #[derive(Clone, Copy)]
    struct Star {
        x: f32,
        y: f32,
        z: f32,
        tw: f32,
    }
    let mut stars: Vec<Star> = Vec::new();

    let mut last = Instant::now();
    let mut t = 0.0f32;
    let mut needs_clear = true;

    loop {
        // Handle input
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    match k.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(()),
                        KeyCode::Char(' ') => cfg.paused = !cfg.paused,
                        KeyCode::Char('h') | KeyCode::Char('H') => cfg.show_help = !cfg.show_help,
                        KeyCode::Char('s') | KeyCode::Char('S') => cfg.stars = !cfg.stars,

                        // Intensity and shape
                        KeyCode::Up => cfg.activity = (cfg.activity + 0.05).clamp(0.0, 2.0),
                        KeyCode::Down => cfg.activity = (cfg.activity - 0.05).clamp(0.0, 2.0),
                        KeyCode::Left => cfg.wind = (cfg.wind - 0.05).clamp(-2.0, 2.0),
                        KeyCode::Right => cfg.wind = (cfg.wind + 0.05).clamp(-2.0, 2.0),

                        KeyCode::Char('[') => cfg.curtains = (cfg.curtains - 0.05).clamp(0.0, 2.0),
                        KeyCode::Char(']') => cfg.curtains = (cfg.curtains + 0.05).clamp(0.0, 2.0),

                        KeyCode::Char('-') => cfg.height = (cfg.height - 0.03).clamp(0.1, 0.95),
                        KeyCode::Char('=') => cfg.height = (cfg.height + 0.03).clamp(0.1, 0.95),

                        // Palette
                        KeyCode::Char(',') => cfg.hue_shift -= 0.5,
                        KeyCode::Char('.') => cfg.hue_shift += 0.5,

                        // Reset
                        KeyCode::Char('r') | KeyCode::Char('R') => {
                            cfg = Config::default();
                            stars.clear();
                        }
                        _ => {}
                    }
                }
                Event::Resize(_, _) => {
                    // Force full redraw by clearing prev buffers
                    prev_chars.clear();
                    prev_rgb.clear();
                    needs_clear = true;
                }
                _ => {}
            }
        }

        // Timing
        let now = Instant::now();
        let dt = (now - last).as_secs_f32();
        last = now;

        if !cfg.paused {
            t += dt;
        }

        // FPS cap
        let frame_min = Duration::from_secs_f32(1.0 / (cfg.fps_cap as f32));
        let frame_end = Instant::now() + frame_min;

        // Terminal size and braille grid
        let (tw, th) = terminal::size()?;
        let w = max(1, tw as usize);
        let h = max(1, th as usize);

        let n = w * h;
        if prev_chars.len() != n {
            prev_chars = vec!['\0'; n];
            prev_rgb = vec![Rgb { r: 0, g: 0, b: 0 }; n];
            needs_clear = true;
        }

        // Stars init (based on cell-space, stable)
        if cfg.stars && stars.is_empty() {
            let count = ((w as f32) * (h as f32) * 0.06).clamp(40.0, 260.0) as usize;
            for _ in 0..count {
                stars.push(Star {
                    x: rng.gen::<f32>() * (w as f32),
                    y: rng.gen::<f32>() * (h as f32),
                    z: rng.gen::<f32>().clamp(0.1, 1.0),
                    tw: rng.gen_range(0.7..1.4),
                });
            }
        }

        // Build current frame buffers
        let mut cur_chars = vec![' '; n];
        let mut cur_rgb = vec![Rgb { r: 0, g: 0, b: 0 }; n];

        // Background star draw first
        if cfg.stars {
            // simple drift downward with slight wind
            let spd = 0.35 + 1.25 * (cfg.activity.clamp(0.0, 2.0) / 2.0);
            for s in &mut stars {
                if !cfg.paused {
                    s.y += dt * spd * (0.4 + 1.2 * (1.0 - s.z));
                    s.x += dt * cfg.wind * 0.18 * (0.3 + 0.7 * (1.0 - s.z));
                }
                if s.y >= h as f32 {
                    s.y -= h as f32;
                }
                if s.x < 0.0 {
                    s.x += w as f32;
                }
                if s.x >= w as f32 {
                    s.x -= w as f32;
                }

                let ix = s.x.floor() as i32;
                let iy = s.y.floor() as i32;
                if ix < 0 || iy < 0 || ix >= w as i32 || iy >= h as i32 {
                    continue;
                }
                let i = (iy as usize) * w + (ix as usize);

                let twk = 0.6 + 0.4 * ((t * 1.8 * s.tw + (s.x * 0.17)).sin() * 0.5 + 0.5);
                let v = (0.12 + 0.35 * (1.0 - s.z)) * twk;
                let c = Rgb {
                    r: (200.0 * v) as u8,
                    g: (220.0 * v) as u8,
                    b: (255.0 * v) as u8,
                };
                cur_chars[i] = '·';
                cur_rgb[i] = c;
            }
        }

        // Aurora field: sample at braille subpixels
        // Each cell has 2x4 subpixels, so effective resolution is high.
        let seed = 0xC0FFEE_u32;

        let sky_base = Rgb { r: 8, g: 10, b: 18 };

        // Write faint sky background to make stars less harsh
        for i in 0..n {
            if cur_chars[i] == ' ' {
                cur_rgb[i] = sky_base;
            }
        }

        for cy in 0..h {
            for cx in 0..w {
                let mut bits: u8 = 0;
                let mut acc = 0.0f32;
                let mut acc_h = 0.0f32;
                let mut on_count = 0.0f32;

                // normalized cell coordinates
                let fx = cx as f32 / (w as f32);
                let fy = cy as f32 / (h as f32);

                // Aurora mostly in upper portion, reaching down based on cfg.height.
                // height=0.62 means it extends to about 62% down the screen.
                let reach = cfg.height.clamp(0.1, 0.95);
                let aurora_mask = smoothstep(1.0, reach, fy); // strong at top, fades as y increases

                // Sample 8 subpixels
                for sy in 0..4 {
                    for sx in 0..2 {
                        let subx = (cx as f32) + (sx as f32) * 0.5 + 0.25;
                        let suby = (cy as f32) + (sy as f32) * 0.25 + 0.125;

                        let nx = subx / (w as f32);
                        let ny = suby / (h as f32);

                        // Large-scale shape
                        let drift_x = t * (0.04 + 0.07 * cfg.wind);
                        let drift_y = t * 0.02;

                        let base = fbm2(nx * 2.4 + drift_x, ny * 1.3 + drift_y, seed, 5, 2.0, 0.55);

                        // Curtain vertical striations: high frequency along x, modulated by y noise
                        let stripe = fbm2(
                            nx * 26.0 + t * (0.45 + 0.15 * cfg.wind),
                            ny * 2.2 - t * 0.12,
                            seed.wrapping_add(911),
                            4,
                            2.1,
                            0.55,
                        );
                        let curtain = ((stripe * 2.0 - 1.0).abs()).powf(1.35);
                        let curtain = 1.0 - curtain; // bright ridges

                        // Some rolling waves so it does not look static
                        let wave = fbm2(
                            nx * 6.0 - t * 0.18,
                            ny * 4.0 + t * 0.22,
                            seed.wrapping_add(1777),
                            4,
                            2.0,
                            0.55,
                        );

                        let mut intensity = 0.0f32;

                        // Combine fields
                        // base defines where aurora exists, curtain defines the ridges.
                        let presence = smoothstep(0.35, 0.78, base);
                        let ridge = smoothstep(0.25, 0.95, curtain);

                        // Vertical shaping: brighter near a moving band
                        let band_center = 0.12 + 0.10 * (t * 0.35 + nx * 2.5).sin();
                        let band = 1.0 - ((ny - band_center).abs() / 0.22).clamp(0.0, 1.0);
                        let band = band.powf(1.8);

                        let activity = cfg.activity.clamp(0.0, 2.0);

                        intensity += presence
                            * (0.55 + 0.45 * wave)
                            * (0.35 + 0.65 * ridge * cfg.curtains.clamp(0.0, 2.0));
                        intensity *= 0.40 + 0.85 * band;
                        intensity *= activity;
                        intensity *= aurora_mask;

                        // Slight flicker
                        let flick = 0.92 + 0.08 * (t * 7.0 + nx * 19.0 + ny * 3.0).sin();
                        intensity *= flick;

                        // Tone mapping
                        intensity = intensity.clamp(0.0, 1.6);
                        let lit = (intensity / 1.2).clamp(0.0, 1.0);
                        let lit = lit.powf(cfg.gamma);

                        // Threshold for dot on/off
                        let on = lit > 0.22;
                        if on {
                            bits |= braille_bit(sx, sy);
                            on_count += 1.0;
                        }

                        acc += lit;
                        acc_h += 1.0 - ny; // higher pixels bias toward "higher" color
                    }
                }

                // If nothing lit, keep background char or star
                if bits == 0 {
                    continue;
                }

                let avg = (acc / 8.0).clamp(0.0, 1.0);
                let h01 = (acc_h / 8.0).clamp(0.0, 1.0);

                // Color from palette, boosted by intensity and number of lit subpixels
                let coverage = (on_count / 8.0).clamp(0.0, 1.0);
                let col = aurora_palette(h01, avg, cfg.hue_shift)
                    .scale(0.55 + 0.85 * avg * (0.55 + 0.45 * coverage));

                let idx = cy * w + cx;
                cur_chars[idx] = braille_char(bits);
                cur_rgb[idx] = col;
            }
        }

        // HUD/help overlay
        if cfg.show_help {
            let line1 = format!("Aurora  |  Q quit  Space pause  H help  S stars  R reset");
            let line2 = format!(
                "Arrows: Up/Down activity {:.2}  Left/Right wind {:.2}  [ ] curtains {:.2}  -/= reach {:.2}  ,/. hue {:.1}",
                cfg.activity, cfg.wind, cfg.curtains, cfg.height, cfg.hue_shift
            );

            draw_text(
                &mut cur_chars,
                &mut cur_rgb,
                w,
                h,
                1,
                0,
                &line1,
                Rgb {
                    r: 200,
                    g: 210,
                    b: 230,
                },
            );
            draw_text(
                &mut cur_chars,
                &mut cur_rgb,
                w,
                h,
                1,
                1,
                &line2,
                Rgb {
                    r: 170,
                    g: 190,
                    b: 220,
                },
            );
        }

        // Render diff
        queue!(stdout, BeginSynchronizedUpdate, cursor::MoveTo(0, 0))?;
        if needs_clear {
            // Clear only when we know the terminal changed.
            queue!(stdout, Clear(ClearType::All))?;
            needs_clear = false;
        }

        let mut last_color: Option<Rgb> = None;

        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;

                let ch = cur_chars[i];
                let rgb = cur_rgb[i];

                if prev_chars[i] == ch
                    && prev_rgb[i].r == rgb.r
                    && prev_rgb[i].g == rgb.g
                    && prev_rgb[i].b == rgb.b
                {
                    continue;
                }

                queue!(stdout, cursor::MoveTo(x as u16, y as u16))?;

                if last_color
                    .map(|c| c.r != rgb.r || c.g != rgb.g || c.b != rgb.b)
                    .unwrap_or(true)
                {
                    queue!(
                        stdout,
                        SetForegroundColor(Color::Rgb {
                            r: rgb.r,
                            g: rgb.g,
                            b: rgb.b
                        })
                    )?;
                    last_color = Some(rgb);
                }

                queue!(stdout, Print(ch))?;
                prev_chars[i] = ch;
                prev_rgb[i] = rgb;
            }
        }

        queue!(stdout, ResetColor, EndSynchronizedUpdate)?;
        stdout.flush()?;

        // Busy-wait sleep until cap
        while Instant::now() < frame_end {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

fn draw_text(
    chars: &mut [char],
    rgbs: &mut [Rgb],
    w: usize,
    h: usize,
    x: usize,
    y: usize,
    s: &str,
    color: Rgb,
) {
    if y >= h {
        return;
    }
    let mut xx = x;
    for ch in s.chars() {
        if xx >= w {
            break;
        }
        let i = y * w + xx;
        chars[i] = ch;
        rgbs[i] = color;
        xx += 1;
    }
}

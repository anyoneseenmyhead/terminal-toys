use std::cmp::{max, min};
use std::env;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, Clear, ClearType, DisableLineWrap, EnableLineWrap,
        EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use rand::{rngs::StdRng, Rng, SeedableRng};

#[derive(Clone, Copy)]
struct Star {
    // position in "subpixels" (2x4 per terminal cell), as floats for smooth motion
    x: f32,
    y: f32,
    z: f32,  // depth in (0..1], smaller is nearer/faster/brighter
    vx: f32,
    vy: f32,
    tw: f32, // twinkle phase
    hue: f32,
    // previous position for streaks
    px: f32,
    py: f32,
}

#[derive(Clone, Copy)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

#[derive(Clone, Copy)]
enum ThemeMode {
    Mono,
    Hue {
        base: f32,
        span: f32,
        sat_min: f32,
        sat_max: f32,
        val_min: f32,
        val_max: f32,
    },
    Joshnet,
    Disco,
}

#[derive(Clone, Copy)]
struct Theme {
    name: &'static str,
    mode: ThemeMode,
}

const THEMES: [Theme; 11] = [
    Theme {
        name: "neon",
        mode: ThemeMode::Hue {
            base: 0.62,
            span: 0.08,
            sat_min: 0.12,
            sat_max: 0.55,
            val_min: 0.12,
            val_max: 1.0,
        },
    },
    Theme {
        name: "warm",
        mode: ThemeMode::Hue {
            base: 0.04,
            span: 0.07,
            sat_min: 0.18,
            sat_max: 0.65,
            val_min: 0.10,
            val_max: 1.0,
        },
    },
    Theme {
        name: "icy",
        mode: ThemeMode::Hue {
            base: 0.56,
            span: 0.05,
            sat_min: 0.10,
            sat_max: 0.45,
            val_min: 0.12,
            val_max: 1.0,
        },
    },
    Theme {
        name: "mono",
        mode: ThemeMode::Mono,
    },
    Theme {
        name: "joshnet",
        mode: ThemeMode::Joshnet,
    },
    Theme {
        name: "disco",
        mode: ThemeMode::Disco,
    },
    Theme {
        name: "ember",
        mode: ThemeMode::Hue {
            base: 0.02,
            span: 0.05,
            sat_min: 0.30,
            sat_max: 0.85,
            val_min: 0.10,
            val_max: 1.0,
        },
    },
    Theme {
        name: "seafoam",
        mode: ThemeMode::Hue {
            base: 0.44,
            span: 0.06,
            sat_min: 0.18,
            sat_max: 0.60,
            val_min: 0.10,
            val_max: 1.0,
        },
    },
    Theme {
        name: "orchid",
        mode: ThemeMode::Hue {
            base: 0.78,
            span: 0.05,
            sat_min: 0.20,
            sat_max: 0.70,
            val_min: 0.12,
            val_max: 1.0,
        },
    },
    Theme {
        name: "sunset",
        mode: ThemeMode::Hue {
            base: 0.98,
            span: 0.10,
            sat_min: 0.25,
            sat_max: 0.80,
            val_min: 0.10,
            val_max: 1.0,
        },
    },
    Theme {
        name: "aurora",
        mode: ThemeMode::Hue {
            base: 0.36,
            span: 0.08,
            sat_min: 0.18,
            sat_max: 0.65,
            val_min: 0.10,
            val_max: 1.0,
        },
    },
];
const DEFAULT_THEME_INDEX: usize = 5;

fn clampf(v: f32, a: f32, b: f32) -> f32 {
    v.max(a).min(b)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Rgb {
    // h: 0..1
    let h = (h.fract() + 1.0).fract() * 6.0;
    let i = h.floor() as i32;
    let f = h - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    let (r, g, b) = match i.rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Rgb {
        r: (clampf(r, 0.0, 1.0) * 255.0) as u8,
        g: (clampf(g, 0.0, 1.0) * 255.0) as u8,
        b: (clampf(b, 0.0, 1.0) * 255.0) as u8,
    }
}

fn braille_char(mask: u8) -> char {
    // Dots mapping for Unicode braille:
    // subpixel grid is 2x4:
    // (0,0)=dot1, (0,1)=dot2, (0,2)=dot3, (0,3)=dot7
    // (1,0)=dot4, (1,1)=dot5, (1,2)=dot6, (1,3)=dot8
    // We store bits in this dot order already (1..8) using the standard braille mask layout.
    // Unicode braille starts at 0x2800.
    char::from_u32(0x2800 + mask as u32).unwrap_or(' ')
}

fn dot_bit(dx: usize, dy: usize) -> u8 {
    match (dx, dy) {
        (0, 0) => 0x01, // dot1
        (0, 1) => 0x02, // dot2
        (0, 2) => 0x04, // dot3
        (0, 3) => 0x40, // dot7
        (1, 0) => 0x08, // dot4
        (1, 1) => 0x10, // dot5
        (1, 2) => 0x20, // dot6
        (1, 3) => 0x80, // dot8
        _ => 0,
    }
}

fn idx(x: usize, y: usize, w: usize) -> usize {
    y * w + x
}

fn parse_args() -> (u64, usize, u64, bool) {
    // --fps N --stars N --seed N --warp
    let mut fps: u64 = 60;
    let mut stars: usize = 500;
    let mut seed: u64 = 0;
    let mut warp = false;

    let mut it = env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--fps" => {
                if let Some(v) = it.next() {
                    fps = v.parse().unwrap_or(fps);
                }
            }
            "--stars" => {
                if let Some(v) = it.next() {
                    stars = v.parse().unwrap_or(stars);
                }
            }
            "--seed" => {
                if let Some(v) = it.next() {
                    seed = v.parse().unwrap_or(seed);
                }
            }
            "--warp" => warp = true,
            "--help" | "-h" => {
                println!(
                    "starfield\n\n\
                     Usage:\n\
                     \tstarfield [--fps N] [--stars N] [--seed N] [--warp]\n\n\
                     Controls:\n\
                     \tQ / Esc quit\n\
                     \tSpace pause\n\
                     \tUp/Down speed\n\
                     \tLeft/Right density\n\
                     \tT trails toggle\n\
                     \tC cycle theme\n\
                     \tR reseed\n\
                     \tH help overlay\n"
                );
                std::process::exit(0);
            }
            _ => {}
        }
    }

    (fps, stars, seed, warp)
}

fn main() -> io::Result<()> {
    let (fps, mut target_stars, seed_arg, warp_arg) = parse_args();

    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        DisableLineWrap,
        cursor::Hide,
        Clear(ClearType::All)
    )?;
    terminal::enable_raw_mode()?;

    let mut cleanup = || -> io::Result<()> {
        let mut out = io::stdout();
        execute!(out, EndSynchronizedUpdate).ok();
        execute!(out, ResetColor, Clear(ClearType::All), cursor::Show, EnableLineWrap, LeaveAlternateScreen)?;
        terminal::disable_raw_mode()?;
        Ok(())
    };

    let res = (|| -> io::Result<()> {
        let mut rng = if seed_arg != 0 {
            StdRng::seed_from_u64(seed_arg)
        } else {
            StdRng::seed_from_u64(
                (Instant::now().elapsed().as_nanos() as u64) ^ 0x9E3779B97F4A7C15u64,
            )
        };

        // Rendering grid:
        // terminal cells: tw x th
        // subpixels: sw=tw*2, sh=th*4
        let mut tw: u16 = 0;
        let mut th: u16 = 0;
        let mut sw: usize = 0;
        let mut sh: usize = 0;

        let mut sub = Vec::<f32>::new();       // intensity buffer per subpixel (0..)
        let mut prev_mask = Vec::<u8>::new();  // diff-based: last braille mask per cell
        let mut prev_color = Vec::<Rgb>::new(); // diff-based: last color per cell

        let mut stars = Vec::<Star>::new();

        // Toggles/settings
        let mut paused = false;
        let mut show_help = false;
        let mut trails = true;
        let mut theme_index: usize = DEFAULT_THEME_INDEX;
        let mut warp = warp_arg;

        let mut speed = if warp { 2.15_f32 } else { 1.0_f32 };
        let mut exposure = 1.0_f32;

        // Visual tuning
        let mut decay_trails = 0.90_f32; // subpixel persistence per frame at ~60fps
        let mut decay_no_trails = 0.80_f32;
        let mut streak_gain = 1.60_f32;
        let mut point_gain = 1.25_f32;

        let mut last = Instant::now();
        let start_time = Instant::now();
        let dt_cap = 0.060_f32;

        let frame_dt = Duration::from_nanos((1_000_000_000u64 / max(1, fps)) as u64);

        fn resize_buffers(
            tw: u16,
            th: u16,
            sw: usize,
            sh: usize,
            sub: &mut Vec<f32>,
            prev_mask: &mut Vec<u8>,
            prev_color: &mut Vec<Rgb>,
        ) {
            let sub_len = sw * sh;
            sub.clear();
            sub.resize(sub_len, 0.0);

            let cells = (tw as usize) * (th as usize);
            prev_mask.clear();
            prev_mask.resize(cells, 0);

            prev_color.clear();
            prev_color.resize(cells, Rgb { r: 0, g: 0, b: 0 });
        }

        let mut reseed = |rng: &mut StdRng, stars: &mut Vec<Star>, sw: usize, sh: usize, count: usize| {
            stars.clear();
            stars.reserve(count);
            let cx = (sw as f32) * 0.5;
            let cy = (sh as f32) * 0.5;

            for _ in 0..count {
                // Start distributed with a slight bias away from exact center to avoid clumping.
                let ang = rng.gen_range(0.0..std::f32::consts::TAU);
                let rad = (rng.gen::<f32>().powf(0.55)) * 0.95;
                let x = cx + ang.cos() * rad * cx;
                let y = cy + ang.sin() * rad * cy;

                let z = clampf(rng.gen_range(0.08..1.0), 0.06, 1.0);
                let hue = rng.gen_range(0.0..1.0);
                let tw = rng.gen_range(0.0..10.0);

                // Vel is radial in screen space, scaled by depth.
                let dx = x - cx;
                let dy = y - cy;
                let len = (dx * dx + dy * dy).sqrt().max(1.0);
                let nx = dx / len;
                let ny = dy / len;

                let base = lerp(35.0, 260.0, 1.0 - z); // nearer = faster
                let vx = nx * base;
                let vy = ny * base;

                stars.push(Star { x, y, z, vx, vy, tw, hue, px: x, py: y });
            }
        };

        // Initial size
        {
            let (w, h) = terminal::size()?;
            tw = max(10, w);
            th = max(6, h);
            sw = (tw as usize) * 2;
            sh = (th as usize) * 4;
            resize_buffers(tw, th, sw, sh, &mut sub, &mut prev_mask, &mut prev_color);
            reseed(&mut rng, &mut stars, sw, sh, target_stars);
        }

        // Helpers
        let mut accum = Duration::ZERO;
        let mut fps_smooth = 60.0_f32;
        let mut last_fps_stamp = Instant::now();

        'main: loop {
            // Events (non-blocking)
            while event::poll(Duration::from_millis(0))? {
                match event::read()? {
                    Event::Key(k) if k.kind != KeyEventKind::Release => {
                        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                        match k.code {
                            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => break 'main,
                            KeyCode::Char(' ') => paused = !paused,
                            KeyCode::Char('h') | KeyCode::Char('H') => show_help = !show_help,
                            KeyCode::Char('t') | KeyCode::Char('T') => trails = !trails,
                            KeyCode::Char('c') | KeyCode::Char('C') => {
                                theme_index = (theme_index + 1) % THEMES.len();
                            }
                            KeyCode::Char('r') | KeyCode::Char('R') => {
                                reseed(&mut rng, &mut stars, sw, sh, target_stars);
                                for v in sub.iter_mut() { *v = 0.0; }
                            }
                            KeyCode::Up => {
                                speed = (speed * 1.08).min(6.0);
                                exposure = (exposure * 1.02).min(2.2);
                            }
                            KeyCode::Down => {
                                speed = (speed / 1.08).max(0.12);
                                exposure = (exposure / 1.02).max(0.55);
                            }
                            KeyCode::Left => {
                                target_stars = target_stars.saturating_sub(80).max(120);
                                reseed(&mut rng, &mut stars, sw, sh, target_stars);
                            }
                            KeyCode::Right => {
                                target_stars = (target_stars + 80).min(6000);
                                reseed(&mut rng, &mut stars, sw, sh, target_stars);
                            }
                            KeyCode::Char('w') | KeyCode::Char('W') if ctrl => {
                                warp = !warp;
                                speed = if warp { 2.15 } else { 1.0 };
                            }
                            _ => {}
                        }
                    }
                    Event::Resize(w, h) => {
                        tw = max(10, w);
                        th = max(6, h);
                        sw = (tw as usize) * 2;
                        sh = (th as usize) * 4;
                        resize_buffers(tw, th, sw, sh, &mut sub, &mut prev_mask, &mut prev_color);
                        reseed(&mut rng, &mut stars, sw, sh, target_stars);
                    }
                    _ => {}
                }
            }

            // time step
            let now = Instant::now();
            let mut dt = (now - last).as_secs_f32();
            last = now;
            dt = clampf(dt, 0.0, dt_cap);

            // frame pacing
            accum += Duration::from_secs_f32(dt);
            if accum < frame_dt {
                let sleep_for = frame_dt - accum;
                std::thread::sleep(sleep_for);
                continue;
            }
            accum = Duration::ZERO;

            // fps smoothing
            let since = last_fps_stamp.elapsed().as_secs_f32();
            if since >= 0.25 {
                let inst = 1.0 / dt.max(1e-6);
                fps_smooth = fps_smooth * 0.85 + inst * 0.15;
                last_fps_stamp = Instant::now();
            }

            // Update sim
            if !paused {
                // decay tuned to dt so it behaves similarly across fps
                let decay = if trails { decay_trails } else { decay_no_trails };
                let decay_dt = decay.powf(dt * 60.0);
                for v in sub.iter_mut() {
                    *v *= decay_dt;
                }

                let cx = (sw as f32) * 0.5;
                let cy = (sh as f32) * 0.5;

                // A subtle "lens breathing" to feel less static
                let breath = 1.0 + 0.015 * (now.elapsed().as_secs_f32() * 0.0); // deterministic zero; keep hook
                let warp_boost = if warp { 1.85 } else { 1.0 };
                let vscale = speed * warp_boost * breath;

                // stars
                for s in stars.iter_mut() {
                    s.px = s.x;
                    s.py = s.y;

                    // depth-dependent acceleration outward (screensaver feel)
                    let dx = s.x - cx;
                    let dy = s.y - cy;
                    let len = (dx * dx + dy * dy).sqrt().max(1.0);
                    let nx = dx / len;
                    let ny = dy / len;

                    // Pull stars a bit toward radial direction, avoids drift without axis bias.
                    let inertia = 0.10;
                    let vmag = (s.vx * s.vx + s.vy * s.vy).sqrt().max(10.0);
                    s.vx = lerp(s.vx, nx * vmag, inertia);
                    s.vy = lerp(s.vy, ny * vmag, inertia);

                    // Nearer = faster
                    let z = clampf(s.z, 0.06, 1.0);
                    let sp = lerp(0.85, 3.2, 1.0 - z) * vscale;

                    // subtle twinkle
                    s.tw += dt * lerp(0.6, 2.2, 1.0 - z);
                    let tw = (s.tw * 2.1 + s.hue * 6.0).sin() * 0.5 + 0.5;

                    s.x += s.vx * dt * sp * 0.06;
                    s.y += s.vy * dt * sp * 0.06;

                    // if out of bounds, recycle near center with new depth/hue
                    let out = s.x < -8.0 || s.x > (sw as f32) + 8.0 || s.y < -8.0 || s.y > (sh as f32) + 8.0;
                    if out {
                        // respawn near center
                        let ang = rng.gen_range(0.0..std::f32::consts::TAU);
                        // Avoid exact-center respawns to prevent a visible cross at the origin.
                        let rad = 0.01 + rng.gen::<f32>().powf(2.0) * 0.05;
                        s.x = cx + ang.cos() * rad * cx + rng.gen_range(-0.35..0.35);
                        s.y = cy + ang.sin() * rad * cy + rng.gen_range(-0.35..0.35);
                        s.px = s.x;
                        s.py = s.y;
                        s.z = clampf(rng.gen_range(0.08..1.0), 0.06, 1.0);
                        s.hue = rng.gen_range(0.0..1.0);
                        s.tw = rng.gen_range(0.0..10.0);

                        let dx = s.x - cx;
                        let dy = s.y - cy;
                        let len = (dx * dx + dy * dy).sqrt().max(1.0);
                        let nx = dx / len;
                        let ny = dy / len;
                        let base = lerp(35.0, 260.0, 1.0 - s.z);
                        s.vx = nx * base;
                        s.vy = ny * base;
                        continue;
                    }

                    // Deposit into subpixel buffer
                    // brightness: nearer + twinkle + warp exaggeration
                    let base_b = lerp(0.18, 1.95, 1.0 - z) * (0.65 + 0.70 * tw) * exposure;
                    let b = base_b * point_gain;

                    // point
                    let xi = s.x.round() as i32;
                    let yi = s.y.round() as i32;
                    if xi >= 0 && yi >= 0 && (xi as usize) < sw && (yi as usize) < sh {
                        let i = idx(xi as usize, yi as usize, sw);
                        sub[i] += b;
                    }

                    // micro-bloom (small gaussian-ish stamp)
                    if b > 0.65 {
                        let r = if warp { 2 } else { 1 };
                        for oy in -r..=r {
                            for ox in -r..=r {
                                let xx = xi + ox;
                                let yy = yi + oy;
                                if xx < 0 || yy < 0 || xx >= sw as i32 || yy >= sh as i32 {
                                    continue;
                                }
                                let d2 = (ox * ox + oy * oy) as f32;
                                let w = (-d2 * 0.55).exp();
                                sub[idx(xx as usize, yy as usize, sw)] += b * 0.18 * w;
                            }
                        }
                    }

                    // streak/trail
                    if trails {
                        let dx = s.x - s.px;
                        let dy = s.y - s.py;
                        let dist = (dx * dx + dy * dy).sqrt();
                        let steps = min(10, max(2, dist.ceil() as i32)) as i32;

                        for tstep in 0..steps {
                            let tt = (tstep as f32) / (steps as f32);
                            let x = s.px + dx * tt;
                            let y = s.py + dy * tt;
                            let xx = x.round() as i32;
                            let yy = y.round() as i32;
                            if xx < 0 || yy < 0 || xx >= sw as i32 || yy >= sh as i32 {
                                continue;
                            }
                            let fade = (1.0 - tt).powf(1.6);
                            let sb = b * streak_gain * fade * 0.55;
                            sub[idx(xx as usize, yy as usize, sw)] += sb;
                        }
                    }
                }
            }

            // Render (diff-based per cell)
            // Map subpixels -> braille cell (2x4), mask with per-dot threshold, choose a color.
            let mut out = io::stdout();
            queue!(out, BeginSynchronizedUpdate)?;
            queue!(out, SetBackgroundColor(Color::Black))?;

            let cells_w = tw as usize;
            let cells_h = th as usize;

            // HUD lines
            let hud1 = format!(
                "starfield  stars:{}  speed:{:.2}  trails:{}  theme:{}  fps:{:.0}  (Q quit, H help)",
                target_stars,
                speed,
                if trails { "on" } else { "off" },
                THEMES[theme_index].name,
                fps_smooth
            );
            let hud2 = "Up/Down speed  Left/Right density  Space pause  T trails  C theme  R reseed";

            // Clear only when needed; rely on full redraw of changed cells + HUD overwrite.
            // Overwrite HUD areas each frame for stability.
            queue!(out, cursor::MoveTo(0, 0), SetForegroundColor(Color::DarkGrey), Print(pad_to(&hud1, cells_w)))?;
            queue!(out, cursor::MoveTo(0, 1), SetForegroundColor(Color::DarkGrey), Print(pad_to(hud2, cells_w)))?;

            if show_help {
                draw_help(&mut out, cells_w, cells_h)?;
            }

            // Render field starting at y=2 (leave HUD)
            let y_start = 2usize;
            for cy in y_start..cells_h {
                for cx in 0..cells_w {
                    let cell_i = idx(cx, cy, cells_w);

                    let sx0 = cx * 2;
                    let sy0 = cy * 4;

                    // compute per-dot intensity + mask
                    let mut mask: u8 = 0;
                    let mut sum = 0.0f32;
                    let mut mx = 0.0f32;

                    for dy in 0..4 {
                        for dx in 0..2 {
                            let sx = sx0 + dx;
                            let sy = sy0 + dy;
                            if sx >= sw || sy >= sh {
                                continue;
                            }
                            let v = sub[idx(sx, sy, sw)];
                            sum += v;
                            mx = mx.max(v);
                            // threshold (tuned)
                            let thr = 0.10 + 0.18 * (1.0 - clampf(mx, 0.0, 1.0));
                            if v > thr {
                                mask |= dot_bit(dx, dy);
                            }
                        }
                    }

                    // Choose brightness for coloring
                    let avg = sum / 8.0;
                    let lum = clampf(avg, 0.0, 2.2);

                    // mask 0 => space, but still allow faint haze as dots via threshold; keep it space if empty.
                    let ch = if mask == 0 { ' ' } else { braille_char(mask) };

                    // Color (cheap: near-white for bright; optionally tinted)
                    let rgb = match THEMES[theme_index].mode {
                        ThemeMode::Mono => {
                            let v = (clampf(lum * 0.95, 0.0, 1.0) * 255.0) as u8;
                            Rgb { r: v, g: v, b: v }
                        }
                        ThemeMode::Hue {
                            base,
                            span,
                            sat_min,
                            sat_max,
                            val_min,
                            val_max,
                        } => {
                            let hue = (base + (cy as f32 / cells_h as f32) * span + (lum * 0.06)).fract();
                            let sat = clampf(sat_min + lum * (sat_max - sat_min), 0.0, 1.0);
                            let val = clampf(val_min + lum * (val_max - val_min), 0.0, 1.0);
                            hsv_to_rgb(hue, sat, val)
                        }
                        ThemeMode::Joshnet => {
                            let t = clampf((lum - 0.12) / 1.15, 0.0, 1.0);
                            let pr = 180.0;
                            let pg = 70.0;
                            let pb = 255.0;
                            let gr = 255.0;
                            let gg = 200.0;
                            let gb = 70.0;
                            let r = lerp(pr, gr, t);
                            let g = lerp(pg, gg, t);
                            let b = lerp(pb, gb, t);
                            let strength = clampf(0.25 + lum * 0.85, 0.0, 1.0);
                            Rgb {
                                r: (r * strength).min(255.0) as u8,
                                g: (g * strength).min(255.0) as u8,
                                b: (b * strength).min(255.0) as u8,
                            }
                        }
                        ThemeMode::Disco => {
                            let t = start_time.elapsed().as_secs_f32();
                            let hue = (t * 0.07 + (lum * 0.08) + (cy as f32 / cells_h as f32) * 0.10).fract();
                            let sat = clampf(0.35 + lum * 0.45, 0.0, 1.0);
                            let val = clampf(0.15 + lum * 0.90, 0.0, 1.0);
                            hsv_to_rgb(hue, sat, val)
                        }
                    };

                    // Diff-based update
                    let pm = prev_mask[cell_i];
                    let pc = prev_color[cell_i];
                    if pm == mask && pc.r == rgb.r && pc.g == rgb.g && pc.b == rgb.b {
                        continue;
                    }
                    prev_mask[cell_i] = mask;
                    prev_color[cell_i] = rgb;

                    queue!(
                        out,
                        cursor::MoveTo(cx as u16, cy as u16),
                        SetForegroundColor(Color::Rgb { r: rgb.r, g: rgb.g, b: rgb.b }),
                        Print(ch)
                    )?;
                }
            }

            queue!(out, ResetColor, EndSynchronizedUpdate)?;
            out.flush()?;
        }

        Ok(())
    })();

    let _ = cleanup();
    res
}

fn pad_to(s: &str, w: usize) -> String {
    if s.chars().count() >= w {
        s.chars().take(w).collect()
    } else {
        let mut out = String::with_capacity(w);
        out.push_str(s);
        let n = w - s.chars().count();
        for _ in 0..n {
            out.push(' ');
        }
        out
    }
}

fn draw_help<W: Write>(out: &mut W, w: usize, h: usize) -> io::Result<()> {
    let box_w = min(w.saturating_sub(4), 68);
    let box_h = min(h.saturating_sub(6), 10);
    let x0 = (w.saturating_sub(box_w)) / 2;
    let y0 = (h.saturating_sub(box_h)) / 2;

    let title = "HELP";
    let lines = [
        "Q / Esc      Quit",
        "Space        Pause",
        "Up/Down      Speed",
        "Left/Right   Star count",
        "T            Trails",
        "C            Cycle theme",
        "R            Reseed",
        "H            Toggle this overlay",
    ];

    // simple bordered box (ASCII)
    queue!(
        out,
        SetForegroundColor(Color::DarkGrey),
        cursor::MoveTo(x0 as u16, y0 as u16),
        Print(format!("+{}+", "-".repeat(box_w.saturating_sub(2))))
    )?;
    for i in 1..box_h.saturating_sub(1) {
        queue!(
            out,
            cursor::MoveTo(x0 as u16, (y0 + i) as u16),
            Print(format!("|{}|", " ".repeat(box_w.saturating_sub(2))))
        )?;
    }
    queue!(
        out,
        cursor::MoveTo(x0 as u16, (y0 + box_h.saturating_sub(1)) as u16),
        Print(format!("+{}+", "-".repeat(box_w.saturating_sub(2))))
    )?;

    let tline = format!("{}{}", title, " ".repeat(box_w.saturating_sub(2 + title.len())));
    queue!(
        out,
        SetForegroundColor(Color::Grey),
        cursor::MoveTo((x0 + 2) as u16, (y0 + 1) as u16),
        Print(tline.chars().take(box_w.saturating_sub(2)).collect::<String>())
    )?;

    let mut yy = y0 + 2;
    for l in lines.iter() {
        if yy >= y0 + box_h.saturating_sub(1) {
            break;
        }
        let text = pad_to(l, box_w.saturating_sub(4));
        queue!(
            out,
            SetForegroundColor(Color::Grey),
            cursor::MoveTo((x0 + 2) as u16, yy as u16),
            Print(text)
        )?;
        yy += 1;
    }

    Ok(())
}

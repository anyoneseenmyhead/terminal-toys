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
    io::{self, Stdout, Write},
    time::{Duration, Instant},
};

const FPS_CAP: u64 = 30;

// Braille cell is 2x4 subpixels
const SUB_W: usize = 2;
const SUB_H: usize = 4;

// Base tuning
const BASE_STREAMERS: usize = 6;
const MAX_STREAMERS: usize = 44;
const MIN_STREAMERS: usize = 2;

const DECAY: f32 = 0.865;          // persistence of glow per frame
const DEPOSIT: f32 = 1.28;         // deposit strength
const CORE_GLOW: f32 = 0.85;       // central electrode glow

const STEP_LEN: f32 = 0.85;
const BASE_STEPS_PER_STREAMER: usize = 120;

const TURB_SLOW_SCALE: f32 = 0.055;
const TURB_FAST_SCALE: f32 = 0.14;
const TURB_SLOW_SPEED: f32 = 0.55;
const TURB_FAST_SPEED: f32 = 2.15;

const JITTER: f32 = 0.30;
const CURL: f32 = 1.10;

const DIFFUSE_EVERY: u32 = 4;      // do a cheap diffusion pass every N frames
const DIFFUSE_STRENGTH: f32 = 0.22;
const PALETTE_COUNT: usize = PALETTES.len();

#[derive(Clone)]
struct Streamer {
    base_ang: f32,
    phase: f32,
    ttl: f32,
    age: f32,
    wiggle: f32,
}

#[derive(Clone, Copy)]
struct Palette {
    core: Color,
    mid: Color,
    tip: Color,
    haze: Color,
    glass: Color,
    hud: Color,
}

const PALETTES: [Palette; 6] = [
    Palette {
        core: Color::White,
        mid: Color::Magenta,
        tip: Color::Blue,
        haze: Color::DarkMagenta,
        glass: Color::DarkGrey,
        hud: Color::DarkGrey,
    },
    Palette {
        core: Color::White,
        mid: Color::Cyan,
        tip: Color::Blue,
        haze: Color::DarkCyan,
        glass: Color::DarkGrey,
        hud: Color::DarkGrey,
    },
    Palette {
        core: Color::White,
        mid: Color::Red,
        tip: Color::Magenta,
        haze: Color::DarkRed,
        glass: Color::DarkGrey,
        hud: Color::DarkGrey,
    },
    Palette {
        core: Color::White,
        mid: Color::Yellow,
        tip: Color::Red,
        haze: Color::DarkYellow,
        glass: Color::DarkGrey,
        hud: Color::DarkGrey,
    },
    Palette {
        core: Color::White,
        mid: Color::Green,
        tip: Color::Cyan,
        haze: Color::DarkGreen,
        glass: Color::DarkGrey,
        hud: Color::DarkGrey,
    },
    Palette {
        core: Color::White,
        mid: Color::White,
        tip: Color::Cyan,
        haze: Color::Grey,
        glass: Color::DarkGrey,
        hud: Color::DarkGrey,
    },
];

#[inline]
fn clamp(v: f32, a: f32, b: f32) -> f32 {
    v.max(a).min(b)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn main() -> io::Result<()> {
    let mut out = io::stdout();

    // Flicker-safe setup
    execute!(out, EnterAlternateScreen, cursor::Hide, DisableLineWrap)?;
    terminal::enable_raw_mode()?;

    let res = run(&mut out);

    terminal::disable_raw_mode().ok();
    execute!(out, ResetColor, EnableLineWrap, cursor::Show, LeaveAlternateScreen).ok();

    res
}

fn run(out: &mut Stdout) -> io::Result<()> {
    let mut rng = StdRng::from_entropy();

    let mut palette_i: usize = 0;
    let mut streamers_target: usize = BASE_STREAMERS;

    // UI toggles
    let mut show_hud = true;
    let mut show_glass = true;
    let mut show_branch = true;
    let mut paused = false;
    let mut step_once = false;
    let mut sim_speed: f32 = 1.0;
    let mut disco_mode = false;
    let mut disco_tick: u32 = 0;

    // Adaptive quality
    let target_frame = 1.0 / (FPS_CAP as f32);
    let mut quality: f32 = 1.0;
    let mut ema_frame: f32 = target_frame;

    // Terminal + buffers
    let mut last_size = (0u16, 0u16);
    let mut w_cells: usize = 0;
    let mut h_cells: usize = 0;
    let mut w_sub: usize = 0;
    let mut h_sub: usize = 0;

    let mut glow_layers: Vec<Vec<f32>> = Vec::new();
    let mut tmp_layers: Vec<Vec<f32>> = Vec::new(); // for diffusion
    let mut frame_idx: u32 = 0;

    // Streamers
    let mut streamers: Vec<Streamer> = Vec::new();

    let mut sim_time: f32 = 0.0;
    let mut last = Instant::now();

    execute!(out, Clear(ClearType::All))?;

    loop {
        // Input
        while event::poll(Duration::from_millis(0))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(()),
                    KeyCode::Char('c') | KeyCode::Char('C') => palette_i = (palette_i + 1) % PALETTES.len(),
                    KeyCode::Char('+') | KeyCode::Char('=') => streamers_target = (streamers_target + 1).min(MAX_STREAMERS),
                    KeyCode::Char('-') | KeyCode::Char('_') => streamers_target = streamers_target.saturating_sub(1).max(MIN_STREAMERS),
                    KeyCode::Char('h') | KeyCode::Char('H') => show_hud = !show_hud,
                    KeyCode::Char('g') | KeyCode::Char('G') => show_glass = !show_glass,
                    KeyCode::Char('b') | KeyCode::Char('B') => show_branch = !show_branch,
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        disco_mode = !disco_mode;
                        for layer in glow_layers.iter_mut() {
                            layer.fill(0.0);
                        }
                    }
                    KeyCode::Char('p') | KeyCode::Char('P') => paused = !paused,
                    KeyCode::Char('n') | KeyCode::Char('N') => {
                        if paused {
                            step_once = true;
                        }
                    }
                    KeyCode::Char('[') => sim_speed = (sim_speed - 0.25).max(0.25),
                    KeyCode::Char(']') => sim_speed = (sim_speed + 0.25).min(4.0),
                    _ => {}
                }
            }
        }

        // Resize
        let (tw, th) = terminal::size()?;
        if (tw, th) != last_size {
            last_size = (tw, th);

            w_cells = tw.max(10) as usize;
            h_cells = th.saturating_sub(1).max(6) as usize;

            w_sub = w_cells * SUB_W;
            h_sub = h_cells * SUB_H;

            glow_layers = vec![vec![0.0; w_sub * h_sub]; PALETTE_COUNT];
            tmp_layers = vec![vec![0.0; w_sub * h_sub]; PALETTE_COUNT];

            // Re-seed streamers on resize
            streamers.clear();
            for i in 0..streamers_target {
                streamers.push(new_streamer(&mut rng, i, streamers_target));
            }

            execute!(out, Clear(ClearType::All))?;
        }

        let now = Instant::now();
        let dt = (now - last).as_secs_f32();
        last = now;

        // Smooth frame time and adapt quality (steps)
        ema_frame = lerp(ema_frame, dt.max(0.0001), 0.10);
        if ema_frame > target_frame * 1.18 {
            quality = (quality * 0.94).max(0.62);
        } else if ema_frame < target_frame * 0.95 {
            quality = (quality * 1.02).min(1.0);
        }

        let do_sim = !paused || step_once;
        if step_once {
            step_once = false;
        }

        let dt_sim = if do_sim { dt * sim_speed } else { 0.0 };
        if do_sim {
            sim_time += dt_sim;
        }
        let t = sim_time;
        let pal = PALETTES[palette_i];

        if do_sim {
            if disco_mode {
                for layer in glow_layers.iter_mut() {
                    for v in layer.iter_mut() {
                        *v *= DECAY;
                    }
                }

                if DIFFUSE_EVERY > 0 && (frame_idx % DIFFUSE_EVERY == 0) {
                    for (src, dst) in glow_layers.iter().zip(tmp_layers.iter_mut()) {
                        diffuse(src, dst, w_sub, h_sub, DIFFUSE_STRENGTH);
                    }
                    for (src, dst) in tmp_layers.iter().zip(glow_layers.iter_mut()) {
                        dst.copy_from_slice(src);
                    }
                }
            } else if !glow_layers.is_empty() {
                let glow = &mut glow_layers[0];
                let tmp = &mut tmp_layers[0];
                for v in glow.iter_mut() {
                    *v *= DECAY;
                }
                if DIFFUSE_EVERY > 0 && (frame_idx % DIFFUSE_EVERY == 0) {
                    diffuse(glow, tmp, w_sub, h_sub, DIFFUSE_STRENGTH);
                    glow.copy_from_slice(tmp);
                }
            }

            // Globe geometry in subpixel space
            let cx = (w_sub as f32) * 0.5;
            let cy = (h_sub as f32) * 0.48;
            let r = (w_sub.min(h_sub) as f32) * 0.46;
            let r_inner = r * 0.16;

            // Electrode: bright core + small stem
            let core_layer = if disco_mode {
                (disco_tick as usize) % PALETTE_COUNT
            } else {
                0
            };
            let glow = &mut glow_layers[core_layer];
            deposit_radial(glow, w_sub, h_sub, cx, cy, r_inner * 0.88, CORE_GLOW * 1.15);
            // Stem downward
            let stem_len = r * 0.52;
            deposit_thick_line(
                glow,
                w_sub,
                h_sub,
                cx,
                cy + r_inner * 0.35,
                cx,
                (cy + r_inner * 0.35 + stem_len).min((h_sub - 2) as f32),
                0.85,
                0.85,
            );

            // Adjust streamer list size toward target
            if streamers.len() < streamers_target {
                let start = streamers.len();
                for i in start..streamers_target {
                    streamers.push(new_streamer(&mut rng, i, streamers_target));
                }
            } else if streamers.len() > streamers_target {
                streamers.truncate(streamers_target);
            }

            // Streamer lifetimes
            let streamer_count = streamers.len().max(1);
            for (i, s) in streamers.iter_mut().enumerate() {
                s.age += dt_sim;
                if s.age >= s.ttl {
                    *s = new_streamer(&mut rng, i, streamer_count);
                }
            }

            let steps_per = ((BASE_STEPS_PER_STREAMER as f32) * quality).round() as usize;
            let steps_per = steps_per.clamp(70, BASE_STEPS_PER_STREAMER);

            // Streamers
            for (si, s) in streamers.iter().enumerate() {
                // Base angle plus slow drift
                let drift = 0.22 * (t * 0.35 + s.phase).sin() + 0.10 * (t * 0.21 + s.phase * 1.7).cos();
                let ang = s.base_ang + drift;

                let mut x = cx + ang.cos() * r_inner * 0.70;
                let mut y = cy + ang.sin() * r_inner * 0.70;

                let mut dirx = ang.cos();
                let mut diry = ang.sin();

                for step in 0..steps_per {
                    let px = x;
                    let py = y;

                    // Outward basis
                    let vx = x - cx;
                    let vy = y - cy;
                    let inv = 1.0 / (vx * vx + vy * vy + 1.0).sqrt();
                    let ox = vx * inv;
                    let oy = vy * inv;

                    // Curl basis
                    let cxv = -oy;
                    let cyv = ox;

                    // Layered turbulence
                    let slow_n = noise2((x * TURB_SLOW_SCALE) + s.phase, (y * TURB_SLOW_SCALE) - s.phase, t * TURB_SLOW_SPEED);
                    let fast_n = noise2((x * TURB_FAST_SCALE) - s.phase, (y * TURB_FAST_SCALE) + s.phase, t * TURB_FAST_SPEED);

                    let jx = ((slow_n * 2.0 - 1.0) * (JITTER * 0.55)) + ((fast_n * 2.0 - 1.0) * (JITTER * 0.65));
                    let jy = ((noise2((x * TURB_SLOW_SCALE) + 9.3, (y * TURB_SLOW_SCALE) + 2.1, t * TURB_SLOW_SPEED) * 2.0 - 1.0)
                        * (JITTER * 0.55))
                        + ((noise2((x * TURB_FAST_SCALE) - 4.7, (y * TURB_FAST_SCALE) - 8.1, t * TURB_FAST_SPEED) * 2.0 - 1.0)
                            * (JITTER * 0.65));

                    let curl_scale = CURL + 0.25 * (s.wiggle * (t * 0.8 + s.phase).sin());

                    dirx = lerp(dirx, ox + cxv * curl_scale + jx, 0.40);
                    diry = lerp(diry, oy + cyv * curl_scale + jy, 0.40);

                    // Normalize
                    let mag = (dirx * dirx + diry * diry + 1e-6).sqrt();
                    dirx /= mag;
                    diry /= mag;

                    x += dirx * STEP_LEN;
                    y += diry * STEP_LEN;

                    // stop outside globe
                    let dx = x - cx;
                    let dy = y - cy;
                    if dx * dx + dy * dy > r * r {
                        break;
                    }

                    // Deposit with stronger intensity near core
                    let frac = step as f32 / (steps_per as f32);
                    let amp = DEPOSIT * (1.0 - frac).powf(0.55);
                    if disco_mode {
                        let layer = ((disco_tick + step as u32 + (si as u32 * 7)) as usize) % PALETTE_COUNT;
                        deposit_line(&mut glow_layers[layer], w_sub, h_sub, px, py, x, y, amp);
                    } else {
                        deposit_line(&mut glow_layers[0], w_sub, h_sub, px, py, x, y, amp);
                    }

                    // Branching: occasional short side arc
                    if show_branch && step > 10 && step + 18 < steps_per {
                        // stable-ish stochastic trigger
                        if hash01(si as u32, step as u32, (t * 20.0) as u32) < 0.012 {
                            let side = if hash01(step as u32, si as u32, (t * 7.0) as u32) < 0.5 { -1.0 } else { 1.0 };
                            let bx = -diry * side;
                            let by = dirx * side;
                            if disco_mode {
                                let layer = ((disco_tick + step as u32 + (si as u32 * 7)) as usize) % PALETTE_COUNT;
                                branch_deposit(
                                    &mut glow_layers[layer],
                                    w_sub,
                                    h_sub,
                                    x,
                                    y,
                                    bx,
                                    by,
                                    r,
                                    cx,
                                    cy,
                                    amp * 0.75,
                                );
                            } else {
                                branch_deposit(&mut glow_layers[0], w_sub, h_sub, x, y, bx, by, r, cx, cy, amp * 0.75);
                            }
                        }
                    }
                }
            }

            frame_idx = frame_idx.wrapping_add(1);
            if disco_mode {
                disco_tick = disco_tick.wrapping_add(1);
            }
        }

        // Render (diff draw only)
        render(
            out,
            &glow_layers,
            w_cells,
            h_cells,
            w_sub,
            h_sub,
            last_size,
            pal,
            disco_mode,
            show_branch,
            show_glass,
            show_hud,
            streamers_target,
            quality,
            sim_speed,
            paused,
        )?;
        // Frame cap
        sleep_to_cap(now, FPS_CAP);
    }
}

fn render(
    out: &mut Stdout,
    glow_layers: &[Vec<f32>],
    w_cells: usize,
    h_cells: usize,
    w_sub: usize,
    h_sub: usize,
    term_size: (u16, u16),
    pal: Palette,
    disco_mode: bool,
    show_branch: bool,
    show_glass: bool,
    show_hud: bool,
    streamers: usize,
    quality: f32,
    sim_speed: f32,
    paused: bool,
) -> io::Result<()> {
    let cx = (w_sub as f32) * 0.5;
    let cy = (h_sub as f32) * 0.48;
    let r = (w_sub.min(h_sub) as f32) * 0.46;

    // Braille dot mapping:
    // (x,y) -> dot:
    // (0,0)=1, (0,1)=2, (0,2)=4, (0,3)=64
    // (1,0)=8, (1,1)=16,(1,2)=32,(1,3)=128
    const DOTS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

    // Static across calls: use terminal cell scratch stored in global vectors is ideal,
    // but keeping it simple by using thread_local-ish state is not worth it here.
    // Instead, we store prev state in the alternate screen by reading nothing and reusing
    // diff logic using per-cell caches passed via static mut is not acceptable.
    //
    // So we do a minimal approach: keep caches in a singleton via once_cell is overkill.
    // We'll rebuild caches by storing them in the terminal itself is not possible.
    //
    // Solution: keep them in static mut is not used. Instead, we attach them to stdout via
    // a hidden global is also not used.
    //
    // Practical approach: keep caches in a global using a function-local static with UnsafeCell is messy.
    // So, we store them in the terminal each frame by printing all is worse.
    //
    // Best approach: keep caches in the run loop. Here, we rely on the run loop's caches.
    //
    // This render() is called from run() and uses the caches there. To keep this file standalone,
    // we keep caches in a module-level static with safe interior mutability.

    RENDER_CACHE.with(|cache| {
        let mut c = cache.borrow_mut();
        if c.w_cells != w_cells || c.h_cells != h_cells {
            c.w_cells = w_cells;
            c.h_cells = h_cells;
            c.prev_chars = vec!['\0'; w_cells * h_cells];
            c.prev_cols = vec![u8::MAX; w_cells * h_cells];
        }
        c.flags = Flags {
            branch: show_branch,
            glass: show_glass,
        };

        queue!(out, BeginSynchronizedUpdate)?;

        let mut changed_any = false;

        for cy_cell in 0..h_cells {
            for cx_cell in 0..w_cells {
                let cell_i = cy_cell * w_cells + cx_cell;

                let sub_x0 = cx_cell * SUB_W;
                let sub_y0 = cy_cell * SUB_H;

                let mut bits: u8 = 0;
                let mut sum = 0.0f32;
                let mut peak = 0.0f32;
                let mut sum_by_pal = [0.0f32; PALETTE_COUNT];

                // Pre-pass: sum + peak inside globe
                for sy in 0..SUB_H {
                    for sx in 0..SUB_W {
                        let sxp = (sub_x0 + sx) as f32 + 0.5;
                        let syp = (sub_y0 + sy) as f32 + 0.5;

                        let dx = sxp - cx;
                        let dy = syp - cy;
                        let d2 = dx * dx + dy * dy;
                        if d2 > r * r {
                            continue;
                        }

                        let gi = (sub_y0 + sy) * w_sub + (sub_x0 + sx);
                        let mut v = if disco_mode {
                            let mut total = 0.0f32;
                            for (pi, layer) in glow_layers.iter().enumerate() {
                                let lv = layer[gi];
                                total += lv;
                                sum_by_pal[pi] += lv;
                            }
                            total
                        } else {
                            glow_layers[0][gi]
                        };

                        // Glass rim highlight (thin ring near boundary)
                        if show_glass {
                            let dist = d2.sqrt();
                            let edge = (dist - (r * 0.965)).max(0.0);
                            if edge > 0.0 {
                                // sharp-ish falloff for rim
                                let rim = (1.0 - clamp(edge / (r * 0.045), 0.0, 1.0)).powf(2.2);
                                v += rim * 0.35;
                            }
                        }

                        sum += v;
                        if v > peak {
                            peak = v;
                        }
                    }
                }

                let mut ch = ' ';
                let mut col_bucket: u8 = 0;
                let mut fg = pal.haze;

                if peak > 0.02 {
                    let thr = peak * 0.52;
                    for sy in 0..SUB_H {
                        for sx in 0..SUB_W {
                            let sxp = (sub_x0 + sx) as f32 + 0.5;
                            let syp = (sub_y0 + sy) as f32 + 0.5;

                            let dx = sxp - cx;
                            let dy = syp - cy;
                            let d2 = dx * dx + dy * dy;
                            if d2 > r * r {
                                continue;
                            }

                            let gi = (sub_y0 + sy) * w_sub + (sub_x0 + sx);
                            let mut v = if disco_mode {
                                let mut total = 0.0f32;
                                for layer in glow_layers.iter() {
                                    total += layer[gi];
                                }
                                total
                            } else {
                                glow_layers[0][gi]
                            };

                            if show_glass {
                                let dist = d2.sqrt();
                                let edge = (dist - (r * 0.965)).max(0.0);
                                if edge > 0.0 {
                                    let rim = (1.0 - clamp(edge / (r * 0.045), 0.0, 1.0)).powf(2.2);
                                    v += rim * 0.35;
                                }
                            }

                            if v >= thr {
                                bits |= DOTS[sx][sy];
                            }
                        }
                    }

                    if bits != 0 {
                        ch = char::from_u32(0x2800 + bits as u32).unwrap_or(' ');
                    } else if sum > 0.22 {
                        ch = '·';
                    }

                    if ch != ' ' {
                        // Heat estimate
                        let heat = clamp(sum / 6.5, 0.0, 1.0);

                        // Radial factor: closer to center tends toward "core"
                        let cell_cx = (sub_x0 as f32) + 1.0;
                        let cell_cy = (sub_y0 as f32) + 2.0;
                        let dx = cell_cx - cx;
                        let dy = cell_cy - cy;
                        let dist = (dx * dx + dy * dy).sqrt();
                        let radial = clamp(dist / r, 0.0, 1.0);

                        // Combine for a simple 3-point gradient:
                        // high heat or near center => core
                        // mid => mid
                        // outer and cooler => tip
                        let core_score = (1.0 - radial) * 0.85 + heat * 0.45;
                        let tip_score = radial * 0.85 + (1.0 - heat) * 0.25;
                        let pal = if disco_mode {
                            let mut best_i = 0usize;
                            let mut best_v = sum_by_pal[0];
                            for i in 1..PALETTE_COUNT {
                                if sum_by_pal[i] > best_v {
                                    best_v = sum_by_pal[i];
                                    best_i = i;
                                }
                            }
                            PALETTES[best_i]
                        } else {
                            pal
                        };

                        if core_score > 0.78 {
                            fg = pal.core;
                            col_bucket = 2;
                        } else if tip_score > 0.72 {
                            fg = pal.tip;
                            col_bucket = 0;
                        } else {
                            fg = pal.mid;
                            col_bucket = 1;
                        }

                        // If this is mostly rim highlight, bias to glass color
                        if show_glass && radial > 0.94 && heat < 0.25 && sum > 0.10 {
                            fg = pal.glass;
                            col_bucket = 3;
                        }
                    }
                }

                if c.prev_chars[cell_i] != ch || c.prev_cols[cell_i] != col_bucket {
                    c.prev_chars[cell_i] = ch;
                    c.prev_cols[cell_i] = col_bucket;
                    changed_any = true;

                    queue!(
                        out,
                        cursor::MoveTo(cx_cell as u16, cy_cell as u16),
                        SetForegroundColor(fg),
                        Print(ch)
                    )?;
                }
            }
        }

        if show_hud {
            let hud_y = h_cells as u16;
            let status = if paused { "paused" } else { "running" };
            queue!(
                out,
                SetForegroundColor(pal.hud),
                cursor::MoveTo(0, hud_y),
                Clear(ClearType::CurrentLine),
                Print(format!(
                    "plasma-globe | Q quit | C palette | D disco {} | +/- streamers {} | B branch {} | G glass {} | [ ] speed {:.2}x | P pause | N step | H hud | quality {:.2} | {}x{} | {}",
                    onoff_str(disco_mode),
                    streamers,
                    onoff_str(show_branch),
                    onoff_str(show_glass),
                    sim_speed,
                    quality,
                    term_size.0,
                    term_size.1,
                    status,
                )),
                ResetColor
            )?;
        } else {
            // Clear HUD line once if hidden (avoid stale text)
            let hud_y = h_cells as u16;
            queue!(out, cursor::MoveTo(0, hud_y), Clear(ClearType::CurrentLine))?;
        }

        queue!(out, EndSynchronizedUpdate)?;
        out.flush()?;

        // If nothing changed, still yield slightly
        if !changed_any {
            std::thread::sleep(Duration::from_millis(2));
        }

        Ok(())
    })
}

// Render cache with toggles stored for HUD correctness
use std::cell::RefCell;

struct RenderCache {
    w_cells: usize,
    h_cells: usize,
    prev_chars: Vec<char>,
    prev_cols: Vec<u8>,
    flags: Flags,
}

#[derive(Clone, Copy)]
struct Flags {
    branch: bool,
    glass: bool,
}

thread_local! {
    static RENDER_CACHE: RefCell<RenderCache> = RefCell::new(RenderCache{
        w_cells: 0,
        h_cells: 0,
        prev_chars: Vec::new(),
        prev_cols: Vec::new(),
        flags: Flags{branch: true, glass: true},
    });
}

fn onoff_str(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn sleep_to_cap(frame_start: Instant, fps: u64) {
    let frame_ms = 1000 / fps.max(1);
    let elapsed_ms = frame_start.elapsed().as_millis() as u64;
    if elapsed_ms < frame_ms {
        std::thread::sleep(Duration::from_millis(frame_ms - elapsed_ms));
    }
}

fn new_streamer(rng: &mut StdRng, i: usize, n: usize) -> Streamer {
    let base = (i as f32 / n.max(1) as f32) * std::f32::consts::TAU;
    Streamer {
        base_ang: base + rng.gen_range(-0.15..0.15),
        phase: rng.gen_range(0.0..2000.0),
        ttl: rng.gen_range(2.4..6.5),
        age: rng.gen_range(0.0..2.0),
        wiggle: rng.gen_range(0.7..1.4),
    }
}

fn diffuse(src: &[f32], dst: &mut [f32], w: usize, h: usize, strength: f32) {
    // 3x3-ish blur using two passes: horizontal then vertical
    // dst used as scratch then written back by caller
    // Horizontal into dst
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let a = src[row + x];
            let l = if x > 0 { src[row + (x - 1)] } else { a };
            let r = if x + 1 < w { src[row + (x + 1)] } else { a };
            dst[row + x] = lerp(a, (l + a + r) / 3.0, strength);
        }
    }
    // Vertical back into dst (in-place from current dst)
    // Use a temp copy of the horizontal result by reading from dst and writing to a local scratch would be ideal.
    // Instead, do vertical into a small rolling buffer per column using src2 = dst cloned once.
    let src2 = dst.to_vec();
    for y in 0..h {
        for x in 0..w {
            let a = src2[y * w + x];
            let u = if y > 0 { src2[(y - 1) * w + x] } else { a };
            let d = if y + 1 < h { src2[(y + 1) * w + x] } else { a };
            dst[y * w + x] = lerp(a, (u + a + d) / 3.0, strength);
        }
    }
}

fn deposit_radial(glow: &mut [f32], w: usize, h: usize, cx: f32, cy: f32, r: f32, amp: f32) {
    let r2 = r * r;
    let minx = ((cx - r - 1.0).floor() as i32).max(0) as usize;
    let maxx = ((cx + r + 1.0).ceil() as i32).min((w - 1) as i32) as usize;
    let miny = ((cy - r - 1.0).floor() as i32).max(0) as usize;
    let maxy = ((cy + r + 1.0).ceil() as i32).min((h - 1) as i32) as usize;

    for y in miny..=maxy {
        for x in minx..=maxx {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d2 = dx * dx + dy * dy;
            if d2 > r2 {
                continue;
            }
            let t = 1.0 - (d2 / (r2 + 1e-6));
            let v = amp * t.powf(1.9);
            glow[y * w + x] = (glow[y * w + x] + v).min(4.0);
        }
    }
}

fn deposit_line(glow: &mut [f32], w: usize, h: usize, x0: f32, y0: f32, x1: f32, y1: f32, amp: f32) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let steps = (len * 1.6).ceil() as i32;

    for i in 0..=steps {
        let t = i as f32 / steps.max(1) as f32;
        let x = x0 + dx * t;
        let y = y0 + dy * t;
        stamp(glow, w, h, x, y, amp);
    }
}

fn deposit_thick_line(
    glow: &mut [f32],
    w: usize,
    h: usize,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    amp: f32,
    radius: f32,
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let steps = (len * 1.4).ceil() as i32;

    for i in 0..=steps {
        let t = i as f32 / steps.max(1) as f32;
        let x = x0 + dx * t;
        let y = y0 + dy * t;
        stamp_radius(glow, w, h, x, y, amp, radius);
    }
}

fn branch_deposit(glow: &mut [f32], w: usize, h: usize, x: f32, y: f32, bx: f32, by: f32, r: f32, cx: f32, cy: f32, amp: f32) {
    let mut px = x;
    let mut py = y;
    let mut dirx = bx;
    let mut diry = by;

    for s in 0..18 {
        // curve it slightly to look like a real branch
        let bend = 0.18 + (s as f32) * 0.01;
        let nx = lerp(dirx, -diry, bend);
        let ny = lerp(diry, dirx, bend);
        dirx = nx;
        diry = ny;
        let mag = (dirx * dirx + diry * diry + 1e-6).sqrt();
        dirx /= mag;
        diry /= mag;

        let nxp = px + dirx * (STEP_LEN * 0.85);
        let nyp = py + diry * (STEP_LEN * 0.85);

        let dx = nxp - cx;
        let dy = nyp - cy;
        if dx * dx + dy * dy > r * r {
            break;
        }

        let frac = s as f32 / 18.0;
        deposit_line(glow, w, h, px, py, nxp, nyp, amp * (1.0 - frac).powf(0.7));
        px = nxp;
        py = nyp;
    }
}

fn stamp(glow: &mut [f32], w: usize, h: usize, x: f32, y: f32, amp: f32) {
    stamp_radius(glow, w, h, x, y, amp, 1.0);
}

fn stamp_radius(glow: &mut [f32], w: usize, h: usize, x: f32, y: f32, amp: f32, radius: f32) {
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;

    let r = radius.max(0.8);
    let rr = (r * 1.35).ceil() as i32;

    for oy in -rr..=rr {
        for ox in -rr..=rr {
            let xx = ix + ox;
            let yy = iy + oy;
            if xx < 0 || yy < 0 || xx >= w as i32 || yy >= h as i32 {
                continue;
            }
            let fx = xx as f32 + 0.5 - x;
            let fy = yy as f32 + 0.5 - y;
            let d2 = fx * fx + fy * fy;
            let wgt = (-d2 * (1.8 / (r * r))).exp();
            let idx = yy as usize * w + xx as usize;
            glow[idx] = (glow[idx] + amp * wgt).min(5.0);
        }
    }
}

// Value noise
fn noise2(x: f32, y: f32, t: f32) -> f32 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;

    let xf = x - xi as f32;
    let yf = y - yi as f32;

    let a = hash2(xi, yi, t);
    let b = hash2(xi + 1, yi, t);
    let c = hash2(xi, yi + 1, t);
    let d = hash2(xi + 1, yi + 1, t);

    let u = smoothstep(xf);
    let v = smoothstep(yf);

    let ab = lerp(a, b, u);
    let cd = lerp(c, d, u);
    lerp(ab, cd, v)
}

fn smoothstep(x: f32) -> f32 {
    let x = clamp(x, 0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn hash2(x: i32, y: i32, t: f32) -> f32 {
    let tt = (t * 2.1) as i32;
    let mut n = (x as i64) * 374761393 + (y as i64) * 668265263 + (tt as i64) * 69069;
    n = (n ^ (n >> 13)) * 1274126177;
    n ^= n >> 16;
    let v = (n & 0x7fffffff) as u32;
    (v as f32) / (0x7fffffff as f32)
}

fn hash01(a: u32, b: u32, c: u32) -> f32 {
    let mut x = a.wrapping_mul(1664525).wrapping_add(1013904223);
    x ^= b.wrapping_mul(2246822519);
    x = x.rotate_left(13);
    x ^= c.wrapping_mul(3266489917);
    (x as f32) / (u32::MAX as f32)
}

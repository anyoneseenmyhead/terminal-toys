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

#[derive(Clone, Copy)]
struct Vec2 {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec2 {
    fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl Vec3 {
    fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    fn mul(self, s: f32) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
    fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn len(self) -> f32 {
        self.dot(self).sqrt()
    }
    fn norm(self) -> Vec3 {
        let l = self.len().max(1e-6);
        self.mul(1.0 / l)
    }
}

fn clampf(v: f32, a: f32, b: f32) -> f32 {
    v.max(a).min(b)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mix(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    Vec3::new(lerp(a.x, b.x, t), lerp(a.y, b.y, t), lerp(a.z, b.z, t))
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clampf((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

fn shade(uv: Vec2, time: f32, speed: f32, lane: f32, warp: f32) -> Vec3 {
    let t = time * speed;
    let lane = lane * 1.6;

    let ro = Vec3::new(lane, 1.05, -6.0 + t * 6.0);

    let mut rd = Vec3::new(uv.x * 0.85, uv.y * 0.55 - 0.08, 1.0).norm();
    rd.x += (time * 0.65).sin() * 0.03;
    rd.y += (time * 0.42).sin() * 0.02;

    let roll = (time * 0.25).sin() * 0.03;
    let (cr, sr) = (roll.cos(), roll.sin());
    let rx = rd.x * cr - rd.y * sr;
    let ry = rd.x * sr + rd.y * cr;
    rd.x = rx;
    rd.y = ry;

    let y = rd.y * 0.5 + 0.5;
    let sky_top = Vec3::new(0.08, 0.02, 0.20);
    let sky_bot = Vec3::new(0.55, 0.05, 0.25);
    let mut col = mix(sky_bot, sky_top, clampf(y, 0.0, 1.0));

    let sun_dir = Vec3::new(0.0, 0.10, 1.0).norm();
    let sun_dot = rd.dot(sun_dir).max(0.0);
    let sun = sun_dot.powf(520.0);
    let sun_glow = sun_dot.powf(14.0);
    let sun_col = Vec3::new(1.0, 0.85, 0.25);
    col = col.add(sun_col.mul(sun_glow * 0.95));
    col = mix(col, sun_col, sun * 1.25);

    // Side mountains: intersect vertical planes at +/- wall_x and clip by heightfield.
    let wall_x = 6.2;
    let mut mountain_col = Vec3::new(0.0, 0.0, 0.0);
    let mut mountain_hit = false;
    let mut t_mountain = f32::INFINITY;

    for side in [-1.0f32, 1.0f32] {
        let plane_x = side * wall_x;
        if rd.x.abs() < 1e-4 {
            continue;
        }
        let t_wall = (plane_x - ro.x) / rd.x;
        if t_wall <= 0.0 {
            continue;
        }
        let p = ro.add(rd.mul(t_wall));
        if p.z < -4.0 {
            continue;
        }

        let hz = p.z * 0.22 + time * 1.15;
        let ridge = (hz.sin() * 0.5 + 0.5).powf(2.4);
        let rid2 = ((hz * 1.7 + 1.3).sin() * 0.5 + 0.5).powf(3.0);
        let height = 0.9 + ridge * 3.2 + rid2 * 1.6;

        if p.y <= height {
            let dist = (height - p.y).max(0.0);
            let glow = smoothstep(0.08, 0.0, dist);
            let base = Vec3::new(0.05, 0.02, 0.09);
            let neon = Vec3::new(0.4, 0.9, 1.0);
            mountain_col = base.add(neon.mul(glow * 1.4));
            mountain_hit = true;
            t_mountain = t_wall;
            break;
        }
    }

    let mut floor_col = Vec3::new(0.0, 0.0, 0.0);
    let mut t_floor = f32::INFINITY;
    if rd.y < -0.001 {
        let t_hit = (0.0 - ro.y) / rd.y;
        if t_hit > 0.0 {
            t_floor = t_hit;
            let p = ro.add(rd.mul(t_hit));
            let wz = p.z
                + (p.z * 0.20 + time * 1.8).sin() * 0.45 * warp
                + (p.z * 0.06 + time * 0.9).sin() * 0.85 * warp;

            let scale = 0.55;
            let gx = (fract(p.x * scale) - 0.5).abs();
            let gz = (fract(wz * scale) - 0.5).abs();
            let thin = 0.018 + t_hit * 0.00022;
            let lx = smoothstep(thin, 0.0, gx);
            let lz = smoothstep(thin, 0.0, gz);
            let line = lx.max(lz);
            let fade = 1.0 / (1.0 + t_hit * 0.05);

            let road = smoothstep(1.4, 0.0, p.x.abs());
            let neon = Vec3::new(1.0, 0.2, 0.9);
            let mut base = neon.mul((line * 1.6 + 0.04) * fade);
            base = base.add(Vec3::new(0.2, 0.05, 0.25).mul(road * fade));
            floor_col = base;
        }
    }

    if mountain_hit && t_mountain < t_floor {
        col = mix(col, mountain_col, 0.95);
    } else if t_floor < f32::INFINITY {
        col = col.add(floor_col);
    }

    let mapped = Vec3::new(
        col.x / (col.x + 1.0),
        col.y / (col.y + 1.0),
        col.z / (col.z + 1.0),
    );
    Vec3::new(
        mapped.x.powf(0.95),
        mapped.y.powf(0.95),
        mapped.z.powf(0.95),
    )
}

fn pack_rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

fn to_rgb(c: Vec3) -> (u8, u8, u8) {
    let r = (clampf(c.x, 0.0, 1.0) * 255.0) as u8;
    let g = (clampf(c.y, 0.0, 1.0) * 255.0) as u8;
    let b = (clampf(c.z, 0.0, 1.0) * 255.0) as u8;
    (r, g, b)
}

struct Frame {
    cols: u16,
    rows: u16,
    glyphs: Vec<char>,
    fg: Vec<u32>,
    bg: Vec<u32>,
    last_glyphs: Vec<char>,
    last_fg: Vec<u32>,
    last_bg: Vec<u32>,
}

impl Frame {
    fn new(cols: u16, rows: u16) -> Self {
        let cells = (cols as usize) * (rows as usize);
        Self {
            cols,
            rows,
            glyphs: vec![' '; cells],
            fg: vec![0; cells],
            bg: vec![0; cells],
            last_glyphs: vec!['\0'; cells],
            last_fg: vec![u32::MAX; cells],
            last_bg: vec![u32::MAX; cells],
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        let cells = (cols as usize) * (rows as usize);
        self.glyphs.resize(cells, ' ');
        self.fg.resize(cells, 0);
        self.bg.resize(cells, 0);
        self.last_glyphs.resize(cells, '\0');
        self.last_fg.resize(cells, u32::MAX);
        self.last_bg.resize(cells, u32::MAX);
    }
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

fn braille_char(mask: u8) -> char {
    char::from_u32(0x2800 + mask as u32).unwrap_or(' ')
}

fn draw_diff(stdout: &mut io::Stdout, frame: &mut Frame) -> io::Result<()> {
    let cols = frame.cols as usize;
    let rows = frame.rows as usize;

    queue!(stdout, BeginSynchronizedUpdate)?;
    queue!(stdout, SetBackgroundColor(Color::Black))?;

    for y in 0..rows {
        for x in 0..cols {
            let i = y * cols + x;
            let ch = frame.glyphs[i];
            let fg = frame.fg[i];
            let bg = frame.bg[i];
            if ch == frame.last_glyphs[i] && fg == frame.last_fg[i] && bg == frame.last_bg[i] {
                continue;
            }
            frame.last_glyphs[i] = ch;
            frame.last_fg[i] = fg;
            frame.last_bg[i] = bg;

            let (fr, fgx, fb) = if fg == 0 {
                (0, 0, 0)
            } else {
                ((fg >> 16) as u8, (fg >> 8) as u8, fg as u8)
            };
            let (br, bgx, bb) = if bg == 0 {
                (0, 0, 0)
            } else {
                ((bg >> 16) as u8, (bg >> 8) as u8, bg as u8)
            };
            queue!(
                stdout,
                cursor::MoveTo(x as u16, y as u16),
                SetForegroundColor(Color::Rgb {
                    r: fr,
                    g: fgx,
                    b: fb
                }),
                SetBackgroundColor(Color::Rgb {
                    r: br,
                    g: bgx,
                    b: bb
                }),
                Print(ch)
            )?;
        }
    }

    queue!(stdout, ResetColor, EndSynchronizedUpdate)?;
    stdout.flush()?;
    Ok(())
}

fn put_text(frame: &mut Frame, x: usize, y: usize, s: &str, fg: u32, bg: u32) {
    let cols = frame.cols as usize;
    let rows = frame.rows as usize;
    if y >= rows {
        return;
    }
    let mut cx = x;
    for ch in s.chars() {
        if cx >= cols {
            break;
        }
        let i = y * cols + cx;
        frame.glyphs[i] = ch;
        frame.fg[i] = fg;
        frame.bg[i] = bg;
        cx += 1;
    }
}

fn main() -> io::Result<()> {
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
        execute!(
            out,
            ResetColor,
            Clear(ClearType::All),
            cursor::Show,
            EnableLineWrap,
            LeaveAlternateScreen
        )?;
        terminal::disable_raw_mode()?;
        Ok(())
    };

    let res = (|| -> io::Result<()> {
        let (mut cols, mut rows) = terminal::size()?;
        cols = cols.max(20);
        rows = rows.max(10);
        let mut frame = Frame::new(cols, rows);

        let mut speed = 1.0f32;
        let mut lane = 0.0f32;
        let mut warp = 0.55f32;
        let mut speed_target = speed;
        let mut lane_target = lane;
        let mut show_help = true;

        let start = Instant::now();
        let mut last = Instant::now();
        let frame_dt = Duration::from_millis(16);
        let mut sub_rgb: Vec<u32> = Vec::new();
        let mut sub_lum: Vec<f32> = Vec::new();

        loop {
            while event::poll(Duration::from_millis(0))? {
                match event::read()? {
                    Event::Resize(c, r) => {
                        cols = c.max(20);
                        rows = r.max(10);
                        frame.resize(cols, rows);
                    }
                    Event::Key(k) if k.kind != KeyEventKind::Release => match k.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('h') | KeyCode::Char('H') => show_help = !show_help,
                        KeyCode::Up => {
                            speed_target = (speed_target + 0.15).min(3.0);
                        }
                        KeyCode::Down => {
                            speed_target = (speed_target - 0.15).max(0.1);
                        }
                        KeyCode::Left => {
                            lane_target = (lane_target - 0.10).max(-1.0);
                        }
                        KeyCode::Right => {
                            lane_target = (lane_target + 0.10).min(1.0);
                        }
                        KeyCode::Char('r') | KeyCode::Char('R') => {
                            speed_target = 1.0;
                            lane_target = 0.0;
                        }
                        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Ok(());
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }

            let now = Instant::now();
            let dt = (now - last).as_secs_f32().min(0.05);
            last = now;

            let k = 0.10;
            let warp_target = clampf(0.25 + speed_target * 0.55, 0.15, 1.6);
            speed += (speed_target - speed) * k;
            lane += (lane_target - lane) * k;
            warp += (warp_target - warp) * k;

            let time = start.elapsed().as_secs_f32();
            let cols_usize = cols as usize;
            let rows_usize = rows as usize;
            let sw = cols_usize * 2;
            let sh = rows_usize * 4;
            let aspect = sw as f32 / sh as f32;

            sub_rgb.resize(sw * sh, 0);
            sub_lum.resize(sw * sh, 0.0);

            for y in 0..sh {
                for x in 0..sw {
                    let fx = (x as f32 + 0.5) / sw as f32;
                    let fy = (y as f32 + 0.5) / sh as f32;
                    let uv = Vec2::new(fx * 2.0 - 1.0, 1.0 - fy * 2.0);
                    let uv = Vec2::new(uv.x * aspect, uv.y);
                    let col = shade(uv, time, speed, lane, warp);
                    let (r, g, b) = to_rgb(col);
                    let lum = (0.2126 * col.x + 0.7152 * col.y + 0.0722 * col.z).min(1.0);
                    let i = y * sw + x;
                    sub_rgb[i] = pack_rgb(r, g, b);
                    sub_lum[i] = lum;
                }
            }

            for y in 0..rows_usize {
                for x in 0..cols_usize {
                    let i = y * cols_usize + x;
                    let mut mask: u8 = 0;
                    let mut best_lum = -1.0f32;
                    let mut best_rgb = 0u32;
                    let mut acc = Vec3::new(0.0, 0.0, 0.0);

                    for dy in 0..4 {
                        for dx in 0..2 {
                            let sx = x * 2 + dx;
                            let sy = y * 4 + dy;
                            let si = sy * sw + sx;
                            let lum = sub_lum[si];
                            let rgb = sub_rgb[si];
                            let r = ((rgb >> 16) & 0xFF) as f32 / 255.0;
                            let g = ((rgb >> 8) & 0xFF) as f32 / 255.0;
                            let b = (rgb & 0xFF) as f32 / 255.0;
                            acc = acc.add(Vec3::new(r, g, b));

                            if lum > 0.12 {
                                mask |= dot_bit(dx, dy);
                            }
                            if lum > best_lum {
                                best_lum = lum;
                                best_rgb = rgb;
                            }
                        }
                    }

                    let avg = acc.mul(1.0 / 8.0);
                    let bgc = Vec3::new(avg.x * 0.35, avg.y * 0.35, avg.z * 0.35);
                    let (bgr, bgg, bgb) = to_rgb(bgc);

                    if mask == 0 {
                        frame.glyphs[i] = ' ';
                        frame.fg[i] = 0;
                        frame.bg[i] = pack_rgb(bgr, bgg, bgb);
                    } else {
                        frame.glyphs[i] = braille_char(mask);
                        frame.fg[i] = best_rgb;
                        frame.bg[i] = pack_rgb(bgr, bgg, bgb);
                    }
                }
            }

            if show_help {
                let line1 = format!(
                    "retrowave  speed {:.2}  lane {:.2}  warp {:.2}",
                    speed, lane, warp
                );
                let line2 = "Arrows speed/lane  speed drives warp  R reset  H help  Q quit";
                put_text(
                    &mut frame,
                    0,
                    0,
                    &line1,
                    pack_rgb(210, 210, 210),
                    pack_rgb(0, 0, 0),
                );
                put_text(
                    &mut frame,
                    0,
                    1,
                    line2,
                    pack_rgb(160, 160, 160),
                    pack_rgb(0, 0, 0),
                );
            }

            draw_diff(&mut stdout, &mut frame)?;

            let elapsed = now.elapsed();
            if elapsed < frame_dt {
                std::thread::sleep(frame_dt - elapsed);
            }
        }
    })();

    let _ = cleanup();
    res
}

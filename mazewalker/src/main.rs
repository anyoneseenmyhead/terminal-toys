use std::f32::consts::PI;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};

const MAP_W: usize = 24;
const MAP_H: usize = 24;

// '#' = wall, '.' = empty
static MAP: [&str; MAP_H] = [
    "########################",
    "#......................#",
    "#.######.......######..#",
    "#.#....#.......#....#..#",
    "#.#....#.#####.#....#..#",
    "#.#....#.#...#.#....#..#",
    "#.#....###...#.#....#..#",
    "#........#...#.........#",
    "###.##.#.#...#.#.##.#..#",
    "#......................#",
    "#.#.####.########.####.#",
    "#.#....#....#.....#....#",
    "#.#....####.#.#####.####",
    "#.#...........#........#",
    "#.#############.######.#",
    "#........#.............#",
    "######...#.##########..#",
    "#........#....#........#",
    "#.######.###..#.######.#",
    "#..........#..#........#",
    "#.######...#..######...#",
    "#......#........#......#",
    "#......................#",
    "########################",
];

fn is_wall_cell(cx: i32, cy: i32) -> bool {
    if cx < 0 || cy < 0 || cx as usize >= MAP_W || cy as usize >= MAP_H {
        return true;
    }
    MAP[cy as usize].as_bytes()[cx as usize] == b'#'
}

fn is_wall_world(x: f32, y: f32) -> bool {
    is_wall_cell(x.floor() as i32, y.floor() as i32)
}

fn clamp01(x: f32) -> f32 {
    if x < 0.0 {
        0.0
    } else if x > 1.0 {
        1.0
    } else {
        x
    }
}

// Tiny RNG (no dependency)
#[derive(Clone)]
struct Rng64(u64);
impl Rng64 {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        ((x.wrapping_mul(0x2545F4914F6CDD1D)) >> 32) as u32
    }
    fn one_in(&mut self, n: u32) -> bool {
        if n == 0 {
            return false;
        }
        (self.next_u32() % n) == 0
    }
    fn f32_01(&mut self) -> f32 {
        // [0,1)
        (self.next_u32() as f32) / (u32::MAX as f32 + 1.0)
    }
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    Turning { from: f32, to: f32, t: f32 },
    Moving { from: (i32, i32), to: (i32, i32), t: f32 },
}

fn dir_to_angle(dir: i32) -> f32 {
    match dir.rem_euclid(4) {
        0 => 0.0,
        1 => PI * 0.5,
        2 => PI,
        _ => PI * 1.5,
    }
}

fn dir_step(dir: i32) -> (i32, i32) {
    match dir.rem_euclid(4) {
        0 => (1, 0),
        1 => (0, 1),
        2 => (-1, 0),
        _ => (0, -1),
    }
}

fn angle_lerp(mut a: f32, mut b: f32, t: f32) -> f32 {
    while b - a > PI {
        b -= 2.0 * PI;
    }
    while a - b > PI {
        b += 2.0 * PI;
    }
    a + (b - a) * t
}

fn cell_center(cell: (i32, i32)) -> (f32, f32) {
    (cell.0 as f32 + 0.5, cell.1 as f32 + 0.5)
}

fn dir_is_open(dir: i32, cx: i32, cy: i32) -> bool {
    let (dx, dy) = dir_step(dir);
    !is_wall_cell(cx + dx, cy + dy)
}

fn choose_turn(cur_dir: i32, cx: i32, cy: i32, rng: &mut Rng64) -> i32 {
    // Right-hand rule with occasional randomness
    let right = (cur_dir + 1).rem_euclid(4);
    let left = (cur_dir + 3).rem_euclid(4);
    let back = (cur_dir + 2).rem_euclid(4);

    let r_open = dir_is_open(right, cx, cy);
    let f_open = dir_is_open(cur_dir, cx, cy);
    let l_open = dir_is_open(left, cx, cy);

    if r_open && (f_open || l_open) && rng.one_in(10) {
        if f_open && rng.one_in(2) {
            return cur_dir;
        }
        if l_open {
            return left;
        }
    }

    if r_open {
        right
    } else if f_open {
        cur_dir
    } else if l_open {
        left
    } else {
        back
    }
}

// ---------- Braille helpers (2x4 subpixels per terminal cell) ----------

const BRAILLE_BASE: u32 = 0x2800;

// (dx,dy) bit mapping per Unicode braille dots:
//
// dx=0: dot1,2,3,7  -> bits 1,2,4,64
// dx=1: dot4,5,6,8  -> bits 8,16,32,128
//
// dy: 0..3 top->bottom
const BRAILLE_BITS: [[u8; 4]; 2] = [
    [0x01, 0x02, 0x04, 0x40], // left column
    [0x08, 0x10, 0x20, 0x80], // right column
];

// 4x4 Bayer matrix for ordered dithering (0..15)
const BAYER4: [[u8; 4]; 4] = [
    [0, 8, 2, 10],
    [12, 4, 14, 6],
    [3, 11, 1, 9],
    [15, 7, 13, 5],
];

fn dither_on(brightness: f32, px: usize, py: usize) -> bool {
    let b = clamp01(brightness);
    let t = (BAYER4[py & 3][px & 3] as f32 + 0.5) / 16.0;
    b > t
}

fn cp_to_char(cp: u32) -> char {
    // safe for our usage; fallback to space on invalid
    char::from_u32(cp).unwrap_or(' ')
}

fn main() -> io::Result<()> {
    let mut stdout = io::stdout();

    terminal::enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    let _cleanup = Cleanup;

    // Color palettes
    let fg_choices: [Color; 8] = [
        Color::White,
        Color::Cyan,
        Color::Yellow,
        Color::Green,
        Color::Magenta,
        Color::Blue,
        Color::Red,
        Color::Black,
    ];
    let bg_choices: [Color; 8] = [
        Color::Black,
        Color::DarkBlue,
        Color::DarkGreen,
        Color::DarkCyan,
        Color::DarkRed,
        Color::DarkMagenta,
        Color::DarkYellow,
        Color::DarkGrey,
    ];
    let mut fg_idx: usize = 0;
    let mut bg_idx: usize = 0;

    execute!(
        stdout,
        SetBackgroundColor(bg_choices[bg_idx]),
        SetForegroundColor(fg_choices[fg_idx]),
        Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    // Start at a known empty cell, centered
    let mut cell: (i32, i32) = (1, 1);
    if is_wall_cell(cell.0, cell.1) {
        cell = (2, 2);
    }

    let mut dir: i32 = 0; // East
    let mut ang: f32 = dir_to_angle(dir);

    let seed = Instant::now().elapsed().as_nanos() as u64 ^ 0xA5A5_5A5A_D00D_F00D;
    let mut rng = Rng64::new(seed);

    // Initialize mode
    let mut mode: Mode = if dir_is_open(dir, cell.0, cell.1) {
        let (dx, dy) = dir_step(dir);
        Mode::Moving {
            from: cell,
            to: (cell.0 + dx, cell.1 + dy),
            t: 0.0,
        }
    } else {
        let new_dir = choose_turn(dir, cell.0, cell.1, &mut rng);
        Mode::Turning {
            from: ang,
            to: dir_to_angle(new_dir),
            t: 0.0,
        }
    };

    // Timing
    let fps: u64 = 15;
    let frame_dt = Duration::from_millis(1000 / fps);
    let mut next_tick = Instant::now() + frame_dt;
    let mut last = Instant::now();

    // Buffers (codepoints per terminal cell)
    let mut prev_w: u16 = 0;
    let mut prev_h: u16 = 0;
    let mut frame_cp: Vec<u32> = Vec::new();
    let mut last_cp: Vec<u32> = Vec::new();
    let mut force_full_redraw = true;

    // Subpixel brightness buffer
    let mut sub_b: Vec<f32> = Vec::new();
    let mut sub_w: usize = 0;
    let mut sub_h: usize = 0;

    loop {
        // Input:
        // c -> cycle foreground
        // x -> cycle background
        // anything else -> exit
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        fg_idx = (fg_idx + 1) % fg_choices.len();
                        force_full_redraw = true;
                    }
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        bg_idx = (bg_idx + 1) % bg_choices.len();
                        force_full_redraw = true;
                    }
                    _ => return Ok(()),
                },
                Event::Mouse(_) => return Ok(()),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        // Pace
        let now = Instant::now();
        if now < next_tick {
            std::thread::sleep(next_tick - now);
        }
        next_tick = Instant::now() + frame_dt;

        let dt = (Instant::now() - last).as_secs_f32();
        last = Instant::now();

        // Terminal size
        let (tw, th) = terminal::size()?;
        let w = tw.max(40) as usize;
        let h = th.max(12) as usize;
        let view_h = (h.saturating_sub(1)).max(1); // last row is status

        // Resize buffers
        if tw != prev_w || th != prev_h || frame_cp.len() != w * h {
            prev_w = tw;
            prev_h = th;

            frame_cp = vec![0u32; w * h];
            last_cp = vec![0u32; w * h]; // force redraw
            force_full_redraw = true;

            // subpixels: 2x horizontally, 4x vertically for view area only
            sub_w = w * 2;
            sub_h = view_h * 4;
            sub_b = vec![0.0; sub_w * sub_h];
        } else {
            // clear current frame
            frame_cp.fill(0u32);

            // keep sub buffer sized but refresh contents
            for v in &mut sub_b {
                *v = 0.0;
            }
        }

        // Motion parameters
        let move_cells_per_sec = 1.35;
        let turn_time = 0.22;
        let turn_speed = 1.0 / turn_time;

        // Determine camera position for this frame
        let (px, py) = match mode {
            Mode::Moving { from, to, mut t } => {
                t += move_cells_per_sec * dt;

                if t >= 1.0 {
                    // Arrive at next cell
                    cell = to;

                    // Decide next action: continue or turn
                    let mut next_dir = dir;
                    if !dir_is_open(dir, cell.0, cell.1) {
                        next_dir = choose_turn(dir, cell.0, cell.1, &mut rng);
                    } else if rng.one_in(240) {
                        next_dir = choose_turn(dir, cell.0, cell.1, &mut rng);
                    }

                    if next_dir != dir {
                        let from_ang = ang;
                        dir = next_dir;
                        let to_ang = dir_to_angle(dir);
                        mode = Mode::Turning {
                            from: from_ang,
                            to: to_ang,
                            t: 0.0,
                        };
                        cell_center(cell)
                    } else {
                        let (dx, dy) = dir_step(dir);
                        mode = Mode::Moving {
                            from: cell,
                            to: (cell.0 + dx, cell.1 + dy),
                            t: 0.0,
                        };
                        cell_center(cell)
                    }
                } else {
                    mode = Mode::Moving { from, to, t };

                    let (fx, fy) = cell_center(from);
                    let (tx, ty) = cell_center(to);
                    let x = fx + (tx - fx) * t;
                    let y = fy + (ty - fy) * t;

                    if is_wall_world(x, y) {
                        cell = from;
                        mode = Mode::Turning {
                            from: ang,
                            to: dir_to_angle(choose_turn(dir, cell.0, cell.1, &mut rng)),
                            t: 0.0,
                        };
                        cell_center(cell)
                    } else {
                        (x, y)
                    }
                }
            }
            Mode::Turning { from, to, mut t } => {
                t += turn_speed * dt;
                if t >= 1.0 {
                    ang = to;

                    if dir_is_open(dir, cell.0, cell.1) {
                        let (dx, dy) = dir_step(dir);
                        mode = Mode::Moving {
                            from: cell,
                            to: (cell.0 + dx, cell.1 + dy),
                            t: 0.0,
                        };
                    } else {
                        let new_dir = choose_turn(dir, cell.0, cell.1, &mut rng);
                        let from_ang = ang;
                        dir = new_dir;
                        mode = Mode::Turning {
                            from: from_ang,
                            to: dir_to_angle(dir),
                            t: 0.0,
                        };
                    }
                    cell_center(cell)
                } else {
                    ang = angle_lerp(from, to, t);
                    mode = Mode::Turning { from, to, t };
                    cell_center(cell)
                }
            }
        };

        // -------- Build subpixel background brightness --------
        // We render into sub_b (sub_w x sub_h), then pack 2x4 into braille chars.
        let half_px = (sub_h / 2).max(1);
        for y in 0..sub_h {
            let is_ceiling = y < half_px;
            let t = if is_ceiling {
                y as f32 / half_px as f32
            } else {
                (y - half_px) as f32 / ((sub_h - half_px).max(1) as f32)
            };
            // Slight texture so the dithering has something to chew on
            for x in 0..sub_w {
                let wobble = (0.5
                    + 0.5
                        * ((x as f32 * 0.09) + (y as f32 * 0.05) + (ang * 0.6)).sin())
                    * 0.06;

                let base = if is_ceiling {
                    // darker near top, brighter toward horizon
                    0.05 + t * 0.18
                } else {
                    // brighter near horizon, darker toward bottom
                    0.22 - t * 0.14
                };

                // add a little random grain (very subtle)
                let grain = (rng.f32_01() - 0.5) * 0.03;

                sub_b[y * sub_w + x] = clamp01(base + wobble + grain);
            }
        }

        // -------- Raycast walls at subpixel horizontal resolution --------
        let fov = 70.0_f32.to_radians();
        let max_dist = 24.0_f32;
        let step_size = 0.03_f32;

        for sx in 0..sub_w {
            let cam_x = (2.0 * sx as f32 / sub_w as f32) - 1.0;
            let ray_ang = ang + cam_x * (fov / 2.0);

            let mut dist = 0.0_f32;
            while dist < max_dist {
                dist += step_size;
                let rx = px + ray_ang.cos() * dist;
                let ry = py + ray_ang.sin() * dist;
                if is_wall_world(rx, ry) {
                    break;
                }
            }

            let corrected = dist * (ray_ang - ang).cos().max(0.001);

            // Wall height in subpixels
            let wall_h_px = (sub_h as f32 / corrected).min(sub_h as f32) as i32;
            let top = (half_px as i32 - wall_h_px / 2).max(0);
            let bot = (half_px as i32 + wall_h_px / 2).min(sub_h as i32 - 1);

            // Brightness: closer walls -> brighter (more dots on)
            let mut b = 1.0 - (corrected / max_dist);
            b = clamp01(b);
            // Gentle contrast curve
            b = b * b * (3.0 - 2.0 * b);

            // Slight vertical banding like scanlines (subtle)
            let band = 0.04 * (0.5 + 0.5 * ((sx as f32) * 0.11).sin());
            let wall_b = clamp01(0.15 + b * 0.85 + band);

            for y in top..=bot {
                let idx = y as usize * sub_w + sx;
                // Wall should dominate background
                if wall_b > sub_b[idx] {
                    sub_b[idx] = wall_b;
                }
            }
        }

        // -------- Pack subpixels into braille codepoints for the view area --------
        for cy in 0..view_h {
            for cx in 0..w {
                let mut bits: u8 = 0;

                let px0 = cx * 2;
                let py0 = cy * 4;

                for dy in 0..4 {
                    for dx in 0..2 {
                        let px = px0 + dx;
                        let py = py0 + dy;
                        let bri = sub_b[py * sub_w + px];

                        if dither_on(bri, px, py) {
                            bits |= BRAILLE_BITS[dx][dy];
                        }
                    }
                }

                frame_cp[cy * w + cx] = BRAILLE_BASE + bits as u32;
            }
        }

        // Status line (ASCII on the last row)
        let status = format!(
            " mazewalker  cell:{},{} dir:{}   [c] fg  [x] bg  (other key exits)",
            cell.0,
            cell.1,
            match dir.rem_euclid(4) {
                0 => "E",
                1 => "S",
                2 => "W",
                _ => "N",
            }
        );
        let status_row = (h - 1) * w;
        for i in 0..w {
            frame_cp[status_row + i] = ' ' as u32;
        }
        for (i, ch) in status.chars().take(w).enumerate() {
            frame_cp[status_row + i] = ch as u32;
        }

        // Render
        if force_full_redraw {
            last_cp.fill(0u32);
        }

        queue!(stdout, BeginSynchronizedUpdate)?;
        queue!(
            stdout,
            SetBackgroundColor(bg_choices[bg_idx]),
            SetForegroundColor(fg_choices[fg_idx])
        )?;
        if force_full_redraw {
            queue!(stdout, Clear(ClearType::All))?;
        }

        for y in 0..h {
            let a = &frame_cp[y * w..(y + 1) * w];
            let b = &last_cp[y * w..(y + 1) * w];
            if a != b {
                let mut line = String::with_capacity(w * 3);
                for &cp in a {
                    line.push(cp_to_char(cp));
                }
                queue!(stdout, cursor::MoveTo(0, y as u16), Print(line))?;
            }
        }

        queue!(stdout, EndSynchronizedUpdate)?;
        stdout.flush()?;

        last_cp.copy_from_slice(&frame_cp);
        force_full_redraw = false;
    }
}

struct Cleanup;
impl Drop for Cleanup {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, ResetColor, cursor::Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

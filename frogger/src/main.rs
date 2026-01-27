// src/main.rs
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::cmp::{max, min};
use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

const HUD_ROWS: u16 = 2;
const FOOTER_ROWS: u16 = 1;

const LANES: usize = 13; // classic-ish: homes + 5 water + safe + 5 road + start
const HOMES: usize = 5;

const FIXED_DT: f32 = 1.0 / 60.0;
const MAX_FRAME_DT: f32 = 1.0 / 20.0; // clamp if system hiccups
const WRAP_PAD: f32 = 8.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum LaneKind {
    Homes,
    Water,
    Safe,
    Road,
    Start,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ObjKind {
    Car,
    Log,
    Turtle,
}

#[derive(Clone)]
struct Obj {
    kind: ObjKind,
    x: f32,   // px
    len: i32, // px
    vx: f32,  // px/sec
    // turtle timing
    phase: f32,
    period: f32,
    duty: f32, // fraction visible
}

#[derive(Clone)]
struct Lane {
    kind: LaneKind,
    y_lane: i32, // lane index 0..LANES-1 (top to bottom)
    vx: f32,
    objs: Vec<Obj>,
    // lane decoration seed
    deco: u32,
}

#[derive(Clone, Copy)]
struct Theme {
    name: &'static str,
    hud_fg: Color,
    hud_bg: Color,

    safe_bg: Color,
    road_bg: Color,
    water_bg: Color,
    start_bg: Color,
    homes_bg: Color,

    frog_fg: Color,
    car_fg: Color,
    log_fg: Color,
    turtle_fg: Color,
    home_fg: Color,

    accent_fg: Color,
}

fn themes() -> Vec<Theme> {
    vec![
        Theme {
            name: "Mint CRT",
            hud_fg: Color::Rgb {
                r: 160,
                g: 255,
                b: 210,
            },
            hud_bg: Color::Rgb { r: 5, g: 7, b: 10 },
            safe_bg: Color::Rgb {
                r: 10,
                g: 16,
                b: 10,
            },
            road_bg: Color::Rgb {
                r: 10,
                g: 10,
                b: 14,
            },
            water_bg: Color::Rgb { r: 7, g: 12, b: 20 },
            start_bg: Color::Rgb {
                r: 10,
                g: 16,
                b: 10,
            },
            homes_bg: Color::Rgb { r: 7, g: 14, b: 12 },
            frog_fg: Color::Rgb {
                r: 180,
                g: 255,
                b: 120,
            },
            car_fg: Color::Rgb {
                r: 255,
                g: 160,
                b: 140,
            },
            log_fg: Color::Rgb {
                r: 210,
                g: 190,
                b: 140,
            },
            turtle_fg: Color::Rgb {
                r: 160,
                g: 210,
                b: 255,
            },
            home_fg: Color::Rgb {
                r: 240,
                g: 240,
                b: 240,
            },
            accent_fg: Color::Rgb {
                r: 255,
                g: 220,
                b: 140,
            },
        },
        Theme {
            name: "Amber Terminal",
            hud_fg: Color::Rgb {
                r: 255,
                g: 190,
                b: 95,
            },
            hud_bg: Color::Rgb { r: 7, g: 6, b: 3 },
            safe_bg: Color::Rgb { r: 16, g: 12, b: 6 },
            road_bg: Color::Rgb { r: 12, g: 10, b: 8 },
            water_bg: Color::Rgb { r: 8, g: 8, b: 14 },
            start_bg: Color::Rgb { r: 16, g: 12, b: 6 },
            homes_bg: Color::Rgb {
                r: 10,
                g: 10,
                b: 10,
            },
            frog_fg: Color::Rgb {
                r: 255,
                g: 220,
                b: 120,
            },
            car_fg: Color::Rgb {
                r: 255,
                g: 140,
                b: 80,
            },
            log_fg: Color::Rgb {
                r: 220,
                g: 180,
                b: 120,
            },
            turtle_fg: Color::Rgb {
                r: 160,
                g: 200,
                b: 255,
            },
            home_fg: Color::Rgb {
                r: 240,
                g: 240,
                b: 240,
            },
            accent_fg: Color::Rgb {
                r: 255,
                g: 235,
                b: 160,
            },
        },
        Theme {
            name: "JoshNet Purple",
            hud_fg: Color::Rgb {
                r: 200,
                g: 150,
                b: 255,
            },
            hud_bg: Color::Rgb { r: 10, g: 5, b: 15 },
            safe_bg: Color::Rgb {
                r: 14,
                g: 10,
                b: 22,
            },
            road_bg: Color::Rgb { r: 12, g: 8, b: 18 },
            water_bg: Color::Rgb { r: 10, g: 8, b: 22 },
            start_bg: Color::Rgb {
                r: 14,
                g: 10,
                b: 22,
            },
            homes_bg: Color::Rgb {
                r: 10,
                g: 12,
                b: 18,
            },
            frog_fg: Color::Rgb {
                r: 200,
                g: 255,
                b: 180,
            },
            car_fg: Color::Rgb {
                r: 255,
                g: 160,
                b: 220,
            },
            log_fg: Color::Rgb {
                r: 220,
                g: 200,
                b: 150,
            },
            turtle_fg: Color::Rgb {
                r: 160,
                g: 210,
                b: 255,
            },
            home_fg: Color::Rgb {
                r: 240,
                g: 240,
                b: 240,
            },
            accent_fg: Color::Rgb {
                r: 255,
                g: 220,
                b: 140,
            },
        },
    ]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mat {
    Empty = 0,
    Mark = 1,
    Log = 2,
    Turtle = 3,
    Car = 4,
    Home = 5,
    Frog = 6,
}

fn mat_priority(m: Mat) -> u8 {
    match m {
        Mat::Empty => 0,
        Mat::Mark => 1,
        Mat::Log => 2,
        Mat::Turtle => 3,
        Mat::Car => 4,
        Mat::Home => 5,
        Mat::Frog => 6,
    }
}

#[derive(Clone, Copy)]
struct FrameCell {
    ch: char,
    fg: Color,
    bg: Color,
}

struct Renderer {
    w: u16,
    h: u16,
    prev: Vec<FrameCell>,
    cur: Vec<FrameCell>,
}

impl Renderer {
    fn new(w: u16, h: u16) -> Self {
        let blank = FrameCell {
            ch: ' ',
            fg: Color::White,
            bg: Color::Black,
        };
        Self {
            w,
            h,
            prev: vec![blank; (w as usize) * (h as usize)],
            cur: vec![blank; (w as usize) * (h as usize)],
        }
    }

    fn resize(&mut self, w: u16, h: u16) {
        self.w = w;
        self.h = h;
        let blank = FrameCell {
            ch: ' ',
            fg: Color::White,
            bg: Color::Black,
        };
        self.prev = vec![blank; (w as usize) * (h as usize)];
        self.cur = vec![blank; (w as usize) * (h as usize)];
    }

    fn clear_to(&mut self, fg: Color, bg: Color) {
        for c in &mut self.cur {
            c.ch = ' ';
            c.fg = fg;
            c.bg = bg;
        }
    }

    fn put(&mut self, x: u16, y: u16, ch: char, fg: Color, bg: Color) {
        if x >= self.w || y >= self.h {
            return;
        }
        let i = (y as usize) * (self.w as usize) + (x as usize);
        self.cur[i] = FrameCell { ch, fg, bg };
    }

    fn put_str(&mut self, x: u16, y: u16, s: &str, fg: Color, bg: Color) {
        let mut xx = x;
        for ch in s.chars() {
            if xx >= self.w {
                break;
            }
            self.put(xx, y, ch, fg, bg);
            xx += 1;
        }
    }

    fn flush_diff(&mut self, out: &mut Stdout) -> io::Result<()> {
        queue!(out, BeginSynchronizedUpdate)?;
        let mut cur_fg = None::<Color>;
        let mut cur_bg = None::<Color>;

        for y in 0..self.h {
            let row_off = (y as usize) * (self.w as usize);
            for x in 0..self.w {
                let i = row_off + (x as usize);
                let a = self.cur[i];
                let b = self.prev[i];
                if a.ch == b.ch && a.fg == b.fg && a.bg == b.bg {
                    continue;
                }
                queue!(out, cursor::MoveTo(x, y))?;
                if cur_fg != Some(a.fg) {
                    queue!(out, SetForegroundColor(a.fg))?;
                    cur_fg = Some(a.fg);
                }
                if cur_bg != Some(a.bg) {
                    queue!(out, SetBackgroundColor(a.bg))?;
                    cur_bg = Some(a.bg);
                }
                queue!(out, Print(a.ch))?;
            }
        }

        queue!(out, ResetColor, EndSynchronizedUpdate)?;
        out.flush()?;
        self.prev.copy_from_slice(&self.cur);
        Ok(())
    }
}

struct Viewport {
    term_w: u16,
    term_h: u16,

    play_x: u16, // terminal cells
    play_y: u16, // terminal cells
    play_w: u16, // terminal cells (braille cells)
    play_h: u16, // terminal cells (braille cells)

    px_w: i32, // subpixels (2 per cell)
    px_h: i32, // subpixels (4 per cell)

    lane_top: i32, // lane index offset inside play_h (in braille rows)
}

fn fit_view(term_w: u16, term_h: u16) -> Option<Viewport> {
    if term_h <= HUD_ROWS + FOOTER_ROWS + 6 {
        return None;
    }
    if term_w < 40 {
        return None;
    }

    let usable_h = term_h - HUD_ROWS - FOOTER_ROWS;
    let play_h = usable_h;
    let play_w = term_w;

    let px_w = (play_w as i32) * 2;
    let px_h = (play_h as i32) * 4;

    let lane_top = ((play_h as i32) - (LANES as i32)) / 2;

    Some(Viewport {
        term_w,
        term_h,
        play_x: 0,
        play_y: HUD_ROWS,
        play_w,
        play_h,
        px_w,
        px_h,
        lane_top,
    })
}

fn lane_of_pixel(v: &Viewport, py: i32) -> Option<i32> {
    if py < 0 || py >= v.px_h {
        return None;
    }
    let by = py / 4;
    let lane = by - v.lane_top;
    if lane < 0 || lane >= LANES as i32 {
        None
    } else {
        Some(lane)
    }
}

fn lane_kind_by_index(lane: i32) -> LaneKind {
    // 0..12 top to bottom
    match lane {
        0 => LaneKind::Homes,
        1..=5 => LaneKind::Water,
        6 => LaneKind::Safe,
        7..=11 => LaneKind::Road,
        _ => LaneKind::Start, // 12
    }
}

fn bg_for_lane(theme: Theme, kind: LaneKind) -> Color {
    match kind {
        LaneKind::Homes => theme.homes_bg,
        LaneKind::Water => theme.water_bg,
        LaneKind::Safe => theme.safe_bg,
        LaneKind::Road => theme.road_bg,
        LaneKind::Start => theme.start_bg,
    }
}

fn fg_for_mat(theme: Theme, m: Mat) -> Color {
    match m {
        Mat::Frog => theme.frog_fg,
        Mat::Car => theme.car_fg,
        Mat::Log => theme.log_fg,
        Mat::Turtle => theme.turtle_fg,
        Mat::Home => theme.home_fg,
        Mat::Mark => theme.accent_fg,
        Mat::Empty => theme.hud_fg,
    }
}

struct Game {
    score: i32,
    lives: i32,
    level: i32,
    paused: bool,
    game_over: bool,

    homes: [bool; HOMES],
    frog_x: f32, // px
    frog_y: f32, // px
    frog_vx_carry: f32,

    time_max: f32,
    time_left: f32,

    best_lane: i32, // smallest lane reached this life
    lanes: Vec<Lane>,

    rng: StdRng,
    theme_idx: usize,
}

impl Game {
    fn new(seed: u64) -> Self {
        let mut g = Self {
            score: 0,
            lives: 5,
            level: 1,
            paused: false,
            game_over: false,
            homes: [false; HOMES],
            frog_x: 0.0,
            frog_y: 0.0,
            frog_vx_carry: 0.0,
            time_max: 22.0,
            time_left: 22.0,
            best_lane: LANES as i32 - 1,
            lanes: vec![],
            rng: StdRng::seed_from_u64(seed),
            theme_idx: 0,
        };
        g
    }

    fn reset_run(&mut self, v: &Viewport) {
        self.score = 0;
        self.lives = 5;
        self.level = 1;
        self.paused = false;
        self.game_over = false;
        self.homes = [false; HOMES];
        self.time_left = self.time_max;
        self.build_level(v);
        self.reset_frog(v);
    }

    fn next_level(&mut self, v: &Viewport) {
        self.level += 1;
        self.homes = [false; HOMES];
        self.score += 250;
        self.time_max = (self.time_max * 0.98).max(14.0);
        self.time_left = self.time_max;
        self.build_level(v);
        self.reset_frog(v);
    }

    fn reset_frog(&mut self, v: &Viewport) {
        // start lane is bottom lane (LANES-1)
        let start_lane = (LANES as i32) - 1;
        let by = (v.lane_top + start_lane) as f32;
        self.frog_y = (by * 4.0) + 0.6; // px
        self.frog_x = (v.px_w as f32) * 0.5 - 1.0;
        self.frog_vx_carry = 0.0;
        self.time_left = self.time_max;
        self.best_lane = start_lane;
    }

    fn build_level(&mut self, v: &Viewport) {
        let w = v.px_w.max(40);
        let level = self.level;

        let mut lanes: Vec<Lane> = vec![];
        for li in 0..LANES as i32 {
            let kind = lane_kind_by_index(li);
            let deco = self.rng.gen::<u32>();

            let mut lane = Lane {
                kind,
                y_lane: li,
                vx: 0.0,
                objs: vec![],
                deco,
            };

            match kind {
                LaneKind::Road => {
                    // 5 road lanes: alternate direction, varied speed
                    let lane_idx = li - 7; // 0..4
                    let dir = if lane_idx % 2 == 0 { 1.0 } else { -1.0 };
                    let base = 32.0 + (lane_idx as f32) * 7.0;
                    let sp = base * 0.55 * (1.0 + (level as f32 - 1.0) * 0.04);
                    lane.vx = dir * sp;

                    lane.objs =
                        gen_objects(&mut self.rng, w, ObjKind::Car, lane.vx, 8, 18, 18, 40, 24);
                }
                LaneKind::Water => {
                    // 5 water lanes: mix logs and turtles, alternate direction
                    let lane_idx = li - 1; // 0..4
                    let dir = if lane_idx % 2 == 0 { -1.0 } else { 1.0 };
                    let base = 18.0 + (lane_idx as f32) * 5.0;
                    let sp = base * (1.0 + (level as f32 - 1.0) * 0.06);
                    lane.vx = dir * sp;

                    let use_turtles = lane_idx == 1 || lane_idx == 3;
                    if use_turtles {
                        lane.objs = gen_turtles(&mut self.rng, w, lane.vx, 6, 12, 12, 24);
                    } else {
                        lane.objs = gen_objects(
                            &mut self.rng,
                            w,
                            ObjKind::Log,
                            lane.vx,
                            10,
                            24,
                            14,
                            32,
                            20,
                        );
                    }
                }
                _ => {}
            }

            lanes.push(lane);
        }

        self.lanes = lanes;
    }

    fn lose_life(&mut self, v: &Viewport) {
        self.lives -= 1;
        if self.lives <= 0 {
            self.game_over = true;
        } else {
            self.reset_frog(v);
        }
    }

    fn all_homes_filled(&self) -> bool {
        self.homes.iter().all(|&b| b)
    }

    fn try_claim_home(&mut self, v: &Viewport) -> bool {
        // frog must be on homes lane
        let lane = match lane_of_pixel(v, self.frog_y as i32) {
            Some(l) => l,
            None => return false,
        };
        if lane != 0 {
            return false;
        }

        let slot_w = (v.px_w as f32) / (HOMES as f32);
        let slot = ((self.frog_x + 1.0) / slot_w).floor() as i32;
        if slot < 0 || slot >= HOMES as i32 {
            self.lose_life(v);
            return true;
        }
        let s = slot as usize;
        if self.homes[s] {
            self.lose_life(v);
            return true;
        }

        // require frog near top of lane (avoid claiming from below)
        let y_in_lane = (self.frog_y as i32) % 4;
        if y_in_lane > 2 {
            return false;
        }

        self.homes[s] = true;
        self.score += 50;
        if self.all_homes_filled() {
            self.next_level(v);
        } else {
            self.reset_frog(v);
        }
        true
    }

    fn score_progress(&mut self, v: &Viewport) {
        if let Some(lane) = lane_of_pixel(v, self.frog_y as i32) {
            if lane < self.best_lane {
                self.best_lane = lane;
                self.score += 10;
            }
        }
    }

    fn step(&mut self, v: &Viewport, t: f32, dt: f32) {
        if self.paused || self.game_over {
            return;
        }

        self.time_left -= dt;
        if self.time_left <= 0.0 {
            self.lose_life(v);
            return;
        }

        // move lane objects
        for lane in &mut self.lanes {
            if lane.kind == LaneKind::Road || lane.kind == LaneKind::Water {
                for o in &mut lane.objs {
                    o.x += o.vx * dt;
                    let wrap = (v.px_w as f32) + (o.len as f32) + WRAP_PAD;
                    if o.vx > 0.0 {
                        if o.x > wrap {
                            o.x -= wrap;
                        }
                    } else {
                        if o.x < -wrap {
                            o.x += wrap;
                        }
                    }

                    if o.kind == ObjKind::Turtle {
                        // animate phase
                        o.phase += dt;
                        if o.phase > o.period {
                            o.phase -= o.period;
                        }
                    }
                }
            }
        }

        // carry frog on floating object (computed each frame)
        self.frog_x += self.frog_vx_carry * dt;

        // clamp horizontal bounds
        if self.frog_x < 0.0 || self.frog_x > (v.px_w as f32 - 2.0) {
            self.lose_life(v);
            return;
        }

        // win checks (homes)
        if self.try_claim_home(v) {
            return;
        }

        // collisions and drowning
        self.frog_vx_carry = 0.0;

        let lane = match lane_of_pixel(v, self.frog_y as i32) {
            Some(l) => l,
            None => {
                self.lose_life(v);
                return;
            }
        };
        let kind = lane_kind_by_index(lane);

        match kind {
            LaneKind::Road => {
                if self.frog_hits_car(v, lane) {
                    self.lose_life(v);
                    return;
                }
            }
            LaneKind::Water => {
                let (on_float, carry) = self.frog_on_float(v, lane, t);
                if !on_float {
                    self.lose_life(v);
                    return;
                }
                self.frog_vx_carry = carry;
            }
            _ => {}
        }
    }

    fn frog_rect(&self) -> (f32, f32, f32, f32) {
        // frog in pixels: 2x3
        (self.frog_x, self.frog_y, 2.0, 3.0)
    }

    fn frog_hits_car(&self, v: &Viewport, lane: i32) -> bool {
        let (fx, fy, fw, fh) = self.frog_rect();
        let lane_y0 = ((v.lane_top + lane) * 4) as f32;
        let lane_y1 = lane_y0 + 4.0;

        if fy + fh < lane_y0 || fy > lane_y1 {
            return false;
        }

        let lane_ref = &self.lanes[lane as usize];
        for o in &lane_ref.objs {
            if o.kind != ObjKind::Car {
                continue;
            }
            let w = v.px_w as f32;
            let len = o.len as f32;
            for cx in wrap_positions(o.x, len, w) {
                let ox0 = (cx as i32) as f32;
                if !overlaps_screen(ox0, len, w) {
                    continue;
                }
                if aabb(fx, fy, fw, fh, ox0, lane_y0 + 0.5, len, 3.0) {
                    return true;
                }
            }
        }
        false
    }

    fn frog_on_float(&self, v: &Viewport, lane: i32, t: f32) -> (bool, f32) {
        let (fx, fy, fw, fh) = self.frog_rect();
        let lane_y0 = ((v.lane_top + lane) * 4) as f32;

        let lane_ref = &self.lanes[lane as usize];
        for o in &lane_ref.objs {
            let (ok, carry) = match o.kind {
                ObjKind::Log => (true, o.vx),
                ObjKind::Turtle => {
                    let visible = (o.phase / o.period) < o.duty;
                    if visible {
                        (true, o.vx)
                    } else {
                        (false, 0.0)
                    }
                }
                _ => (false, 0.0),
            };
            if !ok {
                continue;
            }

            let w = v.px_w as f32;
            let len = o.len as f32;
            for cx in wrap_positions(o.x, len, w) {
                let ox0 = (cx as i32) as f32;
                if !overlaps_screen(ox0, len, w) {
                    continue;
                }
                if aabb(fx, fy, fw, fh, ox0, lane_y0 + 1.0, len, 2.0) {
                    return (true, carry);
                }
            }
        }

        // subtle mercy: allow pixel-perfect edge on safe water border for one frame
        let _ = t;
        (false, 0.0)
    }

    fn move_frog(&mut self, v: &Viewport, dx: i32, dy: i32) {
        if self.paused || self.game_over {
            return;
        }
        // discrete tile move: 2px horizontally, 4px vertically
        self.frog_x += dx as f32;
        self.frog_y += dy as f32;

        self.frog_x = self.frog_x.clamp(0.0, (v.px_w as f32) - 2.0);
        self.frog_y = self.frog_y.clamp(0.0, (v.px_h as f32) - 3.0);

        self.score_progress(v);
    }
}

fn aabb(ax: f32, ay: f32, aw: f32, ah: f32, bx: f32, by: f32, bw: f32, bh: f32) -> bool {
    ax < bx + bw && ax + aw > bx && ay < by + bh && ay + ah > by
}

fn wrap_positions(x: f32, len: f32, w: f32) -> [f32; 3] {
    let wrap = w + len + WRAP_PAD;
    [x, x + wrap, x - wrap]
}

fn overlaps_screen(x: f32, len: f32, w: f32) -> bool {
    x < w && x + len > 0.0
}

fn gen_objects(
    rng: &mut StdRng,
    width_px: i32,
    kind: ObjKind,
    vx: f32,
    len_min: i32,
    len_max: i32,
    gap_min: i32,
    gap_max: i32,
    max_objs: usize,
) -> Vec<Obj> {
    let mut objs = vec![];
    let mut x = rng.gen_range(0.0..(width_px as f32));
    let mut safety = 0;
    while (objs.len() < max_objs) && safety < 1000 && x < (width_px as f32 + 120.0) {
        safety += 1;
        let len = rng.gen_range(len_min..=len_max);
        objs.push(Obj {
            kind,
            x,
            len,
            vx,
            phase: rng.gen_range(0.0..1.0),
            period: 1.0,
            duty: 1.0,
        });
        let gap = rng.gen_range(gap_min..=gap_max);
        x += (len + gap) as f32;
    }
    // normalize positions into [-len..width+len] for cleaner wrapping
    let w = width_px as f32;
    for o in &mut objs {
        while o.x > w {
            o.x -= w;
        }
        while o.x < -w {
            o.x += w;
        }
    }
    objs
}

fn gen_turtles(
    rng: &mut StdRng,
    width_px: i32,
    vx: f32,
    len_min: i32,
    len_max: i32,
    gap_min: i32,
    gap_max: i32,
) -> Vec<Obj> {
    let mut objs = gen_objects(
        rng,
        width_px,
        ObjKind::Turtle,
        vx,
        len_min,
        len_max,
        gap_min,
        gap_max,
        20,
    );
    for o in &mut objs {
        o.period = rng.gen_range(2.2..4.2);
        o.duty = rng.gen_range(0.45..0.72);
        o.phase = rng.gen_range(0.0..o.period);
    }
    objs
}

fn braille_char(dots: u8) -> char {
    // Unicode braille: U+2800 + dots (bits)
    char::from_u32(0x2800 + dots as u32).unwrap_or(' ')
}

fn braille_bit(dx: i32, dy: i32) -> u8 {
    // dx in {0,1}, dy in {0,1,2,3}
    // dot map:
    // (0,0)=1 (0,1)=2 (0,2)=4 (1,0)=8 (1,1)=16 (1,2)=32 (0,3)=64 (1,3)=128
    match (dx, dy) {
        (0, 0) => 1,
        (0, 1) => 2,
        (0, 2) => 4,
        (1, 0) => 8,
        (1, 1) => 16,
        (1, 2) => 32,
        (0, 3) => 64,
        (1, 3) => 128,
        _ => 0,
    }
}

fn draw_rect(buf: &mut [Mat], w: i32, h: i32, x0: i32, y0: i32, rw: i32, rh: i32, m: Mat) {
    let x1 = x0 + rw;
    let y1 = y0 + rh;
    for y in max(0, y0)..min(h, y1) {
        let row = y * w;
        for x in max(0, x0)..min(w, x1) {
            let i = (row + x) as usize;
            if mat_priority(m) >= mat_priority(buf[i]) {
                buf[i] = m;
            }
        }
    }
}

fn draw_frog(buf: &mut [Mat], w: i32, h: i32, x: i32, y: i32) {
    // 2x3 sprite with a little notch
    let pts = [(0, 0), (1, 0), (0, 1), (1, 1), (0, 2)];
    for (dx, dy) in pts {
        let xx = x + dx;
        let yy = y + dy;
        if xx >= 0 && xx < w && yy >= 0 && yy < h {
            let i = (yy * w + xx) as usize;
            buf[i] = Mat::Frog;
        }
    }
}

fn draw_car(buf: &mut [Mat], w: i32, h: i32, x: i32, y: i32, len: i32) {
    // chunky 3px tall car with small cutouts
    draw_rect(buf, w, h, x, y, len, 3, Mat::Car);
    // windshield hole
    if len >= 10 {
        draw_rect(buf, w, h, x + 2, y + 1, 2, 1, Mat::Empty);
        draw_rect(buf, w, h, x + len - 4, y + 1, 2, 1, Mat::Empty);
    }
}

fn draw_log(buf: &mut [Mat], w: i32, h: i32, x: i32, y: i32, len: i32) {
    draw_rect(buf, w, h, x, y, len, 2, Mat::Log);
    // notch ends
    draw_rect(buf, w, h, x, y, 1, 1, Mat::Empty);
    draw_rect(buf, w, h, x + len - 1, y + 1, 1, 1, Mat::Empty);
}

fn draw_turtle(buf: &mut [Mat], w: i32, h: i32, x: i32, y: i32, len: i32, visible: bool) {
    if !visible {
        // faint bubbles
        for k in 0..max(1, len / 5) {
            let xx = x + 2 + k * 5;
            draw_rect(buf, w, h, xx, y, 1, 1, Mat::Mark);
        }
        return;
    }
    draw_rect(buf, w, h, x, y, len, 2, Mat::Turtle);
    // shell bumps
    for k in 0..max(1, len / 4) {
        let xx = x + 1 + k * 4;
        draw_rect(buf, w, h, xx, y, 1, 1, Mat::Empty);
    }
}

fn render_playfield(r: &mut Renderer, v: &Viewport, g: &Game, now_s: f32) {
    let theme = themes()[g.theme_idx % themes().len()];

    // clear full screen to HUD bg first
    r.clear_to(theme.hud_fg, theme.hud_bg);

    // HUD
    let filled = g.homes.iter().filter(|&&b| b).count();
    let time_bar_w = 22usize;
    let t01 = (g.time_left / g.time_max).clamp(0.0, 1.0);
    let filled_w = (t01 * time_bar_w as f32).round() as usize;
    let bar = format!(
        "[{}{}]",
        "█".repeat(filled_w),
        " ".repeat(time_bar_w - filled_w)
    );

    let line1 = format!(
        "FROGGER  |  Score {:06}  Lives {}  Level {}  Homes {}/{}",
        g.score,
        "♥".repeat(max(0, g.lives) as usize),
        g.level,
        filled,
        HOMES
    );
    let line2 = if g.game_over {
        "GAME OVER  |  R restart   Q quit   T theme".to_string()
    } else if g.paused {
        format!("PAUSED  |  Space resume   T theme   {}", bar)
    } else {
        format!(
            "Arrows/WASD move   Space pause   R restart   Q quit   T theme   {}",
            bar
        )
    };

    // clear HUD rows
    for y in 0..HUD_ROWS {
        r.put_str(
            0,
            y,
            &" ".repeat(v.term_w as usize),
            theme.hud_fg,
            theme.hud_bg,
        );
    }
    r.put_str(0, 0, &line1, theme.hud_fg, theme.hud_bg);
    r.put_str(0, 1, &line2, theme.hud_fg, theme.hud_bg);

    // footer
    let footer_y = v.term_h.saturating_sub(1);
    let footer = format!(
        "Theme: {}   (Terminal: {}x{})",
        theme.name, v.term_w, v.term_h
    );
    r.put_str(
        0,
        footer_y,
        &" ".repeat(v.term_w as usize),
        theme.hud_fg,
        theme.hud_bg,
    );
    r.put_str(0, footer_y, &footer, theme.hud_fg, theme.hud_bg);

    // build material buffer at subpixel resolution
    let mut buf = vec![Mat::Empty; (v.px_w as usize) * (v.px_h as usize)];

    // background lane markings and water ripples
    for py in 0..v.px_h {
        let lane = lane_of_pixel(v, py).unwrap_or(-999);
        let kind = if lane >= 0 {
            lane_kind_by_index(lane)
        } else {
            LaneKind::Safe
        };

        match kind {
            LaneKind::Road => {
                // dashed center line within the 4px band
                let y_in = py % 4;
                if y_in == 2 {
                    for x in (0..v.px_w).step_by(10) {
                        if ((x / 10 + (lane as i32)) % 2) == 0 {
                            let i = (py * v.px_w + x) as usize;
                            buf[i] = Mat::Mark;
                            if x + 1 < v.px_w {
                                buf[(py * v.px_w + x + 1) as usize] = Mat::Mark;
                            }
                        }
                    }
                }
            }
            LaneKind::Water => {
                // ripples: sparse marks moving with time
                let y_in = py % 4;
                if y_in == 0 || y_in == 3 {
                    let phase = (now_s * 2.2 + lane as f32 * 0.9) as f32;
                    for x in (0..v.px_w).step_by(13) {
                        let fx = x as f32 * 0.08 + phase;
                        let on = (fx.sin() * 0.5 + 0.5) > 0.72;
                        if on {
                            let i = (py * v.px_w + x) as usize;
                            buf[i] = Mat::Mark;
                        }
                    }
                }
            }
            LaneKind::Safe | LaneKind::Start | LaneKind::Homes => {
                // speckled grass or dock
                let y_in = py % 4;
                if y_in == 1 {
                    for x in (0..v.px_w).step_by(17) {
                        let i = (py * v.px_w + x) as usize;
                        buf[i] = Mat::Mark;
                    }
                }
            }
        }
    }

    // homes slots at lane 0
    {
        let lane0_by = v.lane_top + 0;
        let y0 = lane0_by * 4;
        if lane0_by >= 0 && lane0_by < v.play_h as i32 {
            let slot_w = v.px_w / HOMES as i32;
            for s in 0..HOMES {
                let cx = s as i32 * slot_w + slot_w / 2;
                let x0 = cx - 2;
                let y = y0 + 1;
                if g.homes[s] {
                    draw_rect(&mut buf, v.px_w, v.px_h, x0, y, 4, 2, Mat::Home);
                } else {
                    // outline
                    draw_rect(&mut buf, v.px_w, v.px_h, x0, y, 4, 1, Mat::Mark);
                    draw_rect(&mut buf, v.px_w, v.px_h, x0, y + 1, 1, 1, Mat::Mark);
                    draw_rect(&mut buf, v.px_w, v.px_h, x0 + 3, y + 1, 1, 1, Mat::Mark);
                }
            }
        }
    }

    // draw lane objects
    for lane in &g.lanes {
        if lane.kind != LaneKind::Road && lane.kind != LaneKind::Water {
            continue;
        }
        let by = v.lane_top + lane.y_lane;
        let y0 = by * 4;
        if y0 < 0 || y0 + 3 >= v.px_h {
            continue;
        }

        for o in &lane.objs {
            let len = o.len;
            let w = v.px_w as f32;
            let len_f = len as f32;
            for cx in wrap_positions(o.x, len_f, w) {
                let xx = cx as i32;
                let x0 = xx as f32;
                if !overlaps_screen(x0, len_f, w) {
                    continue;
                }
                match o.kind {
                    ObjKind::Car => draw_car(&mut buf, v.px_w, v.px_h, xx, y0 + 1, len),
                    ObjKind::Log => draw_log(&mut buf, v.px_w, v.px_h, xx, y0 + 1, len),
                    ObjKind::Turtle => {
                        let visible = (o.phase / o.period) < o.duty;
                        draw_turtle(&mut buf, v.px_w, v.px_h, xx, y0 + 1, len, visible);
                    }
                }
            }
        }
    }

    // draw frog
    draw_frog(
        &mut buf,
        v.px_w,
        v.px_h,
        g.frog_x.round() as i32,
        g.frog_y.round() as i32,
    );

    // pack into braille cells and write into renderer
    for by in 0..(v.play_h as i32) {
        let term_y = v.play_y as i32 + by;
        if term_y < 0 || term_y >= v.term_h as i32 {
            continue;
        }

        let lane_idx = by - v.lane_top;
        let lane_kind = if lane_idx >= 0 && lane_idx < LANES as i32 {
            lane_kind_by_index(lane_idx)
        } else {
            LaneKind::Safe
        };
        let bg = bg_for_lane(theme, lane_kind);

        for bx in 0..(v.play_w as i32) {
            let term_x = v.play_x as i32 + bx;
            if term_x < 0 || term_x >= v.term_w as i32 {
                continue;
            }

            let px0 = bx * 2;
            let py0 = by * 4;

            let mut dots: u8 = 0;
            let mut best = Mat::Empty;

            for dy in 0..4 {
                for dx in 0..2 {
                    let px = px0 + dx;
                    let py = py0 + dy;
                    if px < 0 || px >= v.px_w || py < 0 || py >= v.px_h {
                        continue;
                    }
                    let m = buf[(py * v.px_w + px) as usize];
                    if m != Mat::Empty {
                        dots |= braille_bit(dx, dy);
                        if mat_priority(m) > mat_priority(best) {
                            best = m;
                        }
                    }
                }
            }

            let ch = if dots == 0 { ' ' } else { braille_char(dots) };
            let fg = if best == Mat::Empty {
                theme.hud_fg
            } else {
                fg_for_mat(theme, best)
            };

            r.put(term_x as u16, term_y as u16, ch, fg, bg);
        }
    }
}

fn handle_resize(r: &mut Renderer, v: &mut Viewport, g: &mut Game, new_w: u16, new_h: u16) -> bool {
    if new_w == v.term_w && new_h == v.term_h {
        return false;
    }
    if let Some(nv) = fit_view(new_w, new_h) {
        r.resize(new_w, new_h);
        *v = nv;
        // keep score, but rebuild level geometry and reset frog position
        g.build_level(v);
        g.reset_frog(v);
        true
    } else {
        r.resize(new_w, new_h);
        v.term_w = new_w;
        v.term_h = new_h;
        true
    }
}

fn main() -> io::Result<()> {
    let mut out = io::stdout();

    terminal::enable_raw_mode()?;
    execute!(
        out,
        EnterAlternateScreen,
        cursor::Hide,
        DisableLineWrap,
        terminal::Clear(terminal::ClearType::All)
    )?;

    let res = run(&mut out);

    // restore
    let _ = execute!(
        out,
        EnableLineWrap,
        cursor::Show,
        LeaveAlternateScreen,
        ResetColor
    );
    let _ = terminal::disable_raw_mode();

    res
}

fn run(out: &mut Stdout) -> io::Result<()> {
    let (tw, th) = terminal::size()?;
    let Some(mut v) = fit_view(tw, th) else {
        queue!(
            out,
            terminal::Clear(terminal::ClearType::All),
            cursor::MoveTo(0, 0),
            Print("Terminal too small. Try at least ~40x12.\n")
        )?;
        out.flush()?;
        return Ok(());
    };

    let seed = Instant::now().elapsed().as_nanos() as u64 ^ 0xC0FFEE_u64;
    let mut g = Game::new(seed);
    g.reset_run(&v);

    let mut r = Renderer::new(v.term_w, v.term_h);

    let mut last = Instant::now();
    let mut acc = 0.0f32;
    let mut now_s = 0.0f32;

    loop {
        // input
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(()),
                    KeyCode::Char('r') | KeyCode::Char('R') => g.reset_run(&v),
                    KeyCode::Char('t') | KeyCode::Char('T') => {
                        g.theme_idx = g.theme_idx.wrapping_add(1)
                    }
                    KeyCode::Char(' ') => g.paused = !g.paused,

                    KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('W') => g.move_frog(&v, 0, -4),
                    KeyCode::Down | KeyCode::Char('s') | KeyCode::Char('S') => {
                        g.move_frog(&v, 0, 4)
                    }
                    KeyCode::Left | KeyCode::Char('a') | KeyCode::Char('A') => {
                        g.move_frog(&v, -2, 0)
                    }
                    KeyCode::Right | KeyCode::Char('d') | KeyCode::Char('D') => {
                        g.move_frog(&v, 2, 0)
                    }
                    _ => {}
                },
                Event::Resize(w, h) => {
                    let _ = handle_resize(&mut r, &mut v, &mut g, w, h);
                }
                _ => {}
            }
        }

        // dt
        let now = Instant::now();
        let mut frame_dt = (now - last).as_secs_f32();
        last = now;
        if frame_dt > MAX_FRAME_DT {
            frame_dt = MAX_FRAME_DT;
        }

        if !g.paused && !g.game_over {
            now_s += frame_dt;
        }

        acc += frame_dt;
        while acc >= FIXED_DT {
            g.step(&v, now_s, FIXED_DT);
            acc -= FIXED_DT;
        }

        // draw
        render_playfield(&mut r, &v, &g, now_s);
        r.flush_diff(out)?;

        // light frame cap
        std::thread::sleep(Duration::from_millis(2));
    }
}

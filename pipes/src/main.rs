// main.rs
use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Color, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::{
    collections::HashSet,
    io::{self, Write},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
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

#[derive(Clone, Copy)]
struct Camera {
    yaw: f32,
    pitch: f32,
    dist: f32,
    fov: f32, // radians
}

#[derive(Clone, Copy)]
struct Segment {
    a: Vec3,
    b: Vec3,
    rgb: (u8, u8, u8),
    born: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Dir {
    Xp,
    Xm,
    Yp,
    Ym,
    Zp,
    Zm,
}
impl Dir {
    fn all() -> [Dir; 6] {
        [Dir::Xp, Dir::Xm, Dir::Yp, Dir::Ym, Dir::Zp, Dir::Zm]
    }
    fn opposite(self) -> Dir {
        match self {
            Dir::Xp => Dir::Xm,
            Dir::Xm => Dir::Xp,
            Dir::Yp => Dir::Ym,
            Dir::Ym => Dir::Yp,
            Dir::Zp => Dir::Zm,
            Dir::Zm => Dir::Zp,
        }
    }
    fn delta(self) -> (i32, i32, i32) {
        match self {
            Dir::Xp => (1, 0, 0),
            Dir::Xm => (-1, 0, 0),
            Dir::Yp => (0, 1, 0),
            Dir::Ym => (0, -1, 0),
            Dir::Zp => (0, 0, 1),
            Dir::Zm => (0, 0, -1),
        }
    }
    fn axis(self) -> u8 {
        match self {
            Dir::Xp | Dir::Xm => 0,
            Dir::Yp | Dir::Ym => 1,
            Dir::Zp | Dir::Zm => 2,
        }
    }
    fn idx(self) -> u8 {
        match self {
            Dir::Xp => 0,
            Dir::Xm => 1,
            Dir::Yp => 2,
            Dir::Ym => 3,
            Dir::Zp => 4,
            Dir::Zm => 5,
        }
    }
}

struct World {
    gx: i32,
    gy: i32,
    gz: i32,
    head: (i32, i32, i32),
    dir: Dir,
    occupied_edges: HashSet<u32>,
    rng: StdRng,
    palette: Vec<(u8, u8, u8)>,
    palette_i: usize,
}

impl World {
    fn new(seed: u64, gx: i32, gy: i32, gz: i32) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let head = (gx / 2, gy / 2, gz / 2);
        let dir = Dir::all()[rng.gen_range(0..6)];
        Self {
            gx,
            gy,
            gz,
            head,
            dir,
            occupied_edges: HashSet::with_capacity((gx * gy * gz) as usize),
            rng,
            palette: vec![
                (240, 90, 20),   // orange
                (70, 220, 120),  // green
                (40, 170, 255),  // blue
                (250, 230, 80),  // yellow
                (235, 80, 210),  // magenta
                (210, 210, 210), // gray
            ],
            palette_i: 0,
        }
    }

    fn reset(&mut self) {
        self.occupied_edges.clear();
        self.head = (self.gx / 2, self.gy / 2, self.gz / 2);
        self.dir = Dir::all()[self.rng.gen_range(0..6)];
    }

    fn cycle_palette(&mut self) {
        self.palette_i = (self.palette_i + 1) % self.palette.len();
    }

    fn current_color(&self) -> (u8, u8, u8) {
        self.palette[self.palette_i]
    }

    fn in_bounds(&self, p: (i32, i32, i32)) -> bool {
        p.0 >= 0 && p.0 < self.gx && p.1 >= 0 && p.1 < self.gy && p.2 >= 0 && p.2 < self.gz
    }

    fn edge_key(&self, from: (i32, i32, i32), d: Dir) -> u32 {
        // Encode undirected edge by always storing from=min(endpoint) in lexicographic order,
        // plus axis/sign as direction index from that canonical endpoint.
        let (dx, dy, dz) = d.delta();
        let to = (from.0 + dx, from.1 + dy, from.2 + dz);
        let (a, _b, dir_from_a) = if from <= to {
            (from, to, d)
        } else {
            (to, from, d.opposite())
        };

        // Pack: 10 bits per coord (supports up to 1023), 3 bits for dir idx.
        // key = x | y<<10 | z<<20 | dir<<30
        let x = a.0 as u32;
        let y = a.1 as u32;
        let z = a.2 as u32;
        let di = dir_from_a.idx() as u32;
        x | (y << 10) | (z << 20) | (di << 30)
    }

    fn try_step(&mut self) -> Option<((i32, i32, i32), (i32, i32, i32), Dir)> {
        let mut candidates: Vec<(Dir, f32)> = Vec::with_capacity(6);

        for d in Dir::all() {
            // Avoid immediate backtracking unless forced
            if d == self.dir.opposite() {
                continue;
            }

            let (dx, dy, dz) = d.delta();
            let to = (self.head.0 + dx, self.head.1 + dy, self.head.2 + dz);
            if !self.in_bounds(to) {
                continue;
            }
            let k = self.edge_key(self.head, d);
            if self.occupied_edges.contains(&k) {
                continue;
            }

            // Weighting: prefer straight, slight preference for staying on same axis
            let mut w = 1.0;
            if d == self.dir {
                w *= 4.0;
            } else if d.axis() == self.dir.axis() {
                w *= 1.3;
            } else {
                w *= 1.0;
            }

            // Small bias toward moving "forward" in Z (looks nicer with default camera)
            if matches!(d, Dir::Zp) {
                w *= 1.2;
            }

            candidates.push((d, w));
        }

        if candidates.is_empty() {
            // If stuck, allow backtracking as a last resort
            let d = self.dir.opposite();
            let (dx, dy, dz) = d.delta();
            let to = (self.head.0 + dx, self.head.1 + dy, self.head.2 + dz);
            if self.in_bounds(to) {
                let k = self.edge_key(self.head, d);
                if !self.occupied_edges.contains(&k) {
                    return Some((self.head, to, d));
                }
            }
            return None;
        }

        // Weighted choice
        let sum: f32 = candidates.iter().map(|(_, w)| *w).sum();
        let mut r = self.rng.gen::<f32>() * sum;
        let mut chosen = candidates[0].0;
        for (d, w) in candidates {
            if r <= w {
                chosen = d;
                break;
            }
            r -= w;
        }

        let (dx, dy, dz) = chosen.delta();
        let to = (self.head.0 + dx, self.head.1 + dy, self.head.2 + dz);
        Some((self.head, to, chosen))
    }

    fn step(&mut self) -> Option<((i32, i32, i32), (i32, i32, i32), Dir)> {
        let res = self.try_step();
        if let Some((from, to, d)) = res {
            let k = self.edge_key(from, d);
            self.occupied_edges.insert(k);
            self.head = to;
            self.dir = d;
            Some((from, to, d))
        } else {
            None
        }
    }
}

struct Frame {
    cols: u16,
    rows: u16,
    w: usize, // subpixel width (cols*2)
    h: usize, // subpixel height (rows*4)
    zbuf: Vec<f32>,
    rgbbuf: Vec<u32>, // 0 = off, else 0xRRGGBB
    glyphs: Vec<char>,
    cell_rgb: Vec<u32>,
    last_glyphs: Vec<char>,
    last_rgb: Vec<u32>,
}

fn pack_rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}
fn unpack_rgb(c: u32) -> (u8, u8, u8) {
    (((c >> 16) & 255) as u8, ((c >> 8) & 255) as u8, (c & 255) as u8)
}

fn make_frame(cols: u16, rows: u16) -> Frame {
    let w = cols as usize * 2;
    let h = rows as usize * 4;
    let sp = w * h;
    let cells = cols as usize * rows as usize;

    Frame {
        cols,
        rows,
        w,
        h,
        zbuf: vec![f32::INFINITY; sp],
        rgbbuf: vec![0; sp],
        glyphs: vec![' '; cells],
        cell_rgb: vec![0; cells],
        last_glyphs: vec!['\0'; cells],
        last_rgb: vec![u32::MAX; cells],
    }
}

fn view_transform(cam: &Camera, p: Vec3) -> Vec3 {
    // World is grid-centered around origin. Camera looks toward +Z in view space.
    // Apply yaw (around Y), pitch (around X), then translate by +dist along -Z (i.e., move world away).
    let (sy, cy) = cam.yaw.sin_cos();
    let (sp, cp) = cam.pitch.sin_cos();

    // yaw around Y
    let x1 = p.x * cy - p.z * sy;
    let z1 = p.x * sy + p.z * cy;
    let y1 = p.y;

    // pitch around X
    let y2 = y1 * cp - z1 * sp;
    let z2 = y1 * sp + z1 * cp;

    Vec3::new(x1, y2, z2 + cam.dist)
}

fn project(frame: &Frame, cam: &Camera, p_world: Vec3) -> Option<(f32, f32, f32)> {
    let p = view_transform(cam, p_world);
    if p.z <= 0.05 {
        return None;
    }
    let f = 0.5 * (frame.w as f32) / (cam.fov * 0.5).tan();
    let sx = p.x * f / p.z + (frame.w as f32) * 0.5;
    let sy = -p.y * f / p.z + (frame.h as f32) * 0.5;
    Some((sx, sy, p.z))
}

fn dot_bit(dx: usize, dy: usize) -> u8 {
    // Braille dot mapping for a 2x4 block:
    // left column:  (0,0)=1 (0,1)=2 (0,2)=4 (0,3)=64
    // right column: (1,0)=8 (1,1)=16 (1,2)=32 (1,3)=128
    match (dx, dy) {
        (0, 0) => 0x01,
        (0, 1) => 0x02,
        (0, 2) => 0x04,
        (0, 3) => 0x40,
        (1, 0) => 0x08,
        (1, 1) => 0x10,
        (1, 2) => 0x20,
        (1, 3) => 0x80,
        _ => 0,
    }
}

fn clear_subpixels(frame: &mut Frame) {
    frame.zbuf.fill(f32::INFINITY);
    frame.rgbbuf.fill(0);
}

fn stamp_disc(
    frame: &mut Frame,
    cx: f32,
    cy: f32,
    depth: f32,
    r: f32,
    base_rgb: (u8, u8, u8),
    light: Vec3,
) {
    if r <= 0.2 {
        return;
    }
    let xmin = (cx - r).floor() as i32;
    let xmax = (cx + r).ceil() as i32;
    let ymin = (cy - r).floor() as i32;
    let ymax = (cy + r).ceil() as i32;

    let rr = r * r;

    for y in ymin..=ymax {
        if y < 0 || y >= frame.h as i32 {
            continue;
        }
        for x in xmin..=xmax {
            if x < 0 || x >= frame.w as i32 {
                continue;
            }
            let fx = (x as f32 + 0.5) - cx;
            let fy = (y as f32 + 0.5) - cy;
            let d2 = fx * fx + fy * fy;
            if d2 > rr {
                continue;
            }

            let idx = (y as usize) * frame.w + (x as usize);
            if depth >= frame.zbuf[idx] {
                continue;
            }

            // Fake normal in "screen space": (fx, fy, z) on a sphere-ish cap
            let nz = (rr - d2).max(0.0).sqrt();
            let n = Vec3::new(fx, fy, nz).norm();
            let ndl = n.dot(light).max(0.0);

            // Combine diffuse + a bit of specular
            let mut shade = 0.35 + 0.65 * ndl;
            // cheap highlight: favor top-left ridge
            let h = Vec3::new(-0.4, -0.3, 1.0).norm();
            let spec = n.dot(h).max(0.0).powf(18.0);
            shade = (shade + 0.35 * spec).min(1.0);

            let (br, bg, bb) = base_rgb;
            let r8 = (br as f32 * shade) as u8;
            let g8 = (bg as f32 * shade) as u8;
            let b8 = (bb as f32 * shade) as u8;

            frame.zbuf[idx] = depth;
            frame.rgbbuf[idx] = pack_rgb(r8, g8, b8);
        }
    }
}

fn rasterize_segments(
    frame: &mut Frame,
    cam: &Camera,
    segments: &[Segment],
    pipe_radius: f32,
    time_s: f32,
) {
    clear_subpixels(frame);

    let light = Vec3::new(-0.6, -0.5, 1.0).norm();

    // Depth fog: older segments dim slightly
    for seg in segments {
        // Sample along centerline
        let a = seg.a;
        let b = seg.b;
        let ab = b.sub(a);
        let len = ab.len().max(1e-6);

        // Sample count tied to length, but not too dense
        let steps = (len * 10.0).ceil() as i32; // grid step ~1, so this is plenty
        let steps = steps.clamp(8, 64);

        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let p = a.add(ab.mul(t));

            let Some((sx, sy, depth)) = project(frame, cam, p) else {
                continue;
            };

            // Perspective radius: scale by depth
            // f is implicit; approximate using screen width and fov
            let f = 0.5 * (frame.w as f32) / (cam.fov * 0.5).tan();
            let r_screen = (pipe_radius * f / depth).clamp(0.8, 9.0);

            // Age fade (subtle)
            let age = (time_s - seg.born).max(0.0);
            let fade = (1.0 / (1.0 + 0.08 * age)).clamp(0.55, 1.0);

            let (r0, g0, b0) = seg.rgb;
            let base = (
                (r0 as f32 * fade) as u8,
                (g0 as f32 * fade) as u8,
                (b0 as f32 * fade) as u8,
            );

            stamp_disc(frame, sx, sy, depth, r_screen, base, light);
        }
    }
}

fn subpixels_to_cells(frame: &mut Frame) {
    let cols = frame.cols as usize;
    let rows = frame.rows as usize;

    for cy in 0..rows {
        for cx in 0..cols {
            let cell_i = cy * cols + cx;

            let spx = cx * 2;
            let spy = cy * 4;

            let mut bits: u8 = 0;
            let mut pick_rgb: u32 = 0;
            let mut best_luma: i32 = -1;

            for dy in 0..4 {
                for dx in 0..2 {
                    let x = spx + dx;
                    let y = spy + dy;
                    let idx = y * frame.w + x;
                    let c = frame.rgbbuf[idx];
                    if c != 0 {
                        bits |= dot_bit(dx, dy);
                        let (r, g, b) = unpack_rgb(c);
                        let l = (r as i32) * 3 + (g as i32) * 4 + (b as i32) * 1;
                        if l > best_luma {
                            best_luma = l;
                            pick_rgb = c;
                        }
                    }
                }
            }

            let ch = if bits == 0 {
                ' '
            } else {
                // U+2800 + bits
                char::from_u32(0x2800 + bits as u32).unwrap_or(' ')
            };

            frame.glyphs[cell_i] = ch;
            frame.cell_rgb[cell_i] = pick_rgb;
        }
    }
}

fn draw_diff(stdout: &mut io::Stdout, frame: &mut Frame) -> io::Result<()> {
    let cols = frame.cols as usize;
    let rows = frame.rows as usize;

    queue!(stdout, BeginSynchronizedUpdate)?;

    for y in 0..rows {
        for x in 0..cols {
            let i = y * cols + x;
            let ch = frame.glyphs[i];
            let rgb = frame.cell_rgb[i];

            if ch == frame.last_glyphs[i] && rgb == frame.last_rgb[i] {
                continue;
            }

            frame.last_glyphs[i] = ch;
            frame.last_rgb[i] = rgb;

            queue!(stdout, cursor::MoveTo(x as u16, y as u16))?;
            if rgb != 0 {
                let (r, g, b) = unpack_rgb(rgb);
                queue!(stdout, SetForegroundColor(Color::Rgb { r, g, b }))?;
            } else {
                queue!(stdout, SetForegroundColor(Color::Black))?;
            }

            write!(stdout, "{}", ch)?;
        }
    }

    queue!(stdout, EndSynchronizedUpdate)?;
    stdout.flush()?;
    Ok(())
}

fn grid_to_world(p: (i32, i32, i32), gx: i32, gy: i32, gz: i32) -> Vec3 {
    // Center grid around origin, scale each cell to 1.0
    let cx = (gx as f32 - 1.0) * 0.5;
    let cy = (gy as f32 - 1.0) * 0.5;
    let cz = (gz as f32 - 1.0) * 0.5;
    Vec3::new(p.0 as f32 - cx, p.1 as f32 - cy, p.2 as f32 - cz)
}

fn main() -> Result<()> {
    let mut stdout = io::stdout();

    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    terminal::enable_raw_mode()?;
    execute!(stdout, DisableLineWrap)?;

    let cleanup = || -> io::Result<()> {
        let mut out = io::stdout();
        execute!(out, EnableLineWrap)?;
        terminal::disable_raw_mode()?;
        execute!(out, cursor::Show, LeaveAlternateScreen)?;
        Ok(())
    };

    let res = run(&mut stdout);

    // Always cleanup
    let _ = cleanup();
    res
}

fn run(stdout: &mut io::Stdout) -> Result<()> {
    let (mut cols, mut rows) = terminal::size()?;
    if cols < 20 || rows < 10 {
        cols = cols.max(20);
        rows = rows.max(10);
    }

    let mut frame = make_frame(cols, rows);

    // Simulation settings
    let gx = 24;
    let gy = 24;
    let gz = 24;

    let mut world = World::new(0xC0FFEE, gx, gy, gz);
    let mut segments: Vec<Segment> = Vec::with_capacity(1200);

    let mut cam = Camera {
        yaw: 0.65,
        pitch: -0.35,
        dist: 38.0,
        fov: 70f32.to_radians(),
    };

    let mut pipe_radius: f32 = 0.42; // in world units (cell size = 1.0)

    let mut paused = false;
    let mut speed: f32 = 1.0; // segments per tick multiplier
    let mut seg_timer = 0.0f32;
    let mut auto_color = false;
    let mut last_color_cycle = 0.0f32;
    let mut show_hud = true;

    let max_segments: usize = 1200;
    let seg_interval = 0.03f32; // seconds per segment at speed=1

    let mut t0 = Instant::now();
    let mut last = Instant::now();

    loop {
        // Handle resize + input
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Resize(c, r) => {
                    cols = c;
                    rows = r;
                    frame = make_frame(cols, rows);
                }
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    match (k.code, k.modifiers) {
                        (KeyCode::Char('q'), _) | (KeyCode::Char('Q'), _) => return Ok(()),
                        (KeyCode::Char('c'), _) | (KeyCode::Char('C'), _) => world.cycle_palette(),
                        (KeyCode::Char('a'), _) | (KeyCode::Char('A'), _) => auto_color = !auto_color,
                        (KeyCode::Char('h'), _) | (KeyCode::Char('H'), _) => show_hud = !show_hud,
                        (KeyCode::Char('r'), _) | (KeyCode::Char('R'), _) => {
                            world.reset();
                            segments.clear();
                        }
                        (KeyCode::Char(' '), _) => paused = !paused,

                        (KeyCode::Up, _) => cam.pitch = (cam.pitch - 0.08).clamp(-1.35, 1.35),
                        (KeyCode::Down, _) => cam.pitch = (cam.pitch + 0.08).clamp(-1.35, 1.35),
                        (KeyCode::Left, _) => cam.yaw -= 0.10,
                        (KeyCode::Right, _) => cam.yaw += 0.10,

                        (KeyCode::Char('+'), _) | (KeyCode::Char('='), _) => cam.dist = (cam.dist - 2.0).max(8.0),
                        (KeyCode::Char('-'), _) => cam.dist = (cam.dist + 2.0).min(120.0),

                        (KeyCode::Char(']'), _) => pipe_radius = (pipe_radius + 0.03_f32).min(0.9_f32),
                        (KeyCode::Char('['), _) => pipe_radius = (pipe_radius - 0.03_f32).max(0.18_f32),

                        (KeyCode::Char('<'), _) | (KeyCode::Char(','), _) => {
                            speed = (speed * 0.8_f32).max(0.15_f32);
                        }
                        (KeyCode::Char('>'), _) | (KeyCode::Char('.'), _) => {
                            speed = (speed * 1.25_f32).min(10.0_f32);
                        }

                        // Ctrl+L: quick clear/reseed
                        (KeyCode::Char('l'), m) if m.contains(KeyModifiers::CONTROL) => {
                            world.reset();
                            segments.clear();
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
        let time_s = (now - t0).as_secs_f32();

        if auto_color && (time_s - last_color_cycle) >= 6.0 {
            world.cycle_palette();
            last_color_cycle = time_s;
        }

        // Simulation advance
        if !paused {
            seg_timer += dt * speed;
            while seg_timer >= seg_interval {
                seg_timer -= seg_interval;

                let step = world.step();
                if let Some((from, to, _d)) = step {
                    let a = grid_to_world(from, gx, gy, gz);
                    let b = grid_to_world(to, gx, gy, gz);
                    let rgb = world.current_color();

                    segments.push(Segment { a, b, rgb, born: time_s });

                    if segments.len() > max_segments {
                        let overflow = segments.len() - max_segments;
                        segments.drain(0..overflow);
                    }
                } else {
                    // If fully stuck, reseed
                    world.reset();
                }
            }
        }

        // Render
        rasterize_segments(&mut frame, &cam, &segments, pipe_radius, time_s);
        subpixels_to_cells(&mut frame);

        // HUD overlay (top-left) by overwriting a few cells in the char buffer
        if show_hud {
            overlay_hud(&mut frame, paused, speed, pipe_radius, segments.len(), auto_color);
        }

        // Draw diff
        draw_diff(stdout, &mut frame)?;

        // Frame cap
        std::thread::sleep(Duration::from_millis(12));
    }
}

fn overlay_hud(frame: &mut Frame, paused: bool, speed: f32, r: f32, segs: usize, auto_color: bool) {
    let cols = frame.cols as usize;
    if cols < 20 || frame.rows < 2 {
        return;
    }

    let line1 = format!(
        "pipes  | {}  | speed {:.2}  | r {:.2}  | seg {}  | auto {}",
        if paused { "paused" } else { "run" },
        speed,
        r,
        segs,
        if auto_color { "on" } else { "off" }
    );
    let line2 =
        "Q quit  Arrows rotate  +/- zoom  [ ] radius  C palette  A auto  R reset  Space pause  < > speed";

    put_text(frame, 0, 0, &line1, pack_rgb(210, 210, 210));
    put_text(frame, 0, 1, line2, pack_rgb(160, 160, 160));
}

fn put_text(frame: &mut Frame, x: usize, y: usize, s: &str, rgb: u32) {
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
        frame.cell_rgb[i] = rgb;
        cx += 1;
    }
}

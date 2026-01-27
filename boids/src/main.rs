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

#[derive(Clone, Copy, Debug)]
struct Vec2 {
    x: f32,
    y: f32,
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
    fn mul(self, k: f32) -> Vec2 {
        Vec2 {
            x: self.x * k,
            y: self.y * k,
        }
    }
    fn len(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    fn norm(self) -> Vec2 {
        let l = self.len();
        if l <= 1e-6 {
            Vec2 { x: 0.0, y: 0.0 }
        } else {
            self.mul(1.0 / l)
        }
    }
    fn limit(self, max: f32) -> Vec2 {
        let l = self.len();
        if l > max {
            self.mul(max / l)
        } else {
            self
        }
    }
}

#[derive(Clone, Copy)]
struct Boid {
    p: Vec2, // 0..1
    v: Vec2, // "world pixels per second" in 0..1-ish units
}

#[derive(Clone, Copy)]
struct Params {
    neigh_r: f32,
    sep_r: f32,
    w_align: f32,
    w_coh: f32,
    w_sep: f32,
    max_speed: f32,
    max_force: f32,
}

fn wrap01(mut x: f32) -> f32 {
    if x >= 1.0 {
        x -= 1.0;
    }
    if x < 0.0 {
        x += 1.0;
    }
    x
}

// Braille cell is 2x4 dots.
// Dots are numbered (1..8) with this layout:
// (0,0)=1 (0,1)=2 (0,2)=3 (0,3)=7
// (1,0)=4 (1,1)=5 (1,2)=6 (1,3)=8
fn braille_mask(dx: usize, dy: usize) -> u8 {
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

#[inline]
fn imod(a: i32, m: i32) -> i32 {
    // positive modulo
    let r = a % m;
    if r < 0 { r + m } else { r }
}

fn render_braille(
    out: &mut Stdout,
    boids: &[Boid],
    cw: u16,
    ch: u16,
    paused: bool,
    p: Params,
    frame_ms: u64,
    show_help: bool,
    fg: Color,
    // buffers
    cells: &mut Vec<u8>,
    prev_cells: &mut Vec<u8>,
    line_buf: &mut String,
    needs_full_redraw: &mut bool,
) -> io::Result<()> {
    let status_rows = 1u16;
    let bh = ch.saturating_sub(status_rows);

    let bw = cw as usize;
    let bh_usize = bh as usize;

    // World is dots: w = cw*2, h = bh*4
    let dot_w = bw.saturating_mul(2);
    let dot_h = bh_usize.saturating_mul(4);

    let n_cells = bw.saturating_mul(bh_usize);
    if cells.len() != n_cells {
        cells.resize(n_cells, 0);
        *needs_full_redraw = true;
    }
    cells.fill(0);

    // Plot boids
    if dot_w > 0 && dot_h > 0 && bw > 0 && bh_usize > 0 {
        for b in boids {
            let x = (b.p.x * dot_w as f32) as i32;
            let y = (b.p.y * dot_h as f32) as i32;
            if x < 0 || y < 0 {
                continue;
            }
            let x = (x as usize).min(dot_w - 1);
            let y = (y as usize).min(dot_h - 1);

            let cx = x / 2;
            let cy = y / 4;
            let dx = x % 2;
            let dy = y % 4;

            if cx < bw && cy < bh_usize {
                let i = cy * bw + cx;
                cells[i] |= braille_mask(dx, dy);
            }

            // Optional trailing dot when moving fast.
            let sp = b.v.len();
            if sp > p.max_speed * 0.72 {
                let back = b.v.norm().mul(-1.0);
                let tx = wrap01(b.p.x + back.x * 0.010);
                let ty = wrap01(b.p.y + back.y * 0.010);

                let mut x2 = (tx * dot_w as f32) as usize;
                let mut y2 = (ty * dot_h as f32) as usize;

                x2 = x2.min(dot_w - 1);
                y2 = y2.min(dot_h - 1);

                let cx2 = x2 / 2;
                let cy2 = y2 / 4;
                let dx2 = x2 % 2;
                let dy2 = y2 % 4;

                if cx2 < bw && cy2 < bh_usize {
                    let j = cy2 * bw + cx2;
                    cells[j] |= braille_mask(dx2, dy2);
                }
            }
        }
    }

    if prev_cells.len() != cells.len() {
        prev_cells.resize(cells.len(), 0);
        *needs_full_redraw = true;
    }

    queue!(out, BeginSynchronizedUpdate, ResetColor, SetForegroundColor(fg))?;

    if *needs_full_redraw {
        queue!(out, Clear(ClearType::All))?;
    }

    // Diff render: only redraw changed rows
    if bw > 0 && bh_usize > 0 {
        for y in 0..bh_usize {
            let a = &cells[y * bw..(y + 1) * bw];
            let b = &prev_cells[y * bw..(y + 1) * bw];

            if *needs_full_redraw || a != b {
                line_buf.clear();
                line_buf.reserve(bw);
                for &m in a {
                    line_buf.push(char::from_u32(0x2800 + m as u32).unwrap_or(' '));
                }
                queue!(out, cursor::MoveTo(0, y as u16), Print(&*line_buf))?;
            }
        }
    }

    let help = if show_help {
        "Keys: q quit | space pause | r reset | arrows: boids/params | +/- speed | h help | c color"
    } else {
        "Press h for keys"
    };

    let status = format!(
        "Boids (braille)  [{}]  n:{}  neigh:{:.0} sep:{:.0}  {}ms/f  {}",
        if paused { "paused" } else { "running" },
        boids.len(),
        p.neigh_r,
        p.sep_r,
        frame_ms,
        help
    );

    queue!(
        out,
        cursor::MoveTo(0, bh),
        Clear(ClearType::CurrentLine),
        Print(status)
    )?;

    prev_cells.copy_from_slice(cells);
    *needs_full_redraw = false;

    queue!(out, EndSynchronizedUpdate)?;
    out.flush()?;
    Ok(())
}

fn rebuild_grid(
    boids: &[Boid],
    world_w: f32,
    world_h: f32,
    cell: f32,
    grid_w: i32,
    grid_h: i32,
    head: &mut Vec<i32>,
    next: &mut Vec<i32>,
) {
    head.fill(-1);
    // next is per boid
    for (i, b) in boids.iter().enumerate() {
        let px = b.p.x * world_w;
        let py = b.p.y * world_h;

        let mut cx = (px / cell).floor() as i32;
        let mut cy = (py / cell).floor() as i32;

        cx = imod(cx, grid_w);
        cy = imod(cy, grid_h);

        let idx = (cy * grid_w + cx) as usize;

        next[i] = head[idx];
        head[idx] = i as i32;
    }
}

// Exact neighbor search using a uniform grid.
// Fidelity is preserved because we still do the exact distance check against neigh_r/sep_r.
fn step_boids_grid(
    boids: &mut [Boid],
    dt: f32,
    world_w: f32,
    world_h: f32,
    p: Params,
    // persistent buffers
    acc: &mut Vec<Vec2>,
    head: &mut Vec<i32>,
    next: &mut Vec<i32>,
) {
    let n = boids.len();
    if acc.len() != n {
        acc.resize(n, Vec2 { x: 0.0, y: 0.0 });
    }
    acc.fill(Vec2 { x: 0.0, y: 0.0 });

    if n == 0 || world_w <= 1.0 || world_h <= 1.0 {
        return;
    }

    // Grid cell size: neigh_r ensures all neighbors within radius are in current or adjacent cells.
    let cell = p.neigh_r.max(2.0);
    let grid_w = ((world_w / cell).ceil() as i32).max(1);
    let grid_h = ((world_h / cell).ceil() as i32).max(1);

    let grid_n = (grid_w * grid_h) as usize;
    if head.len() != grid_n {
        head.resize(grid_n, -1);
    } else {
        head.fill(-1);
    }
    if next.len() != n {
        next.resize(n, -1);
    }

    rebuild_grid(boids, world_w, world_h, cell, grid_w, grid_h, head, next);

    let half_w = world_w * 0.5;
    let half_h = world_h * 0.5;

    for i in 0..n {
        let bi = boids[i];

        // Find bi cell
        let pix = bi.p.x * world_w;
        let piy = bi.p.y * world_h;
        let cix = imod((pix / cell).floor() as i32, grid_w);
        let ciy = imod((piy / cell).floor() as i32, grid_h);

        let mut count = 0.0f32;
        let mut align = Vec2 { x: 0.0, y: 0.0 };
        let mut coh = Vec2 { x: 0.0, y: 0.0 };
        let mut sep = Vec2 { x: 0.0, y: 0.0 };

        // 3x3 neighborhood
        for oy in -1..=1 {
            for ox in -1..=1 {
                let nx = imod(cix + ox, grid_w);
                let ny = imod(ciy + oy, grid_h);
                let cell_idx = (ny * grid_w + nx) as usize;

                let mut j = head[cell_idx];
                while j != -1 {
                    let j_usize = j as usize;
                    if j_usize != i {
                        let bj = boids[j_usize];

                        // torus distance in world pixels
                        let mut dx = (bj.p.x - bi.p.x) * world_w;
                        let mut dy = (bj.p.y - bi.p.y) * world_h;

                        if dx > half_w {
                            dx -= world_w;
                        }
                        if dx < -half_w {
                            dx += world_w;
                        }
                        if dy > half_h {
                            dy -= world_h;
                        }
                        if dy < -half_h {
                            dy += world_h;
                        }

                        let d2 = dx * dx + dy * dy;
                        if d2 > 1e-10 {
                            let d = d2.sqrt();
                            if d <= p.neigh_r {
                                count += 1.0;
                                align = align.add(bj.v);

                                // average wrapped position in 0..1 space
                                coh = coh.add(Vec2 {
                                    x: bi.p.x + dx / world_w,
                                    y: bi.p.y + dy / world_h,
                                });

                                if d < p.sep_r {
                                    let away = Vec2 { x: -dx, y: -dy }
                                        .norm()
                                        .mul(1.0 / d.max(0.5));
                                    sep = sep.add(away);
                                }
                            }
                        }
                    }

                    j = next[j_usize];
                }
            }
        }

        if count > 0.0 {
            // alignment
            let desired = align.mul(1.0 / count).norm().mul(p.max_speed);
            let steer_align = desired.sub(bi.v).limit(p.max_force);

            // cohesion
            let center = coh.mul(1.0 / count);
            let to_center = Vec2 {
                x: (center.x - bi.p.x) * world_w,
                y: (center.y - bi.p.y) * world_h,
            };
            let desired2 = to_center.norm().mul(p.max_speed);
            let steer_coh = desired2.sub(bi.v).limit(p.max_force);

            // separation
            let desired3 = sep.norm().mul(p.max_speed);
            let steer_sep = desired3.sub(bi.v).limit(p.max_force);

            acc[i] = acc[i]
                .add(steer_align.mul(p.w_align))
                .add(steer_coh.mul(p.w_coh))
                .add(steer_sep.mul(p.w_sep));
        }
    }

    for i in 0..n {
        boids[i].v = boids[i].v.add(acc[i].mul(dt)).limit(p.max_speed);
        boids[i].p.x = wrap01(boids[i].p.x + boids[i].v.x * dt);
        boids[i].p.y = wrap01(boids[i].p.y + boids[i].v.y * dt);
    }
}

fn reset_boids(rng: &mut StdRng, n: usize) -> Vec<Boid> {
    let mut boids = Vec::with_capacity(n);
    for _ in 0..n {
        let a = rng.gen_range(0.0..std::f32::consts::TAU);
        let sp = rng.gen_range(0.10..0.28);
        boids.push(Boid {
            p: Vec2 {
                x: rng.gen_range(0.0..1.0),
                y: rng.gen_range(0.0..1.0),
            },
            v: Vec2 {
                x: a.cos() * sp,
                y: a.sin() * sp,
            },
        });
    }
    boids
}

fn main() -> io::Result<()> {
    let mut stdout = io::stdout();

    execute!(stdout, EnterAlternateScreen, cursor::Hide, DisableLineWrap)?;
    terminal::enable_raw_mode()?;
    execute!(stdout, Clear(ClearType::All))?;

    let mut rng = StdRng::from_entropy();

    let mut show_help = false;
    let mut paused = false;

    let mut frame_ms: u64 = 33;

    let mut p = Params {
        neigh_r: 16.0,
        sep_r: 7.0,
        w_align: 0.90,
        w_coh: 0.55,
        w_sep: 1.20,
        max_speed: 0.28,
        max_force: 0.060,
    };

    let mut boid_count: usize = 180;
    let mut boids = reset_boids(&mut rng, boid_count);

    // Render buffers (reused)
    let mut cells: Vec<u8> = Vec::new();
    let mut prev_cells: Vec<u8> = Vec::new();
    let mut line_buf: String = String::new();
    let mut needs_full_redraw = true;

    // Sim buffers (reused)
    let mut acc: Vec<Vec2> = Vec::new();
    let mut head: Vec<i32> = Vec::new(); // grid heads
    let mut next: Vec<i32> = Vec::new(); // boid linked list

    // Theme palette (foreground only)
    let themes: [Color; 10] = [
        Color::White,
        Color::Cyan,
        Color::Green,
        Color::Yellow,
        Color::Magenta,
        Color::Blue,
        Color::Red,
        Color::Grey,
        Color::DarkCyan,
        Color::DarkGreen,
    ];
    let mut theme_idx: usize = 0;

    let mut last = Instant::now();

    'outer: loop {
        let frame_start = Instant::now();
        let frame_budget = Duration::from_millis(frame_ms);

        // Drain input without busy-spinning too hard:
        // - We do a quick poll(0) loop to catch bursts
        // - Then the frame pacing below handles sleeping
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => break 'outer,
                    KeyCode::Char(' ') => paused = !paused,
                    KeyCode::Char('h') => {
                        show_help = !show_help;
                        needs_full_redraw = true;
                    }
                    KeyCode::Char('r') => {
                        boids = reset_boids(&mut rng, boid_count);
                        // buffers resize automatically, but force clean redraw
                        needs_full_redraw = true;
                        prev_cells.clear();
                    }
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        theme_idx = (theme_idx + 1) % themes.len();
                        needs_full_redraw = true;
                    }
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        frame_ms = frame_ms.saturating_sub(5).max(10);
                        needs_full_redraw = true;
                    }
                    KeyCode::Char('-') => {
                        frame_ms = (frame_ms + 5).min(120);
                        needs_full_redraw = true;
                    }
                    KeyCode::Up => {
                        boid_count = (boid_count + 20).min(1200);
                        boids = reset_boids(&mut rng, boid_count);
                        needs_full_redraw = true;
                        prev_cells.clear();
                    }
                    KeyCode::Down => {
                        boid_count = boid_count.saturating_sub(20).max(20);
                        boids = reset_boids(&mut rng, boid_count);
                        needs_full_redraw = true;
                        prev_cells.clear();
                    }
                    KeyCode::Left => {
                        p.neigh_r = (p.neigh_r - 2.0).max(8.0);
                        p.sep_r = (p.sep_r - 1.0).max(3.0);
                        needs_full_redraw = true;
                    }
                    KeyCode::Right => {
                        p.neigh_r = (p.neigh_r + 2.0).min(60.0);
                        p.sep_r = (p.sep_r + 1.0).min(30.0);
                        needs_full_redraw = true;
                    }
                    _ => {}
                },
                Event::Resize(_, _) => {
                    needs_full_redraw = true;
                    prev_cells.clear();
                    cells.clear();
                }
                _ => {}
            }
        }

        let (cw, ch) = terminal::size()?;

        let now = Instant::now();
        let dt = (now - last).as_secs_f32();
        last = now;

        if !paused {
            let bh = ch.saturating_sub(1);
            let world_w = (cw as f32) * 2.0;
            let world_h = (bh as f32) * 4.0;

            // clamp dt so tabbing away doesn't explode everything
            let dt = dt.clamp(0.0, 0.050);

            step_boids_grid(&mut boids, dt, world_w, world_h, p, &mut acc, &mut head, &mut next);
        }

        render_braille(
            &mut stdout,
            &boids,
            cw,
            ch,
            paused,
            p,
            frame_ms,
            show_help,
            themes[theme_idx],
            &mut cells,
            &mut prev_cells,
            &mut line_buf,
            &mut needs_full_redraw,
        )?;

        // Frame pacing: sleep only what’s left of the budget.
        let elapsed = frame_start.elapsed();
        if elapsed < frame_budget {
            std::thread::sleep(frame_budget - elapsed);
        }
    }

    terminal::disable_raw_mode()?;
    execute!(
        stdout,
        ResetColor,
        cursor::Show,
        EnableLineWrap,
        LeaveAlternateScreen
    )?;
    Ok(())
}

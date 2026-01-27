use clap::Parser;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute, queue,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::{
    f32::consts::TAU,
    io::{self, stdout, Stdout, Write},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Parser, Debug)]
struct Args {
    /// ms per frame (lower = faster)
    #[arg(long, default_value_t = 40)]
    ms: u64,

    /// number of moving seeds
    #[arg(long, default_value_t = 24)]
    seeds: usize,

    /// animation speed multiplier
    #[arg(long, default_value_t = 1.0)]
    speed: f32,

    /// leave N rows unused at the bottom to avoid scrolling
    #[arg(long, default_value_t = 1)]
    margin_rows: u16,

    /// show HUD on start
    #[arg(long, default_value_t = true)]
    hud: bool,
}

// Braille: each terminal cell represents 2x4 pixels.
fn braille_bit(dx: usize, dy: usize) -> u8 {
    match (dx, dy) {
        (0, 0) => 0x01, // dot 1
        (0, 1) => 0x02, // dot 2
        (0, 2) => 0x04, // dot 3
        (1, 0) => 0x08, // dot 4
        (1, 1) => 0x10, // dot 5
        (1, 2) => 0x20, // dot 6
        (0, 3) => 0x40, // dot 7
        (1, 3) => 0x80, // dot 8
        _ => 0,
    }
}
fn braille_char(mask: u8) -> char {
    char::from_u32(0x2800 + mask as u32).unwrap_or(' ')
}

#[derive(Clone, Copy)]
struct Seed {
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    fx: f32,
    fy: f32,
    phx: f32,
    phy: f32,
}

fn make_seeds(n: usize, rng: &mut StdRng) -> Vec<Seed> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let cx = rng.gen_range(0.15..0.85);
        let cy = rng.gen_range(0.15..0.85);
        let rx = rng.gen_range(0.05..0.35);
        let ry = rng.gen_range(0.05..0.35);
        let fx = rng.gen_range(0.6..2.6);
        let fy = rng.gen_range(0.6..2.6);
        let phx = rng.gen_range(0.0..TAU);
        let phy = rng.gen_range(0.0..TAU);

        out.push(Seed {
            cx,
            cy,
            rx,
            ry,
            fx,
            fy,
            phx,
            phy,
        });
    }
    out
}

fn seed_pos_px(s: Seed, t: f32, w: usize, h: usize) -> (f32, f32) {
    let x = (s.cx + s.rx * (t * s.fx + s.phx).sin()).clamp(0.0, 1.0);
    let y = (s.cy + s.ry * (t * s.fy + s.phy).cos()).clamp(0.0, 1.0);
    (x * (w as f32 - 1.0), y * (h as f32 - 1.0))
}

/// Returns (nearest_id, nearest_dist2, second_nearest_dist2)
fn nearest_two(px: f32, py: f32, seed_xy: &[(f32, f32)]) -> (usize, f32, f32) {
    let mut best_i = 0usize;
    let mut best_d = f32::INFINITY;
    let mut second_d = f32::INFINITY;

    for (i, &(sx, sy)) in seed_xy.iter().enumerate() {
        let dx = px - sx;
        let dy = py - sy;
        let d = dx * dx + dy * dy;

        if d < best_d {
            second_d = best_d;
            best_d = d;
            best_i = i;
        } else if d < second_d {
            second_d = d;
        }
    }

    // If there are only 2 seeds it’s fine. If somehow second is inf, clamp.
    if !second_d.is_finite() {
        second_d = best_d;
    }

    (best_i, best_d, second_d)
}

fn palette(i: usize) -> Color {
    match i % 6 {
        0 => Color::Cyan,
        1 => Color::Green,
        2 => Color::Blue,
        3 => Color::Magenta,
        4 => Color::Yellow,
        _ => Color::White,
    }
}

#[derive(Clone, Copy)]
struct Dims {
    cell_w: usize, // terminal columns used for braille
    cell_h: usize, // terminal rows used for braille (excluding HUD)
    px_w: usize,   // pixel width = cell_w * 2
    px_h: usize,   // pixel height = cell_h * 4
}

fn compute_dims(margin_rows: u16, hud_on: bool) -> io::Result<Dims> {
    let (tw, th) = terminal::size()?;

    let th = th.saturating_sub(margin_rows).max(1);
    let hud_rows = if hud_on { 1 } else { 0 };
    let usable_h = th.saturating_sub(hud_rows).max(1);

    let cell_w = tw as usize;
    let cell_h = usable_h as usize;

    Ok(Dims {
        cell_w,
        cell_h,
        px_w: cell_w * 2,
        px_h: cell_h * 4,
    })
}

fn refit(
    out: &mut Stdout,
    dims: &mut Dims,
    owner: &mut Vec<usize>,
    edge: &mut Vec<bool>,
    margin: &mut Vec<f32>,
    seeds: &mut Vec<Seed>,
    rng: &mut StdRng,
    margin_rows: u16,
    hud_on: bool,
    seeds_n: usize,
) -> io::Result<()> {
    *dims = compute_dims(margin_rows, hud_on)?;

    owner.clear();
    edge.clear();
    margin.clear();

    let n = dims.px_w * dims.px_h;
    owner.resize(n, 0usize);
    edge.resize(n, false);
    margin.resize(n, 0.0);

    *seeds = make_seeds(seeds_n, rng);

    execute!(out, terminal::Clear(ClearType::All))?;
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum RenderMode {
    Edges,
    Fill,
    Gradient,
}
impl RenderMode {
    fn next(self) -> Self {
        match self {
            RenderMode::Edges => RenderMode::Fill,
            RenderMode::Fill => RenderMode::Gradient,
            RenderMode::Gradient => RenderMode::Edges,
        }
    }
    fn name(self) -> &'static str {
        match self {
            RenderMode::Edges => "edges",
            RenderMode::Fill => "fill",
            RenderMode::Gradient => "gradient",
        }
    }
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    let seed_u64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs();
    let mut rng = StdRng::seed_from_u64(seed_u64);

    let mut show_hud = args.hud;
    let mut mode = RenderMode::Edges;

    let mut seeds_n = args.seeds.max(2);
    let mut speed = args.speed.max(0.05);
    let mut ms = args.ms.clamp(10, 250);

    // Gradient threshold (bigger = brighter centers, thinner borders)
    // This is in pixel-space units because margin uses sqrt(dist2) units.
    let mut grad_thresh: f32 = 2.0;

    let mut dims = compute_dims(args.margin_rows, show_hud)?;
    let mut seeds = make_seeds(seeds_n, &mut rng);

    let n = dims.px_w * dims.px_h;
    let mut owner: Vec<usize> = vec![0usize; n];
    let mut edge: Vec<bool> = vec![false; n];
    let mut margin: Vec<f32> = vec![0.0; n];

    let mut out = stdout();
    execute!(
        out,
        EnterAlternateScreen,
        terminal::Clear(ClearType::All),
        cursor::Hide
    )?;
    terminal::enable_raw_mode()?;

    let start = Instant::now();
    let mut last = Instant::now();

    loop {
        if event::poll(Duration::from_millis(1))? {
            match event::read()? {
                Event::Key(k) => match k.code {
                    KeyCode::Char('q') => break,

                    // cycle render modes
                    KeyCode::Char('f') => mode = mode.next(),

                    KeyCode::Char('r') => seeds = make_seeds(seeds_n, &mut rng),

                    KeyCode::Char('h') => {
                        show_hud = !show_hud;
                        refit(
                            &mut out,
                            &mut dims,
                            &mut owner,
                            &mut edge,
                            &mut margin,
                            &mut seeds,
                            &mut rng,
                            args.margin_rows,
                            show_hud,
                            seeds_n,
                        )?;
                    }

                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        seeds_n = (seeds_n + 1).min(200);
                        seeds = make_seeds(seeds_n, &mut rng);
                    }
                    KeyCode::Char('-') => {
                        seeds_n = seeds_n.saturating_sub(1).max(2);
                        seeds = make_seeds(seeds_n, &mut rng);
                    }

                    KeyCode::Char(']') => speed = (speed * 1.1).min(20.0),
                    KeyCode::Char('[') => speed = (speed / 1.1).max(0.05),

                    KeyCode::Char('>') | KeyCode::Char('.') => ms = (ms.saturating_sub(5)).max(10),
                    KeyCode::Char('<') | KeyCode::Char(',') => ms = (ms + 5).min(250),

                    // gradient sensitivity
                    KeyCode::Char('k') => grad_thresh = (grad_thresh + 0.25).min(20.0),
                    KeyCode::Char('j') => grad_thresh = (grad_thresh - 0.25).max(0.0),

                    _ => {}
                },
                Event::Resize(_, _) => {
                    refit(
                        &mut out,
                        &mut dims,
                        &mut owner,
                        &mut edge,
                        &mut margin,
                        &mut seeds,
                        &mut rng,
                        args.margin_rows,
                        show_hud,
                        seeds_n,
                    )?;
                }
                _ => {}
            }
        }

        let dt = Duration::from_millis(ms);
        if last.elapsed() < dt {
            continue;
        }
        last = Instant::now();

        let t = start.elapsed().as_secs_f32() * speed;

        // seed positions
        let mut seed_xy: Vec<(f32, f32)> = Vec::with_capacity(seeds.len());
        for &s in &seeds {
            seed_xy.push(seed_pos_px(s, t, dims.px_w, dims.px_h));
        }

        // owner + margin per pixel
        for y in 0..dims.px_h {
            let py = y as f32;
            let row = y * dims.px_w;
            for x in 0..dims.px_w {
                let px = x as f32;
                let (id, d1, d2) = nearest_two(px, py, &seed_xy);
                owner[row + x] = id;

                // margin: 0 at border, increases toward center of cell
                let m = d2.sqrt() - d1.sqrt();
                margin[row + x] = m.max(0.0);
            }
        }

        // edges only needed for edges mode, but cheap enough to compute always
        for y in 0..dims.px_h {
            for x in 0..dims.px_w {
                let i = y * dims.px_w + x;
                let a = owner[i];
                let mut is_edge = false;

                if x + 1 < dims.px_w && owner[i + 1] != a {
                    is_edge = true;
                } else if x > 0 && owner[i - 1] != a {
                    is_edge = true;
                } else if y + 1 < dims.px_h && owner[i + dims.px_w] != a {
                    is_edge = true;
                } else if y > 0 && owner[i - dims.px_w] != a {
                    is_edge = true;
                }

                edge[i] = is_edge;
            }
        }

        // draw
        queue!(out, cursor::MoveTo(0, 0))?;

        let grid_start_row: u16 = if show_hud { 1 } else { 0 };

        if show_hud {
            let hud = format!(
                "seeds:{:<3}  speed:{:<4.2}  ms:{:<3}  mode:{}  thr:{:<4.2}   (+/- seeds) ([ ] speed) (< > fps) (f mode) (j/k thr) (r regen) (h hud) (q quit)",
                seeds_n, speed, ms, mode.name(), grad_thresh
            );

            let mut line = hud;
            if line.len() < dims.cell_w {
                line.push_str(&" ".repeat(dims.cell_w - line.len()));
            } else if line.len() > dims.cell_w {
                line.truncate(dims.cell_w);
            }

            queue!(
                out,
                SetForegroundColor(Color::DarkGrey),
                SetAttribute(Attribute::Bold),
                Print(line),
                ResetColor,
                SetAttribute(Attribute::Reset),
                Print("\r\n")
            )?;
        }

        // render field
        for cy in 0..dims.cell_h {
            queue!(out, cursor::MoveTo(0, grid_start_row + cy as u16))?;

            let base_y = cy * 4;
            let mut current_color: Option<Color> = None;
            let mut current_attr: Option<Attribute> = None;

            for cx in 0..dims.cell_w {
                let base_x = cx * 2;

                let mut mask = 0u8;

                // majority owner for coloring in this 2x4 block
                let mut counts = [0u8; 16];
                let mut major_id: usize = 0;
                let mut major_ct: u8 = 0;

                // average margin for brightness styling in gradient mode
                let mut m_sum: f32 = 0.0;

                for dy in 0..4 {
                    for dx in 0..2 {
                        let sx = base_x + dx;
                        let sy = base_y + dy;
                        let i = sy * dims.px_w + sx;

                        let on = match mode {
                            RenderMode::Edges => edge[i],
                            RenderMode::Fill => true,
                            RenderMode::Gradient => margin[i] > grad_thresh,
                        };

                        if on {
                            mask |= braille_bit(dx, dy);
                        }

                        let oid = owner[i];
                        if oid < 16 {
                            counts[oid] = counts[oid].saturating_add(1);
                            if counts[oid] > major_ct {
                                major_ct = counts[oid];
                                major_id = oid;
                            }
                        } else {
                            // fallback vote for large IDs
                            if major_ct == 0 {
                                major_id = oid;
                                major_ct = 1;
                            } else if oid == major_id {
                                major_ct = major_ct.saturating_add(1);
                            } else if major_ct > 0 {
                                major_ct -= 1;
                            }
                        }

                        m_sum += margin[i];
                    }
                }

                if mask == 0 {
                    if current_color.is_some() || current_attr.is_some() {
                        queue!(out, ResetColor, SetAttribute(Attribute::Reset))?;
                        current_color = None;
                        current_attr = None;
                    }
                    queue!(out, Print(' '))?;
                    continue;
                }

                // color by majority owner
                let col = palette(major_id);
                if current_color != Some(col) {
                    queue!(out, SetForegroundColor(col))?;
                    current_color = Some(col);
                }

                // in gradient mode, push brightness with Dim/Normal/Bold based on avg margin
                if let RenderMode::Gradient = mode {
                    let m_avg = m_sum / 8.0;
                    let desired_attr = if m_avg < grad_thresh * 1.2 {
                        Attribute::Dim
                    } else if m_avg > grad_thresh * 3.0 {
                        Attribute::Bold
                    } else {
                        Attribute::Reset
                    };

                    if current_attr != Some(desired_attr) {
                        queue!(out, SetAttribute(desired_attr))?;
                        current_attr = Some(desired_attr);
                    }
                } else if current_attr.is_some() {
                    queue!(out, SetAttribute(Attribute::Reset))?;
                    current_attr = None;
                }

                queue!(out, Print(braille_char(mask)))?;
            }

            queue!(out, ResetColor, SetAttribute(Attribute::Reset))?;
        }

        out.flush()?;
    }

    execute!(out, cursor::Show, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    Ok(())
}

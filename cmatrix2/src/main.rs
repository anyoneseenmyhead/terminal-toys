use std::env;

use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, Clear, ClearType, DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::{
    collections::HashMap,
    io::{stdout, Result, Write},
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Column {
    x: u16,
    head_y: i32,
    speed: i32,
    length: i32,
    tick_accum: i32,
}

#[derive(Clone, Copy)]
enum Theme {
    Green,
    Purple,
    Amber,
    Ice,
}

impl Theme {
    fn next(self) -> Theme {
        match self {
            Theme::Green => Theme::Purple,
            Theme::Purple => Theme::Amber,
            Theme::Amber => Theme::Ice,
            Theme::Ice => Theme::Green,
        }
    }
}

#[derive(Clone)]
struct Config {
    frame_ms: u64,      // lower is faster
    density: f64,       // 0..1
    double_chance: f64, // 0..1
    double_enabled: bool,

    tail_draw: i32, // how many cells behind head get fresh shading
    gamma: f32,     // fade curve
    shimmer: bool,

    // Glyph persistence tuning
    head_ttl_min: u8,
    head_ttl_max: u8,
    tail_ttl_min: u8,
    tail_ttl_max: u8,
    shimmer_ttl_min: u8,
    shimmer_ttl_max: u8,
    cache_prune_every: u32, // frames between cache pruning

    // Theme
    theme: Theme,
}

#[derive(Clone, Copy)]
struct GlyphState {
    ch: char,
    ttl: u8,
}

struct GlyphCache {
    map: HashMap<u32, GlyphState>, // packed (x,y) key -> glyph state
}

impl GlyphCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    #[inline]
    fn key(x: u16, y: u16) -> u32 {
        ((x as u32) << 16) | (y as u32)
    }

    fn get_or_roll(
        &mut self,
        x: u16,
        y: u16,
        ttl_min: u8,
        ttl_max: u8,
        rng: &mut StdRng,
    ) -> char {
        let k = Self::key(x, y);

        if let Some(st) = self.map.get_mut(&k) {
            if st.ttl > 0 {
                st.ttl = st.ttl.saturating_sub(1);
                return st.ch;
            }
        }

        // Roll new glyph + TTL
        let ch = rand_glyph(rng);
        let ttl = if ttl_max <= ttl_min {
            ttl_min
        } else {
            rng.gen_range(ttl_min..=ttl_max)
        };

        self.map.insert(k, GlyphState { ch, ttl });
        ch
    }

    fn clear_cell(&mut self, x: u16, y: u16) {
        let k = Self::key(x, y);
        self.map.remove(&k);
    }

    fn prune_outside(&mut self, w_safe: u16, h_safe: u16) {
        let w = w_safe as u32;
        let h = h_safe as u32;

        self.map.retain(|k, _| {
            let x = (k >> 16) & 0xFFFF;
            let y = k & 0xFFFF;
            x < w && y < h
        });
    }
}

fn print_help() {
    println!(
        r#"cmatrix2 — cinematic Matrix-style terminal rain

USAGE:
  cmatrix2 [OPTIONS]

OPTIONS:
  -h, --help        Show this help message
  -V, --version     Show version information

KEYS (runtime):
  q / Esc           Quit
  c                 Cycle color themes
  + / -             Faster / slower
  [ / ]             Less / more density
  d                 Toggle double streams
  s                 Toggle shimmer
  r                 Reseed randomness
"#
    );
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("cmatrix2 {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let mut out = stdout();

    terminal::enable_raw_mode()?;
    execute!(
        out,
        EnterAlternateScreen,
        DisableLineWrap,
        cursor::Hide,
        SetBackgroundColor(Color::Black),
        Clear(ClearType::All)
    )?;

    let result = run(&mut out);

    execute!(
        out,
        ResetColor,
        cursor::Show,
        EnableLineWrap,
        LeaveAlternateScreen
    )?;
    terminal::disable_raw_mode()?;

    result
}

fn run(out: &mut std::io::Stdout) -> Result<()> {
    let mut rng = StdRng::from_entropy();
    let mut cache = GlyphCache::new();

    let mut cfg = Config {
        frame_ms: 40,        // ~25 FPS
        density: 0.75,
        double_chance: 0.30,
        double_enabled: true,

        tail_draw: 26,
        gamma: 1.6,
        shimmer: true,

        head_ttl_min: 0,
        head_ttl_max: 2,
        tail_ttl_min: 6,
        tail_ttl_max: 28,
        shimmer_ttl_min: 3,
        shimmer_ttl_max: 16,
        cache_prune_every: 60,

        theme: Theme::Green,
    };

    let mut last_frame = Instant::now();
    let mut frame_counter: u32 = 0;

    let mut size = terminal::size()?;
    let mut columns = init_columns(size.0, size.1, &mut rng, &cfg);

    loop {
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) => {
                    if handle_key(k.code, &mut cfg, &mut rng, &mut size, &mut columns, out)? {
                        return Ok(());
                    }
                }
                Event::Resize(w, h) => {
                    size = (w, h);
                    columns = init_columns(w, h, &mut rng, &cfg);
                    cache.prune_outside(w.saturating_sub(1), h.saturating_sub(1));
                    queue!(
                        out,
                        SetBackgroundColor(Color::Black),
                        Clear(ClearType::All)
                    )?;
                }
                _ => {}
            }
        }

        let frame_time = Duration::from_millis(cfg.frame_ms);
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(last_frame);
        if elapsed < frame_time {
            std::thread::sleep(frame_time - elapsed);
        }
        last_frame = Instant::now();

        frame_counter = frame_counter.wrapping_add(1);
        if cfg.cache_prune_every > 0 && frame_counter % cfg.cache_prune_every == 0 {
            cache.prune_outside(size.0.saturating_sub(1), size.1.saturating_sub(1));
        }

        step(
            columns.as_mut_slice(),
            size.0,
            size.1,
            &mut rng,
            &cfg,
            &mut cache,
            out,
        )?;
        out.flush()?;
    }
}

fn handle_key(
    code: KeyCode,
    cfg: &mut Config,
    rng: &mut StdRng,
    size: &mut (u16, u16),
    columns: &mut Vec<Column>,
    out: &mut std::io::Stdout,
) -> Result<bool> {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),

        KeyCode::Char('c') => {
            cfg.theme = cfg.theme.next();
        }

        KeyCode::Char('+') | KeyCode::Char('=') => {
            cfg.frame_ms = cfg.frame_ms.saturating_sub(5).max(10);
        }
        KeyCode::Char('-') => {
            cfg.frame_ms = (cfg.frame_ms + 5).min(120);
        }

        KeyCode::Char(']') => {
            cfg.density = (cfg.density + 0.05).min(0.98);
            *columns = init_columns(size.0, size.1, rng, cfg);
            queue!(out, SetBackgroundColor(Color::Black), Clear(ClearType::All))?;
        }
        KeyCode::Char('[') => {
            cfg.density = (cfg.density - 0.05).max(0.05);
            *columns = init_columns(size.0, size.1, rng, cfg);
            queue!(out, SetBackgroundColor(Color::Black), Clear(ClearType::All))?;
        }

        KeyCode::Char('d') => {
            cfg.double_enabled = !cfg.double_enabled;
            *columns = init_columns(size.0, size.1, rng, cfg);
            queue!(out, SetBackgroundColor(Color::Black), Clear(ClearType::All))?;
        }

        KeyCode::Char('s') => {
            cfg.shimmer = !cfg.shimmer;
        }

        KeyCode::Char('r') => {
            *rng = StdRng::from_entropy();
            *columns = init_columns(size.0, size.1, rng, cfg);
            queue!(out, SetBackgroundColor(Color::Black), Clear(ClearType::All))?;
        }

        _ => {}
    }

    Ok(false)
}

fn init_columns(w: u16, h: u16, rng: &mut StdRng, cfg: &Config) -> Vec<Column> {
    let mut cols = Vec::new();
    if w == 0 || h == 0 {
        return cols;
    }

    let w_safe = w.saturating_sub(1);

    for x in 0..w_safe {
        if !rng.gen_bool(cfg.density) {
            continue;
        }

        cols.push(new_column(x, h, rng));

        if cfg.double_enabled && rng.gen_bool(cfg.double_chance) {
            let x2 = if x + 1 < w_safe { x + 1 } else { x };
            cols.push(new_column(x2, h, rng));
        }
    }

    cols
}

fn new_column(x: u16, h: u16, rng: &mut StdRng) -> Column {
    let h_i = h as i32;
    let min_len = (h_i / 6).clamp(10, 20);
    let max_len = (h_i * 3 / 4).clamp(30, 120);

    Column {
        x,
        head_y: rng.gen_range(-h_i..=0),
        speed: rng.gen_range(1..=2),
        length: rng.gen_range(min_len..=max_len),
        tick_accum: 0,
    }
}

fn rand_glyph(rng: &mut StdRng) -> char {
    // 75% Katakana, 15% digits, 8% A-Z, 2% symbols
    let roll: u8 = rng.gen_range(0..=99);

    if roll < 75 {
        let code = rng.gen_range(0x30A0u32..=0x30FFu32);
        char::from_u32(code).unwrap_or('?')
    } else if roll < 90 {
        rng.gen_range(b'0'..=b'9') as char
    } else if roll < 98 {
        rng.gen_range(b'A'..=b'Z') as char
    } else {
        const SYMS: &[u8] = b"@#$%&*+=:/<>-|";
        SYMS[rng.gen_range(0..SYMS.len())] as char
    }
}

fn head_color(theme: Theme) -> Color {
    match theme {
        Theme::Green => Color::Rgb {
            r: 200,
            g: 255,
            b: 200,
        },
        Theme::Purple => Color::Rgb {
            r: 245,
            g: 205,
            b: 255,
        },
        Theme::Amber => Color::Rgb {
            r: 255,
            g: 235,
            b: 180,
        },
        Theme::Ice => Color::Rgb {
            r: 200,
            g: 245,
            b: 255,
        },
    }
}

fn tail_color(theme: Theme, i: i32, max_i: i32, gamma: f32) -> Color {
    let t = (i as f32 / max_i as f32).clamp(0.0, 1.0);
    let fade = (1.0 - t).powf(gamma);
    let intensity = (220.0 * fade + 28.0) as u8;

    match theme {
        Theme::Green => Color::Rgb {
            r: 0,
            g: intensity,
            b: 0,
        },
        Theme::Purple => {
            let r = (intensity as f32 * 0.65) as u8;
            let b = intensity;
            Color::Rgb { r, g: 0, b }
        }
        Theme::Amber => {
            let r = intensity;
            let g = (intensity as f32 * 0.70) as u8;
            Color::Rgb { r, g, b: 0 }
        }
        Theme::Ice => {
            let g = (intensity as f32 * 0.85) as u8;
            let b = intensity;
            Color::Rgb { r: 0, g, b }
        }
    }
}

fn step(
    columns: &mut [Column],
    w: u16,
    h: u16,
    rng: &mut StdRng,
    cfg: &Config,
    cache: &mut GlyphCache,
    out: &mut std::io::Stdout,
) -> Result<()> {
    let w_i = w as i32;
    let h_i = h as i32;

    let drawable_w = (w_i - 1).max(0);
    let drawable_h = (h_i - 1).max(0);

    for col in columns.iter_mut() {
        if col.x as i32 >= drawable_w {
            continue;
        }

        col.tick_accum += col.speed;

        while col.tick_accum > 0 {
            col.tick_accum -= 1;

            let new_head = col.head_y + 1;
            let erase_y = new_head - col.length;

            if erase_y >= 0 && erase_y < drawable_h {
                let y = erase_y as u16;
                cache.clear_cell(col.x, y);
                queue!(
                    out,
                    cursor::MoveTo(col.x, y),
                    SetBackgroundColor(Color::Black),
                    Print(' ')
                )?;
            }

            if new_head >= 0 && new_head < drawable_h {
                let y = new_head as u16;
                let ch = cache.get_or_roll(col.x, y, cfg.head_ttl_min, cfg.head_ttl_max, rng);
                queue!(
                    out,
                    cursor::MoveTo(col.x, y),
                    SetForegroundColor(head_color(cfg.theme)),
                    SetBackgroundColor(Color::Black),
                    Print(ch)
                )?;
            }

            let tail_draw = cfg.tail_draw.max(4);
            for i in 1..=tail_draw {
                let y_i = new_head - i;
                if y_i < 0 || y_i >= drawable_h {
                    continue;
                }

                let y = y_i as u16;
                let ch = cache.get_or_roll(col.x, y, cfg.tail_ttl_min, cfg.tail_ttl_max, rng);

                queue!(
                    out,
                    cursor::MoveTo(col.x, y),
                    SetForegroundColor(tail_color(cfg.theme, i, tail_draw, cfg.gamma)),
                    SetBackgroundColor(Color::Black),
                    Print(ch)
                )?;
            }

            col.head_y = new_head;

            if col.head_y - col.length > drawable_h {
                let min_len = (drawable_h / 6).clamp(10, 20);
                let max_len = (drawable_h * 3 / 4).clamp(30, 120);

                col.head_y = rng.gen_range(-drawable_h..=-1);
                col.speed = rng.gen_range(1..=2);
                col.length = rng.gen_range(min_len..=max_len);
                col.tick_accum = 0;
            }
        }

        if cfg.shimmer && drawable_h > 0 {
            if rng.gen_bool(0.10) {
                let max_dist = (cfg.tail_draw * 2).max(8);
                let dist = rng.gen_range(0..=max_dist);
                let y_i = col.head_y - dist;

                if y_i >= 0 && y_i < drawable_h {
                    let y = y_i as u16;

                    let ch = cache.get_or_roll(
                        col.x,
                        y,
                        cfg.shimmer_ttl_min,
                        cfg.shimmer_ttl_max,
                        rng,
                    );

                    let i = dist.max(1).min(cfg.tail_draw.max(2));
                    queue!(
                        out,
                        cursor::MoveTo(col.x, y),
                        SetForegroundColor(tail_color(cfg.theme, i, cfg.tail_draw.max(2), cfg.gamma)),
                        SetBackgroundColor(Color::Black),
                        Print(ch)
                    )?;
                }
            }
        }
    }

    Ok(())
}

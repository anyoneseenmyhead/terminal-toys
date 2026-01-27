use clap::Parser;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::{
    io::{self, stdout, Write},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BIRTH_1: u8 = 3; // HighLife: B36/S23
const BIRTH_2: u8 = 6;
const SURVIVE_1: u8 = 2;
const SURVIVE_2: u8 = 3;

#[derive(Clone, Copy)]
struct Cell {
    species: u8, // 0 none, 1 A, 2 B
    age: u8,     // 0 dead, 1..=age_max alive
    decay: u8,   // 0 none, 1..=decay_max
}
impl Cell {
    fn empty() -> Self {
        Self { species: 0, age: 0, decay: 0 }
    }
    fn alive(&self) -> bool {
        self.age > 0
    }
}

struct World {
    w: usize,
    h: usize,
    tick: u64,
    age_max: u8,
    decay_max: u8,
    noise_period: u64,
    noise_cells: usize,
    grid: Vec<Cell>,
    rng: StdRng,
}

impl World {
    fn new(w: usize, h: usize, seed: u64) -> Self {
        let mut world = Self {
            w,
            h,
            tick: 0,
            age_max: 18,
            decay_max: 10,
            noise_period: 320,
            noise_cells: 4,
            grid: vec![Cell::empty(); w * h],
            rng: StdRng::seed_from_u64(seed),
        };
        world.seed_random((w * h) / 7);
        world
    }

    fn idx(&self, x: usize, y: usize) -> usize {
        y * self.w + x
    }

    fn clear(&mut self) {
        self.grid.fill(Cell::empty());
        self.tick = 0;
    }

    fn seed_random(&mut self, n: usize) {
        for _ in 0..n {
            let x = self.rng.gen_range(0..self.w);
            let y = self.rng.gen_range(0..self.h);
            let s = if self.rng.gen_bool(0.5) { 1 } else { 2 };
            let i = self.idx(x, y);
            self.grid[i] = Cell { species: s, age: 1, decay: 0 };
        }
    }

    fn neighbors(&self, x: usize, y: usize) -> (u8, u8, u8) {
        let mut total = 0u8;
        let mut a = 0u8;
        let mut b = 0u8;

        for dy in [-1isize, 0, 1] {
            for dx in [-1isize, 0, 1] {
                if dx == 0 && dy == 0 { continue; }
                let nx = (x as isize + dx + self.w as isize) % self.w as isize;
                let ny = (y as isize + dy + self.h as isize) % self.h as isize;
                let c = self.grid[self.idx(nx as usize, ny as usize)];
                if c.alive() {
                    total += 1;
                    if c.species == 1 { a += 1; }
                    if c.species == 2 { b += 1; }
                }
            }
        }
        (total, a, b)
    }

    fn step(&mut self) {
        let mut next = self.grid.clone();

        for y in 0..self.h {
            for x in 0..self.w {
                let i = self.idx(x, y);
                let c = self.grid[i];
                let (n, na, nb) = self.neighbors(x, y);

                if c.alive() {
                    if n == SURVIVE_1 || n == SURVIVE_2 {
                        next[i] = Cell {
                            species: c.species,
                            age: (c.age + 1).min(self.age_max),
                            decay: 0,
                        };
                    } else {
                        next[i] = Cell {
                            species: c.species,
                            age: 0,
                            decay: self.decay_max,
                        };
                    }
                } else {
                    let decay = c.decay.saturating_sub(1);

                    if n == BIRTH_1 || n == BIRTH_2 {
                        let s = if na > nb { 1 }
                        else if nb > na { 2 }
                        else if self.rng.gen_bool(0.5) { 1 } else { 2 };

                        next[i] = Cell { species: s, age: 1, decay: 0 };
                    } else if decay > 0 {
                        next[i] = Cell { species: c.species, age: 0, decay };
                    } else {
                        next[i] = Cell::empty();
                    }
                }
            }
        }

        self.grid = next;
        self.tick += 1;

        if self.noise_period > 0 && self.tick % self.noise_period == 0 {
            self.seed_random(self.noise_cells);
        }
    }
}

// Braille mapping: each terminal cell represents a 2x4 block of "pixels".
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

#[derive(Parser)]
struct Args {
    /// milliseconds between simulation steps
    #[arg(long, default_value_t = 45)]
    ms: u64,

    /// extra safety rows to not use (avoid scrolling)
    #[arg(long, default_value_t = 1)]
    margin_rows: u16,
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let (tw, th) = terminal::size()?;
    let usable_h = th.saturating_sub(args.margin_rows).max(1);

    // Sim grid is 2x width and 4x height of terminal cells.
    let sim_w = (tw as usize) * 2;
    let sim_h = (usable_h as usize) * 4;

    let mut world = World::new(sim_w, sim_h, seed);
    let mut out = stdout();

    execute!(out, EnterAlternateScreen, terminal::Clear(ClearType::All), cursor::Hide)?;
    terminal::enable_raw_mode()?;

    let dt = Duration::from_millis(args.ms.max(10));
    let mut last = Instant::now();

    // Render dims in terminal cells
    let cell_w = tw as usize;
    let cell_h = usable_h as usize;

    // Main loop
    loop {
        // input
        if event::poll(Duration::from_millis(1))? {
            match event::read()? {
                Event::Key(k) => match k.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('r') => world.seed_random(80), // spores
                    KeyCode::Char('c') => { world.clear(); world.seed_random((world.w * world.h)/7); }
                    _ => {}
                },
                // If you resize, we keep it simple: exit and relaunch for a perfect refit.
                // You can remove this exit if you prefer.
                Event::Resize(_, _) => break,
                _ => {}
            }
        }

        if last.elapsed() < dt {
            continue;
        }
        last = Instant::now();

        world.step();

        // draw
        queue!(out, cursor::MoveTo(0, 0))?;

        let mut current_color: Option<Color> = None;

        for cy in 0..cell_h {
            for cx in 0..cell_w {
                let mut mask = 0u8;

                let mut alive_count = 0u8;
                let mut a_count = 0u8;
                let mut b_count = 0u8;

                // Build mask from 2x4 block in sim-grid
                let base_x = cx * 2;
                let base_y = cy * 4;

                for dy in 0..4 {
                    for dx in 0..2 {
                        let sx = base_x + dx;
                        let sy = base_y + dy;
                        let c = world.grid[world.idx(sx, sy)];
                        if c.alive() {
                            mask |= braille_bit(dx, dy);
                            alive_count += 1;
                            if c.species == 1 { a_count += 1; }
                            if c.species == 2 { b_count += 1; }
                        }
                    }
                }

                // Choose color per braille-cell by majority species
                let desired = if mask == 0 {
                    Color::Black
                } else if a_count > b_count {
                    Color::Green
                } else if b_count > a_count {
                    Color::Blue
                } else {
                    Color::White
                };

                if current_color != Some(desired) {
                    queue!(out, SetForegroundColor(desired))?;
                    current_color = Some(desired);
                }

                let ch = braille_char(mask);
                queue!(out, Print(ch))?;
            }

            // newline
            queue!(out, ResetColor, Print("\r\n"))?;
            current_color = None;
        }

        queue!(out, ResetColor)?;
        out.flush()?;
    }

    execute!(out, cursor::Show, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    Ok(())
}

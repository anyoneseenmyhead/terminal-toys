use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use crate::ai;
use crate::util::splitmix64;

pub const WALL_CELL: u16 = u16::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coord {
    pub x: i32,
    pub y: i32,
}

impl Coord {
    pub fn offset(self, dir: Dir) -> Self {
        let (dx, dy) = dir.delta();
        Self {
            x: self.x + dx,
            y: self.y + dy,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];

    pub fn delta(self) -> (i32, i32) {
        match self {
            Dir::Up => (0, -1),
            Dir::Down => (0, 1),
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
        }
    }

    pub fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundOutcome {
    Victory,
    Defeat,
}

#[derive(Clone, Debug)]
pub struct SimConfig {
    pub cycles: usize,
    pub margin: u16,
    pub no_reverse: bool,
    pub strict_sim: bool,
    pub allow_reverse_if_only_safe: bool,
    pub screensaver: bool,
}

#[derive(Clone, Debug)]
pub struct Cycle {
    pub id: u16,
    pub alive: bool,
    pub pos: Coord,
    pub prev_pos: Coord,
    pub dir: Dir,
    pub pending_dir: Option<Dir>,
    pub color_idx: usize,
    pub is_player: bool,
    pub wall_charges: u8,
    pub drop_wall_pending: bool,
}

#[derive(Clone, Debug)]
pub struct Grid {
    pub width: i32,
    pub height: i32,
    pub cells: Vec<u16>,
}

impl Grid {
    pub fn new(width: i32, height: i32) -> Self {
        let len = (width * height).max(0) as usize;
        Self {
            width,
            height,
            cells: vec![0; len],
        }
    }

    pub fn in_bounds(&self, c: Coord) -> bool {
        c.x >= 0 && c.y >= 0 && c.x < self.width && c.y < self.height
    }

    fn idx(&self, c: Coord) -> usize {
        (c.y * self.width + c.x) as usize
    }

    pub fn get(&self, c: Coord) -> u16 {
        if self.in_bounds(c) {
            self.cells[self.idx(c)]
        } else {
            WALL_CELL
        }
    }

    pub fn set(&mut self, c: Coord, v: u16) {
        if self.in_bounds(c) {
            let idx = self.idx(c);
            self.cells[idx] = v;
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Playfield {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Playfield {
    pub fn to_terminal(self, local: Coord) -> Coord {
        Coord {
            x: local.x + self.x,
            y: local.y + self.y,
        }
    }
}

#[derive(Debug)]
pub struct Round {
    pub playfield: Playfield,
    pub grid: Grid,
    pub cycles: Vec<Cycle>,
    pub strict_sim: bool,
    pub no_reverse: bool,
    pub allow_reverse_if_only_safe: bool,
    pub screensaver: bool,
    rng: SmallRng,
}

#[derive(Debug)]
pub struct TickResult {
    pub outcome: Option<RoundOutcome>,
}

impl Round {
    pub fn new(config: &SimConfig, term_w: u16, term_h: u16, seed: u64) -> Option<Self> {
        let playfield = compute_playfield(term_w, term_h, config.margin)?;
        let grid = Grid::new(playfield.width, playfield.height);

        let mut spawn_rng = SmallRng::seed_from_u64(splitmix64(seed ^ 0x1000_0001));
        let spawns = generate_spawns(
            playfield.width,
            playfield.height,
            config.cycles,
            &mut spawn_rng,
        );
        if spawns.len() < config.cycles {
            return None;
        }

        let mut cycles = Vec::with_capacity(config.cycles);
        for i in 0..config.cycles {
            let is_player = i == 0 && !config.screensaver;
            cycles.push(Cycle {
                id: (i + 1) as u16,
                alive: true,
                pos: spawns[i],
                prev_pos: spawns[i],
                dir: initial_dir_for_spawn(spawns[i], playfield.width, playfield.height),
                pending_dir: None,
                color_idx: i,
                is_player,
                wall_charges: if is_player { 3 } else { 0 },
                drop_wall_pending: false,
            });
        }

        Some(Self {
            playfield,
            grid,
            cycles,
            strict_sim: config.strict_sim,
            no_reverse: config.no_reverse,
            allow_reverse_if_only_safe: config.allow_reverse_if_only_safe,
            screensaver: config.screensaver,
            rng: SmallRng::seed_from_u64(splitmix64(seed ^ 0xCAFE_BABE)),
        })
    }

    pub fn alive_count(&self) -> usize {
        self.cycles.iter().filter(|c| c.alive).count()
    }

    pub fn player_alive(&self) -> bool {
        match self.cycles.first() {
            Some(c) if c.is_player => c.alive,
            _ => true,
        }
    }

    pub fn ai_alive_count(&self) -> usize {
        self.cycles
            .iter()
            .filter(|c| c.alive && !c.is_player)
            .count()
    }

    pub fn queue_player_dir(&mut self, dir: Option<Dir>) {
        if self.screensaver {
            return;
        }
        if let (Some(cycle), Some(d)) = (self.cycles.get_mut(0), dir) {
            cycle.pending_dir = Some(d);
        }
    }

    pub fn set_player_start_dir(&mut self, dir: Dir) {
        if self.screensaver {
            return;
        }
        if let Some(cycle) = self.cycles.get_mut(0)
            && cycle.alive
            && cycle.is_player
        {
            cycle.dir = dir;
            cycle.pending_dir = None;
        }
    }

    pub fn queue_drop_wall(&mut self) {
        if self.screensaver {
            return;
        }
        if let Some(cycle) = self.cycles.get_mut(0)
            && cycle.alive
            && cycle.wall_charges > 0
        {
            cycle.drop_wall_pending = true;
        }
    }

    pub fn tick(&mut self) -> TickResult {
        self.apply_ai_intents();
        self.apply_pending_turns();
        self.apply_optional_wall_drops();

        let mut intents: Vec<Option<Coord>> = vec![None; self.cycles.len()];
        for (i, c) in self.cycles.iter().enumerate() {
            if c.alive {
                intents[i] = Some(c.pos.offset(c.dir));
            }
        }

        let mut dead = vec![false; self.cycles.len()];

        // Wall/trail collisions against already occupied grid.
        for (i, target) in intents.iter().enumerate() {
            if let Some(t) = target
                && self.grid.get(*t) != 0
            {
                dead[i] = true;
            }
        }

        // Multiple claimants to same target.
        for i in 0..intents.len() {
            if intents[i].is_none() {
                continue;
            }
            for j in (i + 1)..intents.len() {
                if intents[i] == intents[j] {
                    dead[i] = true;
                    dead[j] = true;
                }
            }
        }

        if self.strict_sim {
            // Strict head swap collision.
            for i in 0..intents.len() {
                let Some(ti) = intents[i] else { continue };
                for j in (i + 1)..intents.len() {
                    let Some(tj) = intents[j] else { continue };
                    if ti == self.cycles[j].pos && tj == self.cycles[i].pos {
                        dead[i] = true;
                        dead[j] = true;
                    }
                }
            }
        }

        // Commit survivors.
        for (i, maybe_target) in intents.into_iter().enumerate() {
            let Some(target) = maybe_target else { continue };
            let cycle = &mut self.cycles[i];
            if !cycle.alive {
                continue;
            }
            if dead[i] {
                cycle.alive = false;
                continue;
            }
            self.grid.set(cycle.pos, cycle.id);
            cycle.prev_pos = cycle.pos;
            cycle.pos = target;
        }

        let outcome = if !self.player_alive() {
            Some(RoundOutcome::Defeat)
        } else if self.ai_alive_count() == 0 {
            Some(RoundOutcome::Victory)
        } else {
            None
        };

        TickResult { outcome }
    }

    fn apply_ai_intents(&mut self) {
        for i in 0..self.cycles.len() {
            if !self.cycles[i].alive || self.cycles[i].is_player {
                continue;
            }
            let seed: u64 = self.rng.random();
            let mut local_rng = SmallRng::seed_from_u64(seed);
            let next = ai::choose_dir(self, i, &mut local_rng);
            self.cycles[i].pending_dir = Some(next);
        }
    }

    fn apply_pending_turns(&mut self) {
        for i in 0..self.cycles.len() {
            if !self.cycles[i].alive {
                continue;
            }
            let Some(desired) = self.cycles[i].pending_dir.take() else {
                continue;
            };
            if self.is_turn_allowed(i, desired) {
                self.cycles[i].dir = desired;
            }
        }
    }

    fn is_turn_allowed(&self, idx: usize, desired: Dir) -> bool {
        let c = &self.cycles[idx];
        if desired == c.dir {
            return true;
        }
        if self.no_reverse && desired == c.dir.opposite() {
            if !self.allow_reverse_if_only_safe {
                return false;
            }
            let mut safe_non_reverse_exists = false;
            for d in Dir::ALL {
                if d == c.dir.opposite() {
                    continue;
                }
                let p = c.pos.offset(d);
                if self.grid.get(p) == 0 {
                    safe_non_reverse_exists = true;
                    break;
                }
            }
            if safe_non_reverse_exists {
                return false;
            }
        }
        true
    }

    fn apply_optional_wall_drops(&mut self) {
        for i in 0..self.cycles.len() {
            let cycle = &self.cycles[i];
            if !cycle.alive || !cycle.drop_wall_pending || cycle.wall_charges == 0 {
                continue;
            }
            let side = match cycle.dir {
                Dir::Up | Dir::Down => [Dir::Left, Dir::Right, cycle.dir.opposite()],
                Dir::Left | Dir::Right => [Dir::Up, Dir::Down, cycle.dir.opposite()],
            };
            let mut placed = None;
            for d in side {
                let p = cycle.pos.offset(d);
                if self.grid.get(p) == 0 {
                    placed = Some(p);
                    break;
                }
            }
            if let Some(p) = placed {
                self.grid.set(p, WALL_CELL);
                if let Some(c) = self.cycles.get_mut(i) {
                    c.wall_charges = c.wall_charges.saturating_sub(1);
                    c.drop_wall_pending = false;
                }
            } else if let Some(c) = self.cycles.get_mut(i) {
                c.drop_wall_pending = false;
            }
        }
    }
}

fn compute_playfield(term_w: u16, term_h: u16, margin: u16) -> Option<Playfield> {
    let tw = i32::from(term_w);
    let th = i32::from(term_h);
    let m = i32::from(margin);
    let hud_rows = 1;

    let width = tw - 2 * m;
    let height = th - hud_rows - 2 * m;
    if width < 12 || height < 8 {
        return None;
    }

    Some(Playfield {
        x: m,
        y: hud_rows + m,
        width,
        height,
    })
}

fn generate_spawns(width: i32, height: i32, cycles: usize, rng: &mut SmallRng) -> Vec<Coord> {
    let mut candidates = Vec::new();
    let cx = width / 2;
    let cy = height / 2;
    let radii = [
        (width.min(height) / 3).max(3),
        (width.min(height) / 4).max(2),
        (width.min(height) / 5).max(2),
    ];

    for &r in &radii {
        for &(dx, dy) in &[
            (r, 0),
            (-r, 0),
            (0, r),
            (0, -r),
            (r, r),
            (r, -r),
            (-r, r),
            (-r, -r),
        ] {
            let x = (cx + dx).clamp(1, width - 2);
            let y = (cy + dy).clamp(1, height - 2);
            candidates.push(Coord { x, y });
        }
    }

    // Add random fallback candidates.
    for _ in 0..(cycles * 8).max(16) {
        candidates.push(Coord {
            x: rng.random_range(1..(width - 1)),
            y: rng.random_range(1..(height - 1)),
        });
    }

    // Deterministic shuffle.
    for i in (1..candidates.len()).rev() {
        let j = rng.random_range(0..=i);
        candidates.swap(i, j);
    }

    let mut spawns: Vec<Coord> = Vec::new();
    let min_dist2 = 16_i32;
    'outer: for c in candidates {
        for s in &spawns {
            let dx = c.x - s.x;
            let dy = c.y - s.y;
            if dx * dx + dy * dy < min_dist2 {
                continue 'outer;
            }
        }
        spawns.push(c);
        if spawns.len() >= cycles {
            break;
        }
    }

    spawns
}

fn initial_dir_for_spawn(spawn: Coord, width: i32, height: i32) -> Dir {
    let cx = width / 2;
    let cy = height / 2;
    let dx = spawn.x - cx;
    let dy = spawn.y - cy;
    if dx.abs() > dy.abs() {
        if dx > 0 { Dir::Right } else { Dir::Left }
    } else if dy > 0 {
        Dir::Down
    } else {
        Dir::Up
    }
}

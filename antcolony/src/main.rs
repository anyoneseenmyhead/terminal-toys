use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::{
    io::{self, Stdout, Write},
    time::{Duration, Instant},
};

const INITIAL_ANTS: usize = 150;
const MAX_ANTS: usize = 900;

// Pheromones - control evaporation, diffusion, deposit behavior and max level.
// EVAP: fraction of pheromone lost per tick (0.0..1.0). Higher -> fades faster.
// DIFF: fraction of neighbouring pheromone mixed in during diffusion (0.0..1.0).
// DEPOSIT_FIND: amount deposited by ants that are searching (home-marking).
// DEPOSIT_RETURN: amount deposited by ants carrying food (food-marking) — typically larger to strengthen return trails.
// PHER_CAP: maximum pheromone value stored per cell to avoid runaway accumulation.
const EVAP: f32 = 0.011;
const DIFF: f32 = 0.120;
const DEPOSIT_FIND: f32 = 2.8; // “home” trail while searching
const DEPOSIT_RETURN: f32 = 3.4; // “food” trail while carrying
const PHER_CAP: f32 = 90.0;

// Movement
const TURN_JITTER: f32 = 0.28;
const SENSOR_DIST: i32 = 3;
const SENSOR_ANGLE_DEG: i32 = 35;
const SENSOR_NOISE: f32 = 0.12;

// Food
const FOOD_TOTAL: u16 = 1500;
const FOOD_PATCHES: usize = 4;
const FOOD_PATCH_RADIUS: i32 = 5;

// Ground + digging
const GROUND_FILL_PCT: f32 = 0.78; // 0..1, higher = more solid
const DIG_CHANCE: f32 = 0.22; // chance to dig when blocked
const DIG_COST: i32 = 4;

// Lifecycle (ticks)
const EGG_TICKS: u32 = 110;
const LARVA_TICKS: u32 = 170;
const PUPA_TICKS: u32 = 210;
const ADULT_LIFESPAN_TICKS: u32 = 32_000; // was 12_000

// Energy
const ENERGY_START: i32 = 420; // was 220
const ENERGY_MOVE_COST: i32 = 1; // keep
const ENERGY_CARRY_COST: i32 = 1; // was 2
const ENERGY_EAT_AT_QUEEN: i32 = 240; // was 90

// Queen / reproduction
const FOOD_VALUE_AT_QUEEN: u64 = 10; // NEW: each delivered food becomes this many colony_food
const SPAWN_COST_FOOD: u64 = 12; // was 25
const QUEEN_LAY_COOLDOWN_TICKS: u32 = 25; // was 60

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Egg,
    Larva,
    Pupa,
    Adult,
}

#[derive(Clone, Copy)]
struct Ant {
    x: i32,
    y: i32,
    dir: i32, // 0..7
    carrying: bool,
    age: u32,
    energy: i32,
    phase: Phase,
}

struct World {
    w: usize,
    h: usize,

    queen: (i32, i32),

    // Terrain: 0 empty tunnel, 1 ground
    terrain: Vec<u8>,

    food: Vec<u16>, // food units in cell (can be embedded; collectible only if tunnel)
    food_patch_id: Vec<i16>,
    food_patch_remaining: Vec<u32>,
    food_patch_active: Vec<bool>,
    colony_food: u64,
    queen_cooldown: u32,

    ants: Vec<Ant>,

    ph_home: Vec<f32>,
    ph_food: Vec<f32>,
    ph_home_tmp: Vec<f32>,
    ph_food_tmp: Vec<f32>,

    occ: Vec<u16>,
    frame: String,
}

fn idx(w: usize, x: usize, y: usize) -> usize {
    y * w + x
}

fn wrap_i32(v: i32, max: i32) -> i32 {
    let mut t = v % max;
    if t < 0 {
        t += max;
    }
    t
}

fn wrap_xy(w: usize, h: usize, x: i32, y: i32) -> (usize, usize) {
    (
        wrap_i32(x, w as i32) as usize,
        wrap_i32(y, h as i32) as usize,
    )
}

fn in_bounds(w: usize, h: usize, x: i32, y: i32) -> bool {
    x >= 0 && y >= 0 && x < w as i32 && y < h as i32
}

fn dir_vec(dir: i32) -> (i32, i32) {
    match dir.rem_euclid(8) {
        0 => (1, 0),
        1 => (1, 1),
        2 => (0, 1),
        3 => (-1, 1),
        4 => (-1, 0),
        5 => (-1, -1),
        6 => (0, -1),
        _ => (1, -1),
    }
}

fn rotate_dir(dir: i32, delta: i32) -> i32 {
    (dir + delta).rem_euclid(8)
}

fn sense(
    w: usize,
    h: usize,
    field: &[f32],
    terrain: &[u8],
    x: i32,
    y: i32,
    dir: i32,
    angle_deg: i32,
    dist: i32,
    wrap: bool,
    rng: &mut StdRng,
) -> f32 {
    let steps = ((angle_deg as f32) / 45.0).round() as i32;
    let sdir = rotate_dir(dir, steps);
    let (dx, dy) = dir_vec(sdir);
    let sx = x + dx * dist;
    let sy = y + dy * dist;
    if !wrap && !in_bounds(w, h, sx, sy) {
        return 0.0;
    }
    let (ux, uy) = wrap_xy(w, h, sx, sy);
    let i = idx(w, ux, uy);

    // Don’t “smell” through solid ground
    if terrain[i] == 1 {
        return 0.0;
    }

    let base = field[i];
    let noise = (rng.gen::<f32>() - 0.5) * SENSOR_NOISE * base.max(1.0);
    base + noise
}

fn diffuse_and_evap(w: usize, h: usize, terrain: &[u8], src: &[f32], dst: &mut [f32]) {
    for y in 0..h {
        for x in 0..w {
            let i0 = idx(w, x, y);

            // Keep pheromones mostly in tunnels
            if terrain[i0] == 1 {
                dst[i0] = (src[i0] * (1.0 - EVAP)).max(0.0);
                continue;
            }

            let mut sum = 0.0;
            let mut n = 0.0;
            for oy in [-1i32, 0, 1] {
                for ox in [-1i32, 0, 1] {
                    let (ux, uy) = wrap_xy(w, h, x as i32 + ox, y as i32 + oy);
                    let ii = idx(w, ux, uy);
                    if terrain[ii] == 0 {
                        sum += src[ii];
                        n += 1.0;
                    }
                }
            }
            let avg = if n > 0.0 { sum / n } else { 0.0 };
            let cur = src[i0];
            let mixed = cur * (1.0 - DIFF) + avg * DIFF;
            dst[i0] = (mixed * (1.0 - EVAP)).max(0.0);
        }
    }
}

fn add_pher(
    w: usize,
    h: usize,
    terrain: &[u8],
    field: &mut [f32],
    x: i32,
    y: i32,
    amt: f32,
    wrap: bool,
) {
    if !wrap && !in_bounds(w, h, x, y) {
        return;
    }
    let (ux, uy) = wrap_xy(w, h, x, y);
    let i = idx(w, ux, uy);
    if terrain[i] == 0 {
        field[i] = (field[i] + amt).min(PHER_CAP);
    }
}

fn pad_or_trunc(s: &str, width: usize) -> String {
    let mut out = s.to_string();
    if out.len() > width {
        out.truncate(width);
    } else if out.len() < width {
        out.extend(std::iter::repeat(' ').take(width - out.len()));
    }
    out
}

fn carve_room(terrain: &mut [u8], w: usize, h: usize, cx: i32, cy: i32, r: i32) {
    for oy in -r..=r {
        for ox in -r..=r {
            if ox * ox + oy * oy > r * r {
                continue;
            }
            let (ux, uy) = wrap_xy(w, h, cx + ox, cy + oy);
            terrain[idx(w, ux, uy)] = 0;
        }
    }
}

fn make_world(rng: &mut StdRng, w: usize, h: usize) -> World {
    let w = w.max(40);
    let h = h.max(18);

    let queen = (w as i32 / 2, h as i32 / 2);

    let mut terrain = vec![1u8; w * h];
    for i in 0..terrain.len() {
        terrain[i] = if rng.gen::<f32>() < GROUND_FILL_PCT {
            1
        } else {
            0
        };
    }

    // Ensure a central chamber for queen and initial tunnels
    carve_room(&mut terrain, w, h, queen.0, queen.1, 6);
    // Carve some starter branches
    for _ in 0..6 {
        let mut x = queen.0;
        let mut y = queen.1;
        let steps = rng.gen_range(30..90);
        let mut dir = rng.gen_range(0..8) as i32;
        for _ in 0..steps {
            if rng.gen::<f32>() < 0.25 {
                dir = rotate_dir(dir, rng.gen_range(-1..=1));
            }
            let (dx, dy) = dir_vec(dir);
            x = wrap_i32(x + dx, w as i32);
            y = wrap_i32(y + dy, h as i32);
            let (ux, uy) = wrap_xy(w, h, x, y);
            terrain[idx(w, ux, uy)] = 0;
        }
    }

    let mut food = vec![0u16; w * h];
    let mut food_patch_id = vec![-1i16; w * h];
    let mut food_patch_remaining = Vec::with_capacity(FOOD_PATCHES);
    let mut food_patch_active = Vec::with_capacity(FOOD_PATCHES);

    // Place food patches, some embedded in ground so ants must tunnel into them.
    let per_patch = (FOOD_TOTAL / (FOOD_PATCHES as u16)).max(1);
    let mut placed = 0usize;
    let mut attempts = 0usize;
    while placed < FOOD_PATCHES && attempts < 4000 {
        attempts += 1;
        let fx = rng.gen_range(0..w as i32);
        let fy = rng.gen_range(0..h as i32);
        let dx = fx - queen.0;
        let dy = fy - queen.1;
        if (dx * dx + dy * dy) < 280 {
            continue;
        }

        // Optional small chamber nearby (not fully open) so it feels like a pocket
        if rng.gen::<f32>() < 0.55 {
            carve_room(&mut terrain, w, h, fx, fy, 2);
        }

        let patch_id = placed as i16;
        let mut patch_total: u32 = 0;
        for oy in -FOOD_PATCH_RADIUS..=FOOD_PATCH_RADIUS {
            for ox in -FOOD_PATCH_RADIUS..=FOOD_PATCH_RADIUS {
                if ox * ox + oy * oy > FOOD_PATCH_RADIUS * FOOD_PATCH_RADIUS {
                    continue;
                }
                let (ux, uy) = wrap_xy(w, h, fx + ox, fy + oy);
                let i = idx(w, ux, uy);
                food[i] = food[i].saturating_add(per_patch);
                food_patch_id[i] = patch_id;
                patch_total = patch_total.saturating_add(per_patch as u32);
            }
        }
        food_patch_remaining.push(patch_total);
        food_patch_active.push(true);
        placed += 1;
    }

    let mut ants = Vec::with_capacity(MAX_ANTS.min(INITIAL_ANTS * 2));
    for _ in 0..INITIAL_ANTS {
        ants.push(Ant {
            x: queen.0 + rng.gen_range(-1..=1),
            y: queen.1 + rng.gen_range(-1..=1),
            dir: rng.gen_range(0..8) as i32,
            carrying: false,
            age: rng.gen_range(0..500) as u32,
            energy: ENERGY_START,
            phase: Phase::Adult,
        });
    }

    World {
        w,
        h,
        queen,
        terrain,
        food,
        colony_food: 0,
        queen_cooldown: 0,
        ants,
        ph_home: vec![0.0; w * h],
        ph_food: vec![0.0; w * h],
        ph_home_tmp: vec![0.0; w * h],
        ph_food_tmp: vec![0.0; w * h],
        occ: vec![0u16; w * h],
        frame: String::new(),
        food_patch_id,
        food_patch_remaining,
        food_patch_active,
    }
}

fn try_dig(world: &mut World, x: i32, y: i32, dir: i32, wrap: bool) -> bool {
    let (dx, dy) = dir_vec(dir);
    let nx = x + dx;
    let ny = y + dy;
    if !wrap && !in_bounds(world.w, world.h, nx, ny) {
        return false;
    }
    let (ux, uy) = wrap_xy(world.w, world.h, nx, ny);
    let i = idx(world.w, ux, uy);
    if world.terrain[i] == 1 {
        world.terrain[i] = 0;
        return true;
    }
    false
}

fn update_phase(ant: &mut Ant) {
    ant.phase = if ant.age < EGG_TICKS {
        Phase::Egg
    } else if ant.age < EGG_TICKS + LARVA_TICKS {
        Phase::Larva
    } else if ant.age < EGG_TICKS + LARVA_TICKS + PUPA_TICKS {
        Phase::Pupa
    } else {
        Phase::Adult
    };
}

fn tick(world: &mut World, rng: &mut StdRng, wrap: bool) {
    // Queen lays eggs based on colony food
    if world.queen_cooldown > 0 {
        world.queen_cooldown -= 1;
    } else if world.colony_food >= SPAWN_COST_FOOD && world.ants.len() < MAX_ANTS {
        world.colony_food -= SPAWN_COST_FOOD;
        world.queen_cooldown = QUEEN_LAY_COOLDOWN_TICKS;

        world.ants.push(Ant {
            x: world.queen.0,
            y: world.queen.1,
            dir: rng.gen_range(0..8) as i32,
            carrying: false,
            age: 0,
            energy: ENERGY_START,
            phase: Phase::Egg,
        });
    }

    // Pheromones
    diffuse_and_evap(
        world.w,
        world.h,
        &world.terrain,
        &world.ph_home,
        &mut world.ph_home_tmp,
    );
    diffuse_and_evap(
        world.w,
        world.h,
        &world.terrain,
        &world.ph_food,
        &mut world.ph_food_tmp,
    );
    std::mem::swap(&mut world.ph_home, &mut world.ph_home_tmp);
    std::mem::swap(&mut world.ph_food, &mut world.ph_food_tmp);

    // Keep a strong “home” source at queen chamber
    add_pher(
        world.w,
        world.h,
        &world.terrain,
        &mut world.ph_home,
        world.queen.0,
        world.queen.1,
        6.0,
        wrap,
    );

    // Ants update
    let qx = world.queen.0;
    let qy = world.queen.1;
    let (uqx, uqy) = wrap_xy(world.w, world.h, qx, qy);
    let queen_i = idx(world.w, uqx, uqy);

    // Iterate and keep only living ants
    let mut write = 0usize;
    for read in 0..world.ants.len() {
        let mut a = world.ants[read];

        a.age = a.age.saturating_add(1);
        update_phase(&mut a);

        // Death rules
        let adult_age = a.age.saturating_sub(EGG_TICKS + LARVA_TICKS + PUPA_TICKS);
        if a.phase == Phase::Adult && adult_age > ADULT_LIFESPAN_TICKS {
            continue;
        }
        if a.phase == Phase::Adult && a.energy <= 0 {
            continue;
        }

        // Non-adults stay at queen location (brood pile)
        if a.phase != Phase::Adult {
            a.x = qx;
            a.y = qy;
            a.carrying = false;
            a.dir = rotate_dir(a.dir, rng.gen_range(-1..=1));
            world.ants[write] = a;
            write += 1;
            continue;
        }

        // If at queen and carrying, deposit food to colony and refuel
        if !wrap && !in_bounds(world.w, world.h, a.x, a.y) {
            continue;
        }
        let (ux, uy) = wrap_xy(world.w, world.h, a.x, a.y);
        let ai = idx(world.w, ux, uy);
        if ai == queen_i {
            if a.carrying {
                a.carrying = false;
                world.colony_food = world.colony_food.saturating_add(FOOD_VALUE_AT_QUEEN);

                a.energy = (a.energy + ENERGY_EAT_AT_QUEEN).min(ENERGY_START);
            } else if a.energy < ENERGY_START / 3 && world.colony_food > 0 {
                // nibble colony stock if very low
                world.colony_food -= 1;
                a.energy = (a.energy + ENERGY_EAT_AT_QUEEN / 2).min(ENERGY_START);
            }
        }

        // Pick up food if on tunnel cell with food
        if !a.carrying && world.terrain[ai] == 0 && world.food[ai] > 0 {
            world.food[ai] -= 1;
            a.carrying = true;
            let patch_id = world.food_patch_id[ai];
            if patch_id >= 0 {
                let pid = patch_id as usize;
                if let Some(remaining) = world.food_patch_remaining.get_mut(pid) {
                    if *remaining > 0 {
                        *remaining -= 1;
                        if *remaining == 0 {
                            if let Some(active) = world.food_patch_active.get_mut(pid) {
                                *active = false;
                            }
                        }
                    }
                }
            }
        }

        // Choose pheromone target
        let follow = if a.carrying {
            &world.ph_home
        } else {
            &world.ph_food
        };

        let f = sense(
            world.w,
            world.h,
            follow,
            &world.terrain,
            a.x,
            a.y,
            a.dir,
            0,
            SENSOR_DIST,
            wrap,
            rng,
        );
        let l = sense(
            world.w,
            world.h,
            follow,
            &world.terrain,
            a.x,
            a.y,
            a.dir,
            -SENSOR_ANGLE_DEG,
            SENSOR_DIST,
            wrap,
            rng,
        );
        let r = sense(
            world.w,
            world.h,
            follow,
            &world.terrain,
            a.x,
            a.y,
            a.dir,
            SENSOR_ANGLE_DEG,
            SENSOR_DIST,
            wrap,
            rng,
        );

        let mut choice = if l > f && l > r {
            -1
        } else if r > f && r > l {
            1
        } else {
            0
        };

        if rng.gen::<f32>() < TURN_JITTER {
            choice = match rng.gen_range(0..3) {
                0 => -1,
                1 => 0,
                _ => 1,
            };
        }

        a.dir = rotate_dir(a.dir, choice);

        // Attempt move; dig if blocked by ground
        let (dx, dy) = dir_vec(a.dir);
        let nx = a.x + dx;
        let ny = a.y + dy;
        if !wrap && !in_bounds(world.w, world.h, nx, ny) {
            a.dir = rotate_dir(a.dir, if rng.gen::<bool>() { 1 } else { -1 });
            world.ants[write] = a;
            write += 1;
            continue;
        }
        let (nux, nuy) = wrap_xy(world.w, world.h, nx, ny);
        let ni = idx(world.w, nux, nuy);

        if world.terrain[ni] == 0 {
            if wrap {
                a.x = wrap_i32(nx, world.w as i32);
                a.y = wrap_i32(ny, world.h as i32);
            } else {
                a.x = nx;
                a.y = ny;
            }
            a.energy -= if a.carrying {
                ENERGY_CARRY_COST
            } else {
                ENERGY_MOVE_COST
            };
        } else {
            // blocked
            if rng.gen::<f32>() < DIG_CHANCE && a.energy > DIG_COST + 5 {
                if try_dig(world, a.x, a.y, a.dir, wrap) {
                    a.energy -= DIG_COST;
                }
            } else {
                // bounce
                a.dir = rotate_dir(a.dir, if rng.gen::<bool>() { 1 } else { -1 });
            }
        }

        // Deposit pheromones (only in tunnels)
        if a.carrying {
            add_pher(
                world.w,
                world.h,
                &world.terrain,
                &mut world.ph_food,
                a.x,
                a.y,
                DEPOSIT_RETURN,
                wrap,
            );
        } else {
            add_pher(
                world.w,
                world.h,
                &world.terrain,
                &mut world.ph_home,
                a.x,
                a.y,
                DEPOSIT_FIND,
                wrap,
            );
        }

        world.ants[write] = a;
        write += 1;
    }
    world.ants.truncate(write);
}

fn draw(
    out: &mut Stdout,
    world: &mut World,
    paused: bool,
    step_ms: u64,
    term_cols: usize,
    term_rows: usize,
    wrap: bool,
) -> io::Result<()> {
    world.occ.fill(0);
    for a in &world.ants {
        if a.phase != Phase::Adult {
            continue;
        }
        if !wrap && !in_bounds(world.w, world.h, a.x, a.y) {
            continue;
        }
        let (ux, uy) = wrap_xy(world.w, world.h, a.x, a.y);
        world.occ[idx(world.w, ux, uy)] = world.occ[idx(world.w, ux, uy)].saturating_add(1);
    }

    let ramp: [char; 10] = [' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];

    let (uqx, uqy) = wrap_xy(world.w, world.h, world.queen.0, world.queen.1);
    let queen_i = idx(world.w, uqx, uqy);

    let sim_rows = world.h.min(term_rows.saturating_sub(3));
    let sim_cols = world.w.min(term_cols);

    queue!(out, BeginSynchronizedUpdate)?;
    queue!(out, cursor::MoveTo(0, 0))?;
    let mut cur_color: Option<Color> = None;
    for row in 0..term_rows {
        if row < sim_rows {
            let y = row;
            for x in 0..sim_cols {
                let i = idx(world.w, x, y);

                let patch_id = world.food_patch_id[i];
                let patch_depleted = if patch_id >= 0 {
                    !world.food_patch_active.get(patch_id as usize).copied().unwrap_or(true)
                } else {
                    false
                };

                let (ch, color) = if i == queen_i {
                    ('Q', Color::Yellow)
                } else if world.occ[i] > 0 {
                    if world.occ[i] >= 4 {
                        ('A', Color::Red)
                    } else {
                        ('a', Color::DarkRed)
                    }
                } else if patch_depleted {
                    ('#', Color::DarkYellow)
                } else if world.terrain[i] == 1 {
                    ('#', Color::DarkYellow)
                } else if world.food[i] > 0 {
                    ('F', Color::Green)
                } else {
                    // pheromone intensity in tunnels
                    let hval = world.ph_home[i];
                    let fval = world.ph_food[i];
                    let val = if fval > hval { fval } else { hval };
                    let t = (val / PHER_CAP).clamp(0.0, 1.0);
                    let ri = (t * (ramp.len() as f32 - 1.0)).round() as usize;
                    let color = if fval > hval {
                        Color::Cyan
                    } else if hval > 0.0 {
                        Color::Blue
                    } else {
                        Color::DarkGrey
                    };
                    (ramp[ri], color)
                };

                if cur_color != Some(color) {
                    queue!(out, SetForegroundColor(color))?;
                    cur_color = Some(color);
                }
                queue!(out, Print(ch))?;
            }
            if sim_cols < term_cols {
                if cur_color.is_some() {
                    queue!(out, ResetColor)?;
                    cur_color = None;
                }
                queue!(
                    out,
                    Print(std::iter::repeat(' ').take(term_cols - sim_cols).collect::<String>())
                )?;
            }
        } else if row == sim_rows {
            if cur_color.is_some() {
                queue!(out, ResetColor)?;
                cur_color = None;
            }
            queue!(
                out,
                Print(pad_or_trunc(
                    &format!(
                        "q quit | r reset | w wrap {} | space pause | +/- speed | ants {} | colony_food {} | step {} ms | {}",
                        if wrap { "ON" } else { "OFF" },
                        world.ants.len(),
                        world.colony_food,
                        step_ms,
                        if paused { "PAUSED" } else { "RUNNING" }
                    ),
                    term_cols,
                ))
            )?;
        } else if row == sim_rows + 1 {
            if cur_color.is_some() {
                queue!(out, ResetColor)?;
                cur_color = None;
            }
            // brood count
            let mut egg = 0usize;
            let mut larva = 0usize;
            let mut pupa = 0usize;
            let mut adult = 0usize;
            for a in &world.ants {
                match a.phase {
                    Phase::Egg => egg += 1,
                    Phase::Larva => larva += 1,
                    Phase::Pupa => pupa += 1,
                    Phase::Adult => adult += 1,
                }
            }
            queue!(
                out,
                Print(pad_or_trunc(
                    &format!(
                        "Brood: egg {} | larva {} | pupa {} | adult {} | spawn_cost {} food",
                        egg, larva, pupa, adult, SPAWN_COST_FOOD
                    ),
                    term_cols,
                ))
            )?;
        } else if row == sim_rows + 2 {
            if cur_color.is_some() {
                queue!(out, ResetColor)?;
                cur_color = None;
            }
            queue!(
                out,
                Print(pad_or_trunc(
                    "Legend: Q queen, # ground, F food (collect in tunnels), a/A ants, background = pheromone | w wrap",
                    term_cols,
                ))
            )?;
        } else {
            if cur_color.is_some() {
                queue!(out, ResetColor)?;
                cur_color = None;
            }
            queue!(
                out,
                Print(std::iter::repeat(' ').take(term_cols).collect::<String>())
            )?;
        }

        if row + 1 < term_rows {
            queue!(out, Print("\r\n"))?;
        }
    }
    queue!(out, ResetColor)?;
    queue!(out, EndSynchronizedUpdate)?;
    out.flush()?;
    Ok(())
}

fn main() -> io::Result<()> {
    let mut out = io::stdout();

    execute!(out, EnterAlternateScreen, cursor::Hide)?;
    terminal::enable_raw_mode()?;
    execute!(out, DisableLineWrap)?;

    let mut rng = StdRng::from_entropy();

    let (cols_u16, rows_u16) = terminal::size()?;
    let mut term_cols = cols_u16 as usize;
    let mut term_rows = rows_u16 as usize;

    let mut world = make_world(
        &mut rng,
        term_cols.max(40),
        term_rows.saturating_sub(3).max(18),
    );

    let mut paused = false;
    let mut step_ms: u64 = 45;
    let mut last_tick = Instant::now();
    let mut wrap_enabled = false;

    'outer: loop {
        if let Ok((c, r)) = terminal::size() {
            let nc = c as usize;
            let nr = r as usize;
            if nc != term_cols || nr != term_rows {
                term_cols = nc;
                term_rows = nr;
                world = make_world(
                    &mut rng,
                    term_cols.max(40),
                    term_rows.saturating_sub(3).max(18),
                );
            }
        }

        while event::poll(Duration::from_millis(0))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => break 'outer,
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        world = make_world(
                            &mut rng,
                            term_cols.max(40),
                            term_rows.saturating_sub(3).max(18),
                        );
                    }
                    KeyCode::Char(' ') => paused = !paused,
                    KeyCode::Char('w') | KeyCode::Char('W') => wrap_enabled = !wrap_enabled,
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        step_ms = step_ms.saturating_sub(5).max(5)
                    }
                    KeyCode::Char('-') => step_ms = (step_ms + 5).min(250),
                    _ => {}
                }
            }
        }

        if last_tick.elapsed() >= Duration::from_millis(step_ms) {
            last_tick = Instant::now();
            if !paused {
                tick(&mut world, &mut rng, wrap_enabled);
            }
            draw(
                &mut out,
                &mut world,
                paused,
                step_ms,
                term_cols,
                term_rows,
                wrap_enabled,
            )?;
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    execute!(out, EnableLineWrap)?;
    terminal::disable_raw_mode()?;
    execute!(out, LeaveAlternateScreen, cursor::Show)?;
    Ok(())
}

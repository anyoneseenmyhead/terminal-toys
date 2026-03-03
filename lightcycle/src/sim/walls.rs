use rand::Rng;
use rand::rngs::SmallRng;

use super::{Coord, Grid, WALL_CELL, WallMode};

pub fn generate_walls(
    grid: &mut Grid,
    spawns: &[Coord],
    mode: WallMode,
    density: f32,
    rng: &mut SmallRng,
) {
    match mode {
        WallMode::Off => {}
        WallMode::Pillars => gen_pillars(grid, spawns, density, rng),
        WallMode::Maze => gen_maze_lite(grid, spawns, density, rng),
    }
}

fn gen_pillars(grid: &mut Grid, spawns: &[Coord], density: f32, rng: &mut SmallRng) {
    let area = (grid.width * grid.height).max(1) as f32;
    let target_fill = (area * density.clamp(0.02, 0.65)) as i32;
    let mut filled = 0_i32;
    let mut attempts = 0;

    while filled < target_fill && attempts < 4000 {
        attempts += 1;

        let w = rng.random_range(2..=5).min(grid.width - 3).max(1);
        let h = rng.random_range(1..=4).min(grid.height - 3).max(1);
        if grid.width - w - 2 <= 1 || grid.height - h - 2 <= 1 {
            break;
        }

        let x = rng.random_range(1..(grid.width - w - 1));
        let y = rng.random_range(1..(grid.height - h - 1));

        if !rect_is_valid(grid, spawns, x, y, w, h, 1, 4) {
            continue;
        }

        for py in y..(y + h) {
            for px in x..(x + w) {
                let p = Coord { x: px, y: py };
                if grid.get(p) == 0 {
                    grid.set(p, WALL_CELL);
                    filled += 1;
                }
            }
        }
    }
}

fn gen_maze_lite(grid: &mut Grid, spawns: &[Coord], density: f32, rng: &mut SmallRng) {
    let area = (grid.width * grid.height).max(1) as f32;
    let target_fill = (area * density.clamp(0.05, 0.45)) as i32;
    let mut filled = 0_i32;

    let segments = ((area / 12.0) * density.clamp(0.05, 0.6)).round() as i32 + 8;
    for _ in 0..segments {
        if filled >= target_fill {
            break;
        }

        let horizontal = rng.random_bool(0.5);
        let len_max = if horizontal { grid.width } else { grid.height };
        if len_max < 8 {
            continue;
        }
        let len = rng.random_range(3..=(len_max / 4).max(3));

        let sx = rng.random_range(1..(grid.width - 1));
        let sy = rng.random_range(1..(grid.height - 1));

        for i in 0..len {
            let p = if horizontal {
                Coord { x: sx + i, y: sy }
            } else {
                Coord { x: sx, y: sy + i }
            };

            if !grid.in_bounds(p)
                || p.x <= 0
                || p.y <= 0
                || p.x >= grid.width - 1
                || p.y >= grid.height - 1
            {
                break;
            }
            if !point_valid_for_wall(grid, spawns, p, 4) {
                continue;
            }
            if grid.get(p) == 0 {
                grid.set(p, WALL_CELL);
                filled += 1;
            }
        }

        if rng.random_bool(0.22) {
            let jx = (sx + len / 2).clamp(1, grid.width - 3);
            let jy = sy.clamp(1, grid.height - 3);
            for y in jy..=(jy + 1) {
                for x in jx..=(jx + 1) {
                    let p = Coord { x, y };
                    if point_valid_for_wall(grid, spawns, p, 4) && grid.get(p) == 0 {
                        grid.set(p, WALL_CELL);
                        filled += 1;
                    }
                }
            }
        }
    }
}

fn rect_is_valid(
    grid: &Grid,
    spawns: &[Coord],
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    spacing: i32,
    spawn_clearance: i32,
) -> bool {
    let min_x = (x - spacing).max(0);
    let min_y = (y - spacing).max(0);
    let max_x = (x + w + spacing).min(grid.width - 1);
    let max_y = (y + h + spacing).min(grid.height - 1);

    for py in min_y..=max_y {
        for px in min_x..=max_x {
            if grid.get(Coord { x: px, y: py }) != 0 {
                return false;
            }
        }
    }

    for s in spawns {
        let nearest_x = s.x.clamp(x, x + w - 1);
        let nearest_y = s.y.clamp(y, y + h - 1);
        let dx = s.x - nearest_x;
        let dy = s.y - nearest_y;
        if dx * dx + dy * dy <= spawn_clearance * spawn_clearance {
            return false;
        }
    }

    true
}

fn point_valid_for_wall(grid: &Grid, spawns: &[Coord], p: Coord, spawn_clearance: i32) -> bool {
    if p.x <= 0 || p.y <= 0 || p.x >= grid.width - 1 || p.y >= grid.height - 1 {
        return false;
    }
    for s in spawns {
        let dx = p.x - s.x;
        let dy = p.y - s.y;
        if dx * dx + dy * dy <= spawn_clearance * spawn_clearance {
            return false;
        }
    }
    true
}

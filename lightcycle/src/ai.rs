use std::collections::VecDeque;

use rand::Rng;
use rand::rngs::SmallRng;

use crate::sim::{Coord, Dir, Round};

pub fn choose_dir(round: &Round, idx: usize, rng: &mut SmallRng) -> Dir {
    let c = &round.cycles[idx];
    let mut candidates = Vec::with_capacity(4);

    for d in Dir::ALL {
        if round.no_reverse && d == c.dir.opposite() {
            continue;
        }
        let target = c.pos.offset(d);
        if round.grid.get(target) != 0 {
            continue;
        }
        candidates.push(d);
    }

    if candidates.is_empty() {
        for d in Dir::ALL {
            let target = c.pos.offset(d);
            if round.grid.get(target) == 0 {
                candidates.push(d);
            }
        }
    }

    if candidates.is_empty() {
        return c.dir;
    }

    let mut best = candidates[0];
    let mut best_score = i32::MIN;
    let bfs_cap = ((round.grid.width * round.grid.height) / 5).clamp(200, 600) as usize;

    for d in candidates {
        let headroom = flood_score(round, c.pos.offset(d), bfs_cap);
        let straight_bias = if d == c.dir { 10 } else { 0 };
        let noise = rng.random_range(-8..=8);
        let score = headroom + straight_bias + noise;
        if score > best_score {
            best_score = score;
            best = d;
        }
    }

    best
}

fn flood_score(round: &Round, start: Coord, cap: usize) -> i32 {
    if round.grid.get(start) != 0 {
        return -10_000;
    }

    let mut seen = vec![false; (round.grid.width * round.grid.height) as usize];
    let mut queue = VecDeque::with_capacity(cap.min(128));
    queue.push_back(start);
    let mut count = 0_i32;

    while let Some(p) = queue.pop_front() {
        let idx = (p.y * round.grid.width + p.x) as usize;
        if seen[idx] {
            continue;
        }
        seen[idx] = true;
        count += 1;
        if count as usize >= cap {
            break;
        }

        for d in Dir::ALL {
            let n = p.offset(d);
            if !round.grid.in_bounds(n) {
                continue;
            }
            if round.grid.get(n) != 0 {
                continue;
            }
            let ni = (n.y * round.grid.width + n.x) as usize;
            if !seen[ni] {
                queue.push_back(n);
            }
        }
    }

    count
}

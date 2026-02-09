/// Guard AI — BFS pathfinding using terrain + occupancy.
///
/// Two modes:
///   1. **Chase** — normal BFS toward player (default).
///   2. **Separation** — move away from nearest guard to avoid clustering.
///      Activated when `guard.separation_timer > 0`.
///
/// Terrain = what the cell IS (passable, climbable, etc.)
/// Occupancy = who is there (trapped guard blocks entry, provides support)

use std::collections::VecDeque;

use super::entity::{ActorState, Guard};
use super::physics;
use super::tile::Tile;

const BFS_MAX_DEPTH: usize = 300;
const DIRS: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];

/// How many ticks guards spend in separation mode after contact.
pub const SEPARATION_TICKS: u32 = 10;

/// Context for physics queries (hole_grid for O(1) lookup).
struct Ctx<'a> {
    tiles: &'a [Vec<Tile>],
    width: usize,
    height: usize,
    hole_grid: &'a [Vec<bool>],
    guards: &'a [Guard],
}

impl<'a> Ctx<'a> {
    fn terrain(&self, x: usize, y: usize) -> physics::TerrainCell {
        physics::terrain_at(self.tiles, self.width, self.height, self.hole_grid, x, y)
    }

    fn support(&self, x: usize, y: usize) -> bool {
        physics::has_support(self.tiles, self.width, self.height, self.hole_grid, self.guards, x, y)
    }

    fn can_enter(&self, x: usize, y: usize) -> bool {
        self.terrain(x, y).passable
    }
}

// ── Chase mode (normal) ──

pub fn find_direction(
    tiles: &[Vec<Tile>],
    width: usize,
    height: usize,
    hole_grid: &[Vec<bool>],
    guards: &[Guard],
    gx: usize, gy: usize,
    gstate: ActorState,
    px: usize, py: usize,
) -> (i32, i32) {
    if gstate == ActorState::InHole || gstate == ActorState::Dead { return (0, 0); }
    if gx == px && gy == py { return (0, 0); }

    let ctx = Ctx { tiles, width, height, hole_grid, guards };
    let mut visited = vec![vec![false; width]; height];
    visited[gy][gx] = true;

    let mut queue: VecDeque<(usize, usize, i32, i32)> = VecDeque::with_capacity(256);

    for &(dx, dy) in &DIRS {
        if let Some((nx, ny)) = try_move(&ctx, gx, gy, dx, dy) {
            if nx == px && ny == py { return (dx, dy); }
            if !visited[ny][nx] {
                visited[ny][nx] = true;
                queue.push_back((nx, ny, dx, dy));
            }
        }
    }

    let mut steps = 0;
    while let Some((cx, cy, fdx, fdy)) = queue.pop_front() {
        steps += 1;
        if steps > BFS_MAX_DEPTH { break; }

        if !ctx.support(cx, cy) {
            if cy + 1 < height && ctx.can_enter(cx, cy + 1) && !visited[cy + 1][cx] {
                if cx == px && cy + 1 == py { return (fdx, fdy); }
                visited[cy + 1][cx] = true;
                queue.push_back((cx, cy + 1, fdx, fdy));
            }
            continue;
        }

        for &(dx, dy) in &DIRS {
            if let Some((nx, ny)) = try_move(&ctx, cx, cy, dx, dy) {
                if !visited[ny][nx] {
                    if nx == px && ny == py { return (fdx, fdy); }
                    visited[ny][nx] = true;
                    queue.push_back((nx, ny, fdx, fdy));
                }
            }
        }
    }

    fallback_chase(&ctx, gx, gy, px, py)
}

// ── Separation mode ──

/// Find a direction that moves AWAY from the nearest other guard.
/// Uses a simple scoring approach: try each legal direction and pick the
/// one that maximizes distance from the nearest active guard.
/// Falls back to the normal chase direction if no separation move helps.
pub fn find_separation_direction(
    tiles: &[Vec<Tile>],
    width: usize,
    height: usize,
    hole_grid: &[Vec<bool>],
    guards: &[Guard],
    guard_idx: usize,
    gx: usize, gy: usize,
    gstate: ActorState,
    px: usize, py: usize,
) -> (i32, i32) {
    if gstate == ActorState::InHole || gstate == ActorState::Dead { return (0, 0); }

    let ctx = Ctx { tiles, width, height, hole_grid, guards };

    // Find nearest active guard (not self)
    let mut nearest_dist = i32::MAX;
    let mut nearest_x = gx;
    let mut nearest_y = gy;
    for (j, other) in guards.iter().enumerate() {
        if j == guard_idx { continue; }
        if other.state == ActorState::Dead || other.state == ActorState::InHole { continue; }
        let dist = (other.x as i32 - gx as i32).abs() + (other.y as i32 - gy as i32).abs();
        if dist < nearest_dist {
            nearest_dist = dist;
            nearest_x = other.x;
            nearest_y = other.y;
        }
    }

    // If no nearby guard found, chase normally
    if nearest_dist > 3 {
        return find_direction(tiles, width, height, hole_grid, guards, gx, gy, gstate, px, py);
    }

    // Try each direction: pick the one that maximizes distance from nearest guard
    // while still generally moving toward the player
    let current_guard_dist = manhattan(gx, gy, nearest_x, nearest_y);
    let mut best_dir: (i32, i32) = (0, 0);
    let mut best_score: i32 = i32::MIN;

    for &(dx, dy) in &DIRS {
        if let Some((nx, ny)) = try_move(&ctx, gx, gy, dx, dy) {
            let guard_dist = manhattan(nx, ny, nearest_x, nearest_y);
            let player_dist = manhattan(nx, ny, px, py);
            let current_player_dist = manhattan(gx, gy, px, py);

            let separation_gain = guard_dist - current_guard_dist;
            let player_gain = current_player_dist - player_dist;

            let score = separation_gain * 10 + player_gain;
            if score > best_score {
                best_score = score;
                best_dir = (dx, dy);
            }
        }
    }

    if best_dir == (0, 0) {
        return find_direction(tiles, width, height, hole_grid, guards, gx, gy, gstate, px, py);
    }

    best_dir
}

fn manhattan(x1: usize, y1: usize, x2: usize, y2: usize) -> i32 {
    (x1 as i32 - x2 as i32).abs() + (y1 as i32 - y2 as i32).abs()
}

// ── Shared helpers ──

fn try_move(ctx: &Ctx, x: usize, y: usize, dx: i32, dy: i32) -> Option<(usize, usize)> {
    let nx = x as i32 + dx;
    let ny = y as i32 + dy;
    if nx < 0 || ny < 0 { return None; }
    let nx = nx as usize;
    let ny = ny as usize;
    if nx >= ctx.width || ny >= ctx.height { return None; }

    if !ctx.can_enter(nx, ny) { return None; }

    let here = ctx.terrain(x, y);

    // Up: must be on climbable
    if dy < 0 && !here.climbable { return None; }

    // Down: must be on climbable/hangable or above a ladder
    if dy > 0 {
        if y + 1 < ctx.height {
            let below = ctx.terrain(x, y + 1);
            if !here.climbable && !here.hangable && !below.climbable {
                if ctx.support(x, y) { return None; }
            }
        }
    }

    // Horizontal: must have support
    if dx != 0 && !ctx.support(x, y) { return None; }

    Some((nx, ny))
}

fn fallback_chase(ctx: &Ctx, gx: usize, gy: usize, px: usize, py: usize) -> (i32, i32) {
    let dx = if px > gx { 1 } else if px < gx { -1 } else { 0 };
    if dx != 0 {
        let nx = (gx as i32 + dx) as usize;
        if nx < ctx.width && ctx.can_enter(nx, gy) {
            return (dx, 0);
        }
    }
    let here = ctx.terrain(gx, gy);
    if here.climbable {
        let dy = if py > gy { 1 } else if py < gy { -1 } else { 0 };
        if dy != 0 {
            let ny = (gy as i32 + dy) as usize;
            if ny < ctx.height && ctx.can_enter(gx, ny) {
                return (0, dy);
            }
        }
    }
    (0, 0)
}

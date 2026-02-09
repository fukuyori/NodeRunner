/// Unified physics layer — single source of truth.
///
/// ## Architecture
///
/// Two distinct concepts:
///   1. TERRAIN  — what the cell IS (tile + hole state)
///   2. OCCUPANCY — who is IN the cell (guards)
///
/// These are queried separately. Movement = terrain.passable && !occupied.
/// Support = terrain support || trapped guard below.
///
/// ## Hole Grid (O(1) lookup)
///
/// Holes are tracked in a boolean grid (`hole_grid[y][x]`) rather than
/// a list of positions. This gives O(1) terrain_at queries instead of O(n).
///
/// ## Support Specification
///
/// An actor has SUPPORT (will not fall) if ANY of:
///   - Standing on a ladder (climbable tile at current position)
///   - Hanging on a rope (hangable tile at current position)
///   - Solid tile directly below (brick, concrete, trap brick)
///   - Climbable tile directly below (top of ladder = supported)
///   - At the bottom row of the map
///   - [Player only] Standing guard below (head-walking)
///   - [Guard only] Trapped guard below (acts as floor), excludes self
///
/// An actor FALLS if:
///   - None of the above support conditions are met
///   - Actor is not Dead or InHole

use super::tile::Tile;
use super::entity::{ActorState, Guard};

// ══════════════════════════════════════════════════════════════
// Layer 1: Terrain (tile + hole — NO entities)
// ══════════════════════════════════════════════════════════════

/// What the terrain looks like at a cell (entities excluded).
#[derive(Clone, Copy, Debug)]
pub struct TerrainCell {
    /// Can an entity occupy this cell? (terrain-wise)
    pub passable: bool,
    /// Can an entity climb here?
    pub climbable: bool,
    /// Can an entity hang here?
    pub hangable: bool,
    /// Is this an open hole?
    pub hole: bool,
}

/// Query terrain at (x, y). Considers tiles and hole_grid only.
/// Holes override the tile (a dug brick becomes passable empty space).
///
/// `hole_grid` is a 2D boolean grid: `true` = active hole at that cell.
/// O(1) lookup instead of linear scan.
#[inline]
pub fn terrain_at(
    tiles: &[Vec<Tile>],
    width: usize,
    height: usize,
    hole_grid: &[Vec<bool>],
    x: usize,
    y: usize,
) -> TerrainCell {
    if x >= width || y >= height {
        return TerrainCell { passable: false, climbable: false, hangable: false, hole: false };
    }

    // O(1) hole check
    if y < hole_grid.len() && x < hole_grid[y].len() && hole_grid[y][x] {
        return TerrainCell { passable: true, climbable: false, hangable: false, hole: true };
    }

    let tile = tiles[y][x];
    TerrainCell {
        passable: tile.is_passable(),
        climbable: tile.is_climbable(),
        hangable: tile.is_hangable(),
        hole: false,
    }
}

/// Does terrain alone provide support at (x, y)?
///
/// Support sources (terrain-only, no entities):
///   - On a ladder or rope
///   - Solid or climbable tile below
///   - Bottom of map
#[inline]
pub fn terrain_support(
    tiles: &[Vec<Tile>],
    width: usize,
    height: usize,
    hole_grid: &[Vec<bool>],
    x: usize,
    y: usize,
) -> bool {
    if y + 1 >= height { return true; }

    let here = terrain_at(tiles, width, height, hole_grid, x, y);
    if here.climbable || here.hangable { return true; }

    let below = terrain_at(tiles, width, height, hole_grid, x, y + 1);
    if !below.passable || below.climbable { return true; }

    false
}

// ══════════════════════════════════════════════════════════════
// Layer 2: Occupancy (who is in a cell)
// ══════════════════════════════════════════════════════════════

/// Is there a trapped (InHole) guard at (x, y)?
pub fn has_trapped_guard(guards: &[Guard], x: usize, y: usize) -> bool {
    guards.iter().any(|g| g.x == x && g.y == y && g.state == ActorState::InHole)
}

/// Is there a trapped guard at (x, y), excluding guard at index `skip`?
pub fn has_trapped_guard_except(guards: &[Guard], x: usize, y: usize, skip: usize) -> bool {
    guards.iter().enumerate().any(|(j, g)| {
        j != skip && g.x == x && g.y == y && g.state == ActorState::InHole
    })
}

/// Is there an active (non-Dead, non-InHole) guard at (x, y)?
#[allow(dead_code)]
pub fn has_active_guard(guards: &[Guard], x: usize, y: usize) -> bool {
    guards.iter().any(|g| {
        g.x == x && g.y == y
        && g.state != ActorState::Dead
        && g.state != ActorState::InHole
    })
}

/// Is there any non-dead, non-falling guard at (x, y)?
/// These guards act as solid floor for the player (head-walking).
pub fn has_standing_guard(guards: &[Guard], x: usize, y: usize) -> bool {
    guards.iter().any(|g| {
        g.x == x && g.y == y
        && g.state != ActorState::Dead
        && g.state != ActorState::Falling
    })
}

/// Is there an active guard at (x, y), excluding guard at index `skip`?
pub fn has_active_guard_except(guards: &[Guard], x: usize, y: usize, skip: usize) -> bool {
    guards.iter().enumerate().any(|(j, g)| {
        j != skip && g.x == x && g.y == y
        && g.state != ActorState::Dead
        && g.state != ActorState::InHole
    })
}

// ══════════════════════════════════════════════════════════════
// Combined queries (terrain + occupancy)
// ══════════════════════════════════════════════════════════════

/// Full support check: terrain support OR trapped guard below acting as floor.
pub fn has_support(
    tiles: &[Vec<Tile>],
    width: usize,
    height: usize,
    hole_grid: &[Vec<bool>],
    guards: &[Guard],
    x: usize,
    y: usize,
) -> bool {
    if terrain_support(tiles, width, height, hole_grid, x, y) {
        return true;
    }
    // Trapped guard directly below = floor
    if y + 1 < height && has_trapped_guard(guards, x, y + 1) {
        return true;
    }
    false
}

/// Player-specific support: terrain + ANY standing guard below.
/// In original Lode Runner, the player can walk on enemies' heads.
/// Standing = not dead, not falling (InHole counts — trapped guard is solid).
pub fn has_support_for_player(
    tiles: &[Vec<Tile>],
    width: usize,
    height: usize,
    hole_grid: &[Vec<bool>],
    guards: &[Guard],
    x: usize,
    y: usize,
) -> bool {
    if terrain_support(tiles, width, height, hole_grid, x, y) {
        return true;
    }
    // Any standing guard below acts as floor for the player
    if y + 1 < height && has_standing_guard(guards, x, y + 1) {
        return true;
    }
    false
}

/// Full support check for a specific guard (excludes self from trapped check).
pub fn has_support_for_guard(
    tiles: &[Vec<Tile>],
    width: usize,
    height: usize,
    hole_grid: &[Vec<bool>],
    guards: &[Guard],
    x: usize,
    y: usize,
    guard_idx: usize,
) -> bool {
    if terrain_support(tiles, width, height, hole_grid, x, y) {
        return true;
    }
    if y + 1 < height && has_trapped_guard_except(guards, x, y + 1, guard_idx) {
        return true;
    }
    false
}

/// Resolve actor state using terrain + occupancy.
///
/// State transition rules:
///   Dead / InHole → unchanged (handled by timers)
///   On climbable  → OnLadder
///   On hangable   → OnRope
///   Has support   → OnGround
///   Otherwise     → Falling
pub fn resolve_state(
    tiles: &[Vec<Tile>],
    width: usize,
    height: usize,
    hole_grid: &[Vec<bool>],
    guards: &[Guard],
    x: usize,
    y: usize,
    current: ActorState,
) -> ActorState {
    if current == ActorState::Dead || current == ActorState::InHole {
        return current;
    }

    let here = terrain_at(tiles, width, height, hole_grid, x, y);
    if here.climbable { return ActorState::OnLadder; }
    if here.hangable { return ActorState::OnRope; }
    if has_support(tiles, width, height, hole_grid, guards, x, y) {
        return ActorState::OnGround;
    }

    ActorState::Falling
}

// ══════════════════════════════════════════════════════════════
// Hole grid construction
// ══════════════════════════════════════════════════════════════

/// Build a boolean grid from a list of Hole entities.
/// `true` at (x, y) means there's an active hole there.
pub fn build_hole_grid(holes: &[super::entity::Hole], width: usize, height: usize) -> Vec<Vec<bool>> {
    let mut grid = vec![vec![false; width]; height];
    for h in holes {
        if h.x < width && h.y < height && h.is_active() {
            grid[h.y][h.x] = true;
        }
    }
    grid
}

// ══════════════════════════════════════════════════════════════
// Unit tests
// ══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entity::{Facing, Guard};
    use crate::domain::tile::Tile;

    fn tiles_from(rows: &[&str]) -> (Vec<Vec<Tile>>, usize, usize) {
        let h = rows.len();
        let w = rows[0].len();
        let mut t = vec![vec![Tile::Empty; w]; h];
        for (y, row) in rows.iter().enumerate() {
            for (x, ch) in row.chars().enumerate() {
                t[y][x] = match ch {
                    '#' => Tile::Brick,
                    '=' => Tile::Concrete,
                    'H' => Tile::Ladder,
                    '-' => Tile::Rope,
                    _   => Tile::Empty,
                };
            }
        }
        (t, w, h)
    }

    fn empty_grid(w: usize, h: usize) -> Vec<Vec<bool>> {
        vec![vec![false; w]; h]
    }

    fn hole_grid_at(w: usize, h: usize, holes: &[(usize, usize)]) -> Vec<Vec<bool>> {
        let mut g = empty_grid(w, h);
        for &(x, y) in holes { g[y][x] = true; }
        g
    }

    fn guard_at(id: usize, x: usize, y: usize, state: ActorState) -> Guard {
        let mut g = Guard::new(id, x, y);
        g.state = state;
        g
    }

    // ── terrain_at ──

    #[test]
    fn terrain_brick_is_impassable() {
        let (t, w, h) = tiles_from(&["#"]);
        let tc = terrain_at(&t, w, h, &empty_grid(w, h), 0, 0);
        assert!(!tc.passable);
        assert!(!tc.hole);
    }

    #[test]
    fn terrain_empty_is_passable() {
        let (t, w, h) = tiles_from(&[" "]);
        let tc = terrain_at(&t, w, h, &empty_grid(w, h), 0, 0);
        assert!(tc.passable);
    }

    #[test]
    fn terrain_hole_overrides_brick() {
        let (t, w, h) = tiles_from(&["#"]);
        let tc = terrain_at(&t, w, h, &hole_grid_at(w, h, &[(0, 0)]), 0, 0);
        assert!(tc.passable);
        assert!(tc.hole);
    }

    #[test]
    fn terrain_out_of_bounds_is_wall() {
        let (t, w, h) = tiles_from(&[" "]);
        let tc = terrain_at(&t, w, h, &empty_grid(w, h), 5, 5);
        assert!(!tc.passable);
    }

    #[test]
    fn terrain_ladder_is_climbable() {
        let (t, w, h) = tiles_from(&["H"]);
        let tc = terrain_at(&t, w, h, &empty_grid(w, h), 0, 0);
        assert!(tc.passable);
        assert!(tc.climbable);
    }

    #[test]
    fn terrain_rope_is_hangable() {
        let (t, w, h) = tiles_from(&["-"]);
        let tc = terrain_at(&t, w, h, &empty_grid(w, h), 0, 0);
        assert!(tc.passable);
        assert!(tc.hangable);
    }

    // ── terrain_support ──

    #[test]
    fn support_bottom_of_map() {
        let (t, w, h) = tiles_from(&[" "]);
        assert!(terrain_support(&t, w, h, &empty_grid(w, h), 0, 0));
    }

    #[test]
    fn support_on_ladder() {
        let (t, w, h) = tiles_from(&["H", " "]);
        assert!(terrain_support(&t, w, h, &empty_grid(w, h), 0, 0));
    }

    #[test]
    fn support_above_solid() {
        let (t, w, h) = tiles_from(&[" ", "#"]);
        assert!(terrain_support(&t, w, h, &empty_grid(w, h), 0, 0));
    }

    #[test]
    fn no_support_above_hole() {
        let (t, w, h) = tiles_from(&[" ", " "]);
        assert!(!terrain_support(&t, w, h, &empty_grid(w, h), 0, 0));
    }

    #[test]
    fn no_support_above_hole_in_brick() {
        let (t, w, h) = tiles_from(&[" ", "#"]);
        assert!(!terrain_support(&t, w, h, &hole_grid_at(w, h, &[(0, 1)]), 0, 0));
    }

    // ── Trapped guard as bridge ──

    #[test]
    fn trapped_guard_provides_support() {
        let (t, w, h) = tiles_from(&[" ", " "]);
        let guards = vec![guard_at(0, 0, 1, ActorState::InHole)];
        let hg = empty_grid(w, h);
        assert!(!terrain_support(&t, w, h, &hg, 0, 0));
        assert!(has_support(&t, w, h, &hg, &guards, 0, 0));
    }

    #[test]
    fn active_guard_not_a_bridge_for_guards() {
        let (t, w, h) = tiles_from(&[" ", " "]);
        let guards = vec![guard_at(0, 0, 1, ActorState::OnGround)];
        assert!(!has_support(&t, w, h, &empty_grid(w, h), &guards, 0, 0));
    }

    // ── Player head-walking ──

    #[test]
    fn active_guard_is_floor_for_player() {
        let (t, w, h) = tiles_from(&[" ", " "]);
        let guards = vec![guard_at(0, 0, 1, ActorState::OnGround)];
        assert!(has_support_for_player(&t, w, h, &empty_grid(w, h), &guards, 0, 0));
    }

    #[test]
    fn falling_guard_not_floor_for_player() {
        let (t, w, h) = tiles_from(&[" ", " "]);
        let guards = vec![guard_at(0, 0, 1, ActorState::Falling)];
        assert!(!has_support_for_player(&t, w, h, &empty_grid(w, h), &guards, 0, 0));
    }

    #[test]
    fn dead_guard_not_floor_for_player() {
        let (t, w, h) = tiles_from(&[" ", " "]);
        let guards = vec![guard_at(0, 0, 1, ActorState::Dead)];
        assert!(!has_support_for_player(&t, w, h, &empty_grid(w, h), &guards, 0, 0));
    }

    #[test]
    fn trapped_guard_is_floor_for_player() {
        let (t, w, h) = tiles_from(&[" ", " "]);
        let guards = vec![guard_at(0, 0, 1, ActorState::InHole)];
        assert!(has_support_for_player(&t, w, h, &empty_grid(w, h), &guards, 0, 0));
    }

    #[test]
    fn on_rope_guard_is_floor_for_player() {
        let (t, w, h) = tiles_from(&[" ", "-"]);
        let guards = vec![guard_at(0, 0, 1, ActorState::OnRope)];
        assert!(has_support_for_player(&t, w, h, &empty_grid(w, h), &guards, 0, 0));
    }

    #[test]
    fn standing_guard_check() {
        let guards = vec![
            guard_at(0, 1, 1, ActorState::OnGround),
            guard_at(1, 2, 1, ActorState::Falling),
            guard_at(2, 3, 1, ActorState::Dead),
            guard_at(3, 4, 1, ActorState::InHole),
        ];

        assert!(has_standing_guard(&guards, 1, 1));
        assert!(!has_standing_guard(&guards, 2, 1));
        assert!(!has_standing_guard(&guards, 3, 1));
        assert!(has_standing_guard(&guards, 4, 1));
        assert!(!has_standing_guard(&guards, 5, 5));
    }

    #[test]
    fn dead_guard_not_a_bridge() {
        let (t, w, h) = tiles_from(&[" ", " "]);
        let guards = vec![guard_at(0, 0, 1, ActorState::Dead)];
        assert!(!has_support(&t, w, h, &empty_grid(w, h), &guards, 0, 0));
    }

    #[test]
    fn guard_support_excludes_self() {
        let (t, w, h) = tiles_from(&[" ", " "]);
        let guards = vec![
            guard_at(0, 0, 0, ActorState::OnGround),
            guard_at(1, 0, 1, ActorState::InHole),
        ];
        let hg = empty_grid(w, h);
        assert!(has_support_for_guard(&t, w, h, &hg, &guards, 0, 0, 0));
        assert!(!has_support_for_guard(&t, w, h, &hg, &guards, 0, 1, 1));
    }

    // ── resolve_state ──

    #[test]
    fn resolve_falls_without_support() {
        let (t, w, h) = tiles_from(&[" ", " ", "#"]);
        let guards: Vec<Guard> = vec![];
        assert_eq!(
            resolve_state(&t, w, h, &empty_grid(w, h), &guards, 0, 0, ActorState::OnGround),
            ActorState::Falling
        );
    }

    #[test]
    fn resolve_lands_on_trapped_guard() {
        let (t, w, h) = tiles_from(&[" ", " "]);
        let guards = vec![guard_at(0, 0, 1, ActorState::InHole)];
        assert_eq!(
            resolve_state(&t, w, h, &empty_grid(w, h), &guards, 0, 0, ActorState::Falling),
            ActorState::OnGround
        );
    }

    // ── build_hole_grid ──

    #[test]
    fn hole_grid_basic() {
        use crate::domain::entity::Hole;
        let holes = vec![
            Hole::new(3, 5, 100, 30),
            Hole::new(7, 2, 50, 30),
        ];
        let grid = build_hole_grid(&holes, 10, 8);
        assert!(grid[5][3]);
        assert!(grid[2][7]);
        assert!(!grid[0][0]);
        assert!(!grid[5][4]);
    }
}

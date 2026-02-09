/// Movement rules and dig rules — truth-table driven.
///
/// Pure functions operating on world state — no side effects.
/// These encode "what is legal" without performing the action.
///
/// ## Movement Truth Table (B)
///
/// Each rule is a conjunction of conditions. If ANY condition in the
/// "deny" column matches, the move is blocked. Otherwise it's allowed.
///
/// ### Horizontal (Left / Right)
/// ┌──────────────────────┬───────────────┬─────────┐
/// │ Condition             │ Allow?        │ Notes   │
/// ├──────────────────────┼───────────────┼─────────┤
/// │ State = Falling       │ DENY          │ no air control │
/// │ State = Dead/InHole   │ DENY          │ terminal states │
/// │ Dest out of bounds    │ DENY          │ map edge │
/// │ Dest tile solid       │ DENY          │ wall/brick │
/// │ Otherwise             │ ALLOW         │ │
/// └──────────────────────┴───────────────┴─────────┘
///
/// ### Up
/// ┌──────────────────────┬───────────────┬─────────┐
/// │ Condition             │ Allow?        │ Notes   │
/// ├──────────────────────┼───────────────┼─────────┤
/// │ State = Falling/Dead/InHole │ DENY   │         │
/// │ y == 0                │ DENY          │ top edge │
/// │ here NOT climbable    │ DENY          │ must be on ladder │
/// │ dest tile solid       │ DENY          │         │
/// │ Otherwise             │ ALLOW         │         │
/// └──────────────────────┴───────────────┴─────────┘
///
/// ### Down
/// ┌──────────────────────┬───────────────┬─────────┐
/// │ Condition             │ Allow?        │ Notes   │
/// ├──────────────────────┼───────────────┼─────────┤
/// │ State = Dead/InHole   │ DENY          │         │
/// │ y+1 >= height         │ DENY          │ bottom edge │
/// │ here climbable, below passable │ ALLOW│ descend ladder │
/// │ here hangable, below passable  │ ALLOW│ drop from rope │
/// │ below climbable       │ ALLOW         │ step onto ladder │
/// │ Otherwise             │ DENY          │ can't walk down │
/// └──────────────────────┴───────────────┴─────────┘
///
/// ### Support (who stands, who falls)
/// ┌──────────────────────────────┬───────────┐
/// │ Condition                     │ Support?  │
/// ├──────────────────────────────┼───────────┤
/// │ y+1 >= height (bottom edge)   │ YES       │
/// │ here is climbable             │ YES       │
/// │ here is hangable              │ YES       │
/// │ below is solid                │ YES       │
/// │ below is climbable            │ YES       │
/// │ (physics layer: trapped guard)│ YES       │
/// │ Otherwise                     │ NO → Fall │
/// └──────────────────────────────┴───────────┘
///
/// ### State Resolution
/// ┌─────────────────────────────┬──────────────┐
/// │ Condition (priority order)   │ New State    │
/// ├─────────────────────────────┼──────────────┤
/// │ current = Dead/InHole        │ unchanged    │
/// │ here is climbable            │ OnLadder     │
/// │ here is hangable             │ OnRope       │
/// │ has_support                   │ OnGround     │
/// │ otherwise                    │ Falling      │
/// └─────────────────────────────┴──────────────┘

use super::entity::{ActorState, Facing};
use super::tile::Tile;

/// Immutable view of the tile map for rule queries.
pub struct MapView<'a> {
    pub tiles: &'a Vec<Vec<Tile>>,
    pub width: usize,
    pub height: usize,
}

impl<'a> MapView<'a> {
    pub fn tile_at(&self, x: usize, y: usize) -> Tile {
        if x >= self.width || y >= self.height {
            return Tile::Concrete; // out of bounds = wall
        }
        self.tiles[y][x]
    }

    pub fn is_passable(&self, x: usize, y: usize) -> bool {
        self.tile_at(x, y).is_passable()
    }

    /// Terrain-only support check.
    /// See truth table above for the complete spec.
    pub fn has_support(&self, x: usize, y: usize) -> bool {
        if y + 1 >= self.height { return true; }
        let here = self.tile_at(x, y);
        if here.is_climbable() || here.is_hangable() { return true; }
        let below = self.tile_at(x, y + 1);
        if below.is_solid() || below.is_climbable() { return true; }
        false
    }
}

// ── State Resolution ──

/// Determine actor state from position. See truth table above.
pub fn resolve_state(map: &MapView, x: usize, y: usize, current: ActorState) -> ActorState {
    // Terminal states are sticky
    if current == ActorState::Dead || current == ActorState::InHole {
        return current;
    }
    let here = map.tile_at(x, y);
    if here.is_climbable() { return ActorState::OnLadder; }
    if here.is_hangable()  { return ActorState::OnRope; }
    if map.has_support(x, y) { return ActorState::OnGround; }
    ActorState::Falling
}

// ── Movement Rules (truth-table driven) ──

/// Is the given state one that blocks voluntary movement?
#[inline]
fn is_immobile(state: ActorState) -> bool {
    matches!(state, ActorState::Falling | ActorState::Dead | ActorState::InHole)
}

pub fn can_move_left(map: &MapView, x: usize, y: usize, state: ActorState) -> bool {
    if x == 0 { return false; }
    if is_immobile(state) { return false; }
    map.is_passable(x - 1, y)
}

pub fn can_move_right(map: &MapView, x: usize, y: usize, state: ActorState) -> bool {
    if x + 1 >= map.width { return false; }
    if is_immobile(state) { return false; }
    map.is_passable(x + 1, y)
}

pub fn can_move_up(map: &MapView, x: usize, y: usize, state: ActorState) -> bool {
    if y == 0 { return false; }
    if is_immobile(state) { return false; }
    // Must be on a climbable tile to go up
    if !map.tile_at(x, y).is_climbable() { return false; }
    map.is_passable(x, y - 1)
}

pub fn can_move_down(map: &MapView, x: usize, y: usize, state: ActorState) -> bool {
    if y + 1 >= map.height { return false; }
    if state == ActorState::Dead || state == ActorState::InHole { return false; }

    let here = map.tile_at(x, y);
    let below = map.tile_at(x, y + 1);

    // Descend ladder / drop from rope: here supports, below is passable
    if (here.is_climbable() || here.is_hangable()) && below.is_passable() {
        return true;
    }
    // Step onto ladder from above
    if below.is_climbable() {
        return true;
    }
    false
}

// ── Dig Rules ──

/// Can the player dig? Returns target (x, y) if legal, None otherwise.
///
/// Conditions:
///   1. Not falling/dead/in-hole
///   2. Has support (ground, ladder, or rope)
///   3. Side cell is passable (line of sight)
///   4. Side cell is NOT a ladder (can't dig under ladders)
///   5. Target (below-side) is a diggable brick
pub fn can_dig(
    map: &MapView, x: usize, y: usize,
    state: ActorState, dir: Facing,
) -> Option<(usize, usize)> {
    if is_immobile(state) { return None; }

    // Must have support
    if !map.has_support(x, y) && state != ActorState::OnLadder && state != ActorState::OnRope {
        return None;
    }

    let side_x = match dir {
        Facing::Left  => { if x == 0 { return None; } x - 1 }
        Facing::Right => { if x + 1 >= map.width { return None; } x + 1 }
    };
    let dig_y = y + 1;
    if dig_y >= map.height { return None; }

    // Side must be passable
    if !map.is_passable(side_x, y) { return None; }
    // Cannot dig below a ladder
    if map.tile_at(side_x, y).is_climbable() { return None; }
    // Target must be diggable
    if map.tile_at(side_x, dig_y).is_diggable() {
        Some((side_x, dig_y))
    } else {
        None
    }
}

/// Should the actor fall? (Terrain-only, no occupancy.)
#[allow(dead_code)]
pub fn should_fall(map: &MapView, x: usize, y: usize) -> bool {
    !map.has_support(x, y)
}

// ══════════════════════════════════════════════════════════════
// Unit tests (C)
// ══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::tile::Tile;

    /// Helper: build a MapView from a string diagram.
    /// Legend:  '#'=Brick  '='=Concrete  'H'=Ladder  '-'=Rope
    ///         '$'=Gold  'T'=TrapBrick  ' '=Empty
    fn map_from(rows: &[&str]) -> (Vec<Vec<Tile>>, usize, usize) {
        let height = rows.len();
        let width = rows[0].len();
        let mut tiles = vec![vec![Tile::Empty; width]; height];
        for (y, row) in rows.iter().enumerate() {
            for (x, ch) in row.chars().enumerate() {
                tiles[y][x] = match ch {
                    '#' => Tile::Brick,
                    '=' => Tile::Concrete,
                    'H' => Tile::Ladder,
                    '-' => Tile::Rope,
                    '$' => Tile::Gold,
                    'T' => Tile::TrapBrick,
                    _   => Tile::Empty,
                };
            }
        }
        (tiles, width, height)
    }

    fn mv(tiles: &Vec<Vec<Tile>>, w: usize, h: usize) -> MapView {
        MapView { tiles, width: w, height: h }
    }

    // ── Horizontal movement ──

    #[test]
    fn horizontal_on_ground() {
        let (t, w, h) = map_from(&[
            "     ",
            "#####",
        ]);
        let m = mv(&t, w, h);
        // middle of empty row, standing on brick
        assert!(can_move_left(&m, 2, 0, ActorState::OnGround));
        assert!(can_move_right(&m, 2, 0, ActorState::OnGround));
    }

    #[test]
    fn horizontal_blocked_by_wall() {
        let (t, w, h) = map_from(&[
            " # # ",
            "#####",
        ]);
        let m = mv(&t, w, h);
        assert!(!can_move_right(&m, 0, 0, ActorState::OnGround)); // wall at (1,0)
        assert!(!can_move_left(&m, 2, 0, ActorState::OnGround));  // wall at (1,0)
    }

    #[test]
    fn horizontal_at_map_edge() {
        let (t, w, h) = map_from(&[
            "   ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert!(!can_move_left(&m, 0, 0, ActorState::OnGround));
        assert!(!can_move_right(&m, 2, 0, ActorState::OnGround));
    }

    #[test]
    fn horizontal_denied_while_falling() {
        let (t, w, h) = map_from(&[
            "   ",
            "   ",
        ]);
        let m = mv(&t, w, h);
        assert!(!can_move_left(&m, 1, 0, ActorState::Falling));
        assert!(!can_move_right(&m, 1, 0, ActorState::Falling));
    }

    #[test]
    fn horizontal_on_ladder() {
        let (t, w, h) = map_from(&[
            " H ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert!(can_move_left(&m, 1, 0, ActorState::OnLadder));
        assert!(can_move_right(&m, 1, 0, ActorState::OnLadder));
    }

    #[test]
    fn horizontal_on_rope() {
        let (t, w, h) = map_from(&[
            "---",
            "   ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert!(can_move_left(&m, 1, 0, ActorState::OnRope));
        assert!(can_move_right(&m, 1, 0, ActorState::OnRope));
    }

    // ── Vertical movement ──

    #[test]
    fn up_on_ladder() {
        let (t, w, h) = map_from(&[
            " H ",
            " H ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert!(can_move_up(&m, 1, 1, ActorState::OnLadder));
    }

    #[test]
    fn up_denied_not_on_ladder() {
        let (t, w, h) = map_from(&[
            "   ",
            "   ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert!(!can_move_up(&m, 1, 1, ActorState::OnGround));
    }

    #[test]
    fn up_denied_at_top() {
        let (t, w, h) = map_from(&[
            " H ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert!(!can_move_up(&m, 1, 0, ActorState::OnLadder));
    }

    #[test]
    fn up_denied_blocked_above() {
        let (t, w, h) = map_from(&[
            " = ",
            " H ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert!(!can_move_up(&m, 1, 1, ActorState::OnLadder));
    }

    #[test]
    fn down_on_ladder() {
        let (t, w, h) = map_from(&[
            " H ",
            " H ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert!(can_move_down(&m, 1, 0, ActorState::OnLadder));
    }

    #[test]
    fn down_from_rope() {
        let (t, w, h) = map_from(&[
            "---",
            "   ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert!(can_move_down(&m, 1, 0, ActorState::OnRope));
    }

    #[test]
    fn down_step_onto_ladder_from_above() {
        let (t, w, h) = map_from(&[
            "   ",
            " H ",
            "###",
        ]);
        let m = mv(&t, w, h);
        // Standing on top of ladder (has support because below is climbable)
        assert!(can_move_down(&m, 1, 0, ActorState::OnGround));
    }

    #[test]
    fn down_denied_at_bottom() {
        let (t, w, h) = map_from(&[
            " H ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert!(!can_move_down(&m, 1, 1, ActorState::OnGround));
    }

    #[test]
    fn down_denied_solid_below() {
        let (t, w, h) = map_from(&[
            "   ",
            "###",
        ]);
        let m = mv(&t, w, h);
        // On ground, solid below, can't move down (not on ladder/rope, below not climbable)
        assert!(!can_move_down(&m, 1, 0, ActorState::OnGround));
    }

    // ── Support / Falling ──

    #[test]
    fn support_on_solid() {
        let (t, w, h) = map_from(&[
            "   ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert!(m.has_support(1, 0));
        assert!(!should_fall(&m, 1, 0));
    }

    #[test]
    fn support_on_ladder() {
        let (t, w, h) = map_from(&[
            " H ",
            "   ",
        ]);
        let m = mv(&t, w, h);
        assert!(m.has_support(1, 0));
    }

    #[test]
    fn support_on_rope() {
        let (t, w, h) = map_from(&[
            " - ",
            "   ",
        ]);
        let m = mv(&t, w, h);
        assert!(m.has_support(1, 0));
    }

    #[test]
    fn support_above_ladder() {
        let (t, w, h) = map_from(&[
            "   ",
            " H ",
            "###",
        ]);
        let m = mv(&t, w, h);
        // Standing above a ladder = supported (below is climbable)
        assert!(m.has_support(1, 0));
    }

    #[test]
    fn no_support_in_air() {
        let (t, w, h) = map_from(&[
            "   ",
            "   ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert!(!m.has_support(1, 0));
        assert!(should_fall(&m, 1, 0));
    }

    #[test]
    fn support_at_bottom_edge() {
        let (t, w, h) = map_from(&[
            "   ",
            "   ",
        ]);
        let m = mv(&t, w, h);
        // Last row always has support
        assert!(m.has_support(1, 1));
    }

    // ── State resolution ──

    #[test]
    fn resolve_state_on_ladder() {
        let (t, w, h) = map_from(&[
            " H ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert_eq!(resolve_state(&m, 1, 0, ActorState::Falling), ActorState::OnLadder);
        assert_eq!(resolve_state(&m, 1, 0, ActorState::OnGround), ActorState::OnLadder);
    }

    #[test]
    fn resolve_state_on_rope() {
        let (t, w, h) = map_from(&[
            " - ",
            "   ",
        ]);
        let m = mv(&t, w, h);
        assert_eq!(resolve_state(&m, 1, 0, ActorState::Falling), ActorState::OnRope);
    }

    #[test]
    fn resolve_state_falling() {
        let (t, w, h) = map_from(&[
            "   ",
            "   ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert_eq!(resolve_state(&m, 1, 0, ActorState::OnGround), ActorState::Falling);
    }

    #[test]
    fn resolve_state_dead_sticky() {
        let (t, w, h) = map_from(&[
            " H ",
            "###",
        ]);
        let m = mv(&t, w, h);
        // Dead stays dead even on a ladder
        assert_eq!(resolve_state(&m, 1, 0, ActorState::Dead), ActorState::Dead);
    }

    #[test]
    fn resolve_state_inhole_sticky() {
        let (t, w, h) = map_from(&[
            "   ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert_eq!(resolve_state(&m, 0, 0, ActorState::InHole), ActorState::InHole);
    }

    // ── Dig rules ──

    #[test]
    fn dig_basic() {
        let (t, w, h) = map_from(&[
            "   ",
            "###",
        ]);
        let m = mv(&t, w, h);
        // Player at (1,0), dig right → target (2,1) which is Brick
        assert_eq!(can_dig(&m, 1, 0, ActorState::OnGround, Facing::Right), Some((2, 1)));
        assert_eq!(can_dig(&m, 1, 0, ActorState::OnGround, Facing::Left), Some((0, 1)));
    }

    #[test]
    fn dig_denied_while_falling() {
        let (t, w, h) = map_from(&[
            "   ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert_eq!(can_dig(&m, 1, 0, ActorState::Falling, Facing::Right), None);
    }

    #[test]
    fn dig_denied_side_blocked() {
        let (t, w, h) = map_from(&[
            " = ",
            "###",
        ]);
        let m = mv(&t, w, h);
        // Player at (0,0), dig right blocked by concrete at (1,0) side
        // Wait, player at (0,0) needs to see side (1,0) which is concrete = not passable
        // Actually tile_at(1,0) = Concrete which is !passable, so dig denied
        // But player at (0,0) has support? (0,1) is Brick = solid. Yes.
        // side_x = 1, is_passable(1, 0) = false (Concrete). DENY.
        assert_eq!(can_dig(&m, 0, 0, ActorState::OnGround, Facing::Right), None);
    }

    #[test]
    fn dig_denied_target_concrete() {
        let (t, w, h) = map_from(&[
            "   ",
            "#=#",
        ]);
        let m = mv(&t, w, h);
        // Dig right from (0,0) → side (1,0) passable, target (1,1) = Concrete = not diggable
        assert_eq!(can_dig(&m, 0, 0, ActorState::OnGround, Facing::Right), None);
    }

    #[test]
    fn dig_denied_under_ladder() {
        let (t, w, h) = map_from(&[
            " H ",
            "###",
        ]);
        let m = mv(&t, w, h);
        // Dig right from (0,0): side (1,0) = Ladder = climbable → can't dig under ladder
        assert_eq!(can_dig(&m, 0, 0, ActorState::OnGround, Facing::Right), None);
    }

    #[test]
    fn dig_from_ladder() {
        let (t, w, h) = map_from(&[
            "H  ",
            "H##",
        ]);
        let m = mv(&t, w, h);
        // Player on ladder at (0,0), dig right: side (1,0) passable, target (1,1) = Brick
        assert_eq!(can_dig(&m, 0, 0, ActorState::OnLadder, Facing::Right), Some((1, 1)));
    }

    #[test]
    fn dig_at_map_edge() {
        let (t, w, h) = map_from(&[
            "   ",
            "###",
        ]);
        let m = mv(&t, w, h);
        assert_eq!(can_dig(&m, 0, 0, ActorState::OnGround, Facing::Left), None);
        assert_eq!(can_dig(&m, 2, 0, ActorState::OnGround, Facing::Right), None);
    }

    #[test]
    fn dig_at_bottom_edge() {
        let (t, w, h) = map_from(&[
            "###",
        ]);
        let m = mv(&t, w, h);
        // dig_y = 0+1 = 1, which is >= height(1), so denied
        assert_eq!(can_dig(&m, 1, 0, ActorState::OnGround, Facing::Left), None);
    }
}

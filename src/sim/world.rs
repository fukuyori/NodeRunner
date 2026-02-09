/// WorldState: the complete snapshot of a running game.
///
/// ## Tile Architecture (A)
///
/// Two tile layers, composed at query time:
///   - `base_tiles` — the level as loaded. **Never mutated** after load.
///   - `tiles`      — the effective terrain (base + runtime changes).
///
/// All tile mutations go through `set_tile()` / `clear_tile()`.
/// `terrain_at()` reads from the effective `tiles`.
/// `restart_level` resets `tiles = base_tiles.clone()`.
///
/// ## Camera / Viewport
///
/// World coordinates and screen coordinates are separate:
///   - `camera` — viewport into the world (top-left corner + size)
///   - Renderer maps: `screen(sx, sy) = world(camera.x + sx, camera.y + sy)`
///   - Camera follows the player with a dead-zone approach
///   - Maps smaller than the viewport are centered

use crate::config::SpeedConfig;
use crate::domain::entity::{DigInProgress, Guard, Hole, Player};
use crate::domain::physics::{self, TerrainCell};
use crate::domain::tile::Tile;

/// Info about a level pack, displayed in the pack selector.
#[derive(Clone, Debug)]
pub struct PackInfo {
    pub name: String,
    pub author: String,
    pub description: String,
    pub level_count: usize,
    pub path: String,        // filesystem path, or "__levels__" / "__embedded__"
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Title,
    LevelSelect,
    PackSelect,
    LevelIntro,
    LevelReady,
    Playing,
    LevelOutro,
    LevelComplete,
    Dying,
    GameOver,
    GameComplete,
}

/// Camera: a viewport into the world.
///
/// `(x, y)` is the world coordinate of the top-left visible cell.
/// `(view_w, view_h)` is how many world cells fit in the viewport.
/// These are computed from terminal size and set during `render()`.
#[derive(Clone, Debug)]
pub struct Camera {
    /// World X of the top-left visible cell (can be negative for centering)
    pub x: i32,
    /// World Y of the top-left visible cell
    pub y: i32,
    /// Number of world columns visible
    pub view_w: usize,
    /// Number of world rows visible
    pub view_h: usize,
}

impl Camera {
    pub fn new() -> Self {
        Camera { x: 0, y: 0, view_w: 0, view_h: 0 }
    }

    /// Update camera to follow a target position within the given world bounds.
    /// Uses a dead-zone approach: only scroll when the target is near the edge
    /// of the viewport. This gives a smooth, non-jerky Lode Runner feel.
    pub fn follow(&mut self, target_x: usize, target_y: usize, world_w: usize, world_h: usize) {
        if self.view_w == 0 || self.view_h == 0 { return; }

        // If map fits entirely in viewport, center it
        if world_w <= self.view_w {
            self.x = -((self.view_w as i32 - world_w as i32) / 2);
        } else {
            // Dead zone: inner 40% of viewport. Player can move freely inside.
            let margin_x = (self.view_w as i32) / 5; // 20% margin on each side
            let left_bound = self.x + margin_x;
            let right_bound = self.x + self.view_w as i32 - margin_x - 1;
            let tx = target_x as i32;

            if tx < left_bound {
                self.x = tx - margin_x;
            } else if tx > right_bound {
                self.x = tx - self.view_w as i32 + margin_x + 1;
            }

            // Clamp to world bounds
            self.x = self.x.max(0).min((world_w as i32 - self.view_w as i32).max(0));
        }

        if world_h <= self.view_h {
            self.y = -((self.view_h as i32 - world_h as i32) / 2);
        } else {
            let margin_y = (self.view_h as i32) / 5;
            let top_bound = self.y + margin_y;
            let bottom_bound = self.y + self.view_h as i32 - margin_y - 1;
            let ty = target_y as i32;

            if ty < top_bound {
                self.y = ty - margin_y;
            } else if ty > bottom_bound {
                self.y = ty - self.view_h as i32 + margin_y + 1;
            }

            self.y = self.y.max(0).min((world_h as i32 - self.view_h as i32).max(0));
        }
    }

    /// Snap camera directly to center on a position (no dead zone).
    /// Used on level load / restart.
    pub fn center_on(&mut self, target_x: usize, target_y: usize, world_w: usize, world_h: usize) {
        if self.view_w == 0 || self.view_h == 0 { return; }

        if world_w <= self.view_w {
            self.x = -((self.view_w as i32 - world_w as i32) / 2);
        } else {
            self.x = target_x as i32 - self.view_w as i32 / 2;
            self.x = self.x.max(0).min((world_w as i32 - self.view_w as i32).max(0));
        }

        if world_h <= self.view_h {
            self.y = -((self.view_h as i32 - world_h as i32) / 2);
        } else {
            self.y = target_y as i32 - self.view_h as i32 / 2;
            self.y = self.y.max(0).min((world_h as i32 - self.view_h as i32).max(0));
        }
    }

    /// Convert world coordinate to viewport coordinate.
    /// Returns None if outside the visible area.
    pub fn world_to_view(&self, wx: usize, wy: usize) -> Option<(usize, usize)> {
        let vx = wx as i32 - self.x;
        let vy = wy as i32 - self.y;
        if vx >= 0 && vx < self.view_w as i32 && vy >= 0 && vy < self.view_h as i32 {
            Some((vx as usize, vy as usize))
        } else {
            None
        }
    }
}

pub struct WorldState {
    // ── Tile layers ──
    /// Original level data. Never mutated after `load_level`.
    pub base_tiles: Vec<Vec<Tile>>,
    /// Effective terrain = base + runtime changes (holes, gold pickup, etc).
    /// Always kept in sync via `set_tile()` / `clear_tile()`.
    pub tiles: Vec<Vec<Tile>>,
    pub width: usize,
    pub height: usize,

    // ── Entities ──
    pub player: Player,
    pub guards: Vec<Guard>,
    pub holes: Vec<Hole>,
    pub digs: Vec<DigInProgress>,

    // ── Derived: O(1) hole lookup grid ──
    /// `hole_grid[y][x] == true` ↔ active hole at (x, y).
    /// Rebuilt automatically by `rebuild_hole_grid()`.
    pub hole_grid: Vec<Vec<bool>>,

    // ── Game tracking ──
    pub gold_remaining: usize,
    pub gold_total: usize,
    pub exit_enabled: bool,

    // ── Speed config ──
    pub speed: SpeedConfig,

    // ── Meta ──
    pub phase: Phase,
    pub score: u32,
    pub lives: u32,
    pub current_level: usize,
    pub total_levels: usize,
    #[allow(dead_code)]
    pub level_name: String,
    pub tick: u64,

    // ── UI ──
    pub message: String,
    pub message_timer: u32,

    // ── Spawn / exit ──
    pub player_spawn: (usize, usize),
    pub exit_columns: Vec<usize>,
    pub hidden_ladder_positions: Vec<(usize, usize)>,

    // ── Animation ──
    pub anim_tick: u32,
    pub anim_player_y: i32,

    // ── Pause ──
    pub paused: bool,

    // ── Camera / Viewport ──
    pub camera: Camera,

    // ── Level select ──
    pub select_cursor: usize,
    pub select_scroll: usize,
    pub level_names: Vec<String>,
    pub has_save: bool,

    // ── Pack select (F3 filer) ──
    pub pack_list: Vec<PackInfo>,
    pub pack_cursor: usize,
    pub pack_scroll: usize,
    pub active_pack: String,       // display name of active pack
    pub active_pack_path: String,  // path or "__levels__" or "__embedded__"
}

// ── Tile query / mutation API ──

impl WorldState {
    /// Query effective terrain at (x, y).
    #[inline]
    pub fn terrain_at(&self, x: usize, y: usize) -> Tile {
        if x < self.width && y < self.height {
            self.tiles[y][x]
        } else {
            Tile::Concrete // out of bounds = wall
        }
    }

    /// Set a tile in the effective layer (runtime change).
    #[inline]
    pub fn set_tile(&mut self, x: usize, y: usize, tile: Tile) {
        if x < self.width && y < self.height {
            self.tiles[y][x] = tile;
        }
    }

    /// Revert a tile to its base layer value.
    #[inline]
    pub fn clear_tile(&mut self, x: usize, y: usize) {
        if x < self.width && y < self.height {
            self.tiles[y][x] = self.base_tiles[y][x];
        }
    }

    /// Reset all tiles to base (used by restart_level).
    pub fn reset_tiles(&mut self) {
        self.tiles = self.base_tiles.clone();
    }
}

// ── Hole grid maintenance ──

impl WorldState {
    /// Rebuild the hole_grid from current holes.
    /// Call after any hole is added, removed, or after load.
    #[inline]
    pub fn rebuild_hole_grid(&mut self) {
        self.hole_grid = physics::build_hole_grid(&self.holes, self.width, self.height);
    }
}

// ── Unified physics queries (single source of truth) ──
//
// All systems (step, AI, renderer) should prefer these methods
// over calling physics:: functions directly with raw parameters.

#[allow(dead_code)]
impl WorldState {
    /// Terrain at (x, y) with holes applied. O(1).
    #[inline]
    pub fn terrain_cell(&self, x: usize, y: usize) -> TerrainCell {
        physics::terrain_at(&self.tiles, self.width, self.height, &self.hole_grid, x, y)
    }

    /// Does terrain alone (no entities) provide support at (x, y)?
    #[inline]
    pub fn terrain_support(&self, x: usize, y: usize) -> bool {
        physics::terrain_support(&self.tiles, self.width, self.height, &self.hole_grid, x, y)
    }

    /// Full support: terrain + trapped guard below.
    #[inline]
    pub fn has_support(&self, x: usize, y: usize) -> bool {
        physics::has_support(
            &self.tiles, self.width, self.height,
            &self.hole_grid, &self.guards, x, y,
        )
    }

    /// Player support: terrain + any standing guard below (head-walking).
    #[inline]
    pub fn has_support_for_player(&self, x: usize, y: usize) -> bool {
        physics::has_support_for_player(
            &self.tiles, self.width, self.height,
            &self.hole_grid, &self.guards, x, y,
        )
    }

    /// Guard support: terrain + trapped guards below (excluding self).
    #[inline]
    pub fn has_support_for_guard(&self, x: usize, y: usize, guard_idx: usize) -> bool {
        physics::has_support_for_guard(
            &self.tiles, self.width, self.height,
            &self.hole_grid, &self.guards, x, y, guard_idx,
        )
    }

    /// Resolve actor state from terrain + occupancy.
    #[inline]
    pub fn resolve_actor_state(&self, x: usize, y: usize, current: crate::domain::entity::ActorState) -> crate::domain::entity::ActorState {
        physics::resolve_state(
            &self.tiles, self.width, self.height,
            &self.hole_grid, &self.guards, x, y, current,
        )
    }

    /// Can an entity enter (x, y)? Terrain passability only.
    #[inline]
    pub fn can_enter(&self, x: usize, y: usize) -> bool {
        self.terrain_cell(x, y).passable
    }
}

// ── Construction ──

impl WorldState {
    pub fn new() -> Self {
        WorldState {
            base_tiles: vec![],
            tiles: vec![],
            width: 0,
            height: 0,
            player: Player::new(0, 0),
            guards: vec![],
            holes: vec![],
            digs: vec![],
            hole_grid: vec![],
            gold_remaining: 0,
            gold_total: 0,
            exit_enabled: false,
            speed: SpeedConfig {
                tick_rate_ms: 75,
                player_move_rate: 2,
                guard_move_rate: 5,
                dig_duration: 5,
                hole_open_ticks: 100,
                hole_close_ticks: 20,
                trap_escape_ticks: 70,
                guard_respawn_ticks: 80,
                gold_carry_ticks: 150,
            },
            phase: Phase::Title,
            score: 0,
            lives: 5,
            current_level: 0,
            total_levels: 0,
            level_name: String::new(),
            tick: 0,
            message: String::new(),
            message_timer: 0,
            player_spawn: (0, 0),
            exit_columns: vec![],
            hidden_ladder_positions: vec![],
            anim_tick: 0,
            anim_player_y: 0,
            paused: false,
            camera: Camera::new(),
            select_cursor: 0,
            select_scroll: 0,
            level_names: vec![],
            has_save: false,
            pack_list: vec![],
            pack_cursor: 0,
            pack_scroll: 0,
            active_pack: String::from("Built-in Levels"),
            active_pack_path: String::from("__embedded__"),
        }
    }

    pub fn set_message(&mut self, msg: &str, duration: u32) {
        self.message = msg.to_string();
        self.message_timer = duration;
    }
}

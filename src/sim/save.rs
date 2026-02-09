/// Save and load game progress — 4 slot system with mid-game snapshots.
///
/// ## Two save modes:
///
///   **Level-start save** (non-Playing phases):
///     Stores level/score/lives. On load, the level starts fresh.
///
///   **Snapshot save** (Playing phase):
///     Stores complete game state: tiles, player, guards, holes, digs,
///     gold status, tick count. On load, gameplay resumes exactly.
///
/// ## File format:
///   Key-value lines. Snapshot data follows `has_snapshot=1`.
///
/// Slots 1-4 stored as save_1.dat .. save_4.dat.
/// Legacy save.dat (auto-save via ESC) is separate.

use std::path::PathBuf;

use crate::domain::entity::{
    ActorState, DigInProgress, Facing, Guard, Hole, Player,
};
use crate::domain::tile::Tile;
use crate::sim::world::WorldState;

// ══════════════════════════════════════════════════════════════
// Public types
// ══════════════════════════════════════════════════════════════

/// Save data: base info + optional mid-game snapshot.
#[derive(Clone, Debug)]
pub struct SaveData {
    pub level: usize,
    pub score: u32,
    pub lives: u32,
    pub snapshot: Option<Snapshot>,
}

/// Full mid-game state snapshot.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub tick: u64,
    pub width: usize,
    pub height: usize,
    pub tiles: Vec<Vec<Tile>>,
    pub player: SnapshotPlayer,
    pub guards: Vec<SnapshotGuard>,
    pub holes: Vec<SnapshotHole>,
    pub digs: Vec<SnapshotDig>,
    pub gold_remaining: usize,
    pub gold_total: usize,
    pub exit_enabled: bool,
    pub exit_columns: Vec<usize>,
    pub hidden_ladder_positions: Vec<(usize, usize)>,
    pub player_spawn: (usize, usize),
}

#[derive(Clone, Debug)]
pub struct SnapshotPlayer {
    pub x: usize,
    pub y: usize,
    pub facing: Facing,
    pub state: ActorState,
    pub move_cooldown: u32,
}

#[derive(Clone, Debug)]
pub struct SnapshotGuard {
    pub id: usize,
    pub x: usize,
    pub y: usize,
    pub facing: Facing,
    pub state: ActorState,
    pub carry_gold: bool,
    pub carry_gold_timer: u32,
    pub stuck_timer: u32,
    pub move_cooldown: u32,
    pub spawn_x: usize,
    pub spawn_y: usize,
    pub respawn_timer: u32,
    pub separation_timer: u32,
}

#[derive(Clone, Debug)]
pub struct SnapshotHole {
    pub x: usize,
    pub y: usize,
    pub open_remaining: u32,
    pub close_remaining: u32,
}

#[derive(Clone, Debug)]
pub struct SnapshotDig {
    pub x: usize,
    pub y: usize,
    pub ticks_remaining: u32,
    pub total_ticks: u32,
}

// ══════════════════════════════════════════════════════════════
// Paths
// ══════════════════════════════════════════════════════════════

const LEGACY_SAVE: &str = "save.dat";

fn save_dir() -> PathBuf {
    // 1. Try exe directory (works for local/portable installs)
    if let Ok(exe) = std::env::current_exe() {
        let resolved = exe.canonicalize().unwrap_or(exe);
        if let Some(parent) = resolved.parent() {
            // Check if writable (system installs like /usr/games/ won't be)
            let test_path = parent.join(".write_test_noderunner");
            if std::fs::write(&test_path, "").is_ok() {
                let _ = std::fs::remove_file(&test_path);
                return parent.to_path_buf();
            }
        }
    }

    // 2. XDG data home (~/.local/share/noderunner) for system installs
    if let Ok(home) = std::env::var("HOME") {
        let xdg = PathBuf::from(&home).join(".local/share/noderunner");
        if std::fs::create_dir_all(&xdg).is_ok() {
            return xdg;
        }
    }

    // 3. Fallback to CWD
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn slot_filename(slot: u8) -> String {
    format!("save_{}.dat", slot)
}

fn slot_path(slot: u8) -> PathBuf {
    save_dir().join(slot_filename(slot))
}

fn legacy_path() -> PathBuf {
    save_dir().join(LEGACY_SAVE)
}

// ══════════════════════════════════════════════════════════════
// Snapshot capture / restore (WorldState ↔ Snapshot)
// ══════════════════════════════════════════════════════════════

/// Capture a full snapshot from the current world state.
pub fn capture_snapshot(w: &WorldState) -> Snapshot {
    Snapshot {
        tick: w.tick,
        width: w.width,
        height: w.height,
        tiles: w.tiles.clone(),
        player: SnapshotPlayer {
            x: w.player.x,
            y: w.player.y,
            facing: w.player.facing,
            state: w.player.state,
            move_cooldown: w.player.move_cooldown,
        },
        guards: w.guards.iter().map(|g| SnapshotGuard {
            id: g.id,
            x: g.x, y: g.y,
            facing: g.facing,
            state: g.state,
            carry_gold: g.carry_gold,
            carry_gold_timer: g.carry_gold_timer,
            stuck_timer: g.stuck_timer,
            move_cooldown: g.move_cooldown,
            spawn_x: g.spawn_x,
            spawn_y: g.spawn_y,
            respawn_timer: g.respawn_timer,
            separation_timer: g.separation_timer,
        }).collect(),
        holes: w.holes.iter().map(|h| SnapshotHole {
            x: h.x, y: h.y,
            open_remaining: h.open_remaining,
            close_remaining: h.close_remaining,
        }).collect(),
        digs: w.digs.iter().map(|d| SnapshotDig {
            x: d.x, y: d.y,
            ticks_remaining: d.ticks_remaining,
            total_ticks: d.total_ticks(),
        }).collect(),
        gold_remaining: w.gold_remaining,
        gold_total: w.gold_total,
        exit_enabled: w.exit_enabled,
        exit_columns: w.exit_columns.clone(),
        hidden_ladder_positions: w.hidden_ladder_positions.clone(),
        player_spawn: w.player_spawn,
    }
}

/// Restore a snapshot into the world state.
/// Caller must call load_level first to set base_tiles, level_name, etc.,
/// then this function overwrites the runtime state.
pub fn restore_snapshot(w: &mut WorldState, snap: &Snapshot) {
    w.tick = snap.tick;
    w.width = snap.width;
    w.height = snap.height;
    w.tiles = snap.tiles.clone();

    w.player = Player {
        x: snap.player.x,
        y: snap.player.y,
        facing: snap.player.facing,
        state: snap.player.state,
        alive: true,
        move_cooldown: snap.player.move_cooldown,
    };

    w.guards = snap.guards.iter().map(|g| Guard {
        id: g.id,
        x: g.x, y: g.y,
        facing: g.facing,
        state: g.state,
        carry_gold: g.carry_gold,
        carry_gold_timer: g.carry_gold_timer,
        stuck_timer: g.stuck_timer,
        move_cooldown: g.move_cooldown,
        spawn_x: g.spawn_x,
        spawn_y: g.spawn_y,
        respawn_timer: g.respawn_timer,
        separation_timer: g.separation_timer,
    }).collect();

    w.holes = snap.holes.iter().map(|h| Hole::new(
        h.x, h.y, h.open_remaining, h.close_remaining,
    )).collect();

    w.digs = snap.digs.iter().map(|d| DigInProgress::new_with_state(
        d.x, d.y, d.ticks_remaining, d.total_ticks,
    )).collect();

    w.gold_remaining = snap.gold_remaining;
    w.gold_total = snap.gold_total;
    w.exit_enabled = snap.exit_enabled;
    w.exit_columns = snap.exit_columns.clone();
    w.hidden_ladder_positions = snap.hidden_ladder_positions.clone();
    w.player_spawn = snap.player_spawn;

    // Rebuild derived data
    w.rebuild_hole_grid();

    // Camera: center on player
    w.camera.center_on(w.player.x, w.player.y, w.width, w.height);
}

// ══════════════════════════════════════════════════════════════
// Slot operations (F5-F12)
// ══════════════════════════════════════════════════════════════

/// Save to a numbered slot (1-4). Pass snapshot=None for level-start save.
pub fn save_slot(slot: u8, level: usize, score: u32, lives: u32,
                 snapshot: Option<&Snapshot>) -> Result<(), String> {
    let content = serialize(level, score, lives, snapshot);
    let path = slot_path(slot);
    std::fs::write(&path, content)
        .map_err(|e| format!("Save slot {} failed: {}", slot, e))
}

/// Load from a numbered slot (1-4).
pub fn load_slot(slot: u8) -> Option<SaveData> {
    let candidates = [
        slot_path(slot),
        PathBuf::from(slot_filename(slot)),
    ];
    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            return parse_save(&content);
        }
    }
    None
}

/// Check if a numbered slot has data.
#[allow(dead_code)]
pub fn has_slot(slot: u8) -> bool {
    let candidates = [
        slot_path(slot),
        PathBuf::from(slot_filename(slot)),
    ];
    candidates.iter().any(|p| p.exists())
}

// ══════════════════════════════════════════════════════════════
// Legacy auto-save (ESC to title)
// ══════════════════════════════════════════════════════════════

pub fn save_game(level: usize, score: u32, lives: u32,
                 snapshot: Option<&Snapshot>) -> Result<(), String> {
    let content = serialize(level, score, lives, snapshot);
    let path = legacy_path();
    std::fs::write(&path, content)
        .map_err(|e| format!("Save failed: {}", e))
}

pub fn load_save() -> Option<SaveData> {
    let candidates = [
        legacy_path(),
        PathBuf::from(LEGACY_SAVE),
    ];
    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            return parse_save(&content);
        }
    }
    None
}

pub fn has_save() -> bool {
    let candidates = [
        legacy_path(),
        PathBuf::from(LEGACY_SAVE),
    ];
    candidates.iter().any(|p| p.exists())
}

pub fn delete_save() {
    let _ = std::fs::remove_file(legacy_path());
    let _ = std::fs::remove_file(LEGACY_SAVE);
}

// ══════════════════════════════════════════════════════════════
// Serialization
// ══════════════════════════════════════════════════════════════

fn tile_to_char(t: Tile) -> char {
    match t {
        Tile::Empty        => ' ',
        Tile::Brick        => '#',
        Tile::Concrete     => '=',
        Tile::Ladder       => 'H',
        Tile::Rope         => '-',
        Tile::Gold         => '$',
        Tile::HiddenLadder => '~',
        Tile::TrapBrick    => 'T',
    }
}

fn char_to_tile(c: char) -> Tile {
    match c {
        '#' => Tile::Brick,
        '=' => Tile::Concrete,
        'H' => Tile::Ladder,
        '-' => Tile::Rope,
        '$' => Tile::Gold,
        '~' => Tile::HiddenLadder,
        'T' => Tile::TrapBrick,
        _   => Tile::Empty,
    }
}

fn facing_str(f: Facing) -> &'static str {
    match f { Facing::Left => "L", Facing::Right => "R" }
}

fn parse_facing(s: &str) -> Facing {
    if s == "L" { Facing::Left } else { Facing::Right }
}

fn state_str(s: ActorState) -> &'static str {
    match s {
        ActorState::OnGround => "G",
        ActorState::Falling  => "F",
        ActorState::OnLadder => "L",
        ActorState::OnRope   => "R",
        ActorState::InHole   => "H",
        ActorState::Dead     => "D",
    }
}

fn parse_state(s: &str) -> ActorState {
    match s {
        "G" => ActorState::OnGround,
        "F" => ActorState::Falling,
        "L" => ActorState::OnLadder,
        "R" => ActorState::OnRope,
        "H" => ActorState::InHole,
        "D" => ActorState::Dead,
        _   => ActorState::OnGround,
    }
}

fn serialize(level: usize, score: u32, lives: u32, snapshot: Option<&Snapshot>) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str(&format!("level={}\n", level));
    out.push_str(&format!("score={}\n", score));
    out.push_str(&format!("lives={}\n", lives));

    if let Some(snap) = snapshot {
        out.push_str("has_snapshot=1\n");
        out.push_str(&format!("tick={}\n", snap.tick));
        out.push_str(&format!("width={}\n", snap.width));
        out.push_str(&format!("height={}\n", snap.height));
        out.push_str(&format!("gold_remaining={}\n", snap.gold_remaining));
        out.push_str(&format!("gold_total={}\n", snap.gold_total));
        out.push_str(&format!("exit_enabled={}\n", if snap.exit_enabled { 1 } else { 0 }));
        out.push_str(&format!("player_spawn={},{}\n", snap.player_spawn.0, snap.player_spawn.1));

        let p = &snap.player;
        out.push_str(&format!("player={},{},{},{},{}\n",
            p.x, p.y, facing_str(p.facing), state_str(p.state), p.move_cooldown));

        for g in &snap.guards {
            out.push_str(&format!("guard={},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                g.id, g.x, g.y, facing_str(g.facing), state_str(g.state),
                if g.carry_gold { 1 } else { 0 }, g.carry_gold_timer,
                g.stuck_timer, g.move_cooldown,
                g.spawn_x, g.spawn_y, g.respawn_timer, g.separation_timer));
        }

        for h in &snap.holes {
            out.push_str(&format!("hole={},{},{},{}\n",
                h.x, h.y, h.open_remaining, h.close_remaining));
        }

        for d in &snap.digs {
            out.push_str(&format!("dig={},{},{},{}\n",
                d.x, d.y, d.ticks_remaining, d.total_ticks));
        }

        if !snap.exit_columns.is_empty() {
            let cols: Vec<String> = snap.exit_columns.iter().map(|c| c.to_string()).collect();
            out.push_str(&format!("exit_cols={}\n", cols.join(",")));
        }

        for &(x, y) in &snap.hidden_ladder_positions {
            out.push_str(&format!("hidden_ladder={},{}\n", x, y));
        }

        for row in &snap.tiles {
            let s: String = row.iter().map(|t| tile_to_char(*t)).collect();
            out.push_str(&format!("tile_row={}\n", s));
        }
    }

    out
}

// ══════════════════════════════════════════════════════════════
// Parsing
// ══════════════════════════════════════════════════════════════

fn parse_save(content: &str) -> Option<SaveData> {
    let mut level = None;
    let mut score = None;
    let mut lives = None;
    let mut has_snapshot = false;
    let mut tick: u64 = 0;
    let mut width: usize = 0;
    let mut height: usize = 0;
    let mut gold_remaining: usize = 0;
    let mut gold_total: usize = 0;
    let mut exit_enabled = false;
    let mut player_spawn = (0usize, 0usize);
    let mut player: Option<SnapshotPlayer> = None;
    let mut guards: Vec<SnapshotGuard> = vec![];
    let mut holes: Vec<SnapshotHole> = vec![];
    let mut digs: Vec<SnapshotDig> = vec![];
    let mut exit_columns: Vec<usize> = vec![];
    let mut hidden_ladders: Vec<(usize, usize)> = vec![];
    let mut tile_rows: Vec<Vec<Tile>> = vec![];

    for line in content.lines() {
        let line = line.trim_end(); // preserve leading spaces in tile_row

        if let Some(val) = line.strip_prefix("level=") {
            level = val.trim().parse().ok();
        } else if let Some(val) = line.strip_prefix("score=") {
            score = val.trim().parse().ok();
        } else if let Some(val) = line.strip_prefix("lives=") {
            lives = val.trim().parse().ok();
        } else if line.trim() == "has_snapshot=1" {
            has_snapshot = true;
        } else if let Some(val) = line.strip_prefix("tick=") {
            tick = val.trim().parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("width=") {
            width = val.trim().parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("height=") {
            height = val.trim().parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("gold_remaining=") {
            gold_remaining = val.trim().parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("gold_total=") {
            gold_total = val.trim().parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("exit_enabled=") {
            exit_enabled = val.trim() == "1";
        } else if let Some(val) = line.strip_prefix("player_spawn=") {
            let parts: Vec<&str> = val.split(',').collect();
            if parts.len() == 2 {
                player_spawn = (
                    parts[0].trim().parse().unwrap_or(0),
                    parts[1].trim().parse().unwrap_or(0),
                );
            }
        } else if let Some(val) = line.strip_prefix("player=") {
            player = parse_player(val);
        } else if let Some(val) = line.strip_prefix("guard=") {
            if let Some(g) = parse_guard(val) {
                guards.push(g);
            }
        } else if let Some(val) = line.strip_prefix("hole=") {
            if let Some(h) = parse_hole(val) {
                holes.push(h);
            }
        } else if let Some(val) = line.strip_prefix("dig=") {
            if let Some(d) = parse_dig(val) {
                digs.push(d);
            }
        } else if let Some(val) = line.strip_prefix("exit_cols=") {
            exit_columns = val.split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
        } else if let Some(val) = line.strip_prefix("hidden_ladder=") {
            let parts: Vec<&str> = val.split(',').collect();
            if parts.len() == 2 {
                if let (Ok(x), Ok(y)) = (parts[0].trim().parse(), parts[1].trim().parse()) {
                    hidden_ladders.push((x, y));
                }
            }
        } else if let Some(val) = line.strip_prefix("tile_row=") {
            tile_rows.push(val.chars().map(char_to_tile).collect());
        }
    }

    let snapshot = if has_snapshot && player.is_some() && !tile_rows.is_empty() {
        Some(Snapshot {
            tick,
            width,
            height,
            tiles: tile_rows,
            player: player.unwrap(),
            guards,
            holes,
            digs,
            gold_remaining,
            gold_total,
            exit_enabled,
            exit_columns,
            hidden_ladder_positions: hidden_ladders,
            player_spawn,
        })
    } else {
        None
    };

    Some(SaveData {
        level: level?,
        score: score?,
        lives: lives?,
        snapshot,
    })
}

fn parse_player(val: &str) -> Option<SnapshotPlayer> {
    let p: Vec<&str> = val.split(',').collect();
    if p.len() < 5 { return None; }
    Some(SnapshotPlayer {
        x: p[0].trim().parse().ok()?,
        y: p[1].trim().parse().ok()?,
        facing: parse_facing(p[2].trim()),
        state: parse_state(p[3].trim()),
        move_cooldown: p[4].trim().parse().ok()?,
    })
}

fn parse_guard(val: &str) -> Option<SnapshotGuard> {
    let p: Vec<&str> = val.split(',').collect();
    if p.len() < 13 { return None; }
    Some(SnapshotGuard {
        id: p[0].trim().parse().ok()?,
        x: p[1].trim().parse().ok()?,
        y: p[2].trim().parse().ok()?,
        facing: parse_facing(p[3].trim()),
        state: parse_state(p[4].trim()),
        carry_gold: p[5].trim() == "1",
        carry_gold_timer: p[6].trim().parse().ok()?,
        stuck_timer: p[7].trim().parse().ok()?,
        move_cooldown: p[8].trim().parse().ok()?,
        spawn_x: p[9].trim().parse().ok()?,
        spawn_y: p[10].trim().parse().ok()?,
        respawn_timer: p[11].trim().parse().ok()?,
        separation_timer: p[12].trim().parse().ok()?,
    })
}

fn parse_hole(val: &str) -> Option<SnapshotHole> {
    let p: Vec<&str> = val.split(',').collect();
    if p.len() < 4 { return None; }
    Some(SnapshotHole {
        x: p[0].trim().parse().ok()?,
        y: p[1].trim().parse().ok()?,
        open_remaining: p[2].trim().parse().ok()?,
        close_remaining: p[3].trim().parse().ok()?,
    })
}

fn parse_dig(val: &str) -> Option<SnapshotDig> {
    let p: Vec<&str> = val.split(',').collect();
    if p.len() < 4 { return None; }
    Some(SnapshotDig {
        x: p[0].trim().parse().ok()?,
        y: p[1].trim().parse().ok()?,
        ticks_remaining: p[2].trim().parse().ok()?,
        total_ticks: p[3].trim().parse().ok()?,
    })
}

/// Level loader with pack support.
///
/// ## Sources (priority order):
///   1. Active pack file (`.nlp` format)
///   2. `levels/` directory (individual `.txt` files)
///   3. Built-in embedded levels
///
/// ## Pack format (`.nlp` — NodeRunner Level Pack):
///   ```
///   ## Pack Name
///   ## Author: name
///   ## Description: blah blah
///   ---
///   # Level 1 - Name
///   @ 1,2 3,4
///   <16 map rows>
///   ---
///   # Level 2 - Name
///   <16 map rows>
///   ```
///
/// Levels are separated by a line containing only `---`.
/// Pack metadata lines start with `##`.
///
/// ## Single-level format (`.txt`):
///   Line 1: `# Level Name`
///   Optional: `@ x1,y1 x2,y2 ...` (hidden ladder metadata)
///   Lines: map rows
///
/// ## Tile legend:
///   '#' = Firewall (diggable)    '=' = Concrete (indestructible)
///   'H' = Ladder                 '-' = Rope
///   '$' = Token                  'P' = Player spawn
///   'E' = Sentinel spawn         '^' = Exit ladder column marker
///   '~' = Hidden ladder          'T' = Trap brick
///   ' ' = Empty

use std::path::{Path, PathBuf};

use crate::config::GameConfig;
use crate::domain::entity::{Guard, Player};
use crate::domain::tile::Tile;
use crate::sim::world::{PackInfo, Phase, WorldState};

/// Runtime level data (owned strings, loaded from file or embedded).
pub struct LevelDef {
    pub name: String,
    pub rows: Vec<String>,
    pub extra_hidden_ladders: Vec<(usize, usize)>,
}

// ══════════════════════════════════════════════════════════════
// Public API
// ══════════════════════════════════════════════════════════════

/// Load a level into the world state. Preserves score and lives.
pub fn load_level(world: &mut WorldState, level_idx: usize, config: &GameConfig) {
    let levels = load_levels_for_active_pack(world, config);

    if level_idx >= levels.len() {
        world.phase = Phase::GameComplete;
        return;
    }

    let def = &levels[level_idx];
    world.current_level = level_idx;
    world.total_levels = levels.len();
    world.level_name = def.name.clone();

    let height = def.rows.len();
    let width = if height > 0 { def.rows[0].len() } else { 28 };
    world.width = width;
    world.height = height;
    world.tiles = vec![vec![Tile::Empty; width]; height];
    world.guards.clear();
    world.holes.clear();
    world.digs.clear();
    world.exit_columns.clear();
    world.hidden_ladder_positions.clear();
    world.gold_remaining = 0;
    world.exit_enabled = false;
    world.tick = 0;

    let mut guard_id = 0;

    for (y, row) in def.rows.iter().enumerate() {
        for (x, ch) in row.chars().enumerate() {
            if x >= width { break; }
            match ch {
                '#' => world.tiles[y][x] = Tile::Brick,
                '=' => world.tiles[y][x] = Tile::Concrete,
                'H' => world.tiles[y][x] = Tile::Ladder,
                '-' => world.tiles[y][x] = Tile::Rope,
                '$' => {
                    world.tiles[y][x] = Tile::Gold;
                    world.gold_remaining += 1;
                }
                'P' => {
                    world.player = Player::new(x, y);
                    world.player_spawn = (x, y);
                }
                'E' => {
                    let mut g = Guard::new(guard_id, x, y);
                    g.move_cooldown = config.speed.guard_move_rate;
                    world.guards.push(g);
                    guard_id += 1;
                }
                '^' => {
                    if !world.exit_columns.contains(&x) {
                        world.exit_columns.push(x);
                    }
                }
                'T' => world.tiles[y][x] = Tile::TrapBrick,
                '~' => {
                    world.hidden_ladder_positions.push((x, y));
                }
                _ => {}
            }
        }
    }

    for &(x, y) in &def.extra_hidden_ladders {
        if !world.hidden_ladder_positions.contains(&(x, y)) {
            world.hidden_ladder_positions.push((x, y));
        }
    }

    world.gold_total = world.gold_remaining;
    world.base_tiles = world.tiles.clone();
    world.rebuild_hole_grid(); // empty grid for fresh level
    world.phase = Phase::LevelIntro;
    world.anim_tick = 0;
    world.set_message(&def.name, 80);

    world.camera.center_on(
        world.player_spawn.0, world.player_spawn.1,
        world.width, world.height,
    );
}

/// Get list of level names for the currently active pack.
pub fn get_level_list_for_pack(world: &WorldState, config: &GameConfig) -> Vec<String> {
    let levels = load_levels_for_active_pack(world, config);
    levels.iter().map(|l| l.name.clone()).collect()
}

/// Scan for available packs (`.nlp` files) + levels/ dir + embedded.
pub fn scan_packs(config: &GameConfig) -> Vec<PackInfo> {
    let mut packs = vec![];

    // 1. Built-in embedded levels
    let embedded = embedded_levels();
    packs.push(PackInfo {
        name: "Built-in Levels".to_string(),
        author: "NodeRunner".to_string(),
        description: format!("{} levels included with the game", embedded.len()),
        level_count: embedded.len(),
        path: "__embedded__".to_string(),
    });

    // 2. levels/ directory (individual .txt files)
    let dir = &config.levels_dir;
    if dir.is_dir() {
        let dir_levels = load_from_directory(dir);
        if !dir_levels.is_empty() {
            let dir_name = dir.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            packs.push(PackInfo {
                name: format!("{}/  (individual files)", dir_name),
                author: String::new(),
                description: format!("{} levels from {}/", dir_levels.len(), dir_name),
                level_count: dir_levels.len(),
                path: "__levels__".to_string(),
            });
        }
    }

    // 3. .nlp pack files from packs/ directory
    let search_dirs = pack_search_dirs();
    for base in &search_dirs {
        let packs_dir = base.join("packs");
        if !packs_dir.is_dir() { continue; }
        let entries = match std::fs::read_dir(&packs_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "nlp") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let info = parse_pack_info(&content, &path);
                    packs.push(info);
                }
            }
        }
    }

    packs
}

/// Switch active pack, reload level list.
pub fn switch_pack(world: &mut WorldState, pack: &PackInfo, config: &GameConfig) {
    world.active_pack = pack.name.clone();
    world.active_pack_path = pack.path.clone();
    world.level_names = get_level_list_for_pack(world, config);
    world.total_levels = world.level_names.len();
}

// ══════════════════════════════════════════════════════════════
// Internal: load levels for active pack
// ══════════════════════════════════════════════════════════════

fn load_levels_for_active_pack(world: &WorldState, config: &GameConfig) -> Vec<LevelDef> {
    match world.active_pack_path.as_str() {
        "__embedded__" => embedded_levels(),
        "__levels__" => {
            let dir = &config.levels_dir;
            if dir.is_dir() {
                let mut levels = load_from_directory(dir);
                levels.sort_by(|a, b| a.0.cmp(&b.0));
                levels.into_iter().map(|(_, def)| def).collect()
            } else {
                embedded_levels()
            }
        }
        pack_path => {
            let path = PathBuf::from(pack_path);
            if let Ok(content) = std::fs::read_to_string(&path) {
                let levels = parse_pack_levels(&content);
                if !levels.is_empty() {
                    return levels;
                }
            }
            // Fallback
            embedded_levels()
        }
    }
}

// ══════════════════════════════════════════════════════════════
// Pack parsing
// ══════════════════════════════════════════════════════════════

/// Parse pack metadata without fully parsing all levels (fast scan).
fn parse_pack_info(content: &str, path: &Path) -> PackInfo {
    let mut name = String::new();
    let mut author = String::new();
    let mut description = String::new();

    // Read metadata from `##` lines at the top
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## Author:") {
            author = trimmed["## Author:".len()..].trim().to_string();
        } else if trimmed.starts_with("## Description:") {
            description = trimmed["## Description:".len()..].trim().to_string();
        } else if trimmed.starts_with("##") {
            if name.is_empty() {
                name = trimmed[2..].trim().to_string();
            }
        } else if trimmed == "---" || trimmed.starts_with('#') {
            break; // Done with metadata
        }
    }

    // Count levels by counting `---` separators
    let level_count = content.lines()
        .filter(|l| l.trim() == "---")
        .count()
        .max(1); // At least 1 level if file exists

    // Fallback name from filename
    if name.is_empty() {
        name = path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
    }

    PackInfo {
        name,
        author,
        description,
        level_count,
        path: path.to_string_lossy().to_string(),
    }
}

/// Parse all levels from a `.nlp` pack file.
fn parse_pack_levels(content: &str) -> Vec<LevelDef> {
    let mut levels = vec![];
    let mut current_section = String::new();
    let mut in_levels = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "---" {
            // Flush previous section as a level
            if in_levels && !current_section.is_empty() {
                if let Some(def) = parse_level_file(&current_section) {
                    levels.push(def);
                }
            }
            current_section.clear();
            in_levels = true;
            continue;
        }

        if !in_levels {
            // Skip pack metadata lines before first ---
            continue;
        }

        current_section.push_str(line);
        current_section.push('\n');
    }

    // Flush last section
    if !current_section.is_empty() {
        if let Some(def) = parse_level_file(&current_section) {
            levels.push(def);
        }
    }

    levels
}

// ══════════════════════════════════════════════════════════════
// Single-level file parsing
// ══════════════════════════════════════════════════════════════

/// Parse a single level from text content.
fn parse_level_file(content: &str) -> Option<LevelDef> {
    let mut name = String::new();
    let mut rows = vec![];
    let mut extra_hidden_ladders = vec![];

    for line in content.lines() {
        if line.starts_with('#') && name.is_empty() && is_name_line(line) {
            name = line[1..].trim().to_string();
        } else if line.starts_with("@ ") {
            for pair in line[2..].split_whitespace() {
                let parts: Vec<&str> = pair.split(',').collect();
                if parts.len() == 2 {
                    if let (Ok(x), Ok(y)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>()) {
                        extra_hidden_ladders.push((x, y));
                    }
                }
            }
        } else {
            rows.push(line.to_string());
        }
    }

    while rows.last().map_or(false, |r| r.trim().is_empty()) {
        rows.pop();
    }

    if rows.is_empty() {
        return None;
    }

    let max_width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    for row in &mut rows {
        if row.len() < max_width {
            row.extend(std::iter::repeat(' ').take(max_width - row.len()));
        }
    }

    if name.is_empty() {
        name = "Unnamed Node".to_string();
    }

    Some(LevelDef { name, rows, extra_hidden_ladders })
}

/// Distinguish `#Level Name` from `############################` (level data).
/// A name line starts with `#` and contains at least one letter.
/// A data line starts with `#` followed only by level tile characters.
fn is_name_line(line: &str) -> bool {
    let after_hash = &line[1..];
    // If the rest contains any letter, it's a name
    after_hash.chars().any(|c| c.is_alphabetic())
}

// ══════════════════════════════════════════════════════════════
// Directory loading (individual .txt files)
// ══════════════════════════════════════════════════════════════

fn load_from_directory(dir: &Path) -> Vec<(String, LevelDef)> {
    let mut results = vec![];

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return results,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "txt") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Some(def) = parse_level_file(&content) {
                    let filename = path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    results.push((filename, def));
                }
            }
        }
    }

    results
}

/// Search dirs for packs: exe dir, CWD (same logic as config).
fn pack_search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![];

    // 1. Exe directory (resolve symlinks)
    if let Ok(exe) = std::env::current_exe() {
        let resolved = exe.canonicalize().unwrap_or(exe);
        if let Some(parent) = resolved.parent() {
            dirs.push(parent.to_path_buf());
        }
    }

    // 2. CWD
    if let Ok(cwd) = std::env::current_dir() {
        if !dirs.iter().any(|d| d == &cwd) {
            dirs.push(cwd);
        }
    }

    // 3. XDG data home (~/.local/share/noderunner)
    if let Ok(home) = std::env::var("HOME") {
        let xdg = std::path::PathBuf::from(&home).join(".local/share/noderunner");
        if xdg.is_dir() && !dirs.iter().any(|d| d == &xdg) {
            dirs.push(xdg);
        }
    }

    // 4. System data (/usr/share/noderunner)
    let sys = std::path::PathBuf::from("/usr/share/noderunner");
    if sys.is_dir() && !dirs.iter().any(|d| d == &sys) {
        dirs.push(sys);
    }

    if dirs.is_empty() {
        dirs.push(std::path::PathBuf::from("."));
    }
    dirs
}

// ══════════════════════════════════════════════════════════════
// Embedded fallback levels
// ══════════════════════════════════════════════════════════════

fn embedded_levels() -> Vec<LevelDef> {
    vec![
        make_embedded("Node 1 - Genesis Block", &[
            "        ^                ^  ",
            "                            ",
            "    $                       ",
            "########H#######            ",
            "        H----------     $   ",
            "        H    ##H   ######H##",
            "      E H    ##H      $E H  ",
            "##H#################H#######",
            "  H                 H       ",
            "  H           E     H       ",
            "##########H#########H       ",
            "          H         H       ",
            "       $  H---------H  $    ",
            "    H#######        #######H",
            "    H          P  $        H",
            "############################",
        ]),
        make_embedded("Node 2 - Locked Vault", &[
            "   ^                  ^     ",
            "                            ",
            "    $     $    $     $      ",
            "   ###   ###  ###   ###     ",
            "   H                  H     ",
            "   H  --------  ---   H     ",
            "   H  H      H  H H  H     ",
            "   H $H  E   H  H$H  H     ",
            "   H##H######H  H#H  H     ",
            "   H  H      H  H H  H     ",
            "   H  H--  --H--H-H--H     ",
            "   H  H   $     H    H     ",
            " P H  H  ###  E H  $ H     ",
            " ##H==H=========H==##H==   ",
            "   H  H         H    H     ",
            "============================",
        ]),
        make_embedded("Node 3 - Firewall Maze", &[
            "         ^      ^           ",
            "  $  $  $  $  $  $  $  $    ",
            "  ## ## ## ## ## ## ## ##    ",
            "          E         E       ",
            "   ------H------H------    ",
            "         H      H      H   ",
            "    $    H  $   H   $  H   ",
            "   #T##  H #T## H #### H   ",
            "      E  H      H    E H   ",
            "   H-----H--  --H----H-H   ",
            "   H     H      H    H H   ",
            "   H  $  H   $  H  $ H H   ",
            " P H ##T H  ##  H ## H H   ",
            " ##H=====H======H====H=H   ",
            "   H     H      H    H H   ",
            "============================",
        ]),
        make_embedded("Node 4 - Deep Stack", &[
            "    ^                  ^    ",
            "                            ",
            "    --------------------    ",
            "    H    $    $    $   H    ",
            "    H   ###  ###  ###  H    ",
            "    H   H         H   H    ",
            "    H   H    E    H   H    ",
            "    H---H--####---H---H    ",
            "    H   H         H   H    ",
            "    H   H  $   $  H   H    ",
            "    H   H ##  ## EH   H    ",
            "    H   H   H    H    H    ",
            "  P H $ H   H  $ H  $ H    ",
            "  ##H###H===H====H=####    ",
            "    H   H   H    H         ",
            "============================",
        ]),
        make_embedded("Node 5 - Final Fork", &[
            "           ^                ",
            "  $           $          $  ",
            "  ##   ----H----   ----  ## ",
            "       H   H   H  H  H     ",
            "    $  H   H   H  H$ H     ",
            "   ### H E H   H  H##H     ",
            "   H   H###H   H  H  H     ",
            "   H---H   H---H--H--H     ",
            "   H   H $ H      H  H     ",
            "   H   H## H   E  H  H     ",
            "   H   H   H #### H  H     ",
            "   H $EH   H      H$ H     ",
            " P H ##H---H------H##H     ",
            " ##H===H===H======H===     ",
            "   H   H   H      H        ",
            "============================",
        ]),
    ]
}

fn make_embedded(name: &str, map: &[&str]) -> LevelDef {
    LevelDef {
        name: name.to_string(),
        rows: map.iter().map(|s| s.to_string()).collect(),
        extra_hidden_ladders: vec![],
    }
}

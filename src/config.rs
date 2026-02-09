/// External configuration loader.
///
/// Reads `config.toml` from the executable's directory (or CWD).
/// Falls back to sensible defaults if the file is missing or incomplete.

use serde::Deserialize;
use std::path::PathBuf;

// ── Public Config Struct ──

#[derive(Clone, Debug)]
pub struct GameConfig {
    pub speed: SpeedConfig,
    pub gamepad: GamepadConfig,
    pub levels_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct SpeedConfig {
    pub tick_rate_ms: u64,
    pub player_move_rate: u32,
    pub guard_move_rate: u32,
    pub dig_duration: u32,
    pub hole_open_ticks: u32,    // phase 1: fully open, guards can fall in
    pub hole_close_ticks: u32,   // phase 2: filling animation, short
    pub trap_escape_ticks: u32,
    pub guard_respawn_ticks: u32,
    pub gold_carry_ticks: u32,   // max ticks a guard holds gold before dropping
}

#[derive(Clone, Debug)]
pub struct GamepadConfig {
    pub hack_left: Vec<String>,
    pub hack_right: Vec<String>,
    pub confirm: Vec<String>,
    pub cancel: Vec<String>,
    pub restart: Vec<String>,
}

// ── TOML Schema (with serde defaults) ──

#[derive(Deserialize, Debug, Default)]
struct TomlConfig {
    #[serde(default)]
    speed: TomlSpeed,
    #[serde(default)]
    gamepad: TomlGamepad,
    #[serde(default)]
    general: TomlGeneral,
}

#[derive(Deserialize, Debug)]
struct TomlSpeed {
    #[serde(default = "default_tick_rate")]
    tick_rate_ms: u64,
    #[serde(default = "default_player_move")]
    player_move_rate: u32,
    #[serde(default = "default_guard_move")]
    guard_move_rate: u32,
    #[serde(default = "default_dig_duration")]
    dig_duration: u32,
    #[serde(default = "default_hole_open")]
    hole_open_ticks: u32,
    #[serde(default = "default_hole_close")]
    hole_close_ticks: u32,
    #[serde(default = "default_trap_escape")]
    trap_escape_ticks: u32,
    #[serde(default = "default_guard_respawn")]
    guard_respawn_ticks: u32,
    #[serde(default = "default_gold_carry")]
    gold_carry_ticks: u32,
}

#[derive(Deserialize, Debug)]
struct TomlGamepad {
    #[serde(default = "default_hack_left")]
    hack_left: Vec<String>,
    #[serde(default = "default_hack_right")]
    hack_right: Vec<String>,
    #[serde(default = "default_confirm")]
    confirm: Vec<String>,
    #[serde(default = "default_cancel")]
    cancel: Vec<String>,
    #[serde(default = "default_restart")]
    restart: Vec<String>,
}

#[derive(Deserialize, Debug)]
struct TomlGeneral {
    #[serde(default = "default_levels_dir")]
    levels_dir: String,
}

// ── Defaults ──

fn default_tick_rate() -> u64 { 75 }
fn default_player_move() -> u32 { 2 }
fn default_guard_move() -> u32 { 5 }
fn default_dig_duration() -> u32 { 5 }
fn default_hole_open() -> u32 { 100 }    // 7.5s open (original ~10s)
fn default_hole_close() -> u32 { 20 }    // 1.5s fill animation
fn default_trap_escape() -> u32 { 70 }   // 5.25s guard escape (before hole closes)
fn default_guard_respawn() -> u32 { 40 }
fn default_gold_carry() -> u32 { 150 }  // ~11s at 75ms tick = guards drop gold after ~11s

fn default_hack_left() -> Vec<String> { vec!["B".into(), "Y".into(), "L1".into()] }
fn default_hack_right() -> Vec<String> { vec!["A".into(), "X".into(), "R1".into()] }
fn default_confirm() -> Vec<String> { vec!["Start".into()] }
fn default_cancel() -> Vec<String> { vec!["Select".into()] }
fn default_restart() -> Vec<String> { vec!["Start".into()] }
fn default_levels_dir() -> String { "levels".into() }

impl Default for TomlSpeed {
    fn default() -> Self {
        TomlSpeed {
            tick_rate_ms: default_tick_rate(),
            player_move_rate: default_player_move(),
            guard_move_rate: default_guard_move(),
            dig_duration: default_dig_duration(),
            hole_open_ticks: default_hole_open(),
            hole_close_ticks: default_hole_close(),
            trap_escape_ticks: default_trap_escape(),
            guard_respawn_ticks: default_guard_respawn(),
            gold_carry_ticks: default_gold_carry(),
        }
    }
}

impl Default for TomlGamepad {
    fn default() -> Self {
        TomlGamepad {
            hack_left: default_hack_left(),
            hack_right: default_hack_right(),
            confirm: default_confirm(),
            cancel: default_cancel(),
            restart: default_restart(),
        }
    }
}

impl Default for TomlGeneral {
    fn default() -> Self {
        TomlGeneral {
            levels_dir: default_levels_dir(),
        }
    }
}

// ── Loading ──

impl GameConfig {
    /// Load config from `config.toml`.
    /// Search order: (1) exe directory, (2) current working directory.
    /// Missing file or missing keys gracefully fall back to defaults.
    pub fn load() -> Self {
        let search_dirs = candidate_dirs();

        // Find config.toml
        let toml_cfg = load_toml(&search_dirs);

        // Resolve levels directory
        let levels_dir_str = &toml_cfg.general.levels_dir;
        let levels_dir = if PathBuf::from(levels_dir_str).is_absolute() {
            PathBuf::from(levels_dir_str)
        } else {
            // Search candidate dirs for the levels folder
            search_dirs.iter()
                .map(|d| d.join(levels_dir_str))
                .find(|p| p.is_dir())
                .unwrap_or_else(|| {
                    // Default: relative to CWD
                    PathBuf::from(levels_dir_str)
                })
        };

        GameConfig {
            speed: SpeedConfig {
                tick_rate_ms: toml_cfg.speed.tick_rate_ms,
                player_move_rate: toml_cfg.speed.player_move_rate,
                guard_move_rate: toml_cfg.speed.guard_move_rate,
                dig_duration: toml_cfg.speed.dig_duration,
                hole_open_ticks: toml_cfg.speed.hole_open_ticks,
                hole_close_ticks: toml_cfg.speed.hole_close_ticks,
                trap_escape_ticks: toml_cfg.speed.trap_escape_ticks,
                guard_respawn_ticks: toml_cfg.speed.guard_respawn_ticks,
                gold_carry_ticks: toml_cfg.speed.gold_carry_ticks,
            },
            gamepad: GamepadConfig {
                hack_left: toml_cfg.gamepad.hack_left,
                hack_right: toml_cfg.gamepad.hack_right,
                confirm: toml_cfg.gamepad.confirm,
                cancel: toml_cfg.gamepad.cancel,
                restart: toml_cfg.gamepad.restart,
            },
            levels_dir,
        }
    }
}

/// Candidate directories to search: exe dir + CWD + system paths (deduplicated).
fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![];

    // 1. Directory of the running executable
    if let Ok(exe) = std::env::current_exe() {
        // Resolve symlinks so /usr/bin/noderunner → /usr/games/noderunner
        // still finds data relative to the real binary.
        let resolved = exe.canonicalize().unwrap_or(exe);
        if let Some(parent) = resolved.parent() {
            dirs.push(parent.to_path_buf());
        }
    }

    // 2. Current working directory
    if let Ok(cwd) = std::env::current_dir() {
        if !dirs.iter().any(|d| d == &cwd) {
            dirs.push(cwd);
        }
    }

    // 3. XDG data home (~/.local/share/noderunner)
    if let Ok(home) = std::env::var("HOME") {
        let xdg = PathBuf::from(&home).join(".local/share/noderunner");
        if xdg.is_dir() && !dirs.iter().any(|d| d == &xdg) {
            dirs.push(xdg);
        }
    }

    // 4. System data directory (/usr/share/noderunner)
    let sys = PathBuf::from("/usr/share/noderunner");
    if sys.is_dir() && !dirs.iter().any(|d| d == &sys) {
        dirs.push(sys);
    }

    // 5. Fallback
    if dirs.is_empty() {
        dirs.push(PathBuf::from("."));
    }

    dirs
}

/// Search for config.toml in candidate directories.
fn load_toml(search_dirs: &[PathBuf]) -> TomlConfig {
    for dir in search_dirs {
        let path = dir.join("config.toml");
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(text) => match toml::from_str::<TomlConfig>(&text) {
                    Ok(cfg) => return cfg,
                    Err(e) => {
                        eprintln!("Warning: config.toml parse error: {e}");
                        eprintln!("Using default settings.");
                        return TomlConfig::default();
                    }
                },
                Err(e) => {
                    eprintln!("Warning: could not read {}: {e}", path.display());
                }
            }
        }
    }
    TomlConfig::default()
}

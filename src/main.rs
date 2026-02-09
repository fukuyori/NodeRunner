/// Entry point and game loop.

mod config;
mod domain;
mod sim;
mod ui;

use std::time::{Duration, Instant};

use crossterm::event::KeyCode;

use config::GameConfig;
use domain::entity::{Facing, FrameInput, MoveDir};
use sim::event::GameEvent;
use sim::level::{load_level, scan_packs, switch_pack};
use sim::save;
use sim::step;
use sim::world::{Phase, WorldState};
use ui::gamepad::GamepadState;
use ui::input::InputState;
use ui::renderer::Renderer;
use ui::sound::SoundEngine;

const FRAME_SLEEP: Duration = Duration::from_millis(5);

fn main() {
    let config = GameConfig::load();

    let mut world = WorldState::new();
    world.speed = config.speed.clone();

    // Auto-detect initial level source: levels/ dir takes priority if it has files
    if config.levels_dir.is_dir() {
        let has_txt = std::fs::read_dir(&config.levels_dir)
            .map(|e| e.flatten().any(|f| f.path().extension().map_or(false, |x| x == "txt")))
            .unwrap_or(false);
        if has_txt {
            let dir_name = config.levels_dir.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            world.active_pack = format!("{}/  (individual files)", dir_name);
            world.active_pack_path = "__levels__".to_string();
        }
    }

    // Pre-load level list for title/select screens
    world.level_names = sim::level::get_level_list_for_pack(&world, &config);
    world.total_levels = world.level_names.len();
    world.has_save = save::has_save();

    let mut renderer = Renderer::new();

    if let Err(e) = renderer.init() {
        eprintln!("Terminal init failed: {e}");
        return;
    }

    let sound = SoundEngine::new();

    let result = game_loop(&mut world, &mut renderer, sound.as_ref(), &config);

    if let Err(e) = renderer.cleanup() {
        eprintln!("Terminal cleanup failed: {e}");
    }

    if let Err(e) = result {
        eprintln!("Game error: {e}");
    }

    println!();
    println!("Thanks for playing Node Runner: Mainnet Protocol!");
    println!("Final Score: {}", world.score);
}

fn game_loop(
    world: &mut WorldState,
    renderer: &mut Renderer,
    sound: Option<&SoundEngine>,
    config: &GameConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut kb = InputState::new();
    let mut gp = GamepadState::new();
    gp.load_button_config(&config.gamepad);
    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(config.speed.tick_rate_ms);

    let mut pending_dig: Option<Facing> = None;
    let mut prev_intro_rows: usize = 0;

    loop {
        kb.drain_events();
        gp.update();

        if kb.ctrl_c_pressed() {
            break;
        }
        if handle_meta(world, sound, &kb, &gp, config) {
            break;
        }

        if world.phase == Phase::Playing && !world.paused {
            if let Some(dir) = detect_dig_press(&kb, &gp) {
                pending_dig = Some(dir);
            }
        }

        if last_tick.elapsed() >= tick_rate {
            // Pause blocks simulation but allows anim_tick for blink
            if world.paused {
                world.anim_tick = world.anim_tick.wrapping_add(1);
                if world.message_timer > 0 {
                    world.message_timer -= 1;
                    if world.message_timer == 0 { world.message.clear(); }
                }
                last_tick = Instant::now();
            } else {
            match world.phase {
                Phase::Playing => {
                    let frame_input = FrameInput {
                        movement: detect_movement(&kb, &gp),
                        dig: pending_dig.take(),
                    };
                    let events = step::step(world, frame_input);
                    process_sound_events(sound, &events);

                    // Camera follows player
                    world.camera.follow(
                        world.player.x, world.player.y,
                        world.width, world.height,
                    );
                }
                Phase::LevelIntro => {
                    tick_level_intro(world);
                    if let Some(sfx) = sound {
                        let rows_visible = calc_intro_rows_visible(world);
                        if rows_visible > prev_intro_rows && rows_visible <= world.height {
                            sfx.play_intro_blip(rows_visible, world.height);
                        }
                        prev_intro_rows = rows_visible;
                    }
                }
                Phase::LevelReady => {
                    world.anim_tick += 1;
                    prev_intro_rows = 0;
                }
                Phase::LevelOutro => {
                    tick_level_outro(world);
                }
                Phase::Dying => {
                    tick_dying(world, sound);
                }
                Phase::LevelSelect => {
                    world.anim_tick += 1;
                }
                Phase::PackSelect => {
                    world.anim_tick += 1;
                }
                _ => {}
            }

            // Global: tick message timer (works in all phases)
            if world.message_timer > 0 {
                world.message_timer -= 1;
                if world.message_timer == 0 { world.message.clear(); }
            }

            last_tick = Instant::now();
            } // else !paused
        }

        renderer.render(world)?;
        std::thread::sleep(FRAME_SLEEP);
    }

    Ok(())
}

fn calc_intro_rows_visible(world: &WorldState) -> usize {
    let tick = world.anim_tick;
    if tick <= INTRO_NAME_TICKS {
        0
    } else {
        ((tick - INTRO_NAME_TICKS) / INTRO_ROW_INTERVAL).min(world.height as u32) as usize
    }
}

fn process_sound_events(sound: Option<&SoundEngine>, events: &[GameEvent]) {
    let sfx = match sound {
        Some(s) => s,
        None => return,
    };
    for event in events {
        match event {
            GameEvent::GoldPicked { .. } => sfx.play_gold(),
            GameEvent::HoleCreated { .. } => sfx.play_dig(),
            GameEvent::PlayerFallStart => sfx.play_fall(),
            GameEvent::PlayerKilled => sfx.play_die(),
            GameEvent::AllGoldCollected => sfx.play_all_gold(),
            GameEvent::StageCleared => sfx.play_clear(),
            _ => {}
        }
    }
}

// ── Key Constants ──

const KEYS_LEFT: &[KeyCode] = &[KeyCode::Left, KeyCode::Char('a'), KeyCode::Char('A')];
const KEYS_RIGHT: &[KeyCode] = &[KeyCode::Right, KeyCode::Char('d'), KeyCode::Char('D')];
const KEYS_UP: &[KeyCode] = &[KeyCode::Up, KeyCode::Char('w'), KeyCode::Char('W')];
const KEYS_DOWN: &[KeyCode] = &[KeyCode::Down, KeyCode::Char('s'), KeyCode::Char('S')];
const KEYS_DIG_L: &[KeyCode] = &[KeyCode::Char('z'), KeyCode::Char('Z'), KeyCode::Char('q'), KeyCode::Char('Q')];
const KEYS_DIG_R: &[KeyCode] = &[KeyCode::Char('x'), KeyCode::Char('X'), KeyCode::Char('e'), KeyCode::Char('E')];
const KEYS_RESTART: &[KeyCode] = &[KeyCode::Char('r'), KeyCode::Char('R')];
const KEYS_CONFIRM: &[KeyCode] = &[KeyCode::Enter, KeyCode::Char(' ')];

fn detect_dig_press(kb: &InputState, gp: &GamepadState) -> Option<Facing> {
    if kb.any_pressed(KEYS_DIG_L) || gp.dig_left_pressed() {
        Some(Facing::Left)
    } else if kb.any_pressed(KEYS_DIG_R) || gp.dig_right_pressed() {
        Some(Facing::Right)
    } else {
        None
    }
}

fn detect_movement(kb: &InputState, gp: &GamepadState) -> Option<MoveDir> {
    if kb.any_held(KEYS_UP) || kb.any_pressed(KEYS_UP) || gp.up_held() {
        Some(MoveDir::Up)
    } else if kb.any_held(KEYS_DOWN) || kb.any_pressed(KEYS_DOWN) || gp.down_held() {
        Some(MoveDir::Down)
    } else if kb.any_held(KEYS_LEFT) || kb.any_pressed(KEYS_LEFT) || gp.left_held() {
        Some(MoveDir::Left)
    } else if kb.any_held(KEYS_RIGHT) || kb.any_pressed(KEYS_RIGHT) || gp.right_held() {
        Some(MoveDir::Right)
    } else {
        None
    }
}

/// Reset to title screen, preserving config and level list.
fn return_to_title(world: &mut WorldState) {
    let speed = world.speed.clone();
    let names = std::mem::take(&mut world.level_names);
    let total = world.total_levels;
    let active_pack = std::mem::take(&mut world.active_pack);
    let active_pack_path = std::mem::take(&mut world.active_pack_path);
    *world = WorldState::new();
    world.speed = speed;
    world.level_names = names;
    world.total_levels = total;
    world.active_pack = active_pack;
    world.active_pack_path = active_pack_path;
    world.has_save = save::has_save();
    world.paused = false;
    world.phase = Phase::Title;
}

/// Start a new game from level 0.
fn start_new_game(world: &mut WorldState, config: &GameConfig) {
    world.score = 0;
    world.lives = 5;
    load_level(world, 0, config);
}

/// Start game from a specific level.
fn start_from_level(world: &mut WorldState, level: usize, score: u32, lives: u32, config: &GameConfig) {
    world.score = score;
    world.lives = lives;
    load_level(world, level, config);
}

/// Capture snapshot only if currently in Playing phase.
/// Non-playing phases return None → load will restart level from scratch.
fn snapshot_if_playing(world: &WorldState) -> Option<save::Snapshot> {
    if world.phase == Phase::Playing {
        Some(save::capture_snapshot(world))
    } else {
        None
    }
}

/// Load from SaveData: restore snapshot if present, otherwise start level fresh.
fn load_save_data(world: &mut WorldState, data: &save::SaveData, config: &GameConfig) {
    world.score = data.score;
    world.lives = data.lives;
    load_level(world, data.level, config);

    if let Some(ref snap) = data.snapshot {
        // Restore mid-game state on top of the freshly loaded level
        save::restore_snapshot(world, snap);
        world.phase = Phase::Playing;
    }
    // If no snapshot, load_level already set Phase::LevelIntro → normal start
}

/// Open the pack select screen (F3 filer).
fn open_pack_select(world: &mut WorldState, config: &GameConfig) {
    world.pack_list = scan_packs(config);
    // Position cursor on the currently active pack
    world.pack_cursor = world.pack_list.iter()
        .position(|p| p.path == world.active_pack_path)
        .unwrap_or(0);
    world.pack_scroll = 0;
    world.phase = Phase::PackSelect;
    world.anim_tick = 0;
}

fn handle_meta(world: &mut WorldState, _sound: Option<&SoundEngine>, kb: &InputState, gp: &GamepadState, config: &GameConfig) -> bool {
    let confirm = kb.any_pressed(KEYS_CONFIRM) || gp.confirm_pressed();
    let esc = kb.any_pressed(&[KeyCode::Esc]) || gp.cancel_pressed();

    // ── F-key handling (works in Playing, Paused, LevelReady, LevelIntro) ──
    let in_game = matches!(world.phase,
        Phase::Playing | Phase::LevelReady | Phase::LevelIntro
        | Phase::LevelOutro | Phase::LevelComplete
    );

    if in_game || world.paused {
        // F1: Pause / Resume
        if kb.any_pressed(&[KeyCode::F(1)]) {
            world.paused = !world.paused;
            if world.paused {
                world.set_message("PAUSED  [F1] Resume", 0);
            } else {
                world.message.clear();
                world.message_timer = 0;
            }
            return false;
        }

        // While paused: F1/F3/F5-F12 respond
        if world.paused {
            // F3: Pack select (works while paused)
            if kb.any_pressed(&[KeyCode::F(3)]) {
                let snap = save::capture_snapshot(world);
                world.paused = false;
                let _ = save::save_game(world.current_level, world.score, world.lives, Some(&snap));
                open_pack_select(world, config);
                return false;
            }
            // F5-F8: Save to slot (works while paused — snapshot captured)
            for slot in 1..=4u8 {
                let fkey = KeyCode::F(slot + 4);
                if kb.any_pressed(&[fkey]) {
                    let snap = save::capture_snapshot(world);
                    let level = world.current_level;
                    match save::save_slot(slot, level, world.score, world.lives, Some(&snap)) {
                        Ok(_) => world.set_message(
                            &format!("Mid-game Saved Slot {} (Node {})", slot, level + 1), 40,
                        ),
                        Err(_) => world.set_message("Save failed!", 40),
                    }
                    return false;
                }
            }
            // F9-F12: Load from slot (works while paused)
            for slot in 1..=4u8 {
                let fkey = KeyCode::F(slot + 8);
                if kb.any_pressed(&[fkey]) {
                    if let Some(data) = save::load_slot(slot) {
                        world.paused = false;
                        load_save_data(world, &data, config);
                        world.set_message(&format!("Loaded Slot {}", slot), 40);
                    } else {
                        world.set_message(&format!("Slot {} is empty", slot), 40);
                    }
                    return false;
                }
            }
            // ESC while paused: save snapshot and return to title
            if kb.any_pressed(&[KeyCode::Esc]) || gp.cancel_pressed() {
                let snap = save::capture_snapshot(world);
                world.paused = false;
                let _ = save::save_game(world.current_level, world.score, world.lives, Some(&snap));
                return_to_title(world);
                return false;
            }
            return false; // Block all other input while paused
        }

        // F2: Restart level
        if kb.any_pressed(&[KeyCode::F(2)]) {
            if world.phase == Phase::Playing || world.phase == Phase::LevelReady {
                step::restart_level(world);
                world.phase = Phase::Playing;
                world.set_message("Level Restarted", 30);
            }
            return false;
        }

        // F3: Pack select
        if kb.any_pressed(&[KeyCode::F(3)]) {
            let snap = snapshot_if_playing(world);
            let _ = save::save_game(world.current_level, world.score, world.lives, snap.as_ref());
            open_pack_select(world, config);
            return false;
        }

        // F4: Change Level (go to level select)
        if kb.any_pressed(&[KeyCode::F(4)]) {
            let snap = snapshot_if_playing(world);
            let _ = save::save_game(world.current_level, world.score, world.lives, snap.as_ref());
            world.phase = Phase::LevelSelect;
            world.paused = false;
            world.select_cursor = world.current_level;
            let visible = 16_usize;
            world.select_scroll = if world.current_level >= visible {
                world.current_level - visible / 2
            } else {
                0
            };
            world.anim_tick = 0;
            return false;
        }

        // F5-F8: Save to slot 1-4
        for slot in 1..=4u8 {
            let fkey = KeyCode::F(slot + 4); // F5=slot1, F6=slot2, F7=slot3, F8=slot4
            if kb.any_pressed(&[fkey]) {
                let level = world.current_level;
                let score = world.score;
                let lives = world.lives;
                let snap = snapshot_if_playing(world);
                match save::save_slot(slot, level, score, lives, snap.as_ref()) {
                    Ok(_) => {
                        let kind = if snap.is_some() { "Mid-game" } else { "Level" };
                        world.set_message(
                            &format!("{} Saved Slot {} (Node {})", kind, slot, level + 1), 40,
                        );
                    }
                    Err(_) => world.set_message("Save failed!", 40),
                }
                return false;
            }
        }

        // F9-F12: Load from slot 1-4
        for slot in 1..=4u8 {
            let fkey = KeyCode::F(slot + 8); // F9=slot1, F10=slot2, F11=slot3, F12=slot4
            if kb.any_pressed(&[fkey]) {
                if let Some(data) = save::load_slot(slot) {
                    let has_snap = data.snapshot.is_some();
                    load_save_data(world, &data, config);
                    let kind = if has_snap { "Resumed" } else { "Loaded" };
                    world.set_message(&format!("{} Slot {}", kind, slot), 40);
                } else {
                    world.set_message(&format!("Slot {} is empty", slot), 40);
                }
                return false;
            }
        }
    }

    match world.phase {
        // ── Title Screen ──
        Phase::Title => {
            if confirm {
                start_new_game(world, config);
            } else if kb.any_pressed(&[KeyCode::Char('c'), KeyCode::Char('C')]) {
                if let Some(data) = save::load_save() {
                    load_save_data(world, &data, config);
                }
            } else if kb.any_pressed(&[KeyCode::Char('l'), KeyCode::Char('L')]) {
                world.phase = Phase::LevelSelect;
                world.select_cursor = 0;
                world.select_scroll = 0;
                world.anim_tick = 0;
            } else if kb.any_pressed(&[KeyCode::F(3)]) {
                open_pack_select(world, config);
            } else if kb.any_pressed(&[KeyCode::Char('q'), KeyCode::Char('Q')]) || esc {
                return true;
            }
            // F9-F12: Load from slot on title screen
            for slot in 1..=4u8 {
                let fkey = KeyCode::F(slot + 8);
                if kb.any_pressed(&[fkey]) {
                    if let Some(data) = save::load_slot(slot) {
                        load_save_data(world, &data, config);
                        world.set_message(&format!("Loaded Slot {}", slot), 40);
                    } else {
                        world.set_message(&format!("Slot {} is empty", slot), 40);
                    }
                    return false;
                }
            }
        }

        // ── Level Select ──
        Phase::LevelSelect => {
            let total = world.total_levels;
            if total == 0 {
                return_to_title(world);
                return false;
            }

            if kb.any_pressed(&[KeyCode::Up]) || gp.up_held() {
                if world.select_cursor > 0 {
                    world.select_cursor -= 1;
                    if world.select_cursor < world.select_scroll {
                        world.select_scroll = world.select_cursor;
                    }
                }
            } else if kb.any_pressed(&[KeyCode::Down]) || gp.down_held() {
                if world.select_cursor + 1 < total {
                    world.select_cursor += 1;
                    let visible = 16_usize;
                    if world.select_cursor >= world.select_scroll + visible {
                        world.select_scroll = world.select_cursor - visible + 1;
                    }
                }
            } else if kb.any_pressed(&[KeyCode::PageUp]) {
                world.select_cursor = world.select_cursor.saturating_sub(16);
                if world.select_cursor < world.select_scroll {
                    world.select_scroll = world.select_cursor;
                }
            } else if kb.any_pressed(&[KeyCode::PageDown]) {
                world.select_cursor = (world.select_cursor + 16).min(total.saturating_sub(1));
                let visible = 16_usize;
                if world.select_cursor >= world.select_scroll + visible {
                    world.select_scroll = world.select_cursor - visible + 1;
                }
            } else if confirm {
                start_from_level(world, world.select_cursor, 0, 5, config);
            } else if kb.any_pressed(&[KeyCode::F(3)]) {
                open_pack_select(world, config);
            } else if esc {
                return_to_title(world);
            }
        }

        // ── Pack Select (F3 filer) ──
        Phase::PackSelect => {
            let total = world.pack_list.len();
            if total == 0 {
                return_to_title(world);
                return false;
            }

            if kb.any_pressed(&[KeyCode::Up]) || gp.up_held() {
                if world.pack_cursor > 0 {
                    world.pack_cursor -= 1;
                    if world.pack_cursor < world.pack_scroll {
                        world.pack_scroll = world.pack_cursor;
                    }
                }
            } else if kb.any_pressed(&[KeyCode::Down]) || gp.down_held() {
                if world.pack_cursor + 1 < total {
                    world.pack_cursor += 1;
                    let visible = 12_usize;
                    if world.pack_cursor >= world.pack_scroll + visible {
                        world.pack_scroll = world.pack_cursor - visible + 1;
                    }
                }
            } else if confirm {
                // Switch to selected pack
                let pack = world.pack_list[world.pack_cursor].clone();
                switch_pack(world, &pack, config);
                let pack_name = pack.name.clone();
                return_to_title(world);
                world.set_message(&format!("Pack: {}", pack_name), 60);
            } else if esc {
                return_to_title(world);
            }
        }

        // ── Level Intro ──
        Phase::LevelIntro => {
            if confirm {
                world.phase = Phase::LevelReady;
                world.anim_tick = 0;
            } else if esc {
                let _ = save::save_game(world.current_level, world.score, world.lives, None);
                return_to_title(world);
            }
        }

        // ── Level Ready ──
        Phase::LevelReady => {
            let any_key = confirm
                || kb.any_pressed(KEYS_LEFT)
                || kb.any_pressed(KEYS_RIGHT)
                || kb.any_pressed(KEYS_UP)
                || kb.any_pressed(KEYS_DOWN)
                || kb.any_pressed(KEYS_DIG_L)
                || kb.any_pressed(KEYS_DIG_R)
                || gp.left_held() || gp.right_held()
                || gp.up_held() || gp.down_held()
                || gp.confirm_pressed();
            if any_key {
                world.phase = Phase::Playing;
                world.message.clear();
                world.message_timer = 0;
            } else if esc {
                let _ = save::save_game(world.current_level, world.score, world.lives, None);
                return_to_title(world);
            }
        }

        // ── Playing ──
        Phase::Playing => {
            if esc {
                let snap = save::capture_snapshot(world);
                let _ = save::save_game(world.current_level, world.score, world.lives, Some(&snap));
                return_to_title(world);
            }
            if kb.any_pressed(KEYS_RESTART) || gp.restart_pressed() {
                step::restart_level(world);
            }
        }

        // ── Level Outro ──
        Phase::LevelOutro => {
            if esc {
                let next = world.current_level + 1;
                let _ = save::save_game(next, world.score, world.lives, None);
                return_to_title(world);
            }
        }

        // ── Level Complete ──
        Phase::LevelComplete => {
            if confirm {
                let next = world.current_level + 1;
                let _ = save::save_game(next, world.score, world.lives, None);
                load_level(world, next, config);
            } else if esc {
                let next = world.current_level + 1;
                let _ = save::save_game(next, world.score, world.lives, None);
                return_to_title(world);
            }
        }

        // ── Dying ──
        Phase::Dying => {
            // Can't skip
        }

        // ── Game Over ──
        Phase::GameOver => {
            if confirm {
                save::delete_save();
                let speed = world.speed.clone();
                let names = std::mem::take(&mut world.level_names);
                let total = world.total_levels;
                let active_pack = std::mem::take(&mut world.active_pack);
                let active_pack_path = std::mem::take(&mut world.active_pack_path);
                *world = WorldState::new();
                world.speed = speed;
                world.level_names = names;
                world.total_levels = total;
                world.active_pack = active_pack;
                world.active_pack_path = active_pack_path;
                world.has_save = false;
                start_new_game(world, config);
            } else if esc {
                save::delete_save();
                return_to_title(world);
            }
        }

        // ── Game Complete ──
        Phase::GameComplete => {
            if confirm || esc {
                save::delete_save();
                return_to_title(world);
            }
        }
    }

    false
}

// ── Animation tick functions ──

const INTRO_NAME_TICKS: u32 = 8;
const INTRO_ROW_INTERVAL: u32 = 2;
const INTRO_TOTAL: u32 = INTRO_NAME_TICKS + 16 * INTRO_ROW_INTERVAL + 4;

fn tick_level_intro(world: &mut WorldState) {
    world.anim_tick += 1;
    if world.anim_tick >= INTRO_TOTAL {
        world.phase = Phase::LevelReady;
        world.anim_tick = 0;
    }
}

fn tick_level_outro(world: &mut WorldState) {
    world.anim_tick += 1;
    if world.anim_tick % 3 == 0 {
        world.anim_player_y -= 1;
    }
    if world.anim_player_y < -2 {
        world.phase = Phase::LevelComplete;
    }
}

const DYING_TICKS: u32 = 18;

fn tick_dying(world: &mut WorldState, _sound: Option<&SoundEngine>) {
    world.anim_tick += 1;
    if world.anim_tick >= DYING_TICKS {
        world.lives = world.lives.saturating_sub(1);
        if world.lives == 0 {
            world.phase = Phase::GameOver;
            world.set_message("CONNECTION LOST", 120);
        } else {
            step::restart_level(world);
            world.phase = Phase::LevelReady;  // wait for key input before restarting
            world.anim_tick = 0;
        }
    }
}

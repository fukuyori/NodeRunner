/// The step function: advances the world by one tick.
///
/// Processing order:
///   1. Dig resolution
///   2. Movement resolution (player → guards)
///   3. Trap brick collapse
///   4. Gravity resolution
///   5. Hole effects (trap guards)
///   6. Collision / contact events
///   7. Timer updates (hole regen, guard escape)
///   8. Win / lose check
///
/// Physics queries use terrain (physics::terrain_at) + occupancy (physics::has_*).
/// Terrain = what the cell IS.  Occupancy = who is there.
/// Movement = terrain.passable && !occupied.
/// Support = terrain support || trapped guard below.

use crate::domain::entity::{ActorState, DigInProgress, Facing, FrameInput, Hole, MoveDir};
use crate::domain::rules::{self, MapView};
use crate::domain::physics;
use crate::domain::ai;
use crate::domain::tile::Tile;
use super::event::GameEvent;
use super::world::{Phase, WorldState};

// ══════════════════════════════════════════════════════════════
// Main entry point
// ══════════════════════════════════════════════════════════════

pub fn step(world: &mut WorldState, input: FrameInput) -> Vec<GameEvent> {
    if world.phase != Phase::Playing { return vec![]; }

    let mut events: Vec<GameEvent> = Vec::new();
    world.tick += 1;

    if world.message_timer > 0 {
        world.message_timer -= 1;
        if world.message_timer == 0 { world.message.clear(); }
    }

    resolve_dig(world, input.dig, &mut events);
    resolve_dig_progress(world, &mut events);
    world.rebuild_hole_grid(); // holes may have been added by dig completion
    resolve_player_movement(world, input.movement);
    resolve_guard_movement(world);
    resolve_trap_bricks(world, &mut events);
    resolve_gravity(world, &mut events);
    resolve_hole_traps(world, &mut events);
    resolve_gold_pickup(world, &mut events);
    resolve_guard_gold_drop(world, &mut events);
    if resolve_enemy_collision(world, &mut events) { return events; }
    resolve_timers(world, &mut events);
    resolve_win(world, &mut events);

    events
}

// ══════════════════════════════════════════════════════════════
// Helpers: closing hole check
// ══════════════════════════════════════════════════════════════

/// Is the player currently inside a hole that is in its closing phase?
/// If so, the player is trapped and will be buried when it seals.
fn player_in_closing_hole(world: &WorldState) -> bool {
    if !world.player.alive { return false; }
    world.holes.iter().any(|h| {
        h.x == world.player.x && h.y == world.player.y && h.is_closing()
    })
}

/// Can gold be placed at (x, y)?
///
/// Gold may only rest on a solid surface (Brick, Concrete, TrapBrick)
/// or the map bottom.  The tile at (x, y) itself must be Empty (not
/// Ladder, Rope, Gold, or any other non-empty tile).
fn can_drop_gold_at(world: &WorldState, x: usize, y: usize) -> bool {
    if x >= world.width || y >= world.height { return false; }
    let tile = world.terrain_at(x, y);
    // Must be an empty cell — not a ladder, rope, gold, etc.
    if tile != Tile::Empty { return false; }
    // Must have solid ground below (or be at map bottom)
    if y + 1 >= world.height { return true; }
    world.terrain_at(x, y + 1).is_solid()
}

// ══════════════════════════════════════════════════════════════
// Dig
// ══════════════════════════════════════════════════════════════

fn resolve_dig(world: &mut WorldState, dig_dir: Option<Facing>, events: &mut Vec<GameEvent>) {
    let dir = match dig_dir { Some(d) => d, None => return };
    let map = MapView { tiles: &world.tiles, width: world.width, height: world.height };
    let p = &world.player;

    if let Some((dx, dy)) = rules::can_dig(&map, p.x, p.y, p.state, dir) {
        if world.digs.iter().any(|d| d.x == dx && d.y == dy) { return; }
        if world.holes.iter().any(|h| h.x == dx && h.y == dy) { return; }
        // Can't dig under gold (prevents gold falling into hole edge cases)
        if dy > 0 && world.terrain_at(dx, dy - 1) == Tile::Gold { return; }
        world.digs.push(DigInProgress::new(dx, dy, world.speed.dig_duration));
        events.push(GameEvent::HoleCreated { x: dx, y: dy });
    }
}

fn resolve_dig_progress(world: &mut WorldState, _events: &mut Vec<GameEvent>) {
    let mut completed = vec![];
    for (i, dig) in world.digs.iter_mut().enumerate() {
        if dig.ticks_remaining > 0 { dig.ticks_remaining -= 1; }
        if dig.ticks_remaining == 0 { completed.push(i); }
    }
    for &i in completed.iter().rev() {
        let dig = world.digs.remove(i);
        world.set_tile(dig.x, dig.y, Tile::Empty);
        world.holes.push(Hole::new(
            dig.x, dig.y,
            world.speed.hole_open_ticks,
            world.speed.hole_close_ticks,
        ));
    }
}

// ══════════════════════════════════════════════════════════════
// Player movement (uses tile-only rules — player falls through holes)
// ══════════════════════════════════════════════════════════════

fn resolve_player_movement(world: &mut WorldState, movement: Option<MoveDir>) {
    if !world.player.alive { return; }
    if world.player.state == ActorState::Falling { return; }

    // Trapped in closing hole — no escape
    if player_in_closing_hole(world) { return; }

    if world.player.move_cooldown > 0 {
        world.player.move_cooldown -= 1;
        return;
    }

    let (dx, dy): (i32, i32) = match movement {
        Some(MoveDir::Left)  => (-1, 0),
        Some(MoveDir::Right) => (1, 0),
        Some(MoveDir::Up)    => (0, -1),
        Some(MoveDir::Down)  => (0, 1),
        None => return,
    };

    let map = MapView { tiles: &world.tiles, width: world.width, height: world.height };
    let p = &world.player;
    let can_move = match (dx, dy) {
        (-1, 0) => rules::can_move_left(&map, p.x, p.y, p.state),
        (1, 0)  => rules::can_move_right(&map, p.x, p.y, p.state),
        (0, -1) => rules::can_move_up(&map, p.x, p.y, p.state),
        (0, 1)  => rules::can_move_down(&map, p.x, p.y, p.state),
        _ => false,
    };

    if can_move {
        world.player.x = (world.player.x as i32 + dx) as usize;
        world.player.y = (world.player.y as i32 + dy) as usize;
        if dx < 0 { world.player.facing = Facing::Left; }
        if dx > 0 { world.player.facing = Facing::Right; }
        world.player.move_cooldown = world.speed.player_move_rate;
        let map = MapView { tiles: &world.tiles, width: world.width, height: world.height };
        world.player.state = rules::resolve_state(&map, world.player.x, world.player.y, world.player.state);
        // Tile-based resolve doesn't see guards as floor.
        // If resolve says Falling but a standing guard provides support, override.
        if world.player.state == ActorState::Falling {
            if physics::has_support_for_player(
                &world.tiles, world.width, world.height,
                &world.hole_grid, &world.guards,
                world.player.x, world.player.y,
            ) {
                world.player.state = ActorState::OnGround;
            }
        }
    }
}

// ══════════════════════════════════════════════════════════════
// Guard movement — terrain + occupancy
// ══════════════════════════════════════════════════════════════

// ══════════════════════════════════════════════════════════════
// Guard movement — Intent → Resolve model
//
// Phase 1: Collect intents (where each guard wants to go)
// Phase 2: Resolve conflicts (active guard collisions)
// Phase 3: Apply moves + update state
// ══════════════════════════════════════════════════════════════

/// A guard's movement intent.
struct MoveIntent {
    guard_idx: usize,
    target_x: usize,
    target_y: usize,
    dx: i32,
}

fn resolve_guard_movement(world: &mut WorldState) {
    let px = world.player.x;
    let py = world.player.y;

    // ── Phase 0: Tick separation timers ──
    for g in world.guards.iter_mut() {
        if g.separation_timer > 0 {
            g.separation_timer -= 1;
        }
    }

    // ── Phase 1: Collect intents ──
    let mut intents: Vec<MoveIntent> = Vec::new();

    for i in 0..world.guards.len() {
        if world.guards[i].state == ActorState::Dead
            || world.guards[i].state == ActorState::InHole
            || world.guards[i].state == ActorState::Falling
        { continue; }

        if world.guards[i].move_cooldown > 0 {
            world.guards[i].move_cooldown -= 1;
            continue;
        }

        let gx = world.guards[i].x;
        let gy = world.guards[i].y;

        // Choose AI mode: separation or chase
        let (dx, dy) = if world.guards[i].separation_timer > 0 {
            ai::find_separation_direction(
                &world.tiles, world.width, world.height,
                &world.hole_grid, &world.guards,
                i, gx, gy, world.guards[i].state, px, py,
            )
        } else {
            ai::find_direction(
                &world.tiles, world.width, world.height,
                &world.hole_grid, &world.guards,
                gx, gy, world.guards[i].state, px, py,
            )
        };

        if dx == 0 && dy == 0 { continue; }

        let nx = (gx as i32 + dx) as usize;
        let ny = (gy as i32 + dy) as usize;
        if nx >= world.width || ny >= world.height { continue; }

        // TERRAIN: must be passable
        let target = physics::terrain_at(
            &world.tiles, world.width, world.height, &world.hole_grid, nx, ny,
        );
        if !target.passable { continue; }

        intents.push(MoveIntent { guard_idx: i, target_x: nx, target_y: ny, dx });
    }

    // ── Phase 2: Resolve conflicts ──
    // Trapped guards are FLOOR — they don't block movement.
    // Only active (non-Dead, non-InHole) guards block each other.
    // Also check: two intents targeting the same cell → first wins.
    let mut occupied_targets: Vec<(usize, usize)> = Vec::new();
    let mut approved: Vec<usize> = Vec::new(); // indices into intents

    for (idx, intent) in intents.iter().enumerate() {
        let tx = intent.target_x;
        let ty = intent.target_y;

        // Blocked by existing active guard at target?
        let blocked_by_guard = physics::has_active_guard_except(
            &world.guards, tx, ty, intent.guard_idx,
        );
        if blocked_by_guard { continue; }

        // Blocked by another intent already approved for this cell?
        let blocked_by_intent = occupied_targets.iter().any(|&(ox, oy)| ox == tx && oy == ty);
        if blocked_by_intent { continue; }

        occupied_targets.push((tx, ty));
        approved.push(idx);
    }

    // ── Phase 3: Apply moves ──
    for &idx in &approved {
        let intent = &intents[idx];
        let i = intent.guard_idx;
        world.guards[i].x = intent.target_x;
        world.guards[i].y = intent.target_y;
        if intent.dx < 0 { world.guards[i].facing = Facing::Left; }
        if intent.dx > 0 { world.guards[i].facing = Facing::Right; }
        world.guards[i].move_cooldown = world.speed.guard_move_rate;
    }

    // ── Phase 4: Update state for all movable guards ──
    for i in 0..world.guards.len() {
        if world.guards[i].state == ActorState::Dead
            || world.guards[i].state == ActorState::InHole
        { continue; }

        world.guards[i].state = physics::resolve_state(
            &world.tiles, world.width, world.height,
            &world.hole_grid, &world.guards,
            world.guards[i].x, world.guards[i].y, world.guards[i].state,
        );
    }

    // ── Phase 5: Detect guard contact → activate separation ──
    // When two active guards are adjacent or overlapping, both enter
    // separation mode to avoid clustering.
    let n = world.guards.len();
    for i in 0..n {
        if world.guards[i].state == ActorState::Dead
            || world.guards[i].state == ActorState::InHole
        { continue; }
        for j in (i + 1)..n {
            if world.guards[j].state == ActorState::Dead
                || world.guards[j].state == ActorState::InHole
            { continue; }

            let dist = (world.guards[i].x as i32 - world.guards[j].x as i32).abs()
                     + (world.guards[i].y as i32 - world.guards[j].y as i32).abs();

            // Adjacent (dist=1) or overlapping (dist=0) → separate
            if dist <= 1 {
                if world.guards[i].separation_timer == 0 {
                    world.guards[i].separation_timer = ai::SEPARATION_TICKS;
                }
                if world.guards[j].separation_timer == 0 {
                    world.guards[j].separation_timer = ai::SEPARATION_TICKS;
                }
            }
        }
    }
}

// ══════════════════════════════════════════════════════════════
// Trap brick collapse
// ══════════════════════════════════════════════════════════════

fn resolve_trap_bricks(world: &mut WorldState, events: &mut Vec<GameEvent>) {
    let mut positions: Vec<(usize, usize)> = Vec::new();
    if world.player.alive { positions.push((world.player.x, world.player.y)); }
    for g in &world.guards {
        if g.state != ActorState::Dead { positions.push((g.x, g.y)); }
    }
    for (x, y) in positions {
        let below_y = y + 1;
        if below_y >= world.height { continue; }
        if world.terrain_at(x, below_y) == Tile::TrapBrick {
            world.set_tile(x, below_y, Tile::Empty);
            events.push(GameEvent::TrapCollapsed { x, y: below_y });
        }
    }
}

// ══════════════════════════════════════════════════════════════
// Gravity
// ══════════════════════════════════════════════════════════════

fn resolve_gravity(world: &mut WorldState, events: &mut Vec<GameEvent>) {
    // ── Player gravity ──
    {
        // Trapped in closing hole — don't fall, wait to be buried
        if player_in_closing_hole(world) {
            // Stay put; resolve_timers will kill when hole seals
        } else {
            let was_falling = world.player.state == ActorState::Falling;
            let px = world.player.x;
            let py = world.player.y;

            // Player uses player-specific support (active guards = floor)
            let full_support = physics::has_support_for_player(
                &world.tiles, world.width, world.height,
                &world.hole_grid, &world.guards, px, py,
            );
            let map = MapView { tiles: &world.tiles, width: world.width, height: world.height };

            if !full_support {
                // No support at all — fall
                if py + 1 < world.height && world.terrain_at(px, py + 1).is_passable() {
                    world.player.y += 1;
                    world.player.state = ActorState::Falling;
                    if !was_falling {
                        events.push(GameEvent::PlayerFallStart);
                    }
                }
            } else if world.player.state == ActorState::Falling {
                // Just landed (terrain, trapped guard, or active guard head)
                world.player.state = rules::resolve_state(
                    &map, world.player.x, world.player.y, world.player.state,
                );
                // If tile-based resolve still says Falling but we have guard support
                if world.player.state == ActorState::Falling {
                    world.player.state = ActorState::OnGround;
                }
                world.player.move_cooldown = 0;
            }
        }
    }

    // ── Guard gravity ──
    for i in 0..world.guards.len() {
        if world.guards[i].state == ActorState::Dead
            || world.guards[i].state == ActorState::InHole
        { continue; }

        let gx = world.guards[i].x;
        let gy = world.guards[i].y;

        // TERRAIN: is guard currently in a hole?
        let here = physics::terrain_at(
            &world.tiles, world.width, world.height, &world.hole_grid, gx, gy,
        );
        if here.hole {
            // Guard is in a hole cell. Trap if no one else already trapped here.
            if !physics::has_trapped_guard_except(&world.guards, gx, gy, i) {
                // Drop gold above hole (gy-1) if no gold already there
                let drop_y = if gy > 0 { Some(gy - 1) } else { None };
                guard_enter_hole(world, i, gx, drop_y);
                continue;
            }
            // Another guard already trapped — this guard is ON TOP (bridge)
            if world.guards[i].state == ActorState::Falling {
                world.guards[i].state = ActorState::OnGround;
            }
            continue;
        }

        // SUPPORT: terrain + trapped guard below
        let supported = physics::has_support_for_guard(
            &world.tiles, world.width, world.height,
            &world.hole_grid, &world.guards, gx, gy, i,
        );

        if supported {
            if world.guards[i].state == ActorState::Falling {
                world.guards[i].state = ActorState::OnGround;
            }
            continue;
        }

        // No support — fall
        let ny = gy + 1;
        if ny >= world.height {
            world.guards[i].state = ActorState::OnGround;
            continue;
        }

        let below = physics::terrain_at(
            &world.tiles, world.width, world.height, &world.hole_grid, gx, ny,
        );

        if !below.passable {
            world.guards[i].state = ActorState::OnGround;
        } else if below.hole && !physics::has_trapped_guard(&world.guards, gx, ny) {
            // Empty hole below — guard falls in, gold stays at current pos (above hole)
            world.guards[i].y = ny;
            guard_enter_hole(world, i, gx, Some(gy));
        } else if below.hole && physics::has_trapped_guard(&world.guards, gx, ny) {
            // Hole with trapped guard below — acts as floor (bridge)
            world.guards[i].state = ActorState::OnGround;
        } else {
            // Normal fall through empty space
            world.guards[i].y = ny;
            world.guards[i].state = ActorState::Falling;
        }
    }
}

/// Guard enters a hole. If carrying gold and the drop position is free, drops it.
/// Otherwise keeps carrying gold (will try to drop on escape).
///
/// Lode Runner rule: one hole produces at most one gold.
/// If gold already exists at the drop position, guard keeps the gold.
fn guard_enter_hole(world: &mut WorldState, idx: usize, hole_x: usize, drop_y: Option<usize>) {
    world.guards[idx].state = ActorState::InHole;
    world.guards[idx].stuck_timer = world.speed.trap_escape_ticks;

    if world.guards[idx].carry_gold {
        if let Some(dy) = drop_y {
            if can_drop_gold_at(world, hole_x, dy) {
                world.set_tile(hole_x, dy, Tile::Gold);
                world.guards[idx].carry_gold = false;
                world.guards[idx].carry_gold_timer = 0;
            }
            // else: can't drop here → keep carrying
        }
        // drop_y == None (top row): can't drop, keep carrying
    }
}

// ══════════════════════════════════════════════════════════════
// Hole traps (catch guards that walked into a hole)
// ══════════════════════════════════════════════════════════════

fn resolve_hole_traps(world: &mut WorldState, events: &mut Vec<GameEvent>) {
    for i in 0..world.guards.len() {
        if world.guards[i].state == ActorState::InHole
            || world.guards[i].state == ActorState::Dead
        { continue; }

        let gx = world.guards[i].x;
        let gy = world.guards[i].y;

        let here = physics::terrain_at(
            &world.tiles, world.width, world.height, &world.hole_grid, gx, gy,
        );
        if here.hole && !physics::has_trapped_guard_except(&world.guards, gx, gy, i) {
            events.push(GameEvent::GuardTrapped { id: world.guards[i].id, x: gx, y: gy });
            let drop_y = if gy > 0 { Some(gy - 1) } else { None };
            guard_enter_hole(world, i, gx, drop_y);
        }
    }
}

// ══════════════════════════════════════════════════════════════
// Gold & collision
// ══════════════════════════════════════════════════════════════

fn resolve_gold_pickup(world: &mut WorldState, events: &mut Vec<GameEvent>) {
    let px = world.player.x;
    let py = world.player.y;
    if world.terrain_at(px, py) == Tile::Gold {
        world.set_tile(px, py, Tile::Empty);
        world.gold_remaining -= 1;
        world.score += 100;
        events.push(GameEvent::GoldPicked { x: px, y: py });
        if world.gold_remaining == 0 {
            events.push(GameEvent::AllGoldCollected);
            enable_exit(world);
            world.set_message("All tokens mined! Escape to the top!", 80);
        }
    }
    for i in 0..world.guards.len() {
        let g = &world.guards[i];
        if g.state == ActorState::Dead || g.state == ActorState::InHole { continue; }
        if g.carry_gold { continue; }
        if world.terrain_at(g.x, g.y) == Tile::Gold {
            world.set_tile(g.x, g.y, Tile::Empty);
            world.guards[i].carry_gold = true;
            world.guards[i].carry_gold_timer = 0;
        }
    }
}

/// Guards drop gold after carrying it for too long.
/// Gold is placed at the guard's current position only on solid ground.
fn resolve_guard_gold_drop(world: &mut WorldState, events: &mut Vec<GameEvent>) {
    let limit = world.speed.gold_carry_ticks;
    if limit == 0 { return; } // 0 = disabled

    for i in 0..world.guards.len() {
        if !world.guards[i].carry_gold { continue; }
        if world.guards[i].state == ActorState::Dead { continue; }
        // InHole guards still tick — they may have entered with gold
        world.guards[i].carry_gold_timer += 1;
        if world.guards[i].carry_gold_timer >= limit {
            let gx = world.guards[i].x;
            let gy = world.guards[i].y;
            if can_drop_gold_at(world, gx, gy) {
                world.set_tile(gx, gy, Tile::Gold);
                world.guards[i].carry_gold = false;
                world.guards[i].carry_gold_timer = 0;
                events.push(GameEvent::GuardDroppedGold { x: gx, y: gy });
            }
            // Can't drop here (ladder, rope, no solid ground) → keep trying next tick
        }
    }
}

fn resolve_enemy_collision(world: &mut WorldState, events: &mut Vec<GameEvent>) -> bool {
    if !world.player.alive { return false; }
    let px = world.player.x;
    let py = world.player.y;

    for g in &world.guards {
        if g.state == ActorState::Dead || g.state == ActorState::InHole { continue; }

        // Same cell = death
        if g.x == px && g.y == py {
            events.push(GameEvent::PlayerKilled);
            player_die(world);
            return true;
        }

        // Guard directly above player = death (enemy on player's head)
        if py > 0 && g.x == px && g.y == py - 1 {
            events.push(GameEvent::PlayerKilled);
            player_die(world);
            return true;
        }

        // Guard below player = safe (player walks on enemy's head)
        // — no action needed, handled by has_support_for_player
    }

    false
}

// ══════════════════════════════════════════════════════════════
// Timers — escape uses terrain + occupancy
// ══════════════════════════════════════════════════════════════

fn resolve_timers(world: &mut WorldState, events: &mut Vec<GameEvent>) {
    for i in 0..world.guards.len() {
        if world.guards[i].state == ActorState::InHole {
            if world.guards[i].stuck_timer > 0 { world.guards[i].stuck_timer -= 1; }
            if world.guards[i].stuck_timer == 0 { try_escape(world, i); }
        }

        if world.guards[i].state == ActorState::Dead {
            world.guards[i].respawn_timer += 1;
            if world.guards[i].respawn_timer >= world.speed.guard_respawn_ticks {
                let rx = world.guards[i].spawn_x;
                let ry = 1usize;
                let occupied = world.guards.iter().enumerate().any(|(j, other)| {
                    j != i && other.state != ActorState::Dead
                    && other.x == rx && other.y == ry
                });
                if !occupied {
                    world.guards[i].x = rx;
                    world.guards[i].y = ry;
                    world.guards[i].state = ActorState::OnGround;
                    world.guards[i].respawn_timer = 0;
                    world.guards[i].carry_gold = false;
                    world.guards[i].carry_gold_timer = 0;
                    world.guards[i].separation_timer = 0;
                    events.push(GameEvent::GuardRespawned { id: world.guards[i].id });
                }
            }
        }
    }

    // Hole lifecycle (2-phase: open → closing → sealed)
    let mut holes_to_remove = vec![];
    for (idx, hole) in world.holes.iter_mut().enumerate() {
        let done = hole.tick();
        if done { holes_to_remove.push(idx); }
    }
    for &idx in holes_to_remove.iter().rev() {
        let hx = world.holes[idx].x;
        let hy = world.holes[idx].y;

        // Restore brick via clear_tile (reverts to base)
        world.clear_tile(hx, hy);
        events.push(GameEvent::HoleFilled { x: hx, y: hy });

        // Player buried — always killed (no push-up, original Lode Runner behavior)
        if world.player.x == hx && world.player.y == hy && world.player.alive {
            events.push(GameEvent::PlayerKilled);
            player_die(world);
        }

        // Push or kill guards in the hole
        for i in 0..world.guards.len() {
            if world.guards[i].x != hx || world.guards[i].y != hy { continue; }
            if world.guards[i].state == ActorState::InHole {
                world.guards[i].state = ActorState::Dead;
                world.guards[i].respawn_timer = 0;
                world.score += 50;
                events.push(GameEvent::GuardKilled { id: world.guards[i].id, x: hx, y: hy });
                // Guard dies with gold → place above sealed brick
                if world.guards[i].carry_gold {
                    world.guards[i].carry_gold = false;
                    world.guards[i].carry_gold_timer = 0;
                    if hy > 0 && can_drop_gold_at(world, hx, hy - 1) {
                        world.set_tile(hx, hy - 1, Tile::Gold);
                    }
                }
            } else if world.guards[i].state != ActorState::Dead {
                if hy > 0 && world.terrain_at(hx, hy - 1).is_passable() {
                    world.guards[i].y -= 1;
                }
            }
        }
        world.holes.remove(idx);
    }

    // Rebuild hole grid after removals
    if !holes_to_remove.is_empty() {
        world.rebuild_hole_grid();
    }
}

/// Guard escapes hole: diagonal (x±1, y-1) toward player.
fn try_escape(world: &mut WorldState, i: usize) {
    let gx = world.guards[i].x;
    let gy = world.guards[i].y;
    let px = world.player.x;

    let dirs: [i32; 2] = if px > gx { [1, -1] } else { [-1, 1] };

    for &dx in &dirs {
        let ex = (gx as i32 + dx) as usize;
        let ey = if gy > 0 { gy - 1 } else { continue };
        if ex >= world.width { continue; }

        // TERRAIN: passable?
        let target = physics::terrain_at(
            &world.tiles, world.width, world.height, &world.hole_grid, ex, ey,
        );
        if !target.passable { continue; }

        // SUPPORT: must have support at destination
        let supported = physics::has_support(
            &world.tiles, world.width, world.height,
            &world.hole_grid, &world.guards, ex, ey,
        );
        if !supported { continue; }

        // OCCUPANCY: no other guard at target
        let blocked = world.guards.iter().enumerate().any(|(j, other)| {
            j != i && other.state != ActorState::Dead
            && other.x == ex && other.y == ey
        });
        if blocked { continue; }

        world.guards[i].x = ex;
        world.guards[i].y = ey;
        world.guards[i].state = ActorState::OnGround;
        if dx < 0 { world.guards[i].facing = Facing::Left; }
        if dx > 0 { world.guards[i].facing = Facing::Right; }

        // Guard escaping with gold: try to drop at above-hole position (gx, gy-1).
        // Guard escaped to (ex, ey) = (gx±1, gy-1), so it won't stand on the gold.
        if world.guards[i].carry_gold && gy > 0 {
            if can_drop_gold_at(world, gx, gy - 1) {
                world.set_tile(gx, gy - 1, Tile::Gold);
                world.guards[i].carry_gold = false;
                world.guards[i].carry_gold_timer = 0;
            }
            // Can't drop → keep carrying
        }

        world.guards[i].state = physics::resolve_state(
            &world.tiles, world.width, world.height,
            &world.hole_grid, &world.guards,
            ex, ey, world.guards[i].state,
        );
        return;
    }
}

// ══════════════════════════════════════════════════════════════
// Win check
// ══════════════════════════════════════════════════════════════

fn resolve_win(world: &mut WorldState, events: &mut Vec<GameEvent>) {
    if !world.player.alive { return; }
    if world.exit_enabled && world.player.y == 0 {
        // Start exit animation: player climbs off the top
        world.phase = Phase::LevelOutro;
        world.anim_tick = 0;
        world.anim_player_y = 0;  // start at row 0, will go negative
        world.score += 500;
        events.push(GameEvent::StageCleared);
        world.set_message(&format!("Node {} Complete! +500", world.current_level + 1), 80);
    }
}

// ══════════════════════════════════════════════════════════════
// Helpers
// ══════════════════════════════════════════════════════════════

fn enable_exit(world: &mut WorldState) {
    world.exit_enabled = true;

    // Method 1: Exact hidden ladder positions (from binary level data / ~ markers)
    if !world.hidden_ladder_positions.is_empty() {
        for &(x, y) in &world.hidden_ladder_positions.clone() {
            if y < world.height && x < world.width {
                if world.terrain_at(x, y) == Tile::Empty {
                    world.set_tile(x, y, Tile::HiddenLadder);
                }
            }
        }
        return;
    }

    // Method 2: Column-based extension (^ markers or auto-detect)
    let columns: Vec<usize> = if world.exit_columns.is_empty() {
        // Auto-detect: find ALL columns that contain a climbable tile
        (0..world.width)
            .filter(|&x| (0..world.height).any(|y| world.terrain_at(x, y).is_climbable()))
            .collect()
    } else { world.exit_columns.clone() };

    let mut placed_any = false;
    for &x in &columns {
        // Find the topmost climbable tile in this column
        let mut top_ladder_y = None;
        for y in 0..world.height {
            if world.terrain_at(x, y).is_climbable() { top_ladder_y = Some(y); break; }
        }
        if let Some(ly) = top_ladder_y {
            // Extend hidden ladders from row 0 down to the top of the existing ladder
            for y in 0..ly {
                if world.terrain_at(x, y) == Tile::Empty {
                    world.set_tile(x, y, Tile::HiddenLadder);
                    placed_any = true;
                }
            }
        }
    }

    // Fallback: if exit_columns was set but no ladders were placed
    // (e.g. ^ markers in columns without ladders), retry with auto-detection
    if !placed_any && !world.exit_columns.is_empty() {
        let auto_columns: Vec<usize> = (0..world.width)
            .filter(|&x| (0..world.height).any(|y| world.terrain_at(x, y).is_climbable()))
            .collect();
        for &x in &auto_columns {
            let mut top_ladder_y = None;
            for y in 0..world.height {
                if world.terrain_at(x, y).is_climbable() { top_ladder_y = Some(y); break; }
            }
            if let Some(ly) = top_ladder_y {
                for y in 0..ly {
                    if world.terrain_at(x, y) == Tile::Empty {
                        world.set_tile(x, y, Tile::HiddenLadder);
                    }
                }
            }
        }
    }
}

fn player_die(world: &mut WorldState) {
    world.player.alive = false;
    world.phase = Phase::Dying;
    world.anim_tick = 0;
}

pub fn restart_level(world: &mut WorldState) {
    world.reset_tiles(); // restore tiles from base_tiles
    world.player.x = world.player_spawn.0;
    world.player.y = world.player_spawn.1;
    world.player.alive = true;
    world.player.state = ActorState::OnGround;
    world.player.move_cooldown = 0;
    world.holes.clear();
    world.digs.clear();
    world.rebuild_hole_grid();
    world.exit_enabled = false;
    world.gold_remaining = 0;
    for row in &world.tiles {
        for tile in row { if *tile == Tile::Gold { world.gold_remaining += 1; } }
    }
    world.gold_total = world.gold_remaining;
    for g in &mut world.guards {
        g.x = g.spawn_x; g.y = g.spawn_y;
        g.state = ActorState::OnGround;
        g.carry_gold = false; g.carry_gold_timer = 0; g.stuck_timer = 0;
        g.move_cooldown = world.speed.guard_move_rate;
        g.respawn_timer = 0;
        g.separation_timer = 0;
    }

    // Re-center camera on player
    world.camera.center_on(
        world.player_spawn.0, world.player_spawn.1,
        world.width, world.height,
    );
}

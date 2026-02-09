/// Entities: Player, Guard, Hole (as entity, not tile mutation), Gold.
/// State machines are minimal: 7 states max as per spec.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Facing {
    Left,
    Right,
}

/// Actor state machine (shared by Player and Guard).
/// Each state constrains which inputs are valid and defines transitions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActorState {
    OnGround,
    Falling,
    OnLadder,
    OnRope,
    InHole,   // Guard only: trapped in a dug hole
    Dead,
}

/// Movement direction (continuous while key held)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveDir {
    Left,
    Right,
    Up,
    Down,
}

/// Frame input: separates movement from dig so both can fire in one tick.
/// Movement = continuous (held key), Dig = edge-triggered (fresh press).
#[derive(Clone, Copy, Debug)]
pub struct FrameInput {
    pub movement: Option<MoveDir>,
    pub dig: Option<Facing>,
}

#[derive(Clone, Debug)]
pub struct Player {
    pub x: usize,
    pub y: usize,
    pub facing: Facing,
    pub state: ActorState,
    pub alive: bool,
    pub move_cooldown: u32,
}

impl Player {
    pub fn new(x: usize, y: usize) -> Self {
        Player {
            x, y,
            facing: Facing::Right,
            state: ActorState::OnGround,
            alive: true,
            move_cooldown: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Guard {
    pub id: usize,
    pub x: usize,
    pub y: usize,
    pub facing: Facing,
    pub state: ActorState,
    pub carry_gold: bool,
    pub carry_gold_timer: u32,  // ticks since picking up gold; 0 if not carrying
    pub stuck_timer: u32,      // remaining ticks trapped in hole
    pub move_cooldown: u32,    // ticks until next move
    pub spawn_x: usize,       // original position for respawn
    pub spawn_y: usize,
    pub respawn_timer: u32,    // ticks until respawn after death
    pub separation_timer: u32, // >0: avoidance mode, move away from nearest guard
}

impl Guard {
    pub fn new(id: usize, x: usize, y: usize) -> Self {
        Guard {
            id, x, y,
            facing: Facing::Left,
            state: ActorState::OnGround,
            carry_gold: false,
            carry_gold_timer: 0,
            stuck_timer: 0,
            move_cooldown: 0,
            spawn_x: x,
            spawn_y: y,
            respawn_timer: 0,
            separation_timer: 0,
        }
    }
}

/// Hole entity: tracks a dug brick through its lifecycle.
///
/// Two phases:
///   1. **Open** — fully open pit. Guards fall in, entities pass through.
///      Duration: `open_remaining` ticks (long).
///   2. **Closing** — brick regenerating, visual filling animation.
///      Duration: `close_remaining` ticks (short).
///
/// When both reach 0, the hole is done and the brick restores.
#[derive(Clone, Debug)]
pub struct Hole {
    pub x: usize,
    pub y: usize,
    pub open_remaining: u32,   // phase 1: fully open
    pub close_remaining: u32,  // phase 2: filling animation
}

impl Hole {
    pub fn new(x: usize, y: usize, open_ticks: u32, close_ticks: u32) -> Self {
        Hole { x, y, open_remaining: open_ticks, close_remaining: close_ticks }
    }

    /// Is the hole still active (passable)?
    pub fn is_active(&self) -> bool {
        self.open_remaining > 0 || self.close_remaining > 0
    }

    /// Is the hole in the closing/filling phase?
    pub fn is_closing(&self) -> bool {
        self.open_remaining == 0 && self.close_remaining > 0
    }

    /// Closing progress: 0.0 (just started closing) → 1.0 (about to seal).
    /// Only meaningful when `is_closing()` is true.
    pub fn close_progress(&self, total_close: u32) -> f32 {
        if total_close == 0 { return 1.0; }
        1.0 - (self.close_remaining as f32 / total_close as f32)
    }

    /// Advance one tick. Returns true if the hole just expired.
    pub fn tick(&mut self) -> bool {
        if self.open_remaining > 0 {
            self.open_remaining -= 1;
        } else if self.close_remaining > 0 {
            self.close_remaining -= 1;
        }
        !self.is_active()
    }
}

/// Dig-in-progress: a brick being dug, shown with cracking animation.
/// While active, the tile is still Brick (blocks movement).
/// When ticks_remaining reaches 0, the tile becomes Empty and a Hole is spawned.
#[derive(Clone, Debug)]
pub struct DigInProgress {
    pub x: usize,
    pub y: usize,
    pub ticks_remaining: u32,
    total_ticks: u32,
}

impl DigInProgress {
    pub fn new(x: usize, y: usize, duration: u32) -> Self {
        DigInProgress {
            x, y,
            ticks_remaining: duration,
            total_ticks: duration,
        }
    }

    /// Reconstruct with explicit remaining/total (for snapshot restore).
    pub fn new_with_state(x: usize, y: usize, remaining: u32, total: u32) -> Self {
        DigInProgress {
            x, y,
            ticks_remaining: remaining,
            total_ticks: total,
        }
    }

    /// Total ticks for this dig (for snapshot serialization).
    pub fn total_ticks(&self) -> u32 {
        self.total_ticks
    }

    /// Progress ratio from 0.0 (just started) to 1.0 (complete).
    fn progress(&self) -> f32 {
        1.0 - (self.ticks_remaining as f32 / self.total_ticks as f32)
    }

    /// Stage index 0..3 for rendering (0 = just started, 3 = about to open).
    pub fn stage(&self) -> u8 {
        let p = self.progress();
        if p < 0.25 { 0 }
        else if p < 0.50 { 1 }
        else if p < 0.75 { 2 }
        else { 3 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hole_2phase_lifecycle() {
        let mut h = Hole::new(5, 3, 3, 2); // 3 open + 2 close
        assert!(h.is_active());
        assert!(!h.is_closing());
        assert_eq!(h.open_remaining, 3);

        // Tick through open phase
        assert!(!h.tick()); // open_remaining: 3→2
        assert!(!h.tick()); // open_remaining: 2→1
        assert!(!h.tick()); // open_remaining: 1→0, start closing
        assert!(h.is_closing());
        assert_eq!(h.open_remaining, 0);
        assert_eq!(h.close_remaining, 2);

        // Tick through close phase
        assert!(!h.tick()); // close_remaining: 2→1
        assert!(h.tick());  // close_remaining: 1→0, expired!
        assert!(!h.is_active());
    }

    #[test]
    fn hole_close_progress() {
        let mut h = Hole::new(0, 0, 0, 10); // skip open, 10 close ticks
        assert!(h.is_closing());
        // At start of closing: progress = 0.0
        assert!((h.close_progress(10) - 0.0).abs() < 0.01);

        for _ in 0..5 { h.tick(); }
        // Halfway: progress ≈ 0.5
        assert!((h.close_progress(10) - 0.5).abs() < 0.01);

        for _ in 0..4 { h.tick(); }
        // close_remaining = 1, progress = 0.9
        assert!((h.close_progress(10) - 0.9).abs() < 0.01);
    }

    #[test]
    fn hole_zero_close_progress() {
        let h = Hole::new(0, 0, 5, 0); // 5 open, 0 close
        // close_progress with total=0 should return 1.0 (fully sealed)
        assert!((h.close_progress(0) - 1.0).abs() < 0.01);
    }
}

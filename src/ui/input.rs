/// Input state tracker.
///
/// Tracks which keys are currently held down, enabling:
///   - Continuous movement while a key is held
///   - Edge-triggered dig (only fires on initial press)
///   - Simultaneous movement + dig in the same tick
///
/// Uses crossterm's keyboard enhancement for Release events when available.
/// Falls back to timeout-based release detection on terminals that don't support it.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, poll};

/// After this duration without a Press/Repeat event, consider the key released.
/// Only used when the terminal doesn't report Release events.
const HOLD_TIMEOUT: Duration = Duration::from_millis(160);

pub struct InputState {
    /// Timestamp of last Press/Repeat event for each key.
    last_active: HashMap<KeyCode, Instant>,

    /// Keys that transitioned from "not held" → "held" during the
    /// most recent drain_events() call. Used for edge-triggered actions (dig).
    fresh_presses: Vec<KeyCode>,

    /// Raw key events collected during drain, for meta-key handling.
    pub raw_events: Vec<KeyEvent>,

    /// Whether to honor Release events. Only true when keyboard
    /// enhancement is confirmed working.
    pub honor_release: bool,
}

impl InputState {
    pub fn new() -> Self {
        InputState {
            last_active: HashMap::with_capacity(16),
            fresh_presses: Vec::with_capacity(8),
            raw_events: Vec::with_capacity(8),
            honor_release: false,
        }
    }

    /// Drain all pending terminal events and update key states.
    /// Call this once per frame, before simulation tick.
    pub fn drain_events(&mut self) {
        self.fresh_presses.clear();
        self.raw_events.clear();

        // Read all available events without blocking
        while poll(Duration::ZERO).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) => {
                    self.raw_events.push(key);

                    match key.kind {
                        KeyEventKind::Release if self.honor_release => {
                            // Explicit release: remove from active set
                            self.last_active.remove(&key.code);
                        }
                        KeyEventKind::Release => {
                            // Ignore release when enhancement not confirmed;
                            // rely on timeout-based expiry instead
                        }
                        _ => {
                            // Press, Repeat, or any other kind:
                            // treat as active key input
                            let was_held = self.is_held_inner(key.code);
                            self.last_active.insert(key.code, Instant::now());
                            if !was_held {
                                self.fresh_presses.push(key.code);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Expire keys that have timed out (fallback for terminals without Release)
        let now = Instant::now();
        self.last_active.retain(|_, t| now.duration_since(*t) < HOLD_TIMEOUT);
    }

    /// Is this key currently held down?
    /// Used for continuous actions (movement).
    pub fn is_held(&self, code: KeyCode) -> bool {
        self.is_held_inner(code)
    }

    /// Convenience: is any of these keys held?
    pub fn any_held(&self, codes: &[KeyCode]) -> bool {
        codes.iter().any(|c| self.is_held(*c))
    }

    /// Was this key freshly pressed this frame? (edge trigger)
    /// Used for one-shot actions (dig, confirm).
    pub fn was_pressed(&self, code: KeyCode) -> bool {
        self.fresh_presses.contains(&code)
    }

    /// Convenience: was any of these keys freshly pressed?
    pub fn any_pressed(&self, codes: &[KeyCode]) -> bool {
        codes.iter().any(|c| self.was_pressed(*c))
    }

    /// Check if any raw event this frame has Ctrl+C
    pub fn ctrl_c_pressed(&self) -> bool {
        use crossterm::event::KeyModifiers;
        self.raw_events.iter().any(|k| {
            k.modifiers.contains(KeyModifiers::CONTROL)
                && (k.code == KeyCode::Char('c') || k.code == KeyCode::Char('C'))
        })
    }

    // ── Internal ──

    fn is_held_inner(&self, code: KeyCode) -> bool {
        self.last_active.get(&code)
            .map(|t| t.elapsed() < HOLD_TIMEOUT)
            .unwrap_or(false)
    }
}

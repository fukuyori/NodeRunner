/// Gamepad input tracker using gilrs.
///
/// Button mapping is loaded from config.toml via `load_button_config()`.
/// Default mapping:
///   D-pad / Left Stick    →  Movement
///   B / Y / L1            →  Hack Left
///   A / X / R1            →  Hack Right
///   Start                 →  Confirm / Restart
///   Select                →  Quit

#[cfg(feature = "gamepad")]
use gilrs::{Axis, Button, EventType, Gilrs};

use crate::config::GamepadConfig;

#[cfg_attr(not(feature = "gamepad"), allow(dead_code))]
const STICK_DEADZONE: f32 = 0.25;

/// Logical button identifiers (one per physical button).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Btn {
    A,       // South
    B,       // East
    X,       // West
    Y,       // North
    L1,      // LeftTrigger
    R1,      // RightTrigger
    L2,      // LeftTrigger2
    R2,      // RightTrigger2
    Start,
    Select,
}

impl Btn {
    fn from_name(s: &str) -> Option<Btn> {
        match s.to_uppercase().as_str() {
            "A" | "SOUTH"  => Some(Btn::A),
            "B" | "EAST"   => Some(Btn::B),
            "X" | "WEST"   => Some(Btn::X),
            "Y" | "NORTH"  => Some(Btn::Y),
            "L1" | "LB" | "LEFTTRIGGER"  => Some(Btn::L1),
            "R1" | "RB" | "RIGHTTRIGGER" => Some(Btn::R1),
            "L2" | "LT" | "LEFTTRIGGER2"  => Some(Btn::L2),
            "R2" | "RT" | "RIGHTTRIGGER2" => Some(Btn::R2),
            "START" => Some(Btn::Start),
            "SELECT" | "BACK" => Some(Btn::Select),
            _ => None,
        }
    }

    #[cfg(feature = "gamepad")]
    fn from_gilrs(btn: Button) -> Option<Btn> {
        match btn {
            Button::South     => Some(Btn::A),
            Button::East      => Some(Btn::B),
            Button::West      => Some(Btn::X),
            Button::North     => Some(Btn::Y),
            Button::LeftTrigger  => Some(Btn::L1),
            Button::RightTrigger => Some(Btn::R1),
            Button::LeftTrigger2  => Some(Btn::L2),
            Button::RightTrigger2 => Some(Btn::R2),
            Button::Start     => Some(Btn::Start),
            Button::Select    => Some(Btn::Select),
            _ => None,
        }
    }
}

/// Per-button state: held (continuous) and just_pressed (edge).
#[derive(Clone, Copy, Debug, Default)]
struct BtnState {
    held: bool,
    just_pressed: bool,
}

/// Action-to-button mapping (loaded from config).
struct ActionMap {
    hack_left: Vec<Btn>,
    hack_right: Vec<Btn>,
    confirm: Vec<Btn>,
    cancel: Vec<Btn>,
    restart: Vec<Btn>,
}

impl Default for ActionMap {
    fn default() -> Self {
        ActionMap {
            hack_left:  vec![Btn::B, Btn::Y, Btn::L1],
            hack_right: vec![Btn::A, Btn::X, Btn::R1],
            confirm:    vec![Btn::Start],
            cancel:     vec![Btn::Select],
            restart:    vec![Btn::Start],
        }
    }
}

pub struct GamepadState {
    #[cfg(feature = "gamepad")]
    gilrs: Option<Gilrs>,

    // All tracked buttons (indexed by Btn)
    buttons: [BtnState; 10],

    // D-pad
    dpad_up: BtnState,
    dpad_down: BtnState,
    dpad_left: BtnState,
    dpad_right: BtnState,

    // Stick
    stick_up: BtnState,
    stick_down: BtnState,
    stick_left: BtnState,
    stick_right: BtnState,
    stick_x: f32,
    stick_y: f32,

    // Action mapping
    action_map: ActionMap,

    pub connected: bool,
}

fn btn_index(btn: Btn) -> usize {
    btn as usize
}

impl GamepadState {
    pub fn new() -> Self {
        #[cfg(feature = "gamepad")]
        let (gilrs_opt, connected) = {
            match Gilrs::new() {
                Ok(g) => {
                    let has_pad = g.gamepads().next().is_some();
                    (Some(g), has_pad)
                }
                Err(_) => (None, false),
            }
        };
        #[cfg(not(feature = "gamepad"))]
        let connected = false;

        GamepadState {
            #[cfg(feature = "gamepad")]
            gilrs: gilrs_opt,
            buttons: [BtnState::default(); 10],
            dpad_up: BtnState::default(),
            dpad_down: BtnState::default(),
            dpad_left: BtnState::default(),
            dpad_right: BtnState::default(),
            stick_up: BtnState::default(),
            stick_down: BtnState::default(),
            stick_left: BtnState::default(),
            stick_right: BtnState::default(),
            stick_x: 0.0,
            stick_y: 0.0,
            action_map: ActionMap::default(),
            connected,
        }
    }

    /// Load button mapping from config.
    pub fn load_button_config(&mut self, cfg: &GamepadConfig) {
        fn parse_list(names: &[String]) -> Vec<Btn> {
            names.iter().filter_map(|s| Btn::from_name(s)).collect()
        }
        let map = &mut self.action_map;
        let hl = parse_list(&cfg.hack_left);
        if !hl.is_empty() { map.hack_left = hl; }
        let hr = parse_list(&cfg.hack_right);
        if !hr.is_empty() { map.hack_right = hr; }
        let cf = parse_list(&cfg.confirm);
        if !cf.is_empty() { map.confirm = cf; }
        let ca = parse_list(&cfg.cancel);
        if !ca.is_empty() { map.cancel = ca; }
        let rs = parse_list(&cfg.restart);
        if !rs.is_empty() { map.restart = rs; }
    }

    pub fn update(&mut self) {
        self.clear_just_pressed();

        #[cfg(feature = "gamepad")]
        self.poll_gilrs();
    }

    #[cfg(feature = "gamepad")]
    fn poll_gilrs(&mut self) {
        let gilrs = match &mut self.gilrs {
            Some(g) => g,
            None => return,
        };

        let events: Vec<_> = std::iter::from_fn(|| gilrs.next_event()).collect();

        for event in events {
            match event.event {
                EventType::ButtonPressed(btn, _) => {
                    self.connected = true;
                    self.set_button(btn, true, true);
                }
                EventType::ButtonReleased(btn, _) => {
                    self.connected = true;
                    self.set_button(btn, false, false);
                }
                EventType::AxisChanged(axis, value, _) => {
                    self.connected = true;
                    self.update_axis(axis, value);
                }
                EventType::Connected => { self.connected = true; }
                EventType::Disconnected => {
                    self.connected = false;
                    self.release_all();
                }
                _ => {}
            }
        }

        // Derive stick digital states
        let prev_left = self.stick_left.held;
        let prev_right = self.stick_right.held;
        let prev_up = self.stick_up.held;
        let prev_down = self.stick_down.held;

        self.stick_left.held = self.stick_x < -STICK_DEADZONE;
        self.stick_right.held = self.stick_x > STICK_DEADZONE;
        self.stick_up.held = self.stick_y > STICK_DEADZONE;
        self.stick_down.held = self.stick_y < -STICK_DEADZONE;

        if self.stick_left.held && !prev_left { self.stick_left.just_pressed = true; }
        if self.stick_right.held && !prev_right { self.stick_right.just_pressed = true; }
        if self.stick_up.held && !prev_up { self.stick_up.just_pressed = true; }
        if self.stick_down.held && !prev_down { self.stick_down.just_pressed = true; }
    }

    #[cfg(feature = "gamepad")]
    fn set_button(&mut self, gilrs_btn: Button, held: bool, just_pressed: bool) {
        // D-pad handled separately (not in Btn enum)
        match gilrs_btn {
            Button::DPadUp    => { self.dpad_up.held = held; if just_pressed { self.dpad_up.just_pressed = true; } return; }
            Button::DPadDown  => { self.dpad_down.held = held; if just_pressed { self.dpad_down.just_pressed = true; } return; }
            Button::DPadLeft  => { self.dpad_left.held = held; if just_pressed { self.dpad_left.just_pressed = true; } return; }
            Button::DPadRight => { self.dpad_right.held = held; if just_pressed { self.dpad_right.just_pressed = true; } return; }
            _ => {}
        }

        if let Some(btn) = Btn::from_gilrs(gilrs_btn) {
            let idx = btn_index(btn);
            self.buttons[idx].held = held;
            if just_pressed {
                self.buttons[idx].just_pressed = true;
            }
        }
    }

    #[cfg(feature = "gamepad")]
    fn update_axis(&mut self, axis: Axis, value: f32) {
        match axis {
            Axis::LeftStickX => self.stick_x = value,
            Axis::LeftStickY => self.stick_y = value,
            _ => {}
        }
    }

    // ── Action queries (config-driven) ──

    fn any_just_pressed(&self, btns: &[Btn]) -> bool {
        btns.iter().any(|&b| self.buttons[btn_index(b)].just_pressed)
    }

    pub fn dig_left_pressed(&self) -> bool {
        self.any_just_pressed(&self.action_map.hack_left)
    }
    pub fn dig_right_pressed(&self) -> bool {
        self.any_just_pressed(&self.action_map.hack_right)
    }
    pub fn confirm_pressed(&self) -> bool {
        self.any_just_pressed(&self.action_map.confirm)
    }
    pub fn cancel_pressed(&self) -> bool {
        self.any_just_pressed(&self.action_map.cancel)
    }
    pub fn restart_pressed(&self) -> bool {
        self.any_just_pressed(&self.action_map.restart)
    }

    // Movement (continuous, held)
    pub fn up_held(&self) -> bool {
        self.dpad_up.held || self.stick_up.held
    }
    pub fn down_held(&self) -> bool {
        self.dpad_down.held || self.stick_down.held
    }
    pub fn left_held(&self) -> bool {
        self.dpad_left.held || self.stick_left.held
    }
    pub fn right_held(&self) -> bool {
        self.dpad_right.held || self.stick_right.held
    }

    // ── Internal ──

    fn clear_just_pressed(&mut self) {
        for b in &mut self.buttons { b.just_pressed = false; }
        self.dpad_up.just_pressed = false;
        self.dpad_down.just_pressed = false;
        self.dpad_left.just_pressed = false;
        self.dpad_right.just_pressed = false;
        self.stick_up.just_pressed = false;
        self.stick_down.just_pressed = false;
        self.stick_left.just_pressed = false;
        self.stick_right.just_pressed = false;
    }

    fn release_all(&mut self) {
        for b in &mut self.buttons { *b = BtnState::default(); }
        self.dpad_up = BtnState::default();
        self.dpad_down = BtnState::default();
        self.dpad_left = BtnState::default();
        self.dpad_right = BtnState::default();
        self.stick_up = BtnState::default();
        self.stick_down = BtnState::default();
        self.stick_left = BtnState::default();
        self.stick_right = BtnState::default();
        self.stick_x = 0.0;
        self.stick_y = 0.0;
    }
}

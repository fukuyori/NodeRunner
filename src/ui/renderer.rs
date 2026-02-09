/// Presentation layer: double-buffered, diff-based terminal renderer.
///
/// How it works:
///   1. Build the next frame into `front` buffer (array of Cell)
///   2. Compare each cell with `back` buffer (previous frame)
///   3. Only emit terminal commands for cells that changed
///   4. All commands are batched with `queue!`, flushed once at the end
///   5. Swap front/back
///
/// This eliminates flicker caused by full-screen redraws.

use std::io::{self, BufWriter, Write};

use crossterm::{
    cursor::{self, MoveTo},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};

use crate::domain::entity::{ActorState, Facing};
use crate::domain::tile::Tile;
use crate::sim::world::{Phase, WorldState};

// ── Cell: the unit of the back-buffer ──

#[derive(Clone, Copy, PartialEq, Eq)]
struct Cell {
    ch: [u8; 16],  // up to 16 bytes (supports ZWJ emoji sequences)
    ch_len: u8,
    fg: Color,
    bg: Color,
    wide: bool,    // true = this char occupies 2 terminal columns
    cont: bool,    // true = continuation of previous wide char (skip render)
}

impl Cell {
    /// Explicit dark background for all "empty" terminal cells.
    ///
    /// On VTE-based Linux terminals (GNOME Terminal, etc.), the inter-row gap
    /// pixels use the background color from the last Clear or the terminal's
    /// configured default.  By using the SAME explicit RGB for both
    /// `Clear(ClearType::All)` and every cell's background, the gap color
    /// matches the cell color exactly, eliminating visible horizontal lines.
    ///
    /// If your terminal's own background differs from this value, set it to
    /// RGB(22,22,35) in your terminal preferences for a seamless look.
    const BASE_BG: Color = Color::Rgb { r: 22, g: 22, b: 35 };

    const BLANK: Cell = Cell {
        ch: [b' ', 0,0,0, 0,0,0,0, 0,0,0,0, 0,0,0,0],
        ch_len: 1,
        fg: Color::White,
        bg: Cell::BASE_BG,
        wide: false,
        cont: false,
    };

    const WIDE_CONT: Cell = Cell {
        ch: [0; 16],
        ch_len: 0,
        fg: Color::White,
        bg: Cell::BASE_BG,
        wide: false,
        cont: true,
    };

    /// Sentinel cell used to invalidate the back buffer.
    /// Different from any real cell, so every position will be diff'd.
    const INVALID: Cell = Cell {
        ch: [b'?', 0,0,0, 0,0,0,0, 0,0,0,0, 0,0,0,0],
        ch_len: 1,
        fg: Color::Magenta,
        bg: Color::Magenta,
        wide: false,
        cont: false,
    };

    /// Normalize bg: Color::Reset → BASE_BG so that every cell gets an
    /// explicit background color (never terminal-default).
    #[inline]
    fn norm_bg(bg: Color) -> Color {
        match bg {
            Color::Reset => Self::BASE_BG,
            other => other,
        }
    }

    fn from_char(c: char, fg: Color, bg: Color, _bold: bool) -> Self {
        let mut cell = Self::BLANK;
        let len = c.encode_utf8(&mut cell.ch).len() as u8;
        cell.ch_len = len;
        cell.fg = fg;
        cell.bg = Self::norm_bg(bg);
        cell
    }

    fn from_char_wide(c: char, fg: Color, bg: Color, _bold: bool) -> Self {
        let mut cell = Self::BLANK;
        let len = c.encode_utf8(&mut cell.ch).len() as u8;
        cell.ch_len = len;
        cell.fg = fg;
        cell.bg = Self::norm_bg(bg);
        cell.wide = true;
        cell
    }

    /// Create a wide cell from a multi-codepoint string (e.g. ZWJ emoji).
    #[allow(dead_code)]
    fn from_str_wide(s: &str, fg: Color, bg: Color, _bold: bool) -> Self {
        let mut cell = Self::BLANK;
        let bytes = s.as_bytes();
        let len = bytes.len().min(16);
        cell.ch[..len].copy_from_slice(&bytes[..len]);
        cell.ch_len = len as u8;
        cell.fg = fg;
        cell.bg = Self::norm_bg(bg);
        cell.wide = true;
        cell
    }

    fn as_str(&self) -> &str {
        if self.ch_len == 0 { return ""; }
        unsafe { std::str::from_utf8_unchecked(&self.ch[..self.ch_len as usize]) }
    }
}

// ── FrameBuffer: a 2D grid of Cells ──

struct FrameBuffer {
    width: usize,
    height: usize,
    cells: Vec<Cell>,
}

impl FrameBuffer {
    fn new(w: usize, h: usize) -> Self {
        FrameBuffer {
            width: w,
            height: h,
            cells: vec![Cell::BLANK; w * h],
        }
    }

    fn resize(&mut self, w: usize, h: usize) {
        if self.width != w || self.height != h {
            self.width = w;
            self.height = h;
            self.cells = vec![Cell::BLANK; w * h];
        }
    }

    fn clear(&mut self) {
        self.cells.fill(Cell::BLANK);
    }

    fn set(&mut self, x: usize, y: usize, cell: Cell) {
        if x < self.width && y < self.height {
            self.cells[y * self.width + x] = cell;
        }
    }

    fn get(&self, x: usize, y: usize) -> Cell {
        if x < self.width && y < self.height {
            self.cells[y * self.width + x]
        } else {
            Cell::BLANK
        }
    }

    /// Write a string at (x, y) with given colors. Each char occupies 1 column.
    fn put_str(&mut self, x: usize, y: usize, s: &str, fg: Color, bg: Color, _bold: bool) {
        let mut cx = x;
        for ch in s.chars() {
            if cx >= self.width { break; }
            self.set(cx, y, Cell::from_char(ch, fg, bg, false));
            cx += 1;
        }
    }
}

// ── Renderer ──

/// Total terminal columns needed = map_width * 2 (each game cell = 2 terminal cols)
/// We use a 1:1 terminal-column buffer, so game cell (gx) maps to columns (gx*2, gx*2+1).
const CELL_W: usize = 2;

/// Vertical offsets
const HUD_ROW: usize = 0;
const MAP_ROW: usize = 2;

pub struct Renderer {
    writer: BufWriter<io::Stdout>,
    front: FrameBuffer,
    back: FrameBuffer,
    term_w: usize,
    term_h: usize,
    last_phase: Option<Phase>,
}

impl Renderer {
    pub fn new() -> Self {
        Renderer {
            writer: BufWriter::with_capacity(16384, io::stdout()),
            front: FrameBuffer::new(0, 0),
            back: FrameBuffer::new(0, 0),
            term_w: 0,
            term_h: 0,
            last_phase: None,
        }
    }

    pub fn init(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        execute!(
            self.writer,
            terminal::EnterAlternateScreen,
            cursor::Hide,
            SetBackgroundColor(Cell::BASE_BG),
            Clear(ClearType::All)
        )?;

        let (tw, th) = terminal::size().unwrap_or((80, 24));
        self.term_w = tw as usize;
        self.term_h = th as usize;
        self.front.resize(self.term_w, self.term_h);
        self.back.resize(self.term_w, self.term_h);
        // Force full repaint on first frame: back ≠ front for every cell.
        self.back.cells.fill(Cell::INVALID);

        Ok(())
    }

    pub fn cleanup(&mut self) -> io::Result<()> {
        execute!(
            self.writer,
            ResetColor,
            cursor::Show,
            terminal::LeaveAlternateScreen
        )?;
        terminal::disable_raw_mode()
    }

    pub fn render(&mut self, world: &mut WorldState) -> io::Result<()> {
        // Detect terminal resize
        let (tw, th) = terminal::size().unwrap_or((80, 24));
        if tw as usize != self.term_w || th as usize != self.term_h {
            self.term_w = tw as usize;
            self.term_h = th as usize;
            self.front.resize(self.term_w, self.term_h);
            self.back.resize(self.term_w, self.term_h);
            // Force full repaint after resize.
            self.back.cells.fill(Cell::INVALID);
            queue!(self.writer, SetBackgroundColor(Cell::BASE_BG), Clear(ClearType::All))?;
        }

        // Update camera viewport dimensions from terminal size
        // viewport = terminal cols / CELL_W wide, terminal rows - reserved rows high
        let reserved_rows = MAP_ROW + 4; // HUD + gap + msg + help
        world.camera.view_w = self.term_w / CELL_W;
        let max_view_h = if self.term_h > reserved_rows {
            self.term_h - reserved_rows
        } else {
            1
        };
        // Cap to world dimensions so we don't waste space on void
        if world.width > 0 {
            world.camera.view_w = world.camera.view_w.min(world.width);
        }
        world.camera.view_h = if world.height > 0 {
            max_view_h.min(world.height)
        } else {
            max_view_h
        };

        // Detect phase change → clear for clean transition
        let phase_changed = self.last_phase != Some(world.phase);
        if phase_changed {
            self.back.cells.fill(Cell::INVALID);
            queue!(self.writer, SetBackgroundColor(Cell::BASE_BG), Clear(ClearType::All))?;
            self.last_phase = Some(world.phase);
        }

        // Re-center camera now that view_w/view_h are up to date.
        match world.phase {
            Phase::Playing => {
                world.camera.follow(
                    world.player.x, world.player.y,
                    world.width, world.height,
                );
            }
            Phase::LevelIntro | Phase::LevelReady
            | Phase::Dying | Phase::LevelOutro | Phase::LevelComplete => {
                world.camera.center_on(
                    world.player.x, world.player.y,
                    world.width, world.height,
                );
            }
            _ => {}
        }

        // Build front buffer
        self.front.clear();

        match world.phase {
            Phase::Title => self.compose_title(world),
            Phase::LevelSelect => self.compose_level_select(world),
            Phase::PackSelect => self.compose_pack_select(world),
            Phase::LevelIntro => self.compose_level_intro(world),
            Phase::LevelReady => self.compose_level_ready(world),
            Phase::LevelOutro | Phase::LevelComplete => self.compose_game_animated(world),
            Phase::Dying => self.compose_game_animated(world),
            Phase::GameOver => self.compose_game_over(world),
            Phase::GameComplete => self.compose_game_complete(world),
            Phase::Playing => self.compose_game(world),
        }

        // Pause overlay (drawn on top of game)
        if world.paused {
            self.compose_pause_overlay(world);
        }

        // Diff and emit
        self.flush_diff()?;

        // Swap: current front becomes next back
        std::mem::swap(&mut self.front, &mut self.back);

        Ok(())
    }

    // ── Diff flush: only write changed cells ──

    fn flush_diff(&mut self) -> io::Result<()> {
        let mut last_fg = Color::White;
        let mut last_bg = Cell::BASE_BG;
        let mut need_move = true;
        let mut last_x: usize = 0;
        let mut last_y: usize = 0;

        // Set explicit base colors at start of frame.
        // IMPORTANT: Do NOT use ResetColor here — it resets to the terminal's
        // native default, which may differ from BASE_BG and cause line artifacts.
        queue!(self.writer,
            SetForegroundColor(Color::White),
            SetBackgroundColor(Cell::BASE_BG),
        )?;

        for y in 0..self.front.height {
            let mut x = 0;
            while x < self.front.width {
                let cell = self.front.get(x, y);
                let prev = self.back.get(x, y);

                // Skip continuation cells (right half of wide emoji)
                if cell.cont {
                    if cell != prev { need_move = true; }
                    x += 1;
                    continue;
                }

                // For wide cells, also check if the continuation changed
                let cont_changed = cell.wide
                    && x + 1 < self.front.width
                    && self.front.get(x + 1, y) != self.back.get(x + 1, y);

                if cell == prev && !cont_changed {
                    need_move = true;
                    x += 1;
                    continue;
                }

                // Position cursor if needed
                if need_move || x != last_x + 1 || y != last_y {
                    queue!(self.writer, MoveTo(x as u16, y as u16))?;
                    need_move = false;
                }

                // Set colors only if changed
                if cell.fg != last_fg {
                    queue!(self.writer, SetForegroundColor(cell.fg))?;
                    last_fg = cell.fg;
                }
                if cell.bg != last_bg {
                    queue!(self.writer, SetBackgroundColor(cell.bg))?;
                    last_bg = cell.bg;
                }

                queue!(self.writer, Print(cell.as_str()))?;

                if cell.wide {
                    // Wide char printed: cursor advanced 2 columns
                    last_x = x + 1;
                    x += 2; // skip the continuation cell
                } else {
                    last_x = x;
                    x += 1;
                }
                last_y = y;
            }
        }

        self.writer.flush()
    }

    // ── Compose: build front buffer content ──

    fn compose_game(&mut self, w: &WorldState) {
        let buf_w = self.front.width;
        let cam = &w.camera;

        // ── HUD row ──
        let gold_status = if w.exit_enabled {
            "ESCAPE!"
        } else {
            ""
        };
        let hud = format!(
            " Node.{:<2}  Score:{:<7}  ♥×{}  ${}/{}  {} ",
            w.current_level + 1, w.score, w.lives,
            w.gold_total - w.gold_remaining, w.gold_total,
            gold_status,
        );
        // Fill entire HUD row with background
        for x in 0..buf_w {
            self.front.set(x, HUD_ROW, Cell::from_char(' ', Color::White, Color::Rgb{r:20,g:20,b:60}, false));
        }
        self.front.put_str(0, HUD_ROW, &hud, Color::White, Color::Rgb{r:20,g:20,b:60}, false);

        // ── Map (camera viewport) ──
        for vy in 0..cam.view_h {
            let wy = cam.y + vy as i32;
            let row = MAP_ROW + vy;
            if row >= self.front.height { break; }

            for vx in 0..cam.view_w {
                let wx = cam.x + vx as i32;
                let col = vx * CELL_W;
                if col + 1 >= buf_w { break; }

                self.compose_cell_cam(w, wx, wy, col, row);
            }
        }

        // ── Message bar ──
        let msg_row = MAP_ROW + cam.view_h + 1;
        if msg_row < self.front.height {
            if !w.message.is_empty() {
                let msg = format!(" ◈ {} ", w.message);
                for x in 0..buf_w {
                    self.front.set(x, msg_row, Cell::from_char(' ', Color::Black, Color::Rgb{r:200,g:180,b:50}, false));
                }
                self.front.put_str(0, msg_row, &msg, Color::Black, Color::Rgb{r:200,g:180,b:50}, false);
            }
        }

        // ── Help bar ──
        let help_row = MAP_ROW + cam.view_h + 3;
        if help_row < self.front.height {
            let help = " Z/Q:HackL  X/E:HackR  F1:Pause  │  Pad: B/Y/L1:L  A/X/R1:R";
            self.front.put_str(0, help_row, help, Color::DarkGrey, Color::Reset, false);
        }
    }

    /// Render an out-of-bounds / void cell (game background).
    fn compose_void(&mut self, col: usize, row: usize) {
        self.front.set(col, row, Cell::from_char(' ', Color::White, Cell::BASE_BG, false));
        self.front.set(col + 1, row, Cell::from_char(' ', Color::White, Cell::BASE_BG, false));
    }

    /// Render a world cell through the camera. If (wx, wy) is out of world bounds, void.
    fn compose_cell_cam(&mut self, w: &WorldState, wx: i32, wy: i32, col: usize, row: usize) {
        if wx < 0 || wy < 0 || wx >= w.width as i32 || wy >= w.height as i32 {
            self.compose_void(col, row);
        } else {
            self.compose_cell(w, wx as usize, wy as usize, col, row);
        }
    }

    /// Render a world cell (no player) through the camera.
    #[allow(dead_code)]
    fn compose_cell_no_player_cam(&mut self, w: &WorldState, wx: i32, wy: i32, col: usize, row: usize) {
        if wx < 0 || wy < 0 || wx >= w.width as i32 || wy >= w.height as i32 {
            self.compose_void(col, row);
        } else {
            self.compose_cell_no_player(w, wx as usize, wy as usize, col, row);
        }
    }

    /// Render tile only (no entities) through the camera.
    #[allow(dead_code)]
    fn compose_tile_cam(&mut self, w: &WorldState, wx: i32, wy: i32, col: usize, row: usize) {
        if wx < 0 || wy < 0 || wx >= w.width as i32 || wy >= w.height as i32 {
            self.compose_void(col, row);
        } else {
            self.compose_tile_only(w, wx as usize, wy as usize, col, row);
        }
    }

    /// Write the visual for game cell (gx, gy) into the front buffer at (col, row).
    /// Each game cell = 2 terminal columns.
    fn compose_cell(&mut self, w: &WorldState, gx: usize, gy: usize, col: usize, row: usize) {
        // Player: direction-dependent emoji
        if w.player.alive && w.player.x == gx && w.player.y == gy {
            let ch = match w.player.facing {
                Facing::Left  => '🧍',
                Facing::Right => '🧍',
            };
            self.front.set(col, row, Cell::from_char_wide(ch, Color::Reset, Color::Reset, false));
            self.front.set(col + 1, row, Cell::WIDE_CONT);
            return;
        }

        // Sentinels
        for g in &w.guards {
            if g.state == ActorState::Dead { continue; }
            if g.x == gx && g.y == gy {
                self.front.set(col, row, Cell::from_char_wide('🤺', Color::Reset, Color::Reset, false));
                self.front.set(col + 1, row, Cell::WIDE_CONT);
                return;
            }
        }

        // Dig in progress (cracking animation)
        for dig in &w.digs {
            if dig.x == gx && dig.y == gy {
                let (c0, c1, fg, bg) = match dig.stage() {
                    0 => ('▓', '▓', Color::DarkYellow, Color::Rgb{r:80,g:60,b:0}),
                    1 => ('▓', '░', Color::DarkYellow, Color::Rgb{r:60,g:40,b:0}),
                    2 => ('░', '░', Color::DarkYellow, Color::Reset),
                    _ => ('·', '·', Color::DarkYellow, Color::Reset),
                };
                self.front.set(col, row, Cell::from_char(c0, fg, bg, false));
                self.front.set(col + 1, row, Cell::from_char(c1, fg, bg, false));
                return;
            }
        }

        // Open hole (2-phase: open pit → closing/filling)
        for hole in &w.holes {
            if hole.x == gx && hole.y == gy {
                if hole.is_closing() {
                    // Phase 2: filling animation
                    let pct = hole.close_progress(w.speed.hole_close_ticks);
                    let (ch, bg) = if pct < 0.33 {
                        ('▁', Color::Rgb{r:20,g:15,b:0})
                    } else if pct < 0.66 {
                        ('▃', Color::Rgb{r:40,g:30,b:0})
                    } else {
                        ('▅', Color::Rgb{r:60,g:45,b:0})
                    };
                    self.front.set(col, row, Cell::from_char(ch, Color::DarkYellow, bg, false));
                    self.front.set(col + 1, row, Cell::from_char(ch, Color::DarkYellow, bg, false));
                } else {
                    // Phase 1: fully open pit
                    self.front.set(col, row, Cell::from_char(' ', Color::Reset, Color::Rgb{r:10,g:8,b:0}, false));
                    self.front.set(col + 1, row, Cell::from_char(' ', Color::Reset, Color::Rgb{r:10,g:8,b:0}, false));
                }
                return;
            }
        }

        // Tile
        let (c0, c1, fg, bg) = match w.tiles[gy][gx] {
            Tile::Empty => (' ', ' ', Color::Reset, Color::Reset),
            Tile::Brick         => ('░', '░', Color::Rgb{r:180,g:120,b:60}, Color::Rgb{r:100,g:65,b:30}),
            Tile::TrapBrick     => ('░', '░', Color::Rgb{r:180,g:120,b:60}, Color::Rgb{r:100,g:65,b:30}),
            Tile::Concrete      => ('█', '█', Color::Rgb{r:120,g:120,b:120}, Color::Rgb{r:70,g:70,b:70}),
            Tile::Ladder        => ('╠', '╣', Color::Rgb{r:100,g:200,b:255}, Color::Reset),
            Tile::HiddenLadder  => ('╏', '╏', Color::Rgb{r:0,g:180,b:180}, Color::Rgb{r:0,g:40,b:40}),
            Tile::Rope          => ('━', '━', Color::Rgb{r:180,g:100,b:200}, Color::Reset),
            Tile::Gold          => {
                // Token: wide emoji 💰
                self.front.set(col, row, Cell::from_char_wide('💰', Color::Reset, Color::Reset, false));
                self.front.set(col + 1, row, Cell::WIDE_CONT);
                return;
            }
        };
        self.front.set(col, row, Cell::from_char(c0, fg, bg, false));
        self.front.set(col + 1, row, Cell::from_char(c1, fg, bg, false));
    }

    // ── Static screens (title, game over, etc.) ──

    /// Level intro: progressive map reveal from bottom to top
    fn compose_level_intro(&mut self, w: &WorldState) {
        let buf_w = self.front.width;
        let cam = &w.camera;
        let tick = w.anim_tick;

        // Constants matching main.rs
        let intro_name_ticks: u32 = 8;
        let intro_row_interval: u32 = 2;

        // How many rows are visible (from bottom of WORLD)
        let rows_visible = if tick <= intro_name_ticks {
            0
        } else {
            ((tick - intro_name_ticks) / intro_row_interval).min(w.height as u32) as usize
        };

        // Show entities only when all rows revealed
        let show_entities = rows_visible >= w.height;

        // ── HUD ──
        let hud_bg = Color::Rgb{r:20,g:20,b:60};
        for x in 0..buf_w {
            self.front.set(x, HUD_ROW, Cell::from_char(' ', Color::White, hud_bg, false));
        }
        let hud = format!(
            " Node.{:<2}  Score:{:<7}  ♥×{}  ${}/{}",
            w.current_level + 1, w.score, w.lives,
            w.gold_total - w.gold_remaining, w.gold_total,
        );
        self.front.put_str(0, HUD_ROW, &hud, Color::White, hud_bg, false);

        // ── Level name display (centered in viewport) ──
        let name_row = MAP_ROW + cam.view_h / 2 - 1;
        if name_row < self.front.height && rows_visible < w.height {
            let name = format!(" ◈ {} ◈ ", w.level_name);
            let view_cols = cam.view_w * CELL_W;
            let cx = view_cols.saturating_sub(name.len()) / 2;
            self.front.put_str(cx, name_row, &name, Color::Rgb{r:255,g:220,b:50}, Color::Reset, true);

            // "GET READY" below
            let ready = "▸▸▸ GET READY ◂◂◂";
            let rx = view_cols.saturating_sub(ready.len()) / 2;
            self.front.put_str(rx, name_row + 2, ready, Color::Rgb{r:80,g:255,b:80}, Color::Reset, false);
        }

        // ── Map reveal from bottom (camera viewport) ──
        for vy in 0..cam.view_h {
            let wy = cam.y + vy as i32;
            let row = MAP_ROW + vy;
            if row >= self.front.height { break; }

            for vx in 0..cam.view_w {
                let wx = cam.x + vx as i32;
                let col = vx * CELL_W;
                if col + 1 >= buf_w { break; }

                // Out of world bounds → void
                if wx < 0 || wy < 0 || wx >= w.width as i32 || wy >= w.height as i32 {
                    self.compose_void(col, row);
                    continue;
                }

                let gx = wx as usize;
                let gy = wy as usize;

                // Row gy is visible if (height - 1 - gy) < rows_visible
                let from_bottom = w.height - 1 - gy;
                if from_bottom >= rows_visible {
                    // Not yet revealed → void
                    self.compose_void(col, row);
                    continue;
                }

                // Reveal effect: the freshest row gets a highlight
                let is_frontier = from_bottom + 1 == rows_visible;

                if is_frontier {
                    let tile = w.tiles[gy][gx];
                    let (c0, c1) = match tile {
                        Tile::Empty => (' ', ' '),
                        Tile::Brick | Tile::TrapBrick => ('▓', '▓'),
                        Tile::Concrete => ('█', '█'),
                        Tile::Ladder => ('╠', '╣'),
                        Tile::Rope => ('━', '━'),
                        Tile::Gold => ('◆', '◆'),
                        Tile::HiddenLadder => (' ', ' '),
                    };
                    let flash_fg = Color::Rgb{r:180,g:255,b:255};
                    let flash_bg = Color::Rgb{r:0,g:40,b:60};
                    self.front.set(col, row, Cell::from_char(c0, flash_fg, flash_bg, true));
                    self.front.set(col + 1, row, Cell::from_char(c1, flash_fg, flash_bg, true));
                } else if show_entities {
                    self.compose_cell(w, gx, gy, col, row);
                } else {
                    self.compose_tile_only(w, gx, gy, col, row);
                }
            }
        }

        // ── "ENTER to skip" hint ──
        let hint_row = MAP_ROW + cam.view_h + 1;
        if hint_row < self.front.height && rows_visible < w.height {
            let hint = " Press ENTER to skip ";
            self.front.put_str(0, hint_row, hint, Color::DarkGrey, Color::Reset, false);
        }
    }

    /// Level ready: full map visible with entities, blinking "PRESS ANY KEY" prompt
    fn compose_level_ready(&mut self, w: &WorldState) {
        let buf_w = self.front.width;
        let cam = &w.camera;

        // ── HUD ──
        let hud_bg = Color::Rgb{r:20,g:20,b:60};
        for x in 0..buf_w {
            self.front.set(x, HUD_ROW, Cell::from_char(' ', Color::White, hud_bg, false));
        }
        let hud = format!(
            " Node.{:<2}  Score:{:<7}  ♥×{}  ${}/{}",
            w.current_level + 1, w.score, w.lives,
            w.gold_total - w.gold_remaining, w.gold_total,
        );
        self.front.put_str(0, HUD_ROW, &hud, Color::White, hud_bg, false);

        // ── Full map with all entities (camera viewport) ──
        for vy in 0..cam.view_h {
            let wy = cam.y + vy as i32;
            let row = MAP_ROW + vy;
            if row >= self.front.height { break; }
            for vx in 0..cam.view_w {
                let wx = cam.x + vx as i32;
                let col = vx * CELL_W;
                if col + 1 >= buf_w { break; }
                self.compose_cell_cam(w, wx, wy, col, row);
            }
        }

        // ── Blinking "PRESS ANY KEY" prompt ──
        let blink = (w.anim_tick / 5) % 2 == 0;
        let prompt_row = MAP_ROW + cam.view_h + 1;
        if prompt_row < self.front.height {
            if blink {
                let prompt = " ▸▸▸ PRESS ANY KEY TO START ◂◂◂ ";
                let view_cols = cam.view_w * CELL_W;
                let cx = view_cols.saturating_sub(prompt.len()) / 2;
                for x in 0..buf_w {
                    self.front.set(x, prompt_row, Cell::from_char(' ', Color::Black, Color::Rgb{r:200,g:180,b:50}, false));
                }
                self.front.put_str(cx, prompt_row, prompt, Color::Black, Color::Rgb{r:200,g:180,b:50}, true);
            }
        }
    }

    /// Render a tile without entities (for intro animation)
    fn compose_tile_only(&mut self, w: &WorldState, gx: usize, gy: usize, col: usize, row: usize) {
        let (c0, c1, fg, bg) = match w.tiles[gy][gx] {
            Tile::Empty => (' ', ' ', Color::Reset, Color::Reset),
            Tile::Brick         => ('░', '░', Color::Rgb{r:180,g:120,b:60}, Color::Rgb{r:100,g:65,b:30}),
            Tile::TrapBrick     => ('░', '░', Color::Rgb{r:180,g:120,b:60}, Color::Rgb{r:100,g:65,b:30}),
            Tile::Concrete      => ('█', '█', Color::Rgb{r:120,g:120,b:120}, Color::Rgb{r:70,g:70,b:70}),
            Tile::Ladder        => ('╠', '╣', Color::Rgb{r:100,g:200,b:255}, Color::Reset),
            Tile::HiddenLadder  => (' ', ' ', Color::Reset, Color::Reset),
            Tile::Rope          => ('━', '━', Color::Rgb{r:180,g:100,b:200}, Color::Reset),
            Tile::Gold          => {
                self.front.set(col, row, Cell::from_char_wide('💰', Color::Reset, Color::Reset, false));
                self.front.set(col + 1, row, Cell::WIDE_CONT);
                return;
            }
        };
        self.front.set(col, row, Cell::from_char(c0, fg, bg, false));
        self.front.set(col + 1, row, Cell::from_char(c1, fg, bg, false));
    }

    /// Animated game view: handles LevelOutro, LevelComplete, and Dying phases
    fn compose_game_animated(&mut self, w: &WorldState) {
        let buf_w = self.front.width;
        let cam = &w.camera;

        // ── HUD ──
        let gold_status = if w.exit_enabled { "ESCAPE!" } else { "" };
        let hud = format!(
            " Node.{:<2}  Score:{:<7}  ♥×{}  ${}/{}  {} ",
            w.current_level + 1, w.score, w.lives,
            w.gold_total - w.gold_remaining, w.gold_total, gold_status,
        );
        for x in 0..buf_w {
            self.front.set(x, HUD_ROW, Cell::from_char(' ', Color::White, Color::Rgb{r:20,g:20,b:60}, false));
        }
        self.front.put_str(0, HUD_ROW, &hud, Color::White, Color::Rgb{r:20,g:20,b:60}, false);

        // ── Map (camera viewport, tiles + guards, player handled specially) ──
        for vy in 0..cam.view_h {
            let wy = cam.y + vy as i32;
            let row = MAP_ROW + vy;
            if row >= self.front.height { break; }

            for vx in 0..cam.view_w {
                let wx = cam.x + vx as i32;
                let col = vx * CELL_W;
                if col + 1 >= buf_w { break; }

                if wx < 0 || wy < 0 || wx >= w.width as i32 || wy >= w.height as i32 {
                    self.compose_void(col, row);
                    continue;
                }

                let gx = wx as usize;
                let gy = wy as usize;

                // Skip player position - handled below with animation
                let is_player_pos = w.player.x == gx && w.player.y == gy;

                if !is_player_pos {
                    self.compose_cell_no_player(w, gx, gy, col, row);
                } else {
                    // Render tile underneath player position
                    self.compose_tile_only(w, gx, gy, col, row);
                }
            }
        }

        // ── Animated player (world → screen via camera) ──
        match w.phase {
            Phase::LevelOutro | Phase::LevelComplete => {
                // Player climbing up and off screen
                let py_world = w.anim_player_y;   // world Y (can go negative)
                let px_world = w.player.x as i32;
                let vy = py_world - cam.y;
                let vx = px_world - cam.x;
                if vy >= 0 && vx >= 0 && (vx as usize) < cam.view_w {
                    let row = MAP_ROW + vy as usize;
                    let col = vx as usize * CELL_W;
                    if row < self.front.height && col + 1 < buf_w {
                        self.front.set(col, row, Cell::from_char_wide('🧗', Color::Reset, Color::Reset, false));
                        self.front.set(col + 1, row, Cell::WIDE_CONT);
                    }
                }
            }
            Phase::Dying => {
                let visible = (w.anim_tick / 2) % 2 == 0;
                if visible {
                    if let Some((vx, vy)) = cam.world_to_view(w.player.x, w.player.y) {
                        let row = MAP_ROW + vy;
                        let col = vx * CELL_W;
                        if row < self.front.height && col + 1 < buf_w {
                            let flash = if w.anim_tick < 6 {
                                Color::Rgb{r:255,g:60,b:60}
                            } else {
                                Color::Rgb{r:200,g:200,b:200}
                            };
                            self.front.set(col, row, Cell::from_char_wide('🧍', flash, Color::Reset, false));
                            self.front.set(col + 1, row, Cell::WIDE_CONT);
                        }
                    }
                }
            }
            _ => {}
        }

        // ── Message bar ──
        let msg_row = MAP_ROW + cam.view_h + 1;
        if msg_row < self.front.height && !w.message.is_empty() {
            let msg = format!(" ◈ {} ", w.message);
            for x in 0..buf_w {
                self.front.set(x, msg_row, Cell::from_char(' ', Color::Black, Color::Rgb{r:200,g:180,b:50}, false));
            }
            self.front.put_str(0, msg_row, &msg, Color::Black, Color::Rgb{r:200,g:180,b:50}, false);
        }

        // ── Level complete overlay (centered in viewport) ──
        if w.phase == Phase::LevelComplete {
            let cy = MAP_ROW + cam.view_h / 2;
            if cy < self.front.height {
                let border = "╔══════════════════════════════╗";
                let middle = "║   ★ NODE CLEARED ★           ║";
                let prompt = "║  ENTER: Next  ESC: Title     ║";
                let bottom = "╚══════════════════════════════╝";
                let view_cols = cam.view_w * CELL_W;
                let cx = view_cols.saturating_sub(border.len()) / 2;
                let fg = Color::Rgb{r:255,g:220,b:50};
                let bg = Color::Rgb{r:20,g:60,b:20};
                self.front.put_str(cx, cy - 1, border, fg, bg, true);
                self.front.put_str(cx, cy,     middle, fg, bg, true);
                self.front.put_str(cx, cy + 1, prompt, Color::Rgb{r:80,g:255,b:80}, bg, false);
                self.front.put_str(cx, cy + 2, bottom, fg, bg, true);
            }
        }
    }

    /// Compose a game cell without rendering the player (for animated phases)
    fn compose_cell_no_player(&mut self, w: &WorldState, gx: usize, gy: usize, col: usize, row: usize) {
        // Guards
        for g in &w.guards {
            if g.state == ActorState::Dead { continue; }
            if g.x == gx && g.y == gy {
                self.front.set(col, row, Cell::from_char_wide('🤺', Color::Reset, Color::Reset, false));
                self.front.set(col + 1, row, Cell::WIDE_CONT);
                return;
            }
        }

        // Dig in progress
        for dig in &w.digs {
            if dig.x == gx && dig.y == gy {
                let (c0, c1, fg, bg) = match dig.stage() {
                    0 => ('▓', '▓', Color::DarkYellow, Color::Rgb{r:80,g:60,b:0}),
                    1 => ('▓', '░', Color::DarkYellow, Color::Rgb{r:60,g:40,b:0}),
                    2 => ('░', '░', Color::DarkYellow, Color::Reset),
                    _ => ('·', '·', Color::DarkYellow, Color::Reset),
                };
                self.front.set(col, row, Cell::from_char(c0, fg, bg, false));
                self.front.set(col + 1, row, Cell::from_char(c1, fg, bg, false));
                return;
            }
        }

        // Open holes (2-phase: open pit → closing/filling)
        for hole in &w.holes {
            if hole.x == gx && hole.y == gy {
                if hole.is_closing() {
                    let pct = hole.close_progress(w.speed.hole_close_ticks);
                    let (ch, bg) = if pct < 0.33 {
                        ('▁', Color::Rgb{r:20,g:15,b:0})
                    } else if pct < 0.66 {
                        ('▃', Color::Rgb{r:40,g:30,b:0})
                    } else {
                        ('▅', Color::Rgb{r:60,g:45,b:0})
                    };
                    self.front.set(col, row, Cell::from_char(ch, Color::DarkYellow, bg, false));
                    self.front.set(col + 1, row, Cell::from_char(ch, Color::DarkYellow, bg, false));
                } else {
                    self.front.set(col, row, Cell::from_char(' ', Color::Reset, Color::Rgb{r:10,g:8,b:0}, false));
                    self.front.set(col + 1, row, Cell::from_char(' ', Color::Reset, Color::Rgb{r:10,g:8,b:0}, false));
                }
                return;
            }
        }

        // Tile
        self.compose_tile_only(w, gx, gy, col, row);
    }

    fn compose_title(&mut self, w: &WorldState) {
        let title = [
            r"  _  _         _        ___                          ",
            r" | \| | ___  _| | ___  | _ \ _  _  _ _   _ _   ___  _ _ ",
            r" | .` |/ _ \/ _` |/ -_) |   /| || || ' \ | ' \ / -_)| '_|",
            r" |_|\_|\___/\__,_|\___| |_|_\ \_,_||_||_||_||_|\___||_|  ",
        ];

        for (i, line) in title.iter().enumerate() {
            self.front.put_str(2, 2 + i, line, Color::Rgb{r:255,g:200,b:50}, Color::Reset, true);
        }

        let subtitle = "◈◈  Mainnet Protocol  ◈◈";
        let sx = 2 + (title[1].len().saturating_sub(subtitle.len())) / 2;
        self.front.put_str(sx, 7, subtitle, Color::Rgb{r:80,g:255,b:80}, Color::Reset, true);

        let tagline = "━━━ Terminal Edition (Rust) ━━━";
        let tx = 2 + (title[1].len().saturating_sub(tagline.len())) / 2;
        self.front.put_str(tx, 9, tagline, Color::Rgb{r:180,g:140,b:50}, Color::Reset, false);

        // Menu options
        let menu_base = 12;
        let hi = Color::Rgb{r:80,g:255,b:80};
        let dim = Color::DarkGrey;

        self.front.put_str(8, menu_base,     "ENTER   New Game", hi, Color::Reset, true);
        if w.has_save {
            self.front.put_str(8, menu_base + 1, "  C     Continue", Color::Rgb{r:255,g:220,b:50}, Color::Reset, false);
        } else {
            self.front.put_str(8, menu_base + 1, "  C     Continue  (no save)", dim, Color::Reset, false);
        }
        self.front.put_str(8, menu_base + 2, "  L     Level Select", Color::White, Color::Reset, false);
        self.front.put_str(8, menu_base + 3, "  F3    Level Packs", Color::Rgb{r:100,g:200,b:255}, Color::Reset, false);
        self.front.put_str(8, menu_base + 4, "  Q     Quit", Color::White, Color::Reset, false);

        // Pack and level info
        let pack_info = format!("      📦 {}  ({} levels)", w.active_pack, w.total_levels);
        self.front.put_str(8, menu_base + 6, &pack_info, dim, Color::Reset, false);

        // Controls reference
        let help = [
            "Controls",
            "  ←→↑↓ / WASD   Move          Z/Q Hack L",
            "  X/E            Hack R        ESC Title",
            "  F1 Pause   F2 Restart   F3 Level Packs",
            "  F4 Level Select              F5-F8 Save",
            "  F9-F12 Load Slot 1-4",
        ];

        let help_base = menu_base + 8;
        for (i, line) in help.iter().enumerate() {
            let color = if i == 0 { Color::Rgb{r:255,g:200,b:50} } else { Color::White };
            self.front.put_str(8, help_base + i, line, color, Color::Reset, false);
        }

        // Message bar (for pack switch confirmation, etc.)
        if !w.message.is_empty() {
            let msg_row = self.front.height.saturating_sub(1);
            if msg_row > help_base + help.len() {
                let msg = format!(" ◈ {} ", w.message);
                let buf_w = self.front.width;
                for x in 0..buf_w {
                    self.front.set(x, msg_row, Cell::from_char(' ', Color::Black, Color::Rgb{r:200,g:180,b:50}, false));
                }
                self.front.put_str(0, msg_row, &msg, Color::Black, Color::Rgb{r:200,g:180,b:50}, false);
            }
        }
    }

    fn compose_level_select(&mut self, w: &WorldState) {
        let hi = Color::Rgb{r:80,g:255,b:80};
        let normal = Color::White;
        let dim = Color::DarkGrey;
        let cursor_bg = Color::Rgb{r:30,g:60,b:30};

        // Header
        self.front.put_str(2, 1, "╔═══════════════════════════════════════════╗", Color::Rgb{r:255,g:200,b:50}, Color::Reset, true);
        self.front.put_str(2, 2, "║          LEVEL  SELECT                    ║", Color::Rgb{r:255,g:200,b:50}, Color::Reset, true);
        self.front.put_str(2, 3, "╚═══════════════════════════════════════════╝", Color::Rgb{r:255,g:200,b:50}, Color::Reset, true);

        // Active pack indicator
        let pack_str = format!("  📦 {}", w.active_pack);
        self.front.put_str(2, 4, &pack_str, Color::Rgb{r:255,g:180,b:80}, Color::Reset, false);

        // Level list
        let list_top = 6;
        let visible = 16_usize.min(self.front.height.saturating_sub(list_top + 4));
        let total = w.total_levels;
        let scroll = w.select_scroll;

        // Scroll indicators
        if scroll > 0 {
            self.front.put_str(2, list_top - 1, "    ▲ ▲ ▲", dim, Color::Reset, false);
        }

        for i in 0..visible {
            let idx = scroll + i;
            if idx >= total { break; }
            let row = list_top + i;
            if row >= self.front.height { break; }

            let is_selected = idx == w.select_cursor;
            let num_str = format!("{:>3}.", idx + 1);

            let name = if idx < w.level_names.len() {
                &w.level_names[idx]
            } else {
                "???"
            };

            // Truncate name to fit
            let max_name = 40;
            let display_name: String = if name.len() > max_name {
                format!("{}...", &name[..max_name - 3])
            } else {
                name.to_string()
            };

            if is_selected {
                // Blinking cursor indicator
                let blink = (w.anim_tick / 5) % 2 == 0;
                let arrow = if blink { "▸" } else { " " };

                // Fill row with highlight
                for x in 0..48.min(self.front.width) {
                    self.front.set(x, row, Cell::from_char(' ', normal, cursor_bg, false));
                }
                self.front.put_str(2, row, arrow, hi, cursor_bg, true);
                self.front.put_str(3, row, &num_str, hi, cursor_bg, true);
                self.front.put_str(7, row, &display_name, hi, cursor_bg, true);
            } else {
                self.front.put_str(3, row, &num_str, dim, Color::Reset, false);
                self.front.put_str(7, row, &display_name, normal, Color::Reset, false);
            }
        }

        // Scroll down indicator
        if scroll + visible < total {
            let ind_row = list_top + visible;
            if ind_row < self.front.height {
                self.front.put_str(2, ind_row, "    ▼ ▼ ▼", dim, Color::Reset, false);
            }
        }

        // Footer
        let footer_row = list_top + visible + 2;
        if footer_row < self.front.height {
            self.front.put_str(2, footer_row, "  ENTER: Start   ↑↓: Select   PgUp/PgDn   F3: Packs   ESC: Back", dim, Color::Reset, false);
            let count_str = format!("  {}/{} levels", w.select_cursor + 1, total);
            if footer_row + 1 < self.front.height {
                self.front.put_str(2, footer_row + 1, &count_str, dim, Color::Reset, false);
            }
        }
    }

    fn compose_pack_select(&mut self, w: &WorldState) {
        let gold = Color::Rgb{r:255,g:200,b:50};
        let hi = Color::Rgb{r:80,g:255,b:80};
        let cyan = Color::Rgb{r:100,g:200,b:255};
        let normal = Color::White;
        let dim = Color::DarkGrey;
        let cursor_bg = Color::Rgb{r:20,g:50,b:60};
        let active_fg = Color::Rgb{r:255,g:180,b:80};

        // Header
        self.front.put_str(2, 1, "╔═══════════════════════════════════════════════════╗", gold, Color::Reset, true);
        self.front.put_str(2, 2, "║            📦 LEVEL PACK SELECT                   ║", gold, Color::Reset, true);
        self.front.put_str(2, 3, "╚═══════════════════════════════════════════════════╝", gold, Color::Reset, true);

        // Active pack indicator
        let active_str = format!("  Active: {}", w.active_pack);
        self.front.put_str(2, 5, &active_str, active_fg, Color::Reset, false);

        // Pack list
        let list_top = 7;
        let visible = 12_usize.min(self.front.height.saturating_sub(list_top + 8));
        let total = w.pack_list.len();
        let scroll = w.pack_scroll;

        // Scroll up indicator
        if scroll > 0 {
            self.front.put_str(2, list_top - 1, "    ▲ ▲ ▲", dim, Color::Reset, false);
        }

        for i in 0..visible {
            let idx = scroll + i;
            if idx >= total { break; }
            let row = list_top + i * 3; // 3 rows per pack entry
            if row + 2 >= self.front.height { break; }

            let pack = &w.pack_list[idx];
            let is_selected = idx == w.pack_cursor;
            let is_active = pack.path == w.active_pack_path;

            let marker = if is_active { "★" } else { " " };
            let name_line = format!("{}  {}", marker, pack.name);
            let count_str = format!("{} levels", pack.level_count);

            if is_selected {
                let blink = (w.anim_tick / 5) % 2 == 0;
                let arrow = if blink { "▸" } else { " " };

                // Highlight rows
                for r in row..=(row + 2).min(self.front.height - 1) {
                    for x in 0..56.min(self.front.width) {
                        self.front.set(x, r, Cell::from_char(' ', normal, cursor_bg, false));
                    }
                }

                // Row 1: arrow + name
                self.front.put_str(1, row, arrow, hi, cursor_bg, true);
                let name_fg = if is_active { active_fg } else { hi };
                self.front.put_str(2, row, &name_line, name_fg, cursor_bg, true);
                // Level count on the right
                self.front.put_str(46, row, &count_str, cyan, cursor_bg, false);

                // Row 2: author
                if !pack.author.is_empty() {
                    let author_str = format!("     by {}", pack.author);
                    self.front.put_str(2, row + 1, &author_str, normal, cursor_bg, false);
                }

                // Row 3: description
                if !pack.description.is_empty() {
                    let desc: String = if pack.description.len() > 50 {
                        format!("     {}...", &pack.description[..47])
                    } else {
                        format!("     {}", pack.description)
                    };
                    self.front.put_str(2, row + 2, &desc, dim, cursor_bg, false);
                }
            } else {
                let name_fg = if is_active { active_fg } else { normal };
                self.front.put_str(3, row, &name_line, name_fg, Color::Reset, false);
                self.front.put_str(46, row, &count_str, dim, Color::Reset, false);

                if !pack.author.is_empty() {
                    let author_str = format!("     by {}", pack.author);
                    self.front.put_str(3, row + 1, &author_str, dim, Color::Reset, false);
                }
            }
        }

        // Scroll down indicator
        if scroll + visible < total {
            let ind_row = list_top + visible * 3;
            if ind_row < self.front.height {
                self.front.put_str(2, ind_row, "    ▼ ▼ ▼", dim, Color::Reset, false);
            }
        }

        // Detail panel for selected pack
        let detail_row = list_top + visible * 3 + 2;
        if detail_row + 2 < self.front.height && w.pack_cursor < total {
            let pack = &w.pack_list[w.pack_cursor];
            let path_display = if pack.path.starts_with("__") {
                "(built-in)".to_string()
            } else {
                // Show just the filename
                std::path::Path::new(&pack.path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            };
            let detail = format!("  Source: {}", path_display);
            self.front.put_str(2, detail_row, &detail, dim, Color::Reset, false);
        }

        // Footer
        let footer_row = self.front.height.saturating_sub(2);
        if footer_row > list_top {
            self.front.put_str(2, footer_row, "  ENTER: Select Pack   ↑↓: Browse   ESC: Back", dim, Color::Reset, false);
            let hint = "  Place .nlp files in packs/ to add level packs";
            if footer_row + 1 < self.front.height {
                self.front.put_str(2, footer_row + 1, hint, Color::Rgb{r:80,g:80,b:100}, Color::Reset, false);
            }
        }
    }

    fn compose_game_over(&mut self, w: &WorldState) {
        let box_art = [
            "╔════════════════════════════════╗",
            "║     ✕ CONNECTION  LOST  ✕      ║",
            "╚════════════════════════════════╝",
        ];
        for (i, l) in box_art.iter().enumerate() {
            self.front.put_str(6, 4 + i, l, Color::Rgb{r:255,g:60,b:60}, Color::Reset, true);
        }
        let score = format!("◈ Final Score: {}", w.score);
        let level = format!("◈ Reached Node: {}", w.current_level + 1);
        self.front.put_str(8, 9, &score, Color::White, Color::Reset, false);
        self.front.put_str(8, 10, &level, Color::White, Color::Reset, false);
        self.front.put_str(8, 12, "▸ ENTER: Retry from Node 1", Color::Rgb{r:80,g:255,b:80}, Color::Reset, false);
        self.front.put_str(8, 13, "▸ ESC:   Back to Title", Color::DarkGrey, Color::Reset, false);
    }

    fn compose_game_complete(&mut self, w: &WorldState) {
        let box_art = [
            "╔══════════════════════════════════════════╗",
            "║  ★ MAINNET SECURED! PROTOCOL COMPLETE! ★ ║",
            "╚══════════════════════════════════════════╝",
        ];
        for (i, l) in box_art.iter().enumerate() {
            self.front.put_str(4, 4 + i, l, Color::Rgb{r:255,g:220,b:50}, Color::Reset, true);
        }
        let score = format!("◈ Final Score: {}", w.score);
        let levels = format!("◈ All {} nodes cleared!", w.total_levels);
        self.front.put_str(6, 9, &score, Color::White, Color::Reset, false);
        self.front.put_str(6, 10, &levels, Color::Rgb{r:80,g:255,b:80}, Color::Reset, false);
        self.front.put_str(6, 12, "▸ ENTER / ESC: Back to Title", Color::Rgb{r:80,g:255,b:80}, Color::Reset, false);
    }

    fn compose_pause_overlay(&mut self, w: &WorldState) {
        let dim = Color::Rgb{r:40,g:40,b:40};
        let blink = (w.anim_tick / 8) % 2 == 0;
        let cam = &w.camera;

        // Center the overlay in the viewport
        let view_cols = cam.view_w * CELL_W;
        let view_rows = cam.view_h;
        let box_w = 40_usize.min(view_cols);
        let box_h = 16_usize.min(view_rows);
        let box_x = (view_cols.saturating_sub(box_w)) / 2;
        let box_y = MAP_ROW + (view_rows.saturating_sub(box_h)) / 2;

        // Draw dark background box
        for y in box_y..box_y + box_h {
            for x in box_x..box_x + box_w {
                self.front.set(x, y, Cell::from_char(' ', Color::Reset, dim, false));
            }
        }

        let hdr = Color::Rgb{r:255,g:220,b:50};
        let key_c = Color::Rgb{r:100,g:200,b:255};
        let desc_c = Color::Rgb{r:180,g:180,b:180};
        let sep_c = Color::Rgb{r:80,g:80,b:80};

        // Title
        let pause_label = if blink { "║  ▶  PAUSED  ◀  ║" } else { "║     PAUSED      ║" };
        self.front.put_str(box_x + 11, box_y, "╔══════════════════╗", hdr, dim, true);
        self.front.put_str(box_x + 11, box_y + 1, pause_label, hdr, dim, true);
        self.front.put_str(box_x + 11, box_y + 2, "╚══════════════════╝", hdr, dim, true);

        let y0 = box_y + 4;
        self.front.put_str(box_x + 2, y0,     "F1  Resume", key_c, dim, false);
        self.front.put_str(box_x + 2, y0 + 1, "F2  Restart Level", key_c, dim, false);
        self.front.put_str(box_x + 2, y0 + 2, "F3  Level Packs", key_c, dim, false);
        self.front.put_str(box_x + 2, y0 + 3, "F4  Change Level", key_c, dim, false);
        self.front.put_str(box_x + 2, y0 + 4, "────────────────────────", sep_c, dim, false);
        self.front.put_str(box_x + 2, y0 + 5, "F5 Save 1  F6 Save 2", desc_c, dim, false);
        self.front.put_str(box_x + 2, y0 + 6, "F7 Save 3  F8 Save 4", desc_c, dim, false);
        self.front.put_str(box_x + 2, y0 + 7, "────────────────────────", sep_c, dim, false);
        self.front.put_str(box_x + 2, y0 + 8, "F9 Load 1  F10 Load 2", desc_c, dim, false);
        self.front.put_str(box_x + 2, y0 + 9, "F11 Load 3 F12 Load 4", desc_c, dim, false);
        self.front.put_str(box_x + 2, y0 + 10, "────────────────────────", sep_c, dim, false);
        self.front.put_str(box_x + 2, y0 + 11, "ESC Back to Title", key_c, dim, false);
    }
}

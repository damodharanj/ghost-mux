use std::io::{Read, Write};
use std::sync::{mpsc, Arc, Mutex};

use gpui::*;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

// Actions for terminal key handling (override gpui-component Root's Tab focus traversal)
actions!(terminal, [TerminalTab, TerminalShiftTab]);

/// Register terminal key bindings. Call once during app init.
pub fn register_bindings(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", TerminalTab, Some("Terminal")),
        KeyBinding::new("shift-tab", TerminalShiftTab, Some("Terminal")),
    ]);
}

use libghostty_vt::{
    key,
    render::{CellIterator, RowIterator},
    style::RgbColor,
    terminal::ScrollViewport,
    RenderState, Terminal, TerminalOptions,
};

// --- Display constants -------------------------------------------------

pub const COLS: usize = 80;
pub const ROWS: usize = 24;

// --- Helpers -----------------------------------------------------------

fn rgb32(c: RgbColor) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32)
}

// --- TerminalModel -----------------------------------------------------

pub struct TerminalModel {
    pub terminal: Box<Terminal<'static, 'static>>,
    pub render_state: RenderState<'static>,
    pub row_iter: RowIterator<'static>,
    pub cell_iter: CellIterator<'static>,
    pub key_encoder: key::Encoder<'static>,
    pub key_event: key::Event<'static>,

    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub pty_master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pub shell_pid: Option<u32>,

    pub rows: usize,
    pub cols: usize,

    pub focus_handle: FocusHandle,
    pub scroll_handle: UniformListScrollHandle,

    pub new_output: bool,
    pub pending_rows: usize,
    pub pending_cols: usize,
    pub resize_generation: u64,
    pub viewport_bounds: Option<Bounds<Pixels>>,
    pub selection_anchor: Option<(usize, usize)>,
    pub selection_head: Option<(usize, usize)>,
    pub selection_dragging: bool,
    pub cursor_blink_visible: bool,
}

impl TerminalModel {
    pub fn new(cwd: Option<std::path::PathBuf>, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();

        // --- PTY ---
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: ROWS as u16,
                cols: COLS as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("Failed to open PTY");

        let mut cmd = CommandBuilder::new_default_prog();
        if let Some(ref dir) = cwd {
            if dir.exists() {
                cmd.cwd(dir);
            }
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLUMNS", COLS.to_string());
        cmd.env("LINES", ROWS.to_string());

        let child = pair
            .slave
            .spawn_command(cmd)
            .expect("Failed to spawn shell");
        let shell_pid = child.process_id();
        drop(pair.slave);

        let writer_raw = pair.master.take_writer().expect("Failed to get PTY writer");
        let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(writer_raw));
        let mut reader = pair
            .master
            .try_clone_reader()
            .expect("Failed to clone PTY reader");
        let pty_master: Arc<Mutex<Box<dyn MasterPty + Send>>> = Arc::new(Mutex::new(pair.master));

        // --- libghostty-vt terminal ---
        let writer_for_cb = Arc::clone(&writer);

        // Box the terminal BEFORE registering callbacks.
        //
        // `on_pty_write` stores `&self.vtable` (address of the VTable embedded
        // inside Terminal) as raw userdata in the C library.  If the Terminal is
        // a stack variable it gets moved (stack → TerminalModel → GPUI entity
        // heap) and the stored pointer becomes dangling.  Heap-allocating via
        // Box gives a stable address that survives all subsequent moves of the
        // Box itself.
        let mut terminal = Box::new(
            Terminal::new(TerminalOptions {
                cols: COLS as u16,
                rows: ROWS as u16,
                max_scrollback: 5000,
            })
            .expect("Failed to create terminal"),
        );

        // Write-back channel: terminal uses this to send responses (device status
        // reports, mode queries, etc.) back into the PTY.
        terminal
            .on_pty_write(move |_term, data| {
                if let Ok(mut w) = writer_for_cb.lock() {
                    let _ = w.write_all(data);
                }
            })
            .expect("Failed to register on_pty_write");

        // main polling loop ---
        let (tx, rx) = mpsc::channel::<Vec<u8>>();

        std::thread::spawn(move || {
            let _child = child;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let _ = tx.send(buf[..n].to_vec());
                    }
                }
            }
        });

        // Timer: drain channel, feed data to terminal, notify GPUI.
        cx.spawn(async move |entity, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(16))
                .await;
            let mut buf: Vec<u8> = Vec::new();
            while let Ok(chunk) = rx.try_recv() {
                buf.extend_from_slice(&chunk);
            }
            if !buf.is_empty() {
                entity
                    .update(cx, |model, cx| {
                        model.terminal.vt_write(&buf);
                        model.new_output = true;
                        model.cursor_blink_visible = true;
                        cx.notify();
                    })
                    .ok();
            }
        })
        .detach();

        // Blink timer: toggle blink state every 500ms
        cx.spawn(async move |entity, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(500))
                .await;
            let res = entity.update(cx, |model, cx| {
                model.cursor_blink_visible = !model.cursor_blink_visible;
                cx.notify();
            });
            if res.is_err() {
                break;
            }
        })
        .detach();

        Self {
            terminal,
            render_state: RenderState::new().expect("RenderState::new"),
            row_iter: RowIterator::new().expect("RowIterator::new"),
            cell_iter: CellIterator::new().expect("CellIterator::new"),
            key_encoder: key::Encoder::new().expect("key::Encoder::new"),
            key_event: key::Event::new().expect("key::Event::new"),
            writer,
            pty_master,
            shell_pid,
            rows: ROWS,
            cols: COLS,
            focus_handle,
            scroll_handle: UniformListScrollHandle::new(),
            new_output: false,
            pending_rows: ROWS,
            pending_cols: COLS,
            resize_generation: 0,
            viewport_bounds: None,
            selection_anchor: None,
            selection_head: None,
            selection_dragging: false,
            cursor_blink_visible: true,
        }
    }

    // --- Rendering --------------------------------------------------------

    /// Snapshot the current viewport from libghostty-vt and return:
    ///   - row_data: Vec of rows, each row = Vec of (char, fg_rgb, bg_rgb)
    ///   - cursor: Option<(col, row)> if cursor is visible in viewport
    ///   - default_bg: u32 rgb  (for filling the background)
    ///
    /// Uses split borrows so render_state, row_iter, cell_iter, and terminal
    /// can be accessed on a single `&mut self` without borrow conflicts.
    pub fn collect_rows(&mut self, is_focused: bool) -> (Vec<Vec<(char, u32, u32)>>, Option<(usize, usize)>, u32) {
        // Scroll to bottom when new content arrives.
        if self.new_output {
            let _ = self.terminal.scroll_viewport(ScrollViewport::Bottom);
            self.new_output = false;
        }

        // render_state.update borrows render_state mutably and terminal
        // immutably.  The terminal borrow ends when update() returns.
        let snap = self
            .render_state
            .update(&self.terminal)
            .expect("RenderState::update");

        let colors = snap.colors().expect("snapshot colors");
        let default_fg = rgb32(colors.foreground);
        let default_bg = rgb32(colors.background);

        let cursor = if snap.cursor_visible().unwrap_or(false) {
            snap.cursor_viewport()
                .ok()
                .flatten()
                .map(|vp| (vp.x as usize, vp.y as usize))
        } else {
            None
        };

        let mut row_data: Vec<Vec<(char, u32, u32)>> = Vec::with_capacity(self.rows);

        // row_iter borrows self.row_iter (mut) and snap (ref).
        // cell_iter borrows self.cell_iter (mut) and each row (ref).
        // snap borrows self.render_ all different fields: OK.state
        let mut row_it = self.row_iter.update(&snap).expect("row_iter::update");
        let mut row_idx = 0usize;
        while let Some(row) = row_it.next() {
            let mut cell_it = self.cell_iter.update(row).expect("cell_iter::update");
            let mut cells: Vec<(char, u32, u32)> = Vec::with_capacity(self.cols);
            let mut col_idx = 0usize;
            while let Some(cell) = cell_it.next() {
                let grapheme_len = cell.graphemes_len().unwrap_or(0);
                let ch = if grapheme_len > 0 {
                    cell.graphemes()
                        .ok()
                        .and_then(|g| g.into_iter().next())
                        .unwrap_or(' ')
                } else {
                    ' '
                };

                let (fg, bg) = if cursor == Some((col_idx, row_idx)) {
                    let show_cursor_here = if is_focused {
                        self.cursor_blink_visible
                    } else {
                        true // Solid/static cursor when unfocused
                    };

                    if show_cursor_here {
                        // Render cursor as inverse of cell colors.
                        let fg = cell
                            .fg_color()
                            .ok()
                            .flatten()
                            .map(rgb32)
                            .unwrap_or(default_fg);
                        let bg = cell
                            .bg_color()
                            .ok()
                            .flatten()
                            .map(rgb32)
                            .unwrap_or(default_bg);
                        (bg, fg) // swap for cursor
                    } else {
                        let fg = cell
                            .fg_color()
                            .ok()
                            .flatten()
                            .map(rgb32)
                            .unwrap_or(default_fg);
                        let bg = cell
                            .bg_color()
                            .ok()
                            .flatten()
                            .map(rgb32)
                            .unwrap_or(default_bg);
                        (fg, bg)
                    }
                } else {
                    let fg = cell
                        .fg_color()
                        .ok()
                        .flatten()
                        .map(rgb32)
                        .unwrap_or(default_fg);
                    let bg = cell
                        .bg_color()
                        .ok()
                        .flatten()
                        .map(rgb32)
                        .unwrap_or(default_bg);
                    (fg, bg)
                };

                cells.push((ch, fg, bg));
                col_idx += 1;
            }
            row_data.push(cells);
            row_idx += 1;
        }

        (row_data, cursor, default_bg)
    }

    // --- Input ------------------------------------------------------------

    /// Encode a GPUI keystroke using libghostty-vt's key encoder, which
    /// respects terminal modes (application cursor keys, Kitty keyboard
    /// protocol, etc.).  Returns the bytes to write to the PTY.
    pub fn encode_keystroke(&mut self, k: &Keystroke) -> Vec<u8> {
        self.cursor_blink_visible = true;
        let ghost_key = gpui_key_to_ghostty(k.key.as_str());
        let mods = gpui_mods_to_ghostty(&k.modifiers);

        let ucp = unshifted_codepoint(k.key.as_str());

        self.key_event
            .set_action(key::Action::Press)
            .set_key(ghost_key)
            .set_mods(mods)
            .set_unshifted_codepoint(ucp);

        // For printable single-char keys, pass the UTF-8 text so the encoder
        // can handle non-Latin layouts and compose the right legacy sequence.
        if ucp != '\0' && !k.modifiers.control && !k.modifiers.platform {
            let text_owned: String = if k.modifiers.shift && ucp.is_ascii_lowercase() {
                ucp.to_uppercase().collect()
            } else {
                ucp.to_string()
            };
            self.key_event.set_utf8(Some(text_owned.as_str()));
        } else {
            self.key_event.set_utf8::<String>(None);
        }

        // sync_options reads terminal modes (DECCKM, Kitty protocol )flags,
        // so the encoder produces the right sequence for the current app state.
        self.key_encoder.set_options_from_terminal(&self.terminal);

        let mut buf = Vec::new();
        let _ = self.key_encoder.encode_to_vec(&self.key_event, &mut buf);
        buf
    }

    pub fn send_key(&mut self, bytes: &[u8]) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(bytes);
        }
    }

    pub fn set_viewport_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.viewport_bounds = Some(bounds);
    }

    pub fn begin_selection(&mut self, row: usize, col: usize) {
        let point = (row, col);
        self.selection_anchor = Some(point);
        self.selection_head = Some(point);
        self.selection_dragging = true;
    }

    pub fn update_selection(&mut self, row: usize, col: usize) {
        if self.selection_dragging {
            self.selection_head = Some((row, col));
        }
    }

    pub fn end_selection(&mut self) {
        self.selection_dragging = false;
    }

    pub fn selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        let anchor = self.selection_anchor?;
        let head = self.selection_head?;
        if anchor == head {
            return None;
        }
        Some(normalize_selection(anchor, head))
    }

    pub fn selected_text(&self, rows: &[Vec<(char, u32, u32)>]) -> Option<String> {
        let ((start_row, start_col), (end_row, end_col)) = self.selection_range()?;
        let mut lines = Vec::new();
        for row_idx in start_row..=end_row {
            let row = rows.get(row_idx)?;
            if row.is_empty() {
                lines.push(String::new());
                continue;
            }
            let first_col = if row_idx == start_row {
                start_col.min(row.len() - 1)
            } else {
                0
            };
            let last_col = if row_idx == end_row {
                end_col.min(row.len() - 1)
            } else {
                row.len() - 1
            };
            if first_col > last_col {
                lines.push(String::new());
                continue;
            }
            let mut line: String = row[first_col..=last_col].iter().map(|(ch, _, _)| *ch).collect();
            while line.ends_with(' ') {
                line.pop();
            }
            lines.push(line);
        }
        Some(lines.join("\n"))
    }

    // --- Resize -----------------------------------------------------------

    /// Scroll the terminal viewport by `lines` lines.
    /// Negative scrolls up (toward history), positive scrolls down (toward bottom).
    pub fn scroll_by_lines(&mut self, lines: isize) {
        if lines != 0 {
            let _ = self.terminal.scroll_viewport(ScrollViewport::Delta(lines));
        }
    }

    pub fn set_pending_resize(&mut self, rows: usize, cols: usize) -> Option<u64> {
        if self.pending_rows == rows && self.pending_cols == cols {
            return None;
        }
        self.pending_rows = rows;
        self.pending_cols = cols;
        self.resize_generation += 1;
        Some(self.resize_generation)
    }

    pub fn apply_pending_resize(&mut self, gen: u64) {
        if self.resize_generation == gen {
            self.resize_pty(self.pending_rows, self.pending_cols);
        }
    }

    pub fn resize_pty(&mut self, rows: usize, cols: usize) {
        if self.rows == rows && self.cols == cols {
            return;
        }
        self.rows = rows;
        self.cols = cols;
        let _ = self.terminal.resize(cols as u16, rows as u16, 0, 0);
        if let Ok(master) = self.pty_master.lock() {
            let _ = master.resize(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    // --- Tab ------------------------------------------------------------------

    pub fn send_tab(&mut self) {
        self.send_key(b"\t");
    }

    pub fn shell_pid(&self) -> Option<u32> {
        self.shell_pid
    }
}

fn normalize_selection(
    a: (usize, usize),
    b: (usize, usize),
) -> ((usize, usize), (usize, usize)) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

// --- Key mapping ---------------------------------------------------------

fn gpui_key_to_ghostty(key: &str) -> key::Key {
    match key {
        "a" => key::Key::A,
        "b" => key::Key::B,
        "c" => key::Key::C,
        "d" => key::Key::D,
        "e" => key::Key::E,
        "f" => key::Key::F,
        "g" => key::Key::G,
        "h" => key::Key::H,
        "i" => key::Key::I,
        "j" => key::Key::J,
        "k" => key::Key::K,
        "l" => key::Key::L,
        "m" => key::Key::M,
        "n" => key::Key::N,
        "o" => key::Key::O,
        "p" => key::Key::P,
        "q" => key::Key::Q,
        "r" => key::Key::R,
        "s" => key::Key::S,
        "t" => key::Key::T,
        "u" => key::Key::U,
        "v" => key::Key::V,
        "w" => key::Key::W,
        "x" => key::Key::X,
        "y" => key::Key::Y,
        "z" => key::Key::Z,
        "0" => key::Key::Digit0,
        "1" => key::Key::Digit1,
        "2" => key::Key::Digit2,
        "3" => key::Key::Digit3,
        "4" => key::Key::Digit4,
        "5" => key::Key::Digit5,
        "6" => key::Key::Digit6,
        "7" => key::Key::Digit7,
        "8" => key::Key::Digit8,
        "9" => key::Key::Digit9,
        "up" => key::Key::ArrowUp,
        "down" => key::Key::ArrowDown,
        "left" => key::Key::ArrowLeft,
        "right" => key::Key::ArrowRight,
        "enter" | "return" => key::Key::Enter,
        "backspace" => key::Key::Backspace,
        "escape" => key::Key::Escape,
        "tab" => key::Key::Tab,
        "space" => key::Key::Space,
        "delete" => key::Key::Delete,
        "home" => key::Key::Home,
        "end" => key::Key::End,
        "pageup" => key::Key::PageUp,
        "pagedown" => key::Key::PageDown,
        "f1" => key::Key::F1,
        "f2" => key::Key::F2,
        "f3" => key::Key::F3,
        "f4" => key::Key::F4,
        "f5" => key::Key::F5,
        "f6" => key::Key::F6,
        "f7" => key::Key::F7,
        "f8" => key::Key::F8,
        "f9" => key::Key::F9,
        "f10" => key::Key::F10,
        "f11" => key::Key::F11,
        "f12" => key::Key::F12,
        "-" => key::Key::Minus,
        "=" => key::Key::Equal,
        "[" => key::Key::BracketLeft,
        "]" => key::Key::BracketRight,
        ";" => key::Key::Semicolon,
        "'" => key::Key::Quote,
        "," => key::Key::Comma,
        "." => key::Key::Period,
        "/" => key::Key::Slash,
        "\\" => key::Key::Backslash,
        "`" => key::Key::Backquote,
        _ => key::Key::Unidentified,
    }
}

fn gpui_mods_to_ghostty(m: &Modifiers) -> key::Mods {
    let mut mods = key::Mods::empty();
    if m.shift {
        mods |= key::Mods::SHIFT;
    }
    if m.alt {
        mods |= key::Mods::ALT;
    }
    if m.control {
        mods |= key::Mods::CTRL;
    }
    if m.platform {
        mods |= key::Mods::SUPER;
    }
    mods
}

/// The Unicode codepoint the key produces with no modifiers, or '\0' for
/// keys that have no natural printable character.
fn unshifted_codepoint(key: &str) -> char {
    if key == "space" {
        return ' ';
    }
    if key.len() == 1 {
        return key.chars().next().unwrap_or('\0');
    }
    '\0'
}

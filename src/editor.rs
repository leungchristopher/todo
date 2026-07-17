use std::io::{self, Write};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{SetBackgroundColor, SetForegroundColor};
use crossterm::terminal;
use crossterm::{cursor, queue};

use crate::config::Theme;
use crate::ui::{palette, truncate};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Insert,
    Command,
}

/// A deliberately small vim: enough muscle memory to write a log entry without
/// leaving td, and nothing more. Unlike real vim, `:w` closes the editor —
/// there is nowhere to go back to but the project list.
pub struct Editor {
    name: String,
    theme: Theme,
    lines: Vec<Vec<char>>,
    row: usize,
    col: usize,
    /// Index of the first logical line drawn. Long lines wrap onto extra visual
    /// rows rather than scrolling sideways, so a single scroll offset in logical
    /// lines is all we track.
    scroll: usize,
    mode: Mode,
    cmd: String,
    /// First half of a two-key sequence: dd, gg, ZZ.
    pending: Option<char>,
    /// Whole-buffer snapshots. A progress log is small enough that copying it
    /// costs nothing, and it keeps undo honest for free.
    undo: Vec<(Vec<Vec<char>>, usize, usize)>,
    dirty: bool,
    status: Option<String>,
    saved: bool,
    quit: bool,
}

impl Editor {
    pub fn new(name: &str, text: &str, theme: Theme) -> Editor {
        Editor {
            name: name.into(),
            theme,
            lines: text.split('\n').map(|l| l.chars().collect()).collect(),
            row: 0,
            col: 0,
            scroll: 0,
            mode: Mode::Normal,
            cmd: String::new(),
            pending: None,
            undo: Vec::new(),
            dirty: false,
            status: None,
            saved: false,
            quit: false,
        }
    }

    /// Returns the new text if it was written, or None if the edit was thrown
    /// away.
    pub fn run(&mut self, out: &mut impl Write) -> io::Result<Option<String>> {
        loop {
            self.draw(out)?;
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            self.on_key(key);
            if self.quit {
                return Ok(self.saved.then(|| self.text()));
            }
        }
    }

    fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn line_len(&self) -> usize {
        self.lines[self.row].len()
    }

    fn snapshot(&mut self) {
        self.undo.push((self.lines.clone(), self.row, self.col));
    }

    fn on_key(&mut self, key: KeyEvent) {
        self.status = None;
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            // Matches the prompt elsewhere in td: ctrl-c backs out, it never
            // silently commits.
            self.quit = true;
            return;
        }
        match self.mode {
            Mode::Normal => self.normal_key(key),
            Mode::Insert => self.insert_key(key),
            Mode::Command => self.command_key(key),
        }
    }

    fn normal_key(&mut self, key: KeyEvent) {
        // Two-key sequences resolve first, and anything unexpected cancels the
        // pending key rather than doing half of it.
        if let Some(p) = self.pending.take() {
            match (p, key.code) {
                ('d', KeyCode::Char('d')) => return self.delete_line(),
                ('g', KeyCode::Char('g')) => {
                    self.row = 0;
                    self.clamp_col();
                    return;
                }
                ('Z', KeyCode::Char('Z')) => {
                    self.saved = true;
                    self.quit = true;
                    return;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Char('d') | KeyCode::Char('g') | KeyCode::Char('Z') => {
                self.pending = Some(match key.code {
                    KeyCode::Char(c) => c,
                    _ => unreachable!(),
                });
            }
            KeyCode::Char('h') | KeyCode::Left => self.col = self.col.saturating_sub(1),
            KeyCode::Char('l') | KeyCode::Right => {
                self.col = (self.col + 1).min(self.line_len().saturating_sub(1));
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.row = (self.row + 1).min(self.lines.len() - 1);
                self.clamp_col();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.row = self.row.saturating_sub(1);
                self.clamp_col();
            }
            KeyCode::Char('0') | KeyCode::Home => self.col = 0,
            KeyCode::Char('$') | KeyCode::End => self.col = self.line_len().saturating_sub(1),
            KeyCode::Char('G') => {
                self.row = self.lines.len() - 1;
                self.clamp_col();
            }
            KeyCode::Char('w') => self.word_forward(),
            KeyCode::Char('b') => self.word_back(),
            KeyCode::Char('i') => self.enter_insert(),
            KeyCode::Char('I') => {
                self.col = self.first_non_blank();
                self.enter_insert();
            }
            KeyCode::Char('a') => {
                self.col = (self.col + 1).min(self.line_len());
                self.enter_insert();
            }
            KeyCode::Char('A') => {
                self.col = self.line_len();
                self.enter_insert();
            }
            KeyCode::Char('o') => {
                self.enter_insert();
                self.row += 1;
                self.lines.insert(self.row, Vec::new());
                self.col = 0;
            }
            KeyCode::Char('O') => {
                self.enter_insert();
                self.lines.insert(self.row, Vec::new());
                self.col = 0;
            }
            KeyCode::Char('x') if self.col < self.line_len() => {
                self.snapshot();
                self.lines[self.row].remove(self.col);
                self.dirty = true;
                self.clamp_col();
            }
            KeyCode::Char('D') if self.col < self.line_len() => {
                self.snapshot();
                self.lines[self.row].truncate(self.col);
                self.dirty = true;
                self.clamp_col();
            }
            KeyCode::Char('u') => match self.undo.pop() {
                Some((lines, row, col)) => {
                    self.lines = lines;
                    self.row = row.min(self.lines.len() - 1);
                    self.col = col;
                    self.clamp_col();
                    self.dirty = true;
                }
                None => self.status = Some("already at oldest change".into()),
            },
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.cmd.clear();
            }
            _ => {}
        }
    }

    fn insert_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.col = self.col.saturating_sub(1);
                self.clamp_col();
            }
            KeyCode::Enter => {
                let tail = self.lines[self.row].split_off(self.col);
                self.row += 1;
                self.lines.insert(self.row, tail);
                self.col = 0;
                self.dirty = true;
            }
            KeyCode::Backspace => {
                if self.col > 0 {
                    self.col -= 1;
                    self.lines[self.row].remove(self.col);
                    self.dirty = true;
                } else if self.row > 0 {
                    let line = self.lines.remove(self.row);
                    self.row -= 1;
                    self.col = self.line_len();
                    self.lines[self.row].extend(line);
                    self.dirty = true;
                }
            }
            KeyCode::Tab => {
                for _ in 0..2 {
                    self.lines[self.row].insert(self.col, ' ');
                    self.col += 1;
                }
                self.dirty = true;
            }
            KeyCode::Char(c) => {
                self.lines[self.row].insert(self.col, c);
                self.col += 1;
                self.dirty = true;
            }
            KeyCode::Left => self.col = self.col.saturating_sub(1),
            KeyCode::Right => self.col = (self.col + 1).min(self.line_len()),
            KeyCode::Down => {
                self.row = (self.row + 1).min(self.lines.len() - 1);
                self.col = self.col.min(self.line_len());
            }
            KeyCode::Up => {
                self.row = self.row.saturating_sub(1);
                self.col = self.col.min(self.line_len());
            }
            KeyCode::Home => self.col = 0,
            KeyCode::End => self.col = self.line_len(),
            _ => {}
        }
    }

    fn command_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Backspace if !self.cmd.is_empty() => {
                self.cmd.pop();
            }
            // Backspacing over the colon itself abandons the command.
            KeyCode::Backspace => self.mode = Mode::Normal,
            KeyCode::Char(c) => self.cmd.push(c),
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                let cmd = std::mem::take(&mut self.cmd);
                match cmd.trim() {
                    "w" | "wq" | "x" => {
                        self.saved = true;
                        self.quit = true;
                    }
                    "q" if self.dirty => {
                        self.status =
                            Some("unwritten changes — :w to save and close, :q! to discard".into())
                    }
                    "q" | "q!" => self.quit = true,
                    other => self.status = Some(format!("not an editor command: {other}")),
                }
            }
            _ => {}
        }
    }

    fn enter_insert(&mut self) {
        // One snapshot per insert run, so u undoes a typed sentence rather
        // than a single letter.
        self.snapshot();
        self.mode = Mode::Insert;
    }

    fn delete_line(&mut self) {
        self.snapshot();
        self.lines.remove(self.row);
        // The buffer is never empty: a zero-line file has nowhere to put the
        // cursor.
        if self.lines.is_empty() {
            self.lines.push(Vec::new());
        }
        self.row = self.row.min(self.lines.len() - 1);
        self.dirty = true;
        self.clamp_col();
    }

    /// Normal mode sits *on* a character, so the last valid column is one back
    /// from the end; insert mode may sit past it.
    fn clamp_col(&mut self) {
        let max = match self.mode {
            Mode::Insert => self.line_len(),
            _ => self.line_len().saturating_sub(1),
        };
        self.col = self.col.min(max);
    }

    fn first_non_blank(&self) -> usize {
        self.lines[self.row]
            .iter()
            .position(|c| !c.is_whitespace())
            .unwrap_or(0)
    }

    fn word_forward(&mut self) {
        let line = &self.lines[self.row];
        let mut i = self.col;
        while i < line.len() && !line[i].is_whitespace() {
            i += 1;
        }
        while i < line.len() && line[i].is_whitespace() {
            i += 1;
        }
        // Off the end of the line: carry on at the start of the next one.
        if i >= line.len() && self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = self.first_non_blank();
        } else {
            self.col = i.min(line.len().saturating_sub(1));
        }
    }

    fn word_back(&mut self) {
        if self.col == 0 {
            if self.row > 0 {
                self.row -= 1;
                self.col = self.line_len().saturating_sub(1);
            }
            return;
        }
        let line = &self.lines[self.row];
        let mut i = self.col - 1;
        while i > 0 && line[i].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !line[i - 1].is_whitespace() {
            i -= 1;
        }
        self.col = i;
    }

    // ---- rendering -------------------------------------------------------

    /// How many visual rows a logical line of `len` characters occupies when
    /// wrapped at `text_w`. An empty line still takes one row to sit on.
    fn line_rows(len: usize, text_w: usize) -> usize {
        if len == 0 {
            1
        } else {
            (len + text_w - 1) / text_w
        }
    }

    /// The cursor's position within the body, in visual (row, col) cells counted
    /// from the first drawn line. Assumes scroll_into_view has already run, so
    /// `self.scroll <= self.row`.
    fn cursor_visual(&self, text_w: usize) -> (usize, usize) {
        let mut vrow = 0;
        for r in self.scroll..self.row {
            vrow += Self::line_rows(self.lines[r].len(), text_w);
        }
        // A full line the cursor sits just past (insert mode at a wrap boundary)
        // puts the cursor at the start of a fresh visual row.
        (vrow + self.col / text_w, self.col % text_w)
    }

    fn draw(&mut self, out: &mut impl Write) -> io::Result<()> {
        let (w, h) = terminal::size()?;
        let (w, h) = (w as usize, h.max(4) as usize);
        let pal = palette(self.theme);
        let body_h = h.saturating_sub(3);
        let text_w = w.saturating_sub(2).max(1);
        self.scroll_into_view(body_h, text_w);

        queue!(
            out,
            cursor::Hide,
            SetBackgroundColor(pal.bg),
            SetForegroundColor(pal.fg)
        )?;
        for y in 0..h {
            queue!(out, cursor::MoveTo(0, y as u16))?;
            out.write_all(" ".repeat(w).as_bytes())?;
        }

        // Header: which project, and how far into it you are.
        queue!(out, cursor::MoveTo(1, 0), SetForegroundColor(pal.accent))?;
        out.write_all(b"td")?;
        queue!(out, SetForegroundColor(pal.dim))?;
        out.write_all(format!(" · {}", truncate(&self.name, w / 2)).as_bytes())?;
        let right = format!(
            "{}{}:{} · {}",
            if self.dirty { "* " } else { "" },
            self.row + 1,
            self.col + 1,
            match self.mode {
                Mode::Insert => "insert",
                _ => "normal",
            }
        );
        let x = w.saturating_sub(right.chars().count() + 1);
        queue!(
            out,
            cursor::MoveTo(x as u16, 0),
            SetForegroundColor(pal.dim)
        )?;
        out.write_all(right.as_bytes())?;

        // Draw logical lines top to bottom, spilling each onto as many visual
        // rows as it needs, until the body is full.
        let mut y = 0usize;
        'lines: for (li, line) in self.lines.iter().enumerate().skip(self.scroll) {
            let len = line.len();
            for seg in 0..Self::line_rows(len, text_w) {
                if y >= body_h {
                    break 'lines;
                }
                let start = seg * text_w;
                let end = (start + text_w).min(len);
                let text: String = line[start..end].iter().collect();
                queue!(
                    out,
                    cursor::MoveTo(1, 2 + y as u16),
                    SetForegroundColor(pal.fg)
                )?;
                out.write_all(text.as_bytes())?;
                y += 1;
            }
            // Insert mode at the end of a line whose length lands exactly on a
            // wrap boundary needs an extra blank row for the cursor to sit on.
            if li == self.row && self.col == len && len > 0 && len % text_w == 0 && y < body_h {
                y += 1;
            }
        }

        let y = (h - 1) as u16;
        queue!(out, cursor::MoveTo(1, y))?;
        if self.mode == Mode::Command {
            queue!(out, SetForegroundColor(pal.fg))?;
            out.write_all(format!(":{}", self.cmd).as_bytes())?;
            let x = 2 + self.cmd.chars().count();
            queue!(out, cursor::MoveTo(x.min(w - 1) as u16, y), cursor::Show)?;
            return out.flush();
        }
        let hint = match &self.status {
            Some(s) => s.clone(),
            None => {
                "i insert · esc normal · dd cut line · u undo · :w save & close · :q discard".into()
            }
        };
        queue!(
            out,
            SetForegroundColor(if self.status.is_some() {
                pal.accent
            } else {
                pal.dim
            })
        )?;
        out.write_all(truncate(&hint, w.saturating_sub(2)).as_bytes())?;

        let (crow, ccol) = self.cursor_visual(text_w);
        queue!(
            out,
            cursor::MoveTo(1 + ccol as u16, 2 + crow as u16),
            cursor::Show
        )?;
        out.flush()
    }

    fn scroll_into_view(&mut self, body_h: usize, text_w: usize) {
        if self.row < self.scroll {
            self.scroll = self.row;
        }
        // Push the top down one logical line at a time until the cursor's visual
        // row fits. A line taller than the whole body is pinned to the top
        // rather than hiding the cursor entirely.
        while self.scroll < self.row {
            let (crow, _) = self.cursor_visual(text_w);
            if crow < body_h {
                break;
            }
            self.scroll += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed(text: &str) -> Editor {
        Editor::new("test", text, Theme::Dark)
    }

    fn press(e: &mut Editor, keys: &str) {
        for c in keys.chars() {
            e.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    fn esc(e: &mut Editor) {
        e.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }

    #[test]
    fn empty_text_still_has_a_line_to_sit_on() {
        let e = ed("");
        assert_eq!(e.lines.len(), 1);
        assert_eq!(e.text(), "");
    }

    #[test]
    fn insert_types_at_the_cursor() {
        let mut e = ed("bc");
        press(&mut e, "ia");
        assert_eq!(e.text(), "abc");
        esc(&mut e);
        assert_eq!(e.col, 0, "esc steps back onto the last typed character");
    }

    #[test]
    fn o_opens_a_line_below_and_types_into_it() {
        let mut e = ed("one");
        press(&mut e, "otwo");
        assert_eq!(e.text(), "one\ntwo");
    }

    #[test]
    fn dd_deletes_the_line_and_never_empties_the_buffer() {
        let mut e = ed("only");
        press(&mut e, "dd");
        assert_eq!(e.text(), "");
        assert_eq!(e.lines.len(), 1);
        assert_eq!(e.row, 0);
    }

    #[test]
    fn undo_rewinds_a_whole_insert_run() {
        let mut e = ed("x");
        press(&mut e, "ahello");
        esc(&mut e);
        assert_eq!(e.text(), "xhello");
        press(&mut e, "u");
        assert_eq!(e.text(), "x");
    }

    #[test]
    fn normal_mode_cursor_stays_on_a_character() {
        let mut e = ed("ab");
        press(&mut e, "$");
        assert_eq!(e.col, 1);
        press(&mut e, "llll");
        assert_eq!(e.col, 1, "l must not walk off the end");
    }

    #[test]
    fn a_pending_key_is_cancelled_by_an_unrelated_one() {
        let mut e = ed("one\ntwo");
        press(&mut e, "dj");
        assert_eq!(e.text(), "one\ntwo", "d then j must not delete");
        assert!(e.pending.is_none());
    }

    #[test]
    fn write_returns_the_text_and_quit_discards_it() {
        let mut e = ed("start");
        press(&mut e, "ax");
        esc(&mut e);
        press(&mut e, ":w");
        e.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(e.quit && e.saved);
        assert_eq!(e.text(), "sxtart");

        let mut e = ed("start");
        press(&mut e, ":q");
        e.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(e.quit && !e.saved);
    }

    #[test]
    fn bare_q_refuses_to_throw_away_unwritten_changes() {
        let mut e = ed("keep me");
        press(&mut e, "x");
        press(&mut e, ":q");
        e.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!e.quit, "dirty buffer must not close on a bare :q");
        assert!(e.status.is_some());

        press(&mut e, ":q!");
        e.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(e.quit && !e.saved, ":q! discards");
    }

    #[test]
    fn zz_saves_and_closes() {
        let mut e = ed("note");
        press(&mut e, "ZZ");
        assert!(e.quit && e.saved);
    }

    #[test]
    fn a_line_wraps_onto_a_visual_row_per_screen_width() {
        // Empty and short lines take one row; longer ones spill over.
        assert_eq!(Editor::line_rows(0, 4), 1);
        assert_eq!(Editor::line_rows(4, 4), 1);
        assert_eq!(Editor::line_rows(5, 4), 2);
        assert_eq!(Editor::line_rows(9, 4), 3);
    }

    #[test]
    fn cursor_visual_follows_the_wrap() {
        let mut e = ed("0123456789\nnext");
        // Column 6 on a width-4 line sits on the second visual row, offset 2.
        e.col = 6;
        assert_eq!(e.cursor_visual(4), (1, 2));

        // The second logical line starts below all three wrapped rows above it.
        e.row = 1;
        e.col = 0;
        assert_eq!(e.cursor_visual(4), (3, 0));
    }

    #[test]
    fn cursor_at_a_wrap_boundary_lands_on_a_fresh_row() {
        // Insert mode sitting just past a line that fills the width exactly:
        // the cursor moves to the start of the next visual row.
        let mut e = ed("0123");
        e.mode = Mode::Insert;
        e.col = 4;
        assert_eq!(e.cursor_visual(4), (1, 0));
    }
}

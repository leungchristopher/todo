use std::io::{self, Write};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Color, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal;
use crossterm::{cursor, queue};

use crate::config::{Config, Theme};
use crate::date;
use crate::editor::Editor;
use crate::model::{Project, Projects, Store, Todo};

/// 256-colour indices rather than truecolour: macOS Terminal.app speaks 256
/// colours but not 24-bit RGB.
pub struct Palette {
    pub bg: Color,
    pub fg: Color,
    pub dim: Color,
    pub accent: Color,
    pub done: Color,
    pub overdue: Color,
    pub sel: Color,
}

pub fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => Palette {
            bg: Color::AnsiValue(234),
            fg: Color::AnsiValue(252),
            dim: Color::AnsiValue(243),
            accent: Color::AnsiValue(110),
            done: Color::AnsiValue(108),
            overdue: Color::AnsiValue(174),
            sel: Color::AnsiValue(237),
        },
        Theme::Light => Palette {
            bg: Color::AnsiValue(255),
            fg: Color::AnsiValue(236),
            dim: Color::AnsiValue(245),
            accent: Color::AnsiValue(25),
            done: Color::AnsiValue(29),
            overdue: Color::AnsiValue(124),
            sel: Color::AnsiValue(253),
        },
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    List,
    Projects,
    History,
}

enum Row {
    Item(usize),
    Detail(usize, String),
}

pub struct App {
    store: Store,
    projects: Projects,
    cfg: Config,
    view: View,
    sel: usize,
    scroll: usize,
    status: Option<String>,
    trash: Option<Todo>,
    trash_project: Option<Project>,
    help: bool,
    quit: bool,
}

impl App {
    pub fn new(mut store: Store, projects: Projects, cfg: Config) -> App {
        store.sort(cfg.sort);
        App {
            store,
            projects,
            cfg,
            view: View::List,
            sel: 0,
            scroll: 0,
            status: None,
            trash: None,
            trash_project: None,
            help: false,
            quit: false,
        }
    }

    /// The todos behind the current view. Meaningless in Projects — use
    /// `len` for anything that has to work in every view.
    fn items(&self) -> &[Todo] {
        match self.view {
            View::Projects | View::List => &self.store.todos,
            View::History => &self.store.history,
        }
    }

    fn len(&self) -> usize {
        match self.view {
            View::Projects => self.projects.items.len(),
            _ => self.items().len(),
        }
    }

    pub fn run(&mut self, out: &mut impl Write) -> io::Result<()> {
        loop {
            self.draw(out, None)?;
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            self.on_key(out, key)?;
            if self.quit {
                return Ok(());
            }
        }
    }

    fn on_key(&mut self, out: &mut impl Write, key: KeyEvent) -> io::Result<()> {
        self.status = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('c')) {
            self.quit = true;
            return Ok(());
        }
        if self.help {
            // Any key dismisses help, so it never traps the user.
            self.help = false;
            return Ok(());
        }

        // Keys that mean the same thing in every view.
        match key.code {
            KeyCode::Char('q') => {
                self.quit = true;
                return Ok(());
            }
            KeyCode::Char('?') => {
                self.help = true;
                return Ok(());
            }
            KeyCode::Char(':') => return self.command(out),
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_sel(1);
                return Ok(());
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_sel(-1);
                return Ok(());
            }
            KeyCode::Char('g') | KeyCode::Home => {
                self.sel = 0;
                return Ok(());
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.sel = self.len().saturating_sub(1);
                return Ok(());
            }
            KeyCode::Char('1') => {
                self.go(View::List);
                return Ok(());
            }
            KeyCode::Char('2') => {
                self.go(View::Projects);
                return Ok(());
            }
            KeyCode::Enter | KeyCode::Tab => {
                if self.view == View::Projects {
                    return self.open_project(out);
                }
                self.toggle_expand();
                return Ok(());
            }
            KeyCode::Char('h') => {
                self.go(if self.view == View::History {
                    View::List
                } else {
                    View::History
                });
                return Ok(());
            }
            // Esc backs out to the list; it must not open anything.
            KeyCode::Esc if self.view != View::List => {
                self.go(View::List);
                return Ok(());
            }
            KeyCode::Char('m') => return self.toggle_theme(),
            KeyCode::Char('x') => return self.delete(),
            KeyCode::Char('u') => return self.undo_delete(),
            _ => {}
        }

        if self.view == View::Projects {
            match key.code {
                KeyCode::Char('o') => return self.new_project(out),
                KeyCode::Char('e') => return self.rename_project(out),
                _ => {}
            }
            return Ok(());
        }

        if let KeyCode::Char('a') = key.code {
            return self.file_or_restore();
        }
        if self.view == View::History {
            // The rest only makes sense against the live list.
            self.status = Some("history is read-only — a restores, h goes back".into());
            return Ok(());
        }
        match key.code {
            KeyCode::Char(' ') => self.toggle_done()?,
            KeyCode::Char('o') => self.new_todo(out)?,
            KeyCode::Char('e') => self.edit_title(out)?,
            KeyCode::Char('d') => self.edit_details(out)?,
            KeyCode::Char('t') => self.edit_due(out)?,
            KeyCode::Char('s') => self.cycle_sort()?,
            _ => {}
        }
        Ok(())
    }

    /// The `:` line. Only two things live here: the sweep phrase and `:q`,
    /// because Vim muscle memory will type it whether it exists or not.
    fn command(&mut self, out: &mut impl Write) -> io::Result<()> {
        let Some(input) = self.prompt(out, ":", "")? else {
            return Ok(());
        };
        let input = input.trim();
        if input.is_empty() {
            return Ok(());
        }
        if input.eq_ignore_ascii_case("q") || input.eq_ignore_ascii_case("quit") {
            self.quit = true;
            return Ok(());
        }
        if input.eq_ignore_ascii_case(self.cfg.phrase.trim()) {
            return self.sweep();
        }
        self.status = Some(format!("unknown command — try :{}", self.cfg.phrase));
        Ok(())
    }

    fn sweep(&mut self) -> io::Result<()> {
        let n = self.store.sweep();
        self.clamp_sel();
        self.status = Some(match n {
            0 => "nothing ticked off to sweep".into(),
            1 => "1 todo swept to history".into(),
            n => format!("{n} todos swept to history"),
        });
        if n > 0 {
            return self.persist();
        }
        Ok(())
    }

    /// Each view keeps its own cursor conceptually, but not literally: coming
    /// back to a view starts at the top, which is where you look anyway.
    fn go(&mut self, view: View) {
        if self.view == view {
            return;
        }
        self.view = view;
        self.sel = 0;
        self.scroll = 0;
    }

    // ---- projects --------------------------------------------------------

    fn new_project(&mut self, out: &mut impl Write) -> io::Result<()> {
        let Some(name) = self.prompt(out, "project▸ ", "")? else {
            return Ok(());
        };
        if name.trim().is_empty() {
            return Ok(());
        }
        let id = self.projects.next_id();
        self.projects
            .items
            .push(Project::new(id, name.trim().into()));
        self.sel = self.projects.index_of(id).unwrap_or(self.sel);
        self.save_projects()
    }

    fn rename_project(&mut self, out: &mut impl Write) -> io::Result<()> {
        let Some(p) = self.projects.items.get(self.sel) else {
            return Ok(());
        };
        let current = p.name.clone();
        let Some(name) = self.prompt(out, "project▸ ", &current)? else {
            return Ok(());
        };
        if name.trim().is_empty() {
            return Ok(());
        }
        self.projects.items[self.sel].name = name.trim().into();
        self.save_projects()
    }

    /// Hands the whole screen to the editor and takes it back afterwards. The
    /// log is only touched if the editor says it was written.
    fn open_project(&mut self, out: &mut impl Write) -> io::Result<()> {
        let Some(p) = self.projects.items.get(self.sel) else {
            return Ok(());
        };
        let mut ed = Editor::new(&p.name, &p.log, self.cfg.theme);
        let Some(text) = ed.run(out)? else {
            return Ok(());
        };
        let p = &mut self.projects.items[self.sel];
        if p.log == text {
            // An open-and-close with no real change shouldn't make a project
            // look freshly worked on.
            return Ok(());
        }
        p.log = text;
        p.updated = chrono::Local::now();
        let name = truncate(&p.name, 40);
        self.status = Some(format!("“{name}” logged"));
        self.save_projects()
    }

    fn save_projects(&mut self) -> io::Result<()> {
        if let Err(e) = self.projects.save() {
            self.status = Some(format!("could not save: {e}"));
        }
        Ok(())
    }

    /// `a` files a todo away from the list, or pulls one back out of history.
    fn file_or_restore(&mut self) -> io::Result<()> {
        match self.view {
            View::Projects => Ok(()),
            View::List => {
                let Some(t) = self.store.todos.get(self.sel) else {
                    return Ok(());
                };
                let title = truncate(&t.title, 40);
                if self.store.archive(self.sel).is_some() {
                    self.clamp_sel();
                    self.status = Some(format!("“{title}” filed to history"));
                    return self.persist();
                }
                Ok(())
            }
            View::History => {
                let Some(t) = self.store.history.get(self.sel) else {
                    return Ok(());
                };
                let title = truncate(&t.title, 40);
                if self.store.restore(self.sel, self.cfg.sort).is_some() {
                    self.clamp_sel();
                    self.status = Some(format!("“{title}” restored to the list, unticked"));
                    return self.persist();
                }
                Ok(())
            }
        }
    }

    fn move_sel(&mut self, delta: isize) {
        let len = self.len();
        if len == 0 {
            return;
        }
        let next = self.sel as isize + delta;
        self.sel = next.clamp(0, len as isize - 1) as usize;
    }

    fn clamp_sel(&mut self) {
        self.sel = self.sel.min(self.len().saturating_sub(1));
    }

    fn toggle_done(&mut self) -> io::Result<()> {
        let Some(t) = self.store.todos.get_mut(self.sel) else {
            return Ok(());
        };
        t.done = !t.done;
        t.completed = if t.done {
            Some(chrono::Local::now())
        } else {
            None
        };
        self.persist()
    }

    fn toggle_expand(&mut self) {
        let sel = self.sel;
        let items = match self.view {
            View::Projects | View::List => &mut self.store.todos,
            View::History => &mut self.store.history,
        };
        if let Some(t) = items.get_mut(sel) {
            if t.details.is_empty() {
                self.status = Some("no details — press d to add some".into());
            } else {
                t.expanded = !t.expanded;
            }
        }
    }

    fn new_todo(&mut self, out: &mut impl Write) -> io::Result<()> {
        let Some(title) = self.prompt(out, "title▸ ", "")? else {
            return Ok(());
        };
        if title.trim().is_empty() {
            return Ok(());
        }
        let id = self.store.next_id();
        self.store.todos.push(Todo::new(id, title.trim().into()));
        self.resort_keeping(id);
        self.persist()
    }

    fn edit_title(&mut self, out: &mut impl Write) -> io::Result<()> {
        let Some(t) = self.store.todos.get(self.sel) else {
            return Ok(());
        };
        let (id, current) = (t.id, t.title.clone());
        let Some(title) = self.prompt(out, "title▸ ", &current)? else {
            return Ok(());
        };
        if title.trim().is_empty() {
            return Ok(());
        }
        self.store.todos[self.sel].title = title.trim().into();
        self.resort_keeping(id);
        self.persist()
    }

    fn edit_details(&mut self, out: &mut impl Write) -> io::Result<()> {
        let Some(t) = self.store.todos.get(self.sel) else {
            return Ok(());
        };
        let current = t.details.clone();
        let Some(details) = self.prompt(out, "details▸ ", &current)? else {
            return Ok(());
        };
        let t = &mut self.store.todos[self.sel];
        t.details = details.trim().into();
        t.expanded = !t.details.is_empty();
        self.persist()
    }

    fn edit_due(&mut self, out: &mut impl Write) -> io::Result<()> {
        let Some(t) = self.store.todos.get(self.sel) else {
            return Ok(());
        };
        let (id, current) = (t.id, t.due.map(|d| d.to_string()).unwrap_or_default());
        let Some(input) = self.prompt(out, "due▸ ", &current)? else {
            return Ok(());
        };
        match date::parse_due(&input) {
            Ok(due) => {
                self.store.todos[self.sel].due = due;
                self.resort_keeping(id);
                self.persist()
            }
            Err(()) => {
                self.status =
                    Some("try: today, tomorrow, +3d, 2w, 12-25, 2026-12-25, or - to clear".into());
                Ok(())
            }
        }
    }

    fn delete(&mut self) -> io::Result<()> {
        if self.view == View::Projects {
            if self.sel >= self.projects.items.len() {
                return Ok(());
            }
            let p = self.projects.items.remove(self.sel);
            self.status = Some(format!("deleted “{}” — u to undo", truncate(&p.name, 40)));
            self.trash_project = Some(p);
            self.clamp_sel();
            return self.save_projects();
        }
        let list = match self.view {
            View::Projects | View::List => &mut self.store.todos,
            View::History => &mut self.store.history,
        };
        if self.sel >= list.len() {
            return Ok(());
        }
        let t = list.remove(self.sel);
        self.status = Some(format!("deleted “{}” — u to undo", truncate(&t.title, 40)));
        self.trash = Some(t);
        self.clamp_sel();
        self.persist()
    }

    fn undo_delete(&mut self) -> io::Result<()> {
        if self.view == View::Projects {
            let Some(p) = self.trash_project.take() else {
                self.status = Some("nothing to undo".into());
                return Ok(());
            };
            let id = p.id;
            self.projects.items.push(p);
            self.sel = self.projects.index_of(id).unwrap_or(self.sel);
            return self.save_projects();
        }
        let Some(t) = self.trash.take() else {
            self.status = Some("nothing to undo".into());
            return Ok(());
        };
        // A todo remembers which side it came from, so undo puts it back
        // there even if the view has since changed.
        let id = t.id;
        if t.archived.is_some() {
            self.store.push_history(t);
            if self.view == View::History {
                if let Some(i) = self.store.history_index_of(id) {
                    self.sel = i;
                }
            }
        } else {
            self.store.todos.push(t);
            self.resort_keeping(id);
        }
        self.persist()
    }

    fn cycle_sort(&mut self) -> io::Result<()> {
        self.cfg.sort = self.cfg.sort.next();
        let id = self.store.todos.get(self.sel).map(|t| t.id);
        self.store.sort(self.cfg.sort);
        if let Some(id) = id {
            self.sel = self.store.index_of(id).unwrap_or(self.sel);
        }
        self.status = Some(format!("sorting by {}", self.cfg.sort.name()));
        let _ = self.cfg.save();
        Ok(())
    }

    fn toggle_theme(&mut self) -> io::Result<()> {
        self.cfg.theme = self.cfg.theme.toggle();
        let _ = self.cfg.save();
        Ok(())
    }

    /// Re-sorts, then puts the cursor back on the todo the user was acting on
    /// wherever the new order moved it.
    fn resort_keeping(&mut self, id: u64) {
        self.store.sort(self.cfg.sort);
        if let Some(i) = self.store.index_of(id) {
            self.sel = i;
        }
    }

    fn persist(&mut self) -> io::Result<()> {
        if let Err(e) = self.store.save() {
            self.status = Some(format!("could not save: {e}"));
        }
        Ok(())
    }

    // ---- rendering -------------------------------------------------------

    fn rows(&self, width: usize) -> Vec<Row> {
        let mut rows = Vec::new();
        for (i, t) in self.items().iter().enumerate() {
            rows.push(Row::Item(i));
            if t.expanded {
                for line in wrap(&t.details, width.saturating_sub(7)) {
                    rows.push(Row::Detail(i, line));
                }
            }
        }
        rows
    }

    fn draw(&mut self, out: &mut impl Write, prompt: Option<&Prompt>) -> io::Result<()> {
        let (w, h) = terminal::size()?;
        let (w, h) = (w as usize, h.max(4) as usize);
        let pal = palette(self.cfg.theme);

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

        self.draw_header(out, w, &pal)?;

        let top = 2u16;
        let list_h = h.saturating_sub(3);
        if self.help {
            self.draw_help(out, top, w, list_h, &pal)?;
        } else if self.len() == 0 {
            let msg = match self.view {
                View::List => "nothing here yet - press o to add a todo".into(),
                View::Projects => "no projects yet - press o to start one".into(),
                View::History => format!(
                    "history is empty - ticked todos land here when you type :{}",
                    self.cfg.phrase
                ),
            };
            queue!(out, cursor::MoveTo(2, top), SetForegroundColor(pal.dim))?;
            out.write_all(truncate(&msg, w.saturating_sub(3)).as_bytes())?;
        } else if self.view == View::Projects {
            self.draw_projects(out, top, w, list_h, &pal)?;
        } else {
            self.draw_list(out, top, w, list_h, &pal)?;
        }

        self.draw_footer(out, w, h, &pal, prompt)?;
        out.flush()
    }

    fn draw_header(&self, out: &mut impl Write, w: usize, pal: &Palette) -> io::Result<()> {
        queue!(out, cursor::MoveTo(1, 0), SetForegroundColor(pal.accent))?;
        out.write_all(b"td")?;

        // The tabs carry their own switch keys, so the second view is
        // discoverable without opening the help.
        for (key, label, view) in [
            ("1", "todos", View::List),
            ("2", "projects", View::Projects),
        ] {
            let active = self.view == view;
            queue!(
                out,
                SetForegroundColor(if active { pal.accent } else { pal.dim })
            )?;
            out.write_all(format!("  {key} {label}").as_bytes())?;
        }

        let right = match self.view {
            View::List => {
                let done = self.store.todos.iter().filter(|t| t.done).count();
                format!(
                    "{}/{} · {} · {}",
                    done,
                    self.store.todos.len(),
                    self.cfg.sort.name(),
                    self.cfg.theme.name()
                )
            }
            View::Projects => format!(
                "{} logged · {}",
                self.projects.items.len(),
                self.cfg.theme.name()
            ),
            View::History => {
                // History hangs off the todos tab rather than earning one of
                // its own: it is where todos go, not a third place to work.
                queue!(out, SetForegroundColor(pal.accent))?;
                out.write_all("  · history".as_bytes())?;
                format!(
                    "{} filed · {}",
                    self.store.history.len(),
                    self.cfg.theme.name()
                )
            }
        };
        let x = w.saturating_sub(right.chars().count() + 1);
        queue!(
            out,
            cursor::MoveTo(x as u16, 0),
            SetForegroundColor(pal.dim)
        )?;
        out.write_all(right.as_bytes())?;
        Ok(())
    }

    fn draw_list(
        &mut self,
        out: &mut impl Write,
        top: u16,
        w: usize,
        list_h: usize,
        pal: &Palette,
    ) -> io::Result<()> {
        let rows = self.rows(w);
        self.update_scroll(&rows, list_h);
        let today = date::today();
        let history = self.view == View::History;

        for (n, row) in rows.iter().skip(self.scroll).take(list_h).enumerate() {
            let y = top + n as u16;
            match row {
                Row::Item(i) => {
                    let t = &self.items()[*i];
                    let selected = *i == self.sel;
                    let bg = if selected { pal.sel } else { pal.bg };
                    queue!(out, cursor::MoveTo(0, y), SetBackgroundColor(bg))?;
                    out.write_all(" ".repeat(w).as_bytes())?;

                    queue!(out, cursor::MoveTo(0, y), SetForegroundColor(pal.accent))?;
                    out.write_all(if selected { "›".as_bytes() } else { b" " })?;

                    // In history the tick is a record, not a control: an
                    // unticked entry there was filed by hand.
                    queue!(
                        out,
                        SetForegroundColor(if t.done { pal.done } else { pal.dim })
                    )?;
                    out.write_all(if t.done { b" [x] " } else { b" [ ] " })?;

                    // The right column answers the question each view asks:
                    // when is this due, versus when did this get finished.
                    let right = if history {
                        t.filed_at()
                            .map(|d| date::ago(d.date_naive()))
                            .unwrap_or_default()
                    } else {
                        t.due.map(date::label).unwrap_or_default()
                    };
                    let right_w = right.chars().count();
                    let title_w = w.saturating_sub(6 + right_w + 2);
                    queue!(
                        out,
                        SetForegroundColor(if t.done || history { pal.dim } else { pal.fg })
                    )?;
                    out.write_all(truncate(&t.title, title_w).as_bytes())?;

                    if !right.is_empty() {
                        let overdue = !history && t.due.is_some_and(|d| d <= today) && !t.done;
                        let color = if overdue { pal.overdue } else { pal.dim };
                        let x = w.saturating_sub(right_w + 1);
                        queue!(out, cursor::MoveTo(x as u16, y), SetForegroundColor(color))?;
                        out.write_all(right.as_bytes())?;
                    }
                }
                Row::Detail(_, text) => {
                    queue!(
                        out,
                        cursor::MoveTo(6, y),
                        SetBackgroundColor(pal.bg),
                        SetForegroundColor(pal.dim)
                    )?;
                    out.write_all(text.as_bytes())?;
                }
            }
        }
        Ok(())
    }

    /// One line per project: the name, and when it was last written to. No
    /// checkbox — a project is not something you finish.
    fn draw_projects(
        &mut self,
        out: &mut impl Write,
        top: u16,
        w: usize,
        list_h: usize,
        pal: &Palette,
    ) -> io::Result<()> {
        if self.sel < self.scroll {
            self.scroll = self.sel;
        }
        if self.sel >= self.scroll + list_h {
            self.scroll = self.sel + 1 - list_h;
        }

        for (n, (i, p)) in self
            .projects
            .items
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(list_h)
            .enumerate()
        {
            let y = top + n as u16;
            let selected = i == self.sel;
            let bg = if selected { pal.sel } else { pal.bg };
            queue!(out, cursor::MoveTo(0, y), SetBackgroundColor(bg))?;
            out.write_all(" ".repeat(w).as_bytes())?;

            queue!(out, cursor::MoveTo(0, y), SetForegroundColor(pal.accent))?;
            out.write_all(if selected { "›".as_bytes() } else { b" " })?;

            let right = if p.log.is_empty() {
                "empty".to_string()
            } else {
                date::ago(p.updated.date_naive())
            };
            let right_w = right.chars().count();
            queue!(out, cursor::MoveTo(2, y), SetForegroundColor(pal.fg))?;
            out.write_all(truncate(&p.name, w.saturating_sub(right_w + 4)).as_bytes())?;

            let x = w.saturating_sub(right_w + 1);
            queue!(
                out,
                cursor::MoveTo(x as u16, y),
                SetForegroundColor(pal.dim)
            )?;
            out.write_all(right.as_bytes())?;
        }
        Ok(())
    }

    /// Keeps the selected todo on screen, preferring to show its first line
    /// when the todo plus its details is taller than the viewport.
    fn update_scroll(&mut self, rows: &[Row], list_h: usize) {
        let first = rows
            .iter()
            .position(|r| matches!(r, Row::Item(i) if *i == self.sel))
            .unwrap_or(0);
        let last = rows
            .iter()
            .rposition(|r| match r {
                Row::Item(i) | Row::Detail(i, _) => *i == self.sel,
            })
            .unwrap_or(first);

        if last >= self.scroll + list_h {
            self.scroll = last + 1 - list_h;
        }
        if first < self.scroll {
            self.scroll = first;
        }
        let max = rows.len().saturating_sub(list_h);
        self.scroll = self.scroll.min(max);
    }

    fn help_keys(&self) -> Vec<(String, String)> {
        let mut keys: Vec<(&str, String)> = vec![
            ("1 / 2", "todos / projects".into()),
            ("j / k", "move down / up".into()),
            ("g / G", "first / last".into()),
        ];
        match self.view {
            View::List => keys.extend([
                ("enter", "expand or collapse details".to_string()),
                ("space", "toggle done".into()),
                ("o", "new todo".into()),
                ("e", "edit title".into()),
                ("d", "edit details".into()),
                ("t", "set due date".into()),
                ("a", "file to history".into()),
                ("s", "cycle sort: created / due / alpha".into()),
                ("h", "show history".into()),
            ]),
            View::Projects => keys.extend([
                ("enter", "open the log in the editor".to_string()),
                ("o", "new project".into()),
                ("e", "rename".into()),
            ]),
            View::History => keys.extend([
                ("enter", "expand or collapse details".to_string()),
                ("a", "restore to the list, unticked".into()),
                ("h / esc", "back to the list".into()),
            ]),
        }
        keys.extend([
            ("x", "delete".to_string()),
            ("u", "undo delete".into()),
            ("m", "toggle light / dark mode".into()),
            (
                ":",
                format!("type :{} to sweep ticked todos to history", self.cfg.phrase),
            ),
            ("?", "toggle this help".into()),
            ("q", "quit".into()),
        ]);
        keys.into_iter().map(|(k, d)| (k.to_string(), d)).collect()
    }

    fn draw_help(
        &self,
        out: &mut impl Write,
        top: u16,
        w: usize,
        list_h: usize,
        pal: &Palette,
    ) -> io::Result<()> {
        for (n, (key, desc)) in self.help_keys().iter().take(list_h).enumerate() {
            let y = top + n as u16;
            queue!(out, cursor::MoveTo(2, y), SetForegroundColor(pal.accent))?;
            out.write_all(format!("{key:>8}").as_bytes())?;
            queue!(out, SetForegroundColor(pal.dim))?;
            out.write_all(truncate(&format!("  {desc}"), w.saturating_sub(12)).as_bytes())?;
        }
        Ok(())
    }

    fn draw_footer(
        &self,
        out: &mut impl Write,
        w: usize,
        h: usize,
        pal: &Palette,
        prompt: Option<&Prompt>,
    ) -> io::Result<()> {
        let y = (h - 1) as u16;
        queue!(out, cursor::MoveTo(1, y), SetBackgroundColor(pal.bg))?;
        if let Some(p) = prompt {
            queue!(out, SetForegroundColor(pal.accent))?;
            out.write_all(p.prefix.as_bytes())?;
            queue!(out, SetForegroundColor(pal.fg))?;
            let text: String = p.buf.iter().collect();
            out.write_all(text.as_bytes())?;
            let x = 1 + p.prefix.chars().count() + p.cur;
            queue!(out, cursor::MoveTo(x.min(w - 1) as u16, y), cursor::Show)?;
            return Ok(());
        }
        let text = match (&self.status, self.view) {
            (Some(s), _) => s.clone(),
            (None, View::List) => {
                "j/k move · space done · o new · t due · a file · h history · ? help".into()
            }
            (None, View::Projects) => {
                "j/k move · enter open log · o new · e rename · 1 todos · ? help".into()
            }
            (None, View::History) => "j/k move · a restore · h back · ? help".into(),
        };
        queue!(
            out,
            SetForegroundColor(if self.status.is_some() {
                pal.accent
            } else {
                pal.dim
            })
        )?;
        out.write_all(truncate(&text, w.saturating_sub(2)).as_bytes())?;
        Ok(())
    }

    // ---- single-line editor ---------------------------------------------

    fn prompt(
        &mut self,
        out: &mut impl Write,
        prefix: &str,
        initial: &str,
    ) -> io::Result<Option<String>> {
        let mut p = Prompt {
            prefix: prefix.into(),
            buf: initial.chars().collect(),
            cur: initial.chars().count(),
        };
        loop {
            self.draw(out, Some(&p))?;
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Enter => return Ok(Some(p.buf.iter().collect())),
                KeyCode::Esc => return Ok(None),
                KeyCode::Char('c') if ctrl => return Ok(None),
                KeyCode::Char('u') if ctrl => {
                    p.buf.clear();
                    p.cur = 0;
                }
                KeyCode::Char('w') if ctrl => p.delete_word(),
                KeyCode::Char('a') if ctrl => p.cur = 0,
                KeyCode::Char('e') if ctrl => p.cur = p.buf.len(),
                KeyCode::Char(c) => {
                    p.buf.insert(p.cur, c);
                    p.cur += 1;
                }
                KeyCode::Backspace if p.cur > 0 => {
                    p.cur -= 1;
                    p.buf.remove(p.cur);
                }
                KeyCode::Delete if p.cur < p.buf.len() => {
                    p.buf.remove(p.cur);
                }
                KeyCode::Left => p.cur = p.cur.saturating_sub(1),
                KeyCode::Right => p.cur = (p.cur + 1).min(p.buf.len()),
                KeyCode::Home => p.cur = 0,
                KeyCode::End => p.cur = p.buf.len(),
                _ => {}
            }
        }
    }
}

struct Prompt {
    prefix: String,
    buf: Vec<char>,
    cur: usize,
}

impl Prompt {
    fn delete_word(&mut self) {
        let mut i = self.cur;
        while i > 0 && self.buf[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !self.buf[i - 1].is_whitespace() {
            i -= 1;
        }
        self.buf.drain(i..self.cur);
        self.cur = i;
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Greedy word wrap that also honours any newlines already in the text.
fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let wlen = word.chars().count();
            if line.is_empty() {
                line.push_str(word);
            } else if line.chars().count() + 1 + wlen <= width {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(std::mem::take(&mut line));
                line.push_str(word);
            }
            // A single word longer than the viewport gets hard-broken.
            while line.chars().count() > width {
                let head: String = line.chars().take(width).collect();
                let tail: String = line.chars().skip(width).collect();
                out.push(head);
                line = tail;
            }
        }
        out.push(line);
    }
    out
}

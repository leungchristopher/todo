use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate};

use crate::config::Sort;

#[derive(Debug, Clone)]
pub struct Todo {
    /// Session-local handle, used to keep the selection on the same todo
    /// across re-sorts. Not persisted.
    pub id: u64,
    pub done: bool,
    pub created: DateTime<Local>,
    pub due: Option<NaiveDate>,
    pub title: String,
    pub details: String,
    /// When the box was ticked. Cleared if it gets unticked.
    pub completed: Option<DateTime<Local>>,
    /// Set once the todo has been swept or sent to history.
    pub archived: Option<DateTime<Local>>,
    /// UI state. Not persisted: every todo starts collapsed.
    pub expanded: bool,
}

impl Todo {
    pub fn new(id: u64, title: String) -> Self {
        Todo {
            id,
            done: false,
            created: Local::now(),
            due: None,
            title,
            details: String::new(),
            completed: None,
            archived: None,
            expanded: false,
        }
    }

    /// The moment this todo earned its place in history — when it was ticked
    /// off if it ever was, otherwise when it was filed away by hand.
    pub fn filed_at(&self) -> Option<DateTime<Local>> {
        self.completed.or(self.archived)
    }

    /// Fields are appended, never reordered: a file written by an older
    /// version is missing the trailing two and still loads.
    fn encode(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            if self.done { 1 } else { 0 },
            self.created.to_rfc3339(),
            self.due
                .map(|d| d.to_string())
                .unwrap_or_else(|| "-".into()),
            escape(&self.title),
            escape(&self.details),
            stamp(self.completed),
            stamp(self.archived),
        )
    }

    fn decode(line: &str, id: u64) -> Option<Todo> {
        let mut f = line.splitn(7, '\t');
        let done = f.next()? == "1";
        let created = DateTime::parse_from_rfc3339(f.next()?)
            .ok()?
            .with_timezone(&Local);
        let due = match f.next()? {
            "-" => None,
            s => Some(s.parse().ok()?),
        };
        let title = unescape(f.next()?);
        let details = unescape(f.next().unwrap_or(""));
        let completed = f.next().and_then(parse_stamp);
        let archived = f.next().and_then(parse_stamp);
        Some(Todo {
            id,
            done,
            created,
            due,
            title,
            details,
            completed,
            archived,
            expanded: false,
        })
    }
}

fn stamp(t: Option<DateTime<Local>>) -> String {
    t.map(|t| t.to_rfc3339()).unwrap_or_else(|| "-".into())
}

fn parse_stamp(s: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Local))
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

pub struct Store {
    /// The live list.
    pub todos: Vec<Todo>,
    /// Everything filed away, most recent first.
    pub history: Vec<Todo>,
    path: PathBuf,
    next_id: u64,
}

impl Store {
    pub fn load(path: PathBuf) -> io::Result<Store> {
        let (mut todos, mut history) = (Vec::new(), Vec::new());
        let mut next_id = 0;
        if let Ok(text) = fs::read_to_string(&path) {
            for line in text.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                // A malformed line is skipped rather than fatal: one bad row
                // should never cost the user the rest of the list.
                if let Some(todo) = Todo::decode(line, next_id) {
                    if todo.archived.is_some() {
                        history.push(todo);
                    } else {
                        todos.push(todo);
                    }
                    next_id += 1;
                }
            }
        }
        let mut store = Store {
            todos,
            history,
            path,
            next_id,
        };
        store.sort_history();
        Ok(store)
    }

    pub fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Writes via a temp file + rename so an interrupted save cannot leave a
    /// half-written list on disk.
    pub fn save(&self) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut body = String::new();
        for t in self.todos.iter().chain(self.history.iter()) {
            body.push_str(&t.encode());
            body.push('\n');
        }
        let tmp = tmp_path(&self.path);
        fs::write(&tmp, body)?;
        fs::rename(&tmp, &self.path)
    }

    /// Files one todo away, keeping the date it was actually finished rather
    /// than the date it happened to be swept.
    pub fn archive(&mut self, index: usize) -> Option<u64> {
        if index >= self.todos.len() {
            return None;
        }
        let mut t = self.todos.remove(index);
        t.archived = Some(t.completed.unwrap_or_else(Local::now));
        t.expanded = false;
        let id = t.id;
        self.history.push(t);
        self.sort_history();
        Some(id)
    }

    /// Brings a todo back to the live list, unticked: restoring means picking
    /// the work up again, and it also stops the next sweep from immediately
    /// filing it right back.
    pub fn restore(&mut self, index: usize, sort: Sort) -> Option<u64> {
        if index >= self.history.len() {
            return None;
        }
        let mut t = self.history.remove(index);
        t.archived = None;
        t.completed = None;
        t.done = false;
        t.expanded = false;
        let id = t.id;
        self.todos.push(t);
        self.sort(sort);
        Some(id)
    }

    /// Sweeps every ticked todo into history. Returns how many moved.
    pub fn sweep(&mut self) -> usize {
        let mut n = 0;
        for i in (0..self.todos.len()).rev() {
            if self.todos[i].done {
                self.archive(i);
                n += 1;
            }
        }
        n
    }

    pub fn sort(&mut self, sort: Sort) {
        match sort {
            Sort::Created => self.todos.sort_by_key(|t| t.created),
            // Undated todos sink to the bottom; ties fall back to creation
            // order so the list never reshuffles arbitrarily.
            Sort::Due => self.todos.sort_by(|a, b| match (a.due, b.due) {
                (Some(x), Some(y)) => x.cmp(&y).then(a.created.cmp(&b.created)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.created.cmp(&b.created),
            }),
            Sort::Alpha => self.todos.sort_by_key(|t| t.title.to_lowercase()),
        }
    }

    /// Puts a todo back into history in its rightful date position.
    pub fn push_history(&mut self, t: Todo) {
        self.history.push(t);
        self.sort_history();
    }

    /// History reads newest first, regardless of the list's sort setting.
    fn sort_history(&mut self) {
        self.history
            .sort_by_key(|t| std::cmp::Reverse(t.filed_at().unwrap_or(t.created)));
    }

    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.todos.iter().position(|t| t.id == id)
    }

    pub fn history_index_of(&self, id: u64) -> Option<usize> {
        self.history.iter().position(|t| t.id == id)
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store {
            todos: Vec::new(),
            history: Vec::new(),
            path: PathBuf::from("/dev/null"),
            next_id: 0,
        }
    }

    #[test]
    fn escape_round_trips_awkward_text() {
        for s in [
            "plain",
            "tab\there",
            "two\nlines",
            "back\\slash",
            "\\t literal",
            "",
        ] {
            assert_eq!(unescape(&escape(s)), s);
        }
    }

    #[test]
    fn todo_round_trips_through_a_line() {
        let mut t = Todo::new(0, "Buy milk".into());
        t.details = "oat milk\tif they have it\nelse skip".into();
        t.due = NaiveDate::from_ymd_opt(2026, 12, 25);
        t.done = true;
        t.completed = Some(Local::now());
        t.archived = Some(Local::now());

        let line = t.encode();
        assert_eq!(
            line.matches('\t').count(),
            6,
            "fields must stay tab-delimited"
        );

        let back = Todo::decode(&line, 0).expect("should decode");
        assert_eq!(back.title, t.title);
        assert_eq!(back.details, t.details);
        assert_eq!(back.due, t.due);
        assert_eq!(back.done, t.done);
        assert_eq!(back.created.timestamp(), t.created.timestamp());
        assert_eq!(
            back.completed.unwrap().timestamp(),
            t.completed.unwrap().timestamp()
        );
        assert_eq!(
            back.archived.unwrap().timestamp(),
            t.archived.unwrap().timestamp()
        );
    }

    #[test]
    fn reads_files_written_before_history_existed() {
        let old = "1\t2026-07-16T11:00:00-07:00\t2026-12-25\tBuy milk\tsome details";
        let t = Todo::decode(old, 0).expect("old 5-field line should still load");
        assert_eq!(t.title, "Buy milk");
        assert_eq!(t.details, "some details");
        assert!(t.done);
        assert_eq!(t.completed, None);
        assert_eq!(t.archived, None, "old todos must land in the live list");
    }

    #[test]
    fn decode_rejects_junk_without_panicking() {
        for line in ["", "garbage", "1\tnot-a-date\t-\ttitle\t", "1"] {
            assert!(
                Todo::decode(line, 0).is_none(),
                "{line:?} should not decode"
            );
        }
    }

    #[test]
    fn sweep_moves_only_ticked_todos() {
        let mut s = store();
        for (i, title) in ["alpha", "beta", "gamma"].iter().enumerate() {
            s.todos.push(Todo::new(i as u64, (*title).into()));
        }
        s.todos[1].done = true;
        s.todos[1].completed = Some(Local::now());

        assert_eq!(s.sweep(), 1);
        assert_eq!(s.todos.len(), 2);
        assert_eq!(s.history.len(), 1);
        assert_eq!(s.history[0].title, "beta");
        assert!(s.history[0].archived.is_some());
        assert_eq!(
            s.todos.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["alpha", "gamma"],
            "surviving todos keep their order"
        );
    }

    #[test]
    fn sweep_is_a_no_op_when_nothing_is_ticked() {
        let mut s = store();
        s.todos.push(Todo::new(0, "alpha".into()));
        assert_eq!(s.sweep(), 0);
        assert_eq!(s.todos.len(), 1);
        assert!(s.history.is_empty());
    }

    #[test]
    fn archive_keeps_the_completion_date_not_the_sweep_date() {
        let mut s = store();
        let mut t = Todo::new(0, "finished yesterday".into());
        let yesterday = Local::now() - chrono::Duration::days(1);
        t.done = true;
        t.completed = Some(yesterday);
        s.todos.push(t);

        s.sweep();
        assert_eq!(
            s.history[0].archived.unwrap().timestamp(),
            yesterday.timestamp()
        );
    }

    #[test]
    fn manual_archive_files_an_unticked_todo() {
        let mut s = store();
        s.todos.push(Todo::new(0, "not doing this".into()));
        assert_eq!(s.archive(0), Some(0));
        assert_eq!(s.history.len(), 1);
        assert!(
            s.history[0].filed_at().is_some(),
            "must be datable in history"
        );
    }

    #[test]
    fn restore_brings_it_back_unticked_so_the_next_sweep_leaves_it_alone() {
        let mut s = store();
        let mut t = Todo::new(0, "again".into());
        t.done = true;
        t.completed = Some(Local::now());
        s.todos.push(t);
        s.sweep();

        assert_eq!(s.restore(0, Sort::Created), Some(0));
        assert!(s.history.is_empty());
        assert_eq!(s.todos.len(), 1);
        assert!(!s.todos[0].done);
        assert_eq!(s.todos[0].completed, None);
        assert_eq!(
            s.sweep(),
            0,
            "restored todo must not be swept straight back"
        );
    }

    #[test]
    fn history_reads_newest_first() {
        let mut s = store();
        for (i, days) in [3i64, 1, 2].iter().enumerate() {
            let mut t = Todo::new(i as u64, format!("{days} days ago"));
            t.done = true;
            t.completed = Some(Local::now() - chrono::Duration::days(*days));
            s.todos.push(t);
        }
        s.sweep();
        assert_eq!(
            s.history
                .iter()
                .map(|t| t.title.as_str())
                .collect::<Vec<_>>(),
            ["1 days ago", "2 days ago", "3 days ago"]
        );
    }
}

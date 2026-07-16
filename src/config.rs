use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    pub fn toggle(self) -> Theme {
        match self {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Created,
    Due,
    Alpha,
}

impl Sort {
    pub fn next(self) -> Sort {
        match self {
            Sort::Created => Sort::Due,
            Sort::Due => Sort::Alpha,
            Sort::Alpha => Sort::Created,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Sort::Created => "created",
            Sort::Due => "due",
            Sort::Alpha => "alpha",
        }
    }
}

pub const DEFAULT_PHRASE: &str = "morning";

#[derive(Debug, Clone)]
pub struct Config {
    pub theme: Theme,
    pub sort: Sort,
    /// Typed after `:` to sweep the ticked todos into history.
    pub phrase: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            theme: Theme::Dark,
            sort: Sort::Created,
            phrase: DEFAULT_PHRASE.into(),
        }
    }
}

impl Config {
    pub fn load() -> Config {
        let mut cfg = Config::default();
        let Ok(text) = fs::read_to_string(config_path()) else {
            return cfg;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match (key.trim(), value.trim()) {
                ("theme", "light") => cfg.theme = Theme::Light,
                ("theme", "dark") => cfg.theme = Theme::Dark,
                ("sort", "created") => cfg.sort = Sort::Created,
                ("sort", "due") => cfg.sort = Sort::Due,
                ("sort", "alpha") => cfg.sort = Sort::Alpha,
                // An empty phrase would make a bare `:` sweep the list, so
                // fall back to the default rather than arm a hair trigger.
                ("phrase", p) if !p.is_empty() => cfg.phrase = p.into(),
                _ => {}
            }
        }
        cfg
    }

    pub fn save(&self) -> io::Result<()> {
        let path = config_path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(
            path,
            format!(
                "theme={}\nsort={}\nphrase={}\n",
                self.theme.name(),
                self.sort.name(),
                self.phrase
            ),
        )
    }
}

fn home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_path() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("td")
        .join("config")
}

pub fn data_path() -> PathBuf {
    if let Some(p) = env::var_os("TD_FILE") {
        return PathBuf::from(p);
    }
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local").join("share"))
        .join("td")
        .join("todos.txt")
}

/// Projects sit next to the todos, so pointing TD_FILE at a second list gets
/// you that list's projects too.
pub fn projects_path() -> PathBuf {
    data_path().with_file_name("projects.txt")
}

mod config;
mod date;
mod model;
mod ui;

use std::io::{self, Write};
use std::process::ExitCode;

use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute, style};

use config::Config;
use model::Store;
use ui::App;

const USAGE: &str = "\
td — a minimalist terminal todo list

usage:
    td            open the list
    td -h         show this help
    td -v         show version

files:
    todos    $XDG_DATA_HOME/td/todos.txt   (default ~/.local/share/td/todos.txt)
    config   $XDG_CONFIG_HOME/td/config    (default ~/.config/td/config)

    Set TD_FILE to point at a different todo file.

config:
    theme=dark|light          toggle in-app with m
    sort=created|due|alpha    cycle in-app with s
    phrase=morning            type :morning to sweep ticked todos to history

    Press ? in the app for the full key list.
";

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("-h") | Some("--help") => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some("-v") | Some("--version") => {
            println!("td {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some(arg) => {
            eprintln!("td: unknown argument '{arg}' (try td -h)");
            return ExitCode::FAILURE;
        }
    }

    if let Err(e) = run() {
        eprintln!("td: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run() -> io::Result<()> {
    let cfg = Config::load();
    let store = Store::load(config::data_path())?;
    let mut app = App::new(store, cfg);

    let mut out = io::stdout();
    terminal::enable_raw_mode()?;
    execute!(out, EnterAlternateScreen, cursor::Hide)?;

    let result = app.run(&mut out);

    // Restore the terminal even if the app loop failed, so a crash can never
    // leave the user staring at a raw-mode shell.
    let _ = execute!(out, style::ResetColor, cursor::Show, LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
    let _ = out.flush();

    result
}

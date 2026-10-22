//! ratl — declarative terminal dialogs powered by ratatui.

mod app;
mod model;
mod ui;

use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use serde_json::json;

use app::{App, KeyAction, Outcome, OutcomeAction};
use model::{Dialog, control_count};

const DEFAULT_FILE: &str = "ratl.json";

#[derive(Debug, Parser)]
#[command(
    name = "ratl",
    version,
    about = "Declarative terminal dialogs powered by ratatui",
    long_about = "Read a JSON dialog description, display it in the terminal, and print the result as JSON.\n\nIf FILE is omitted, ratl looks for ./ratl.json, then reads JSON from stdin when stdin is not a terminal."
)]
struct Cli {
    /// JSON dialog file to load ("-" reads stdin)
    file: Option<PathBuf>,

    /// Inline JSON dialog document
    #[arg(long, value_name = "JSON", conflicts_with = "file")]
    json: Option<String>,

    /// Validate the document and report a summary without opening the TUI
    #[arg(long)]
    validate: bool,

    /// Print the JSON schema for dialog files
    #[arg(long)]
    schema: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("ratl: {error:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();

    if cli.schema {
        println!("{}", include_str!("../schema.json"));
        return Ok(ExitCode::SUCCESS);
    }

    let dialog = load_dialog(&cli)?;
    dialog.validate().map_err(anyhow::Error::msg)?;

    if cli.validate {
        let summary = json!({
            "valid": true,
            "title": dialog.title,
            "schema": dialog.schema,
            "controls": control_count(&dialog.children),
            "buttons": dialog.buttons.len(),
        });
        println!("{summary}");
        return Ok(ExitCode::SUCCESS);
    }

    let outcome = run_tui(dialog)?;
    println!("{}", serde_json::to_string(&outcome.to_json())?);
    Ok(match outcome.action {
        OutcomeAction::Submit => ExitCode::SUCCESS,
        OutcomeAction::Cancel => ExitCode::from(1),
    })
}

fn load_dialog(cli: &Cli) -> Result<Dialog> {
    let raw = if let Some(json) = &cli.json {
        json.clone()
    } else if let Some(file) = &cli.file {
        read_json_file(file)?
    } else if Path::new(DEFAULT_FILE).is_file() {
        read_json_file(Path::new(DEFAULT_FILE))?
    } else if !io::stdin().is_terminal() {
        let mut buffer = String::new();
        io::stdin()
            .read_to_string(&mut buffer)
            .context("failed to read dialog JSON from stdin")?;
        buffer
    } else {
        bail!("no dialog file given; pass a JSON file, use --json, or create {DEFAULT_FILE}");
    };

    serde_json::from_str(&raw).context("invalid dialog JSON")
}

fn read_json_file(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut buffer = String::new();
        io::stdin()
            .read_to_string(&mut buffer)
            .context("failed to read dialog JSON from stdin")?;
        return Ok(buffer);
    }
    std::fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

fn run_tui(dialog: Dialog) -> Result<Outcome> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("ratl needs an interactive terminal (use --validate for non-interactive checks)");
    }

    let mut app = App::new(dialog).map_err(anyhow::Error::msg)?;
    let mut session = TerminalSession::new()?;

    loop {
        session.terminal.draw(|frame| ui::draw(frame, &mut app))?;
        match event::read().context("failed to read terminal input")? {
            Event::Key(key) if key.kind != KeyEventKind::Release => match app.handle_key(key) {
                KeyAction::Continue => {}
                KeyAction::Finish(outcome) => return Ok(outcome),
            },
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TerminalSession {
    fn new() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw terminal mode")?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error).context("failed to enter the alternate screen");
        }
        match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen);
                Err(error).context("failed to initialize terminal")
            }
        }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = self.terminal.show_cursor();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = io::stdout().flush();
    }
}

//! ratl — declarative terminal dialogs powered by ratatui.

mod app;
mod model;
mod ui;

use std::fs::OpenOptions;
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
use serde_json::{Value, json};

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
    if let Some(json) = &cli.json {
        let mut dialog = parse_dialog(json)?;
        dialog.base_dir = std::env::current_dir().ok();
        return Ok(dialog);
    }

    if let Some(file) = &cli.file {
        let raw = read_json_file(file)?;
        let mut dialog = parse_dialog(&raw)?;
        dialog.base_dir = dialog_base_dir(file);
        return Ok(dialog);
    }

    let default_file = Path::new(DEFAULT_FILE);
    if default_file.is_file() {
        let raw = read_json_file(default_file)?;
        let mut dialog = parse_dialog(&raw)?;
        dialog.base_dir = dialog_base_dir(default_file);
        return Ok(dialog);
    }

    if !io::stdin().is_terminal() {
        let mut buffer = String::new();
        io::stdin()
            .read_to_string(&mut buffer)
            .context("failed to read dialog JSON from stdin")?;
        let mut dialog = parse_dialog(&buffer)?;
        dialog.base_dir = std::env::current_dir().ok();
        return Ok(dialog);
    }

    bail!("no dialog file given; pass a JSON file, use --json, or create {DEFAULT_FILE}");
}

fn dialog_base_dir(path: &Path) -> Option<PathBuf> {
    if path == Path::new("-") {
        return std::env::current_dir().ok();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    Some(if parent.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        parent.to_path_buf()
    })
}

fn parse_dialog(raw: &str) -> Result<Dialog> {
    match serde_json::from_str::<Dialog>(raw) {
        Ok(dialog) => Ok(dialog),
        Err(error) => {
            if let Ok(value) = serde_json::from_str::<Value>(raw)
                && looks_like_schema(&value)
            {
                bail!(
                    "this looks like a JSON Schema, not a ratl dialog; \
                     run `ratl --schema` to print the schema, or pass a dialog file such as `ratl.json`"
                );
            }
            Err(error).context("invalid dialog JSON")
        }
    }
}

fn looks_like_schema(value: &Value) -> bool {
    let object = match value.as_object() {
        Some(object) => object,
        None => return false,
    };
    if object.contains_key("$defs") || object.contains_key("$id") {
        return true;
    }
    object.get("type").and_then(Value::as_str) == Some("object")
        && object.contains_key("properties")
        && !object.contains_key("children")
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
    if !io::stdin().is_terminal() {
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

// The UI normally goes to stdout. When stdout is redirected or captured (for
// example `result=$(ratl ...)`), draw on the controlling terminal instead so
// the JSON result can still be written to stdout. stdin must stay interactive
// because that is where the key events come from.
fn terminal_output() -> Result<Box<dyn Write>> {
    if io::stdout().is_terminal() {
        return Ok(Box::new(io::stdout()));
    }
    let tty = OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .context("ratl needs an interactive terminal (stdout is not a terminal and /dev/tty cannot be opened)")?;
    Ok(Box::new(tty))
}

fn leave_alternate_screen() {
    if let Ok(mut output) = terminal_output() {
        let _ = execute!(output, LeaveAlternateScreen);
        let _ = output.flush();
    }
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Box<dyn Write>>>,
}

impl TerminalSession {
    fn new() -> Result<Self> {
        let mut output = terminal_output()?;
        enable_raw_mode().context("failed to enable raw terminal mode")?;
        if let Err(error) = execute!(output, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error).context("failed to enter the alternate screen");
        }
        match Terminal::new(CrosstermBackend::new(output)) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                let _ = disable_raw_mode();
                leave_alternate_screen();
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
        let _ = self.terminal.flush();
    }
}

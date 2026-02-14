#[cfg(unix)]
use libc::{kill, setsid, SIGTERM};
use std::fs;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{self, disable_raw_mode, enable_raw_mode};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use serde::{Deserialize, Serialize};

use success_core::app::AppState;
use success_core::key_event::{AppKeyCode, AppKeyEvent};
use success_core::types::Mode;
use success_core::ui;

#[cfg(target_os = "macos")]
const FILE_MANAGER_COMMAND: &str = "open";
#[cfg(target_os = "windows")]
const FILE_MANAGER_COMMAND: &str = "explorer";
#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
const FILE_MANAGER_COMMAND: &str = "xdg-open";

const DEFAULT_EDITOR: &str = "nvim";

// ── Key event conversion ─────────────────────────────────────────────────

fn convert_key(key: crossterm::event::KeyEvent) -> AppKeyEvent {
    let code = match key.code {
        KeyCode::Char(c) => AppKeyCode::Char(c),
        KeyCode::Backspace => AppKeyCode::Backspace,
        KeyCode::Enter => AppKeyCode::Enter,
        KeyCode::Left => AppKeyCode::Left,
        KeyCode::Right => AppKeyCode::Right,
        KeyCode::Up => AppKeyCode::Up,
        KeyCode::Down => AppKeyCode::Down,
        KeyCode::Tab => AppKeyCode::Tab,
        KeyCode::BackTab => AppKeyCode::BackTab,
        KeyCode::Delete => AppKeyCode::Delete,
        KeyCode::Home => AppKeyCode::Home,
        KeyCode::End => AppKeyCode::End,
        KeyCode::Esc => AppKeyCode::Esc,
        _ => AppKeyCode::Other,
    };
    AppKeyEvent {
        code,
        ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
        alt: key.modifiers.contains(KeyModifiers::ALT),
        shift: key.modifiers.contains(KeyModifiers::SHIFT),
    }
}

// ── CLI-specific: spawned commands ───────────────────────────────────────

struct SpawnedCommand {
    command: String,
    child: Option<Child>,
    pgid: i32,
}

fn commands_for_goal(state: &AppState, goal_id: u64) -> Vec<String> {
    state
        .goals
        .iter()
        .find(|g| g.id == goal_id)
        .map(|g| g.commands.clone())
        .unwrap_or_default()
}

#[cfg(unix)]
fn spawn_commands(commands: &[String]) -> Vec<SpawnedCommand> {
    commands
        .iter()
        .map(|cmd| {
            let child = {
                let mut command = Command::new("sh");
                command
                    .arg("-c")
                    .arg(format!("exec {cmd}"))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                unsafe {
                    command.pre_exec(|| {
                        setsid();
                        Ok(())
                    });
                }
                command.spawn()
            };
            match child {
                Ok(child) => {
                    let pgid = child.id() as i32;
                    SpawnedCommand {
                        command: cmd.clone(),
                        child: Some(child),
                        pgid,
                    }
                }
                Err(err) => {
                    eprintln!("Failed to start '{cmd}': {err}");
                    SpawnedCommand {
                        command: cmd.clone(),
                        child: None,
                        pgid: 0,
                    }
                }
            }
        })
        .collect()
}

#[cfg(not(unix))]
fn spawn_commands(commands: &[String]) -> Vec<SpawnedCommand> {
    commands
        .iter()
        .map(|cmd| {
            let child = if cfg!(target_os = "windows") {
                Command::new("cmd")
                    .arg("/C")
                    .arg(cmd)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
            } else {
                Command::new("sh")
                    .arg("-c")
                    .arg(format!("exec {cmd}"))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
            };
            match child {
                Ok(child) => SpawnedCommand {
                    command: cmd.clone(),
                    child: Some(child),
                    pgid: 0,
                },
                Err(err) => {
                    eprintln!("Failed to start '{cmd}': {err}");
                    SpawnedCommand {
                        command: cmd.clone(),
                        child: None,
                        pgid: 0,
                    }
                }
            }
        })
        .collect()
}

#[cfg(unix)]
fn kill_spawned(spawned: &mut [SpawnedCommand]) {
    for sc in spawned.iter_mut() {
        if let Some(child) = sc.child.as_mut() {
            let pid = child.id();
            unsafe {
                if sc.pgid != 0 {
                    kill(-sc.pgid, SIGTERM);
                } else {
                    kill(-(pid as i32), SIGTERM);
                }
            }
            std::thread::sleep(Duration::from_millis(150));
            unsafe {
                if sc.pgid != 0 {
                    kill(-sc.pgid, libc::SIGKILL);
                } else {
                    kill(-(pid as i32), libc::SIGKILL);
                }
            }
            if let Err(err) = child.kill() {
                if err.kind() != io::ErrorKind::InvalidInput {
                    eprintln!("Failed to kill '{}': {err}", sc.command);
                }
            }
            let _ = child.wait();
            let _ = Command::new("pkill")
                .arg("-f")
                .arg(&sc.command)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

#[cfg(not(unix))]
fn kill_spawned(spawned: &mut [SpawnedCommand]) {
    for sc in spawned.iter_mut() {
        if let Some(mut child) = sc.child.take() {
            if let Err(err) = child.kill() {
                if err.kind() != io::ErrorKind::InvalidInput {
                    eprintln!("Failed to kill '{}': {err}", sc.command);
                }
            }
            let _ = child.wait();
        }
    }
}

// ── CLI-specific: external editor, file manager ──────────────────────────

fn open_archive_in_file_manager(archive: &Path) -> Result<()> {
    if !archive.exists() {
        fs::create_dir_all(archive)?;
    }
    Command::new(FILE_MANAGER_COMMAND)
        .arg(archive)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to start {FILE_MANAGER_COMMAND}"))?;
    Ok(())
}

fn open_notes_in_external_editor(state: &mut AppState, archive: &Path) -> Result<()> {
    let goal_id = success_core::utils::selected_goal_id(state)
        .context("No goal selected for note editing")?;
    success_core::notes::refresh_notes_for_selection(state);
    success_core::notes::save_notes_for_selection(state);

    let note_path = archive.join("notes").join(format!("goal_{goal_id}.md"));

    let editor_value = std::env::var("EDITOR").unwrap_or_else(|_| DEFAULT_EDITOR.to_string());
    let mut editor_parts = parse_editor_command(editor_value.trim());
    if editor_parts.is_empty() {
        editor_parts.push(DEFAULT_EDITOR.to_string());
    }
    let editor_bin = editor_parts.remove(0);
    let editor_args = editor_parts;

    let note_for_run = note_path.clone();
    with_terminal_suspended(move || {
        println!("Opening notes with {} (goal {})...", editor_bin, goal_id);
        io::stdout().flush().ok();

        let mut command = Command::new(&editor_bin);
        for arg in &editor_args {
            command.arg(arg);
        }
        command.arg(&note_for_run);
        command.stdin(Stdio::inherit());
        command.stdout(Stdio::inherit());
        command.stderr(Stdio::inherit());
        let status = command
            .status()
            .with_context(|| format!("Failed to run {editor_bin}"))?;
        if !status.success() {
            bail!("Editor exited with status {status}");
        }
        Ok(())
    })?;

    success_core::notes::refresh_notes_for_selection(state);
    Ok(())
}

fn with_terminal_suspended<F>(action: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    disable_raw_mode()?;
    {
        let mut stdout = io::stdout();
        execute!(
            stdout,
            terminal::LeaveAlternateScreen,
            event::DisableMouseCapture
        )?;
    }

    let result = action();

    {
        let mut stdout = io::stdout();
        execute!(
            stdout,
            terminal::EnterAlternateScreen,
            event::EnableMouseCapture,
            SetCursorStyle::SteadyBlock,
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
            crossterm::cursor::MoveTo(0, 0)
        )?;
    }
    enable_raw_mode()?;

    result
}

fn parse_editor_command(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            '\\' if !in_single => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}

// ── Config ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CliConfig {
    archive: Option<PathBuf>,
}

fn persist_config(archive: &Path) -> Result<()> {
    let path = config_path()?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let cfg = CliConfig {
        archive: Some(archive.to_path_buf()),
    };
    let content = serde_json::to_string_pretty(&cfg)?;
    fs::write(&path, content)?;
    Ok(())
}

fn resolve_archive_interactive(preferred: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = preferred {
        return Ok(path);
    }
    let cfg_path = config_path()?;
    if cfg_path.exists() {
        if let Ok(content) = fs::read_to_string(&cfg_path) {
            if let Ok(cfg) = serde_json::from_str::<CliConfig>(&content) {
                if let Some(p) = cfg.archive {
                    return Ok(p);
                }
            }
        }
    }
    println!("Archive folder not set. Enter a path to use (will be created if missing):");
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        bail!("Archive folder not provided");
    }
    let path = PathBuf::from(trimmed);
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn config_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set; please set HOME")?;
    Ok(Path::new(&home).join(".config/success-cli/config.json"))
}

// ── Main ─────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "success-cli")]
#[command(about = "CLI for achieving goals", long_about = None)]
struct Args {
    /// Custom archive path (useful for testing)
    #[arg(short, long)]
    archive: Option<PathBuf>,
}

/// CLI-extended state: wraps core state + CLI-only fields
struct CliState {
    app: AppState,
    archive: PathBuf,
    spawned: Vec<SpawnedCommand>,
    needs_full_redraw: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let archive = resolve_archive_interactive(args.archive.clone())?;

    let app = AppState::new(archive.to_string_lossy().to_string());

    let mut cli = CliState {
        app,
        archive: archive.clone(),
        spawned: Vec::new(),
        needs_full_redraw: false,
    };

    if args.archive.is_none() {
        persist_config(&archive).ok();
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        terminal::EnterAlternateScreen,
        event::EnableMouseCapture,
        SetCursorStyle::SteadyBlock
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, &mut cli);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        terminal::LeaveAlternateScreen,
        event::DisableMouseCapture,
        SetCursorStyle::DefaultUserShape
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("Error: {err}");
    }
    Ok(())
}

fn run_app<B: ratatui::backend::Backend<Error: Send + Sync + 'static> + std::io::Write>(
    terminal: &mut Terminal<B>,
    cli: &mut CliState,
) -> Result<()> {
    loop {
        if cli.needs_full_redraw {
            terminal.clear()?;
            cli.needs_full_redraw = false;
        }
        cli.app.tick();

        let header = format!("Archive: {} (open with 'o')", cli.archive.display());
        terminal.draw(|f| ui::ui(f, &cli.app, &header))?;
        execute!(terminal.backend_mut(), get_cursor_style(&cli.app.mode))?;

        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) => {
                    // Handle CLI-specific keys first
                    if handle_cli_key(cli, key)? {
                        break;
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
    kill_spawned(&mut cli.spawned);
    Ok(())
}

/// Handle CLI-specific key actions, then delegate to core.
/// Returns true if the app should quit.
fn handle_cli_key(cli: &mut CliState, key: crossterm::event::KeyEvent) -> Result<bool> {
    // Handle CLI-only keys in View mode
    if matches!(cli.app.mode, Mode::View) {
        match key.code {
            KeyCode::Char('E') => {
                if success_core::utils::selected_goal_id(&cli.app).is_some() {
                    open_notes_in_external_editor(&mut cli.app, &cli.archive)?;
                    cli.needs_full_redraw = true;
                }
                return Ok(false);
            }
            KeyCode::Char('o') => {
                let _ = open_archive_in_file_manager(&cli.archive);
                return Ok(false);
            }
            _ => {}
        }
    }

    // Delegate to core
    let app_key = convert_key(key);
    let quit = cli.app.handle_key(app_key);

    // After core handles a timer start, spawn commands
    if matches!(cli.app.mode, Mode::Timer) {
        if let Some(timer) = &cli.app.timer {
            if cli.spawned.is_empty() {
                let cmds = commands_for_goal(&cli.app, timer.goal_id);
                cli.spawned = spawn_commands(&cmds);
            }
        }
    }

    // When timer finishes and was a reward, kill spawned apps
    if !matches!(cli.app.mode, Mode::Timer) && !cli.spawned.is_empty() {
        kill_spawned(&mut cli.spawned);
    }

    Ok(quit)
}

fn get_cursor_style(mode: &Mode) -> SetCursorStyle {
    match mode {
        Mode::NotesEdit
        | Mode::AddSession
        | Mode::AddReward
        | Mode::GoalForm
        | Mode::QuantityDoneInput { .. }
        | Mode::DurationInput { .. } => SetCursorStyle::SteadyBlock,
        Mode::View | Mode::Timer => SetCursorStyle::SteadyBlock,
    }
}

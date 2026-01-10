use anyhow::{Context, Result, anyhow};
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use edtui::{EditorEventHandler, EditorMode, EditorState, Lines};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use std::{io, time};

mod export;
mod input;
mod ipc;
mod log;
mod model;
mod render;
mod ui;

use crate::log::log_error;
use crate::model::Segment;

fn main() -> Result<()> {
    let debug_mpv = std::env::args().any(|arg| arg == "--debug-mpv");
    let cwd = std::env::current_dir().context("get current dir")?;
    let files = input::discover_mp4s(&cwd)?;
    if files.is_empty() {
        return Err(anyhow!("no .mp4 files found in {cwd:?}"));
    }

    let ipc_path = std::env::temp_dir().join(format!("mpv-ipc-{}.sock", std::process::id()));
    if ipc_path.exists() {
        fs::remove_file(&ipc_path).context("remove stale IPC socket")?;
    }

    let mut child = spawn_mpv(&ipc_path, &files, debug_mpv)?;
    wait_for_socket(&ipc_path, Duration::from_secs(10), &mut child)?;

    let mut terminal = setup_terminal()?;

    let mut pending_in: Option<f64> = None;
    let mut cuts: BTreeMap<String, Vec<Segment>> = BTreeMap::new();
    match export::load_markers_json(&cwd) {
        Ok(Some(export)) => {
            cuts = export::cuts_from_export(&export);
        }
        Ok(None) => {}
        Err(err) => {
            log_error(&format!("failed to load markers.json: {err:#}"));
        }
    }
    let mut speed = ipc::get_f64(&ipc_path, "speed").unwrap_or(1.0);
    let mut volume = ipc::get_f64(&ipc_path, "volume").unwrap_or(100.0);
    let mut show_help = false;
    let mut show_render_prompt = false;
    let mut pending_space: Option<Instant> = None;
    let mut render_request = false;
    let mut render_overlay: Option<ui::RenderOverlay> = None;
    let mut render_done_at: Option<Instant> = None;
    let mut render_rx: Option<mpsc::Receiver<render::RenderEvent>> = None;
    let mut zoom_mode = false;
    let mut zoom_state = ZoomState::default();
    let mut zoom_pause_state: Option<bool> = None;
    let mut editor_active = false;
    let mut editor_state: Option<EditorState> = None;
    let mut editor_handler = EditorEventHandler::default();
    let mut editor_pause_state: Option<bool> = None;
    let mut editor_command: Option<String> = None;
    let mut editor_error: Option<String> = None;
    let files_display: Vec<String> = files
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    draw_ui(DrawContext {
        terminal: &mut terminal,
        files: &files_display,
        ipc_path: &ipc_path,
        pending_in,
        cuts: &cuts,
        speed,
        volume,
        zoom_state,
        zoom_mode,
        show_help,
        show_render_prompt,
        render_overlay: render_overlay.as_ref(),
        editor_active,
        editor_state: editor_state.as_mut(),
        editor_command: editor_command.as_deref(),
        editor_error: editor_error.as_deref(),
    })?;

    let tick_rate = time::Duration::from_millis(100);
    let combo_window = Duration::from_millis(500);
    loop {
        if let Some(_status) = child.try_wait().context("check mpv status")? {
            break;
        }
        if editor_active {
            if event::poll(tick_rate).context("poll key events")? {
                let event = event::read().context("read key event")?;
                match event {
                    Event::Key(key) => {
                        let mut editor_ctx = EditorContext {
                            cuts: &mut cuts,
                            editor_active: &mut editor_active,
                            editor_state: &mut editor_state,
                            editor_pause_state: &mut editor_pause_state,
                            editor_command: &mut editor_command,
                            editor_error: &mut editor_error,
                        };
                        if handle_editor_key(key, &cwd, &ipc_path, &mut editor_ctx)?
                            && let Some(state) = editor_state.as_mut()
                        {
                            editor_handler.on_event(Event::Key(key), state);
                        }
                    }
                    _ => {
                        if let Some(state) = editor_state.as_mut() {
                            editor_handler.on_event(event, state);
                        }
                    }
                }
            }

            draw_ui(DrawContext {
                terminal: &mut terminal,
                files: &files_display,
                ipc_path: &ipc_path,
                pending_in,
                cuts: &cuts,
                speed,
                volume,
                zoom_state,
                zoom_mode,
                show_help,
                show_render_prompt,
                render_overlay: render_overlay.as_ref(),
                editor_active,
                editor_state: editor_state.as_mut(),
                editor_command: editor_command.as_deref(),
                editor_error: editor_error.as_deref(),
            })?;
            continue;
        }
        if let Some(start) = pending_space
            && start.elapsed() > combo_window
        {
            if let Err(err) = ipc::cycle_pause(&ipc_path) {
                log_error(&format!("pause toggle failed: {err:#}"));
            }
            pending_space = None;
        }
        if event::poll(tick_rate).context("poll key events")?
            && let Event::Key(key) = event::read().context("read key event")?
        {
            let mut key_ctx = KeyContext {
                pending_in: &mut pending_in,
                cuts: &mut cuts,
                speed: &mut speed,
                volume: &mut volume,
                show_help: &mut show_help,
                pending_space: &mut pending_space,
                show_render_prompt: &mut show_render_prompt,
                render_request: &mut render_request,
                zoom_mode: &mut zoom_mode,
                zoom_state: &mut zoom_state,
                zoom_pause_state: &mut zoom_pause_state,
                editor_active: &mut editor_active,
                editor_state: &mut editor_state,
                editor_pause_state: &mut editor_pause_state,
            };
            if handle_key(
                key,
                &cwd,
                &ipc_path,
                files.last().map(|p| p.to_path_buf()),
                render_rx.is_some(),
                &mut key_ctx,
            )? {
                break;
            }
        }

        if render_request {
            render_request = false;
            show_render_prompt = false;
            render_done_at = None;
            render_overlay = Some(ui::RenderOverlay {
                title: "Rendering".to_string(),
                lines: vec!["Starting...".to_string()],
            });
            let (tx, rx) = mpsc::channel();
            render_rx = Some(rx);
            let cwd_clone = cwd.clone();
            let files_clone = files.clone();
            let cuts_clone = cuts.clone();
            thread::spawn(move || {
                let _ = render::render_highlights_with_progress(
                    &cwd_clone,
                    &files_clone,
                    &cuts_clone,
                    Some(tx),
                );
            });
        }

        let mut render_finished = false;
        if let Some(rx) = render_rx.as_ref() {
            while let Ok(event) = rx.try_recv() {
                match event {
                    render::RenderEvent::Started { total } => {
                        render_overlay = Some(ui::RenderOverlay {
                            title: "Rendering".to_string(),
                            lines: vec![format!("Segments: 0/{total}")],
                        });
                    }
                    render::RenderEvent::SegmentDone { current, total } => {
                        render_overlay = Some(ui::RenderOverlay {
                            title: "Rendering".to_string(),
                            lines: vec![format!("Segments: {current}/{total}")],
                        });
                    }
                    render::RenderEvent::Concatenating => {
                        render_overlay = Some(ui::RenderOverlay {
                            title: "Rendering".to_string(),
                            lines: vec!["Concatenating segments...".to_string()],
                        });
                    }
                    render::RenderEvent::Done(path) => {
                        render_overlay = Some(ui::RenderOverlay {
                            title: "Render complete".to_string(),
                            lines: vec![format!("Output: {}", path.display())],
                        });
                        render_done_at = Some(Instant::now());
                        render_finished = true;
                    }
                    render::RenderEvent::Error(message) => {
                        render_overlay = Some(ui::RenderOverlay {
                            title: "Render failed".to_string(),
                            lines: vec![message],
                        });
                        render_done_at = Some(Instant::now());
                        render_finished = true;
                    }
                }
            }
        }
        if render_finished {
            render_rx = None;
        }

        if let Some(done_at) = render_done_at
            && done_at.elapsed() > Duration::from_secs(3)
        {
            render_done_at = None;
            render_overlay = None;
        }

        draw_ui(DrawContext {
            terminal: &mut terminal,
            files: &files_display,
            ipc_path: &ipc_path,
            pending_in,
            cuts: &cuts,
            speed,
            volume,
            zoom_state,
            zoom_mode,
            show_help,
            show_render_prompt,
            render_overlay: render_overlay.as_ref(),
            editor_active,
            editor_state: editor_state.as_mut(),
            editor_command: editor_command.as_deref(),
            editor_error: editor_error.as_deref(),
        })?;
    }

    if let Err(err) = export::export_all(&cwd, &cuts) {
        log_error(&format!("export failed: {err:#}"));
    }
    if let Err(err) = ipc::quit(&ipc_path) {
        log_error(&format!("mpv quit failed: {err:#}"));
    }

    if let Err(err) = child.kill()
        && err.kind() != std::io::ErrorKind::InvalidInput
    {
        log_error(&format!("failed to kill mpv: {err}"));
    }

    if ipc_path.exists()
        && let Err(err) = fs::remove_file(&ipc_path)
    {
        log_error(&format!("failed to remove IPC socket: {err}"));
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}

fn spawn_mpv(ipc_path: &Path, files: &[PathBuf], debug_mpv: bool) -> Result<Child> {
    let mut cmd = Command::new("mpv");
    cmd.arg(format!("--input-ipc-server={}", ipc_path.display()))
        .arg("--force-window=yes")
        .arg("--keep-open=yes")
        .arg("--input-default-bindings=no")
        .arg("--input-vo-keyboard=no")
        .arg("--input-terminal=no")
        .arg("--focus-on=never")
        .arg("--ontop");
    let log_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("mpv.log");
    cmd.arg(format!("--log-file={}", log_path.display()));
    if !debug_mpv {
        cmd.arg("--msg-level=all=no");
    }
    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open mpv log {log_path:?}"))?;
    let log_file_err = log_file.try_clone().context("clone mpv log handle")?;
    cmd.stdout(log_file);
    cmd.stderr(log_file_err);
    for file in files {
        cmd.arg(file);
    }
    cmd.spawn().context("spawn mpv")
}

fn wait_for_socket(ipc_path: &Path, timeout: Duration, child: &mut Child) -> Result<()> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().context("check mpv status")? {
            return Err(anyhow!("mpv exited before IPC ready: {status}"));
        }
        if !ipc_path.exists() {
            if start.elapsed() > timeout {
                return Err(anyhow!("mpv IPC socket not ready: timed out"));
            }
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        match std::os::unix::net::UnixStream::connect(ipc_path) {
            Ok(_) => return Ok(()),
            Err(err) => {
                if start.elapsed() > timeout {
                    return Err(anyhow!("mpv IPC socket not ready: {err}"));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

struct KeyContext<'a> {
    pending_in: &'a mut Option<f64>,
    cuts: &'a mut BTreeMap<String, Vec<Segment>>,
    speed: &'a mut f64,
    volume: &'a mut f64,
    show_help: &'a mut bool,
    pending_space: &'a mut Option<Instant>,
    show_render_prompt: &'a mut bool,
    render_request: &'a mut bool,
    zoom_mode: &'a mut bool,
    zoom_state: &'a mut ZoomState,
    zoom_pause_state: &'a mut Option<bool>,
    editor_active: &'a mut bool,
    editor_state: &'a mut Option<EditorState>,
    editor_pause_state: &'a mut Option<bool>,
}

struct EditorContext<'a> {
    cuts: &'a mut BTreeMap<String, Vec<Segment>>,
    editor_active: &'a mut bool,
    editor_state: &'a mut Option<EditorState>,
    editor_pause_state: &'a mut Option<bool>,
    editor_command: &'a mut Option<String>,
    editor_error: &'a mut Option<String>,
}

struct DrawContext<'a> {
    terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>,
    files: &'a [String],
    ipc_path: &'a Path,
    pending_in: Option<f64>,
    cuts: &'a BTreeMap<String, Vec<Segment>>,
    speed: f64,
    volume: f64,
    zoom_state: ZoomState,
    zoom_mode: bool,
    show_help: bool,
    show_render_prompt: bool,
    render_overlay: Option<&'a ui::RenderOverlay>,
    editor_active: bool,
    editor_state: Option<&'a mut EditorState>,
    editor_command: Option<&'a str>,
    editor_error: Option<&'a str>,
}

fn handle_key(
    key: KeyEvent,
    cwd: &Path,
    ipc_path: &Path,
    last_path: Option<PathBuf>,
    rendering_active: bool,
    ctx: &mut KeyContext<'_>,
) -> Result<bool> {
    if rendering_active {
        match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
            _ => return Ok(false),
        }
    }
    if *ctx.show_render_prompt {
        match key.code {
            KeyCode::Char('y') => {
                *ctx.render_request = true;
                *ctx.show_render_prompt = false;
                return Ok(false);
            }
            KeyCode::Char('N') => {
                *ctx.show_render_prompt = false;
                return Ok(false);
            }
            KeyCode::Esc => {
                *ctx.show_render_prompt = false;
                return Ok(false);
            }
            _ => return Ok(false),
        }
    }
    if *ctx.zoom_mode {
        match key.code {
            KeyCode::Char('q') => {
                *ctx.zoom_mode = false;
                if let Some(was_paused) = ctx.zoom_pause_state.take()
                    && !was_paused
                {
                    let _ = ipc::set_bool(ipc_path, "pause", false);
                }
                return Ok(false);
            }
            KeyCode::Char('+') => {
                ctx.zoom_state.zoom = (ctx.zoom_state.zoom + 0.1).min(4.0);
                apply_zoom(ipc_path, *ctx.zoom_state);
                return Ok(false);
            }
            KeyCode::Char('-') => {
                ctx.zoom_state.zoom = (ctx.zoom_state.zoom - 0.1).max(1.0);
                apply_zoom(ipc_path, *ctx.zoom_state);
                return Ok(false);
            }
            KeyCode::Char('0') => {
                *ctx.zoom_state = ZoomState::default();
                apply_zoom(ipc_path, *ctx.zoom_state);
                return Ok(false);
            }
            KeyCode::Char('h') => {
                ctx.zoom_state.pan_x = (ctx.zoom_state.pan_x - 0.1).max(-1.0);
                apply_zoom(ipc_path, *ctx.zoom_state);
                return Ok(false);
            }
            KeyCode::Char('l') => {
                ctx.zoom_state.pan_x = (ctx.zoom_state.pan_x + 0.1).min(1.0);
                apply_zoom(ipc_path, *ctx.zoom_state);
                return Ok(false);
            }
            KeyCode::Char('k') => {
                ctx.zoom_state.pan_y = (ctx.zoom_state.pan_y - 0.1).max(-1.0);
                apply_zoom(ipc_path, *ctx.zoom_state);
                return Ok(false);
            }
            KeyCode::Char('j') => {
                ctx.zoom_state.pan_y = (ctx.zoom_state.pan_y + 0.1).min(1.0);
                apply_zoom(ipc_path, *ctx.zoom_state);
                return Ok(false);
            }
            _ => return Ok(false),
        }
    }
    if ctx.pending_space.is_some() {
        match key.code {
            KeyCode::Char('v') => {
                adjust_volume(ipc_path, ctx.volume, -5.0);
                *ctx.pending_space = None;
                return Ok(false);
            }
            KeyCode::Char('V') => {
                adjust_volume(ipc_path, ctx.volume, 5.0);
                *ctx.pending_space = None;
                return Ok(false);
            }
            KeyCode::Char('m') => {
                if let Err(err) = ipc::cycle_mute(ipc_path) {
                    log_error(&format!("mute toggle failed: {err:#}"));
                }
                *ctx.pending_space = None;
                return Ok(false);
            }
            KeyCode::Char(' ') => {
                toggle_pause(ipc_path);
                *ctx.pending_space = None;
                return Ok(false);
            }
            _ => {
                toggle_pause(ipc_path);
                *ctx.pending_space = None;
            }
        }
    }
    match key {
        KeyEvent {
            code: KeyCode::Char('q'),
            ..
        } => return Ok(true),
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
        KeyEvent {
            code: KeyCode::Char('g'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL) => match open_editor(cwd, ctx.cuts) {
            Ok(state) => {
                *ctx.editor_state = Some(state);
                *ctx.editor_active = true;
                if ctx.editor_pause_state.is_none() {
                    match ipc::get_bool(ipc_path, "pause") {
                        Ok(was_paused) => {
                            *ctx.editor_pause_state = Some(was_paused);
                        }
                        Err(err) => {
                            log_error(&format!("failed to read pause state: {err:#}"));
                        }
                    }
                }
                let _ = ipc::set_bool(ipc_path, "pause", true);
            }
            Err(err) => log_error(&format!("failed to open editor: {err:#}")),
        },
        KeyEvent {
            code: KeyCode::Char('?'),
            ..
        } => {
            *ctx.show_help = !*ctx.show_help;
        }
        KeyEvent {
            code: KeyCode::Char('z'),
            ..
        } => {
            *ctx.zoom_mode = true;
            if ctx.zoom_pause_state.is_none() {
                match ipc::get_bool(ipc_path, "pause") {
                    Ok(was_paused) => {
                        *ctx.zoom_pause_state = Some(was_paused);
                    }
                    Err(err) => {
                        log_error(&format!("failed to read pause state: {err:#}"));
                    }
                }
            }
            let _ = ipc::set_bool(ipc_path, "pause", true);
            apply_zoom(ipc_path, *ctx.zoom_state);
        }
        KeyEvent {
            code: KeyCode::Char('n'),
            ..
        } => {
            let current = ipc::get_string(ipc_path, "path").ok();
            if let (Some(current), Some(last_path)) = (current, last_path.as_ref())
                && current == last_path.to_string_lossy()
            {
                *ctx.show_render_prompt = true;
                return Ok(false);
            }
            if let Err(err) = ipc::playlist_next(ipc_path) {
                log_error(&format!("next failed: {err:#}"));
            }
            *ctx.pending_in = None;
            *ctx.zoom_state = ZoomState::default();
            apply_zoom(ipc_path, *ctx.zoom_state);
            *ctx.zoom_pause_state = None;
        }
        KeyEvent {
            code: KeyCode::Esc, ..
        } => {
            *ctx.pending_in = None;
        }
        KeyEvent {
            code: KeyCode::Char(' '),
            ..
        } => {
            *ctx.pending_space = Some(Instant::now());
        }
        KeyEvent {
            code: KeyCode::Char('h'),
            ..
        } => {
            if let Err(err) = ipc::seek_rel(ipc_path, -5.0) {
                log_error(&format!("seek failed: {err:#}"));
            }
        }
        KeyEvent {
            code: KeyCode::Char('l'),
            ..
        } => {
            if let Err(err) = ipc::seek_rel(ipc_path, 5.0) {
                log_error(&format!("seek failed: {err:#}"));
            }
        }
        KeyEvent {
            code: KeyCode::Char('H'),
            ..
        } => {
            if let Err(err) = ipc::seek_rel(ipc_path, -30.0) {
                log_error(&format!("seek failed: {err:#}"));
            }
        }
        KeyEvent {
            code: KeyCode::Char('L'),
            ..
        } => {
            if let Err(err) = ipc::seek_rel(ipc_path, 30.0) {
                log_error(&format!("seek failed: {err:#}"));
            }
        }
        KeyEvent {
            code: KeyCode::Char('j'),
            ..
        } => {
            let next = (*ctx.speed - 0.25).max(0.25);
            *ctx.speed = next;
            if let Err(err) = ipc::set_f64(ipc_path, "speed", next) {
                log_error(&format!("speed change failed: {err:#}"));
            }
        }
        KeyEvent {
            code: KeyCode::Char('k'),
            ..
        } => {
            let next = (*ctx.speed + 0.25).min(4.0);
            *ctx.speed = next;
            if let Err(err) = ipc::set_f64(ipc_path, "speed", next) {
                log_error(&format!("speed change failed: {err:#}"));
            }
        }
        KeyEvent {
            code: KeyCode::Char('i'),
            ..
        } => match ipc::get_f64(ipc_path, "time-pos") {
            Ok(pos) => {
                *ctx.pending_in = Some(pos);
            }
            Err(err) => log_error(&format!("failed to read time-pos: {err:#}")),
        },
        KeyEvent {
            code: KeyCode::Char('o'),
            ..
        } => match *ctx.pending_in {
            None => {}
            Some(start) => match ipc::get_f64(ipc_path, "time-pos") {
                Ok(end) => {
                    if end <= start {
                    } else {
                        match ipc::get_string(ipc_path, "path") {
                            Ok(path) => {
                                let entry = ctx.cuts.entry(path.clone()).or_default();
                                entry.push(Segment {
                                    start,
                                    end,
                                    zoom: ctx.zoom_state.zoom,
                                    pan_x: ctx.zoom_state.pan_x,
                                    pan_y: ctx.zoom_state.pan_y,
                                });
                                *ctx.pending_in = None;
                            }
                            Err(err) => log_error(&format!("failed to read path: {err:#}")),
                        }
                    }
                }
                Err(err) => log_error(&format!("failed to read time-pos: {err:#}")),
            },
        },
        KeyEvent {
            code: KeyCode::Char('u'),
            ..
        } => match ipc::get_string(ipc_path, "path") {
            Ok(path) => match ctx.cuts.get_mut(&path) {
                Some(segments) if !segments.is_empty() => {
                    segments.pop();
                }
                _ => {}
            },
            Err(err) => log_error(&format!("failed to read path: {err:#}")),
        },
        KeyEvent {
            code: KeyCode::Char('p'),
            ..
        } => {
            if let Err(err) = ipc::playlist_prev(ipc_path) {
                log_error(&format!("prev failed: {err:#}"));
            }
            *ctx.pending_in = None;
            *ctx.zoom_state = ZoomState::default();
            apply_zoom(ipc_path, *ctx.zoom_state);
            *ctx.zoom_pause_state = None;
        }
        _ => {}
    }
    Ok(false)
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    stdout
        .execute(EnterAlternateScreen)
        .context("enter alt screen")?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend).context("init terminal")?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode().context("disable raw mode")?;
    terminal
        .backend_mut()
        .execute(LeaveAlternateScreen)
        .context("leave alt screen")?;
    terminal.show_cursor().context("show cursor")?;
    Ok(())
}

fn draw_ui(ctx: DrawContext<'_>) -> Result<()> {
    if ctx.editor_active {
        if let Some(state) = ctx.editor_state {
            ctx.terminal
                .draw(|frame| {
                    ui::draw_editor(
                        frame,
                        state,
                        "markers.json",
                        ctx.editor_command,
                        ctx.editor_error,
                    );
                })
                .context("draw editor")?;
        }
        return Ok(());
    }
    let current_path = ipc::get_string(ctx.ipc_path, "path").ok();
    let current_time = ipc::get_f64(ctx.ipc_path, "time-pos").unwrap_or(0.0);
    ctx.terminal
        .draw(|frame| {
            ui::draw(
                frame,
                ui::UiState {
                    files: ctx.files,
                    current_path: current_path.as_deref(),
                    current_time,
                    speed: ctx.speed,
                    volume: ctx.volume,
                    zoom: ctx.zoom_state.zoom,
                    pan_x: ctx.zoom_state.pan_x,
                    pan_y: ctx.zoom_state.pan_y,
                    zoom_mode: ctx.zoom_mode,
                    pending_in: ctx.pending_in,
                    cuts: ctx.cuts,
                    show_help: ctx.show_help,
                    show_render_prompt: ctx.show_render_prompt,
                    render_overlay: ctx.render_overlay,
                },
            );
        })
        .context("draw ui")?;
    Ok(())
}

fn toggle_pause(ipc_path: &Path) {
    if let Err(err) = ipc::cycle_pause(ipc_path) {
        log_error(&format!("pause toggle failed: {err:#}"));
    }
}

fn adjust_volume(ipc_path: &Path, volume: &mut f64, delta: f64) {
    let next = (*volume + delta).clamp(0.0, 100.0);
    *volume = next;
    if let Err(err) = ipc::set_f64(ipc_path, "volume", next) {
        log_error(&format!("volume change failed: {err:#}"));
    }
}

fn open_editor(folder: &Path, cuts: &BTreeMap<String, Vec<Segment>>) -> Result<EditorState> {
    let path = folder.join("markers.json");
    if !path.exists() {
        export::export_markers_json(folder, cuts)?;
    }
    let content = fs::read_to_string(&path).with_context(|| format!("read {path:?}"))?;
    Ok(EditorState::new(Lines::from(content.as_str())))
}

fn handle_editor_key(
    key: KeyEvent,
    cwd: &Path,
    ipc_path: &Path,
    ctx: &mut EditorContext<'_>,
) -> Result<bool> {
    if ctx.editor_error.is_some() {
        match key.code {
            KeyCode::Char('f') | KeyCode::Esc => {
                *ctx.editor_error = None;
            }
            KeyCode::Char('d') => {
                *ctx.editor_error = None;
                *ctx.editor_active = false;
                *ctx.editor_state = None;
                if let Some(was_paused) = ctx.editor_pause_state.take()
                    && !was_paused
                {
                    let _ = ipc::set_bool(ipc_path, "pause", false);
                }
            }
            _ => {}
        }
        return Ok(false);
    }

    if let Some(command) = ctx.editor_command.as_mut() {
        let mut should_close = false;
        match key.code {
            KeyCode::Esc => {
                *ctx.editor_command = None;
            }
            KeyCode::Enter => {
                let cmd = command.trim().to_string();
                *ctx.editor_command = None;
                if cmd == "q" {
                    should_close = true;
                }
            }
            KeyCode::Backspace => {
                command.pop();
            }
            KeyCode::Char(c) => {
                if c.is_ascii() && !key.modifiers.contains(KeyModifiers::CONTROL) {
                    command.push(c);
                }
            }
            _ => {}
        }
        if should_close && let Some(state) = ctx.editor_state.as_ref() {
            match try_close_editor(cwd, state, ctx.cuts) {
                Ok(()) => {
                    *ctx.editor_active = false;
                    *ctx.editor_state = None;
                    if let Some(was_paused) = ctx.editor_pause_state.take()
                        && !was_paused
                    {
                        let _ = ipc::set_bool(ipc_path, "pause", false);
                    }
                }
                Err(err) => {
                    *ctx.editor_error = Some(err);
                }
            }
        }
        return Ok(false);
    }

    if let Some(state) = ctx.editor_state.as_ref()
        && state.mode == EditorMode::Normal
        && let KeyEvent {
            code: KeyCode::Char(':'),
            modifiers,
            ..
        } = key
        && !modifiers.contains(KeyModifiers::CONTROL)
    {
        *ctx.editor_command = Some(String::new());
        return Ok(false);
    }

    Ok(true)
}

fn try_close_editor(
    folder: &Path,
    state: &EditorState,
    cuts: &mut BTreeMap<String, Vec<Segment>>,
) -> Result<(), String> {
    let content = state.lines.to_string();
    match serde_json::from_str::<model::Export>(&content) {
        Ok(export) => {
            let path = folder.join("markers.json");
            if let Err(err) = fs::write(&path, content.as_bytes()) {
                return Err(format!("failed to write markers.json: {err}"));
            }
            *cuts = export::cuts_from_export(&export);
            Ok(())
        }
        Err(err) => Err(format!("markers.json is not valid JSON: {err}")),
    }
}

#[derive(Debug, Clone, Copy)]
struct ZoomState {
    zoom: f64,
    pan_x: f64,
    pan_y: f64,
}

impl Default for ZoomState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        }
    }
}

fn apply_zoom(ipc_path: &Path, state: ZoomState) {
    let zoom = state.zoom.max(1.0);
    if zoom == 1.0 && state.pan_x == 0.0 && state.pan_y == 0.0 {
        let _ = ipc::send_cmd(ipc_path, serde_json::json!(["set_property", "vf", ""]));
        return;
    }
    match get_video_dims(ipc_path) {
        Ok((width, height)) => {
            if let Some(filter) = build_crop_filter(state, width, height) {
                let _ = ipc::send_cmd(ipc_path, serde_json::json!(["set_property", "vf", filter]));
            }
        }
        Err(err) => log_error(&format!("failed to get video dimensions: {err:#}")),
    }
}

fn get_video_dims(ipc_path: &Path) -> Result<(i64, i64)> {
    if let Ok(width) = ipc::get_i64(ipc_path, "video-params/w")
        && let Ok(height) = ipc::get_i64(ipc_path, "video-params/h")
    {
        return Ok((width, height));
    }
    let width = ipc::get_i64(ipc_path, "width").context("read width")?;
    let height = ipc::get_i64(ipc_path, "height").context("read height")?;
    Ok((width, height))
}

fn build_crop_filter(state: ZoomState, width: i64, height: i64) -> Option<String> {
    let zoom = state.zoom.max(1.0);
    if zoom == 1.0 && state.pan_x == 0.0 && state.pan_y == 0.0 {
        return None;
    }
    let crop_w = (width as f64 / zoom).round().max(1.0) as i64;
    let crop_h = (height as f64 / zoom).round().max(1.0) as i64;
    let max_x = ((width - crop_w) as f64 / 2.0).max(0.0);
    let max_y = ((height - crop_h) as f64 / 2.0).max(0.0);
    let offset_x = (state.pan_x.clamp(-1.0, 1.0) * max_x).round();
    let offset_y = (state.pan_y.clamp(-1.0, 1.0) * max_y).round();
    let mut x = ((width - crop_w) as f64 / 2.0 + offset_x).round() as i64;
    let mut y = ((height - crop_h) as f64 / 2.0 + offset_y).round() as i64;
    x = x.clamp(0, width - crop_w);
    y = y.clamp(0, height - crop_h);
    Some(format!(
        "crop={crop_w}:{crop_h}:{x}:{y},scale={width}:{height}"
    ))
}

use crate::model::{Segment, fmt_time_hhmmss_millis};
use edtui::{EditorState, EditorTheme, EditorView, LineNumbers, SyntaxHighlighter};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use std::collections::BTreeMap;

pub struct UiState<'a> {
    pub files: &'a [String],
    pub current_path: Option<&'a str>,
    pub current_time: f64,
    pub speed: f64,
    pub volume: f64,
    pub zoom: f64,
    pub pan_x: f64,
    pub pan_y: f64,
    pub zoom_mode: bool,
    pub pending_in: Option<f64>,
    pub cuts: &'a BTreeMap<String, Vec<Segment>>,
    pub show_help: bool,
    pub show_render_prompt: bool,
    pub render_overlay: Option<&'a RenderOverlay>,
}

pub struct RenderOverlay {
    pub title: String,
    pub lines: Vec<String>,
}

pub fn draw(frame: &mut Frame, state: UiState<'_>) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(1),
            ]
            .as_ref(),
        )
        .split(frame.area());

    let info = build_info_line(&state);
    let info_block = Paragraph::new(info)
        .block(Block::default().borders(Borders::ALL).title("Status"))
        .wrap(Wrap { trim: true });
    frame.render_widget(info_block, layout[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)].as_ref())
        .split(layout[1]);

    render_files_list(frame, body[0], state.files, state.current_path);
    render_segments(
        frame,
        body[1],
        state.cuts,
        state.current_path,
        state.pending_in,
    );

    let footer_text = if state.zoom_mode {
        "ZOOM MODE: +/- zoom | hjkl pan | 0 reset | q exit"
    } else {
        "Press ? for shortcuts | q to quit"
    };
    let footer_style = if state.zoom_mode {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    let footer = Paragraph::new(footer_text).style(footer_style);
    frame.render_widget(footer, layout[2]);

    if state.show_render_prompt {
        render_render_prompt(frame, frame.area());
    } else if let Some(overlay) = state.render_overlay {
        render_overlay(frame, frame.area(), overlay);
    } else if state.show_help {
        render_help_overlay(frame, frame.area());
    }
}

pub fn draw_editor(
    frame: &mut Frame,
    state: &mut EditorState,
    title: &str,
    command: Option<&str>,
    error: Option<&str>,
) {
    let area = frame.area();
    let theme = EditorTheme::default().block(Block::default().borders(Borders::ALL).title(title));
    let syntax = SyntaxHighlighter::new("base16-ocean.dark", "json").ok();
    let view = EditorView::new(state)
        .theme(theme)
        .line_numbers(LineNumbers::Absolute)
        .syntax_highlighter(syntax);
    frame.render_widget(view, area);
    if let Some(error) = error {
        render_editor_error(frame, area, error);
    } else if let Some(command) = command {
        render_editor_command(frame, area, command);
    }
}

fn render_editor_command(frame: &mut Frame, area: Rect, command: &str) {
    let height = 3;
    let command_area = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(height),
        area.width,
        height,
    );
    let text = format!(":{command}");
    let block = Block::default().borders(Borders::ALL).title("Command");
    let paragraph = Paragraph::new(text)
        .block(block)
        .style(Style::default().bg(Color::Black).fg(Color::Yellow));
    frame.render_widget(Clear, command_area);
    frame.render_widget(paragraph, command_area);
}

fn render_editor_error(frame: &mut Frame, area: Rect, error: &str) {
    let lines = vec![
        Line::from("markers.json is invalid:"),
        Line::from(error),
        Line::from(" "),
        Line::from("f  fix in editor"),
        Line::from("d  discard changes"),
    ];
    let block = Block::default()
        .title("Invalid JSON")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black).fg(Color::Red));
    let popup_area = centered_rect(70, 40, area);
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(Clear, popup_area);
    frame.render_widget(paragraph, popup_area);
}

fn build_info_line(state: &UiState<'_>) -> Line<'static> {
    let path = state.current_path.unwrap_or("-").to_string();
    let time_fmt = fmt_time_hhmmss_millis(state.current_time);
    let speed_fmt = format!("{:.2}x", state.speed);
    let volume_fmt = format!("{:.0}%", state.volume);
    let zoom_fmt = format!("{:.2}x", state.zoom);
    let pending_fmt = state
        .pending_in
        .map(fmt_time_hhmmss_millis)
        .unwrap_or_else(|| "-".to_string());
    let mut spans = vec![
        Span::styled("File: ", Style::default().fg(Color::Yellow)),
        Span::raw(path),
        Span::raw("  |  "),
        Span::styled("Time: ", Style::default().fg(Color::Yellow)),
        Span::raw(time_fmt),
        Span::raw("  |  "),
        Span::styled("Speed: ", Style::default().fg(Color::Yellow)),
        Span::raw(speed_fmt),
        Span::raw("  |  "),
        Span::styled("Vol: ", Style::default().fg(Color::Yellow)),
        Span::raw(volume_fmt),
        Span::raw("  |  "),
        Span::styled("Zoom: ", Style::default().fg(Color::Yellow)),
        Span::raw(zoom_fmt),
        Span::raw("  |  "),
        Span::styled("IN: ", Style::default().fg(Color::Yellow)),
        Span::raw(pending_fmt),
    ];
    if state.zoom_mode {
        spans.push(Span::raw("  |  "));
        spans.push(Span::styled(
            "MODE: ZOOM",
            Style::default().fg(Color::Magenta),
        ));
        spans.push(Span::raw(format!(
            " ({:.2},{:.2})",
            state.pan_x, state.pan_y
        )));
    }
    Line::from(spans)
}

fn render_files_list(frame: &mut Frame, area: Rect, files: &[String], current_path: Option<&str>) {
    let items: Vec<ListItem> = files
        .iter()
        .map(|path| {
            let is_current = current_path.map(|p| p == path).unwrap_or(false);
            let style = if is_current {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(path.clone(), style)))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Videos"))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_widget(list, area);
}

fn render_segments(
    frame: &mut Frame,
    area: Rect,
    cuts: &BTreeMap<String, Vec<Segment>>,
    current_path: Option<&str>,
    pending_in: Option<f64>,
) {
    let segments = current_path
        .and_then(|path| cuts.get(path))
        .cloned()
        .unwrap_or_default();

    let mut items: Vec<ListItem> = Vec::new();
    if let Some(start) = pending_in {
        let start_fmt = fmt_time_hhmmss_millis(start);
        let line = Line::from(vec![
            Span::styled("..  ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("{start_fmt} -> (open)"),
                Style::default().fg(Color::Yellow),
            ),
        ]);
        items.push(ListItem::new(line));
    }
    items.extend(segments.iter().enumerate().map(|(idx, segment)| {
        let start = fmt_time_hhmmss_millis(segment.start);
        let end = fmt_time_hhmmss_millis(segment.end);
        ListItem::new(Line::from(format!("{idx:02}  {start} -> {end}")))
    }));

    let list = if items.is_empty() {
        List::new(vec![ListItem::new("(no markers)")])
    } else {
        List::new(items)
    };

    let list = list.block(Block::default().borders(Borders::ALL).title("Markers"));
    frame.render_widget(list, area);
}

fn render_help_overlay(frame: &mut Frame, area: Rect) {
    let help_lines = vec![
        Line::from("Shortcuts"),
        Line::from(" "),
        Line::from("Space  toggle pause"),
        Line::from("h/l    seek -5s / +5s"),
        Line::from("H/L    seek -30s / +30s"),
        Line::from("j/k    speed -0.25 / +0.25"),
        Line::from("space+v/V  volume -5 / +5"),
        Line::from("space+m    mute toggle"),
        Line::from("i      mark IN"),
        Line::from("o      mark OUT"),
        Line::from("u      undo last segment"),
        Line::from("n/p    next / previous file"),
        Line::from("z      enter zoom mode"),
        Line::from("zoom: + / - / 0 / hjkl / q"),
        Line::from("Ctrl+g edit markers.json"),
        Line::from("q      export and quit"),
        Line::from("?      toggle this help"),
    ];

    let block = Block::default()
        .title("Help")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black).fg(Color::White));

    let popup_area = centered_rect(60, 60, area);
    let paragraph = Paragraph::new(help_lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(Clear, popup_area);
    frame.render_widget(paragraph, popup_area);
}

fn render_render_prompt(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from("Generate highlights video?"),
        Line::from(" "),
        Line::from("y  yes, render with ffmpeg"),
        Line::from("N  no, keep reviewing"),
    ];
    let block = Block::default()
        .title("Render")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black).fg(Color::White));
    let popup_area = centered_rect(60, 40, area);
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(Clear, popup_area);
    frame.render_widget(paragraph, popup_area);
}

fn render_overlay(frame: &mut Frame, area: Rect, overlay: &RenderOverlay) {
    let lines: Vec<Line> = overlay
        .lines
        .iter()
        .map(|line| Line::from(line.as_str()))
        .collect();
    let block = Block::default()
        .title(overlay.title.as_str())
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black).fg(Color::White));
    let popup_area = centered_rect(70, 40, area);
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(Clear, popup_area);
    frame.render_widget(paragraph, popup_area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1])[1]
}

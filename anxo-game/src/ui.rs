use bevy::prelude::*;
use bevy_egui::EguiContexts;
use bevy_egui::egui;
use egui::text::{CCursor, CCursorRange};
use egui_code_editor::{CodeEditor, ColorTheme, Syntax};

use crate::{GamePhase, UiLayout};

#[derive(Resource)]
pub struct EditorState {
    pub code: String,
    pub error: Option<String>,
}

#[derive(Message)]
pub struct RunRequest(pub String);

#[derive(Message)]
pub struct ResetRequest;

#[derive(Default)]
pub(crate) struct AutocompleteState {
    seed_prefix: String,
    seed_prefix_set: bool,
    last_applied: String,
    index: usize,
    last_cursor: usize,
}

#[derive(Default)]
struct ShortcutState {
    tab: bool,
    run: bool,
    reset: bool,
}

#[derive(Default)]
pub(crate) struct FocusState {
    editor_focused: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn ui_system(
    mut contexts: EguiContexts,
    mut editor: ResMut<EditorState>,
    mut layout: ResMut<UiLayout>,
    mut run_events: MessageWriter<RunRequest>,
    mut reset_events: MessageWriter<ResetRequest>,
    mut autocomplete: Local<AutocompleteState>,
    mut focus_state: Local<FocusState>,
    phase: Res<GamePhase>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let shortcuts = capture_shortcuts(ctx, focus_state.editor_focused);
    let panel = egui::SidePanel::right("editor_panel")
        .resizable(true)
        .min_width(320.0)
        .show(ctx, |ui| {
            ui.heading("Code");
            ui.horizontal(|ui| {
                if ui.button("Run").clicked() {
                    run_events.write(RunRequest(editor.code.clone()));
                }
                if ui.button("Reset").clicked() {
                    reset_events.write(ResetRequest);
                }
            });

            let mut editor_widget = CodeEditor::default()
                .id_source("code_editor")
                .with_rows(20)
                .with_fontsize(14.0)
                .with_theme(ColorTheme::GRUVBOX)
                .with_syntax(Syntax::python())
                .with_numlines(true)
                .vscroll(true);

            let mut output = editor_widget.show(ui, &mut editor.code);

            let cursor_index = output
                .state
                .cursor
                .char_range()
                .map(|range| range.primary.index)
                .unwrap_or(0);
            autocomplete.last_cursor = cursor_index;
            focus_state.editor_focused = output.response.has_focus();

            ctx.memory_mut(|mem| {
                mem.set_focus_lock_filter(
                    output.response.id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                );
            });

            let mut tab_consumed = false;
            if shortcuts.tab
                && output.response.has_focus()
                && let Some((prefix_start, prefix_end, prefix)) =
                    completion_span(&editor.code, cursor_index)
            {
                let use_seed = autocomplete.seed_prefix_set && prefix == autocomplete.last_applied;
                let base_prefix = if use_seed {
                    autocomplete.seed_prefix.clone()
                } else {
                    prefix.clone()
                };
                let matches: Vec<&str> = HERO_COMPLETIONS
                    .iter()
                    .copied()
                    .filter(|option| option.starts_with(&base_prefix))
                    .collect();
                if !matches.is_empty() {
                    if !use_seed {
                        autocomplete.seed_prefix = base_prefix.clone();
                        autocomplete.seed_prefix_set = true;
                        autocomplete.index = 0;
                    }
                    let choice = matches[autocomplete.index % matches.len()];
                    autocomplete.index = autocomplete.index.saturating_add(1);
                    autocomplete.last_applied = choice.to_string();
                    tab_consumed = true;

                    replace_range(&mut editor.code, prefix_start, prefix_end, choice);
                    let prefix_chars = prefix.chars().count();
                    let choice_chars = choice.chars().count();
                    let start_char = cursor_index.saturating_sub(prefix_chars);
                    let new_cursor = CCursor::new(start_char + choice_chars);
                    output
                        .state
                        .cursor
                        .set_char_range(Some(CCursorRange::one(new_cursor)));
                    output.state.store(ctx, output.response.id);
                }
            }

            if let Some((_, _, prefix)) = completion_span(&editor.code, cursor_index) {
                if !tab_consumed && prefix != autocomplete.last_applied {
                    autocomplete.seed_prefix = prefix.clone();
                    autocomplete.seed_prefix_set = true;
                    autocomplete.index = 0;
                }

                let mut any = false;
                ui.separator();
                ui.label("Autocomplete");
                for option in HERO_COMPLETIONS {
                    if option.starts_with(&prefix) {
                        any = true;
                        let remaining = &option[prefix.len()..];
                        if ui.button(option).clicked() && !remaining.is_empty() {
                            insert_at_cursor(&mut editor.code, cursor_index, remaining);
                        }
                    }
                }
                if !any {
                    ui.label("No matches");
                }
            }

            ui.separator();
            if let Some(error) = &editor.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            } else if *phase == GamePhase::Won {
                ui.colored_label(egui::Color32::LIGHT_GREEN, "Success! You reached the flag.");
            } else {
                ui.label("Ready.");
            }

            if shortcuts.run && output.response.has_focus() {
                run_events.write(RunRequest(editor.code.clone()));
            }

            if shortcuts.reset && output.response.has_focus() {
                reset_events.write(ResetRequest);
            }
        });

    layout.editor_width = panel.response.rect.width().max(0.0);
    layout.editor_left = panel.response.rect.left().max(0.0);
    layout.pixels_per_point = ctx.pixels_per_point().max(0.1);

    Ok(())
}

const HERO_COMPLETIONS: [&str; 2] = ["move_left()", "move_right()"];

fn completion_span(code: &str, cursor_char_index: usize) -> Option<(usize, usize, String)> {
    let cursor_byte_index = char_to_byte_index(code, cursor_char_index);
    let before = &code[..cursor_byte_index.min(code.len())];
    if let Some(idx) = before.rfind("hero.") {
        let start = idx + "hero.".len();
        let after = &before[start..];
        if after
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '(' || c == ')')
        {
            return Some((start, cursor_byte_index, after.to_string()));
        }
    }
    None
}

fn char_to_byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(idx, _)| idx)
        .unwrap_or_else(|| text.len())
}

fn insert_at_cursor(text: &mut String, cursor_char_index: usize, insert: &str) {
    let byte_index = char_to_byte_index(text, cursor_char_index);
    text.insert_str(byte_index, insert);
}

fn replace_range(text: &mut String, start: usize, end: usize, replacement: &str) {
    if start <= end && end <= text.len() {
        text.replace_range(start..end, replacement);
    }
}

fn capture_shortcuts(ctx: &egui::Context, editor_focused: bool) -> ShortcutState {
    let mut shortcuts = ShortcutState::default();
    if !editor_focused {
        return shortcuts;
    }
    ctx.input_mut(|input| {
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
            shortcuts.tab = true;
        }
        let run_shortcut = egui::KeyboardShortcut::new(
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            },
            egui::Key::Enter,
        );
        if input.consume_shortcut(&run_shortcut) {
            shortcuts.run = true;
        }
        let reset_shortcut = egui::KeyboardShortcut::new(
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            },
            egui::Key::R,
        );
        if input.consume_shortcut(&reset_shortcut) {
            shortcuts.reset = true;
        }
    });
    shortcuts
}

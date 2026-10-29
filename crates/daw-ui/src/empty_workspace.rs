//! Quiet next steps for an empty musical workspace. The caller owns commands,
//! navigation, document state, and placement within the available canvas.
use crate::{text, theme, tokens};
use egui::{Align, Layout, Rect, Response, Ui, UiBuilder, vec2};

pub struct EmptyWorkspaceAction<'a> {
    pub id: &'a str,
    pub label: &'a str,
    /// Details stay in the hover hint instead of repeating below each action.
    pub description: &'a str,
    pub primary: bool,
}

pub struct EmptyWorkspaceResponse {
    pub action: Option<String>,
    pub rect: Rect,
    pub actions: Vec<(String, Response)>,
}

/// Center a bounded set of real actions, with one clear starting point. This
/// consumes only its content height and never claims the empty canvas behind it.
/// Use a caller-owned ID scope when showing more than one in the same parent.
pub fn empty_workspace(
    ui: &mut Ui,
    title: &str,
    description: &str,
    actions: &[EmptyWorkspaceAction<'_>],
) -> EmptyWorkspaceResponse {
    let available = ui.available_rect_before_wrap();
    let width = available.width().clamp(0.0, 560.0);
    let rect = Rect::from_min_size(
        egui::pos2(available.center().x - width * 0.5, available.top()),
        vec2(width, available.height()),
    );
    let mut result = EmptyWorkspaceResponse {
        action: None,
        rect: Rect::NOTHING,
        actions: Vec::new(),
    };
    let actions = &actions[..actions.len().min(4)];
    let primary = actions.iter().position(|action| action.primary);
    let response = ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Center)),
        |ui| {
            ui.set_width(width);
            text::bounded_block(ui, text::TextRole::Title, bounded(title, 160), 2);
            if !description.is_empty() {
                ui.add_space(tokens::SPACE_XS);
                text::bounded_block(ui, text::TextRole::Note, bounded(description, 512), 3);
            }
            if actions.is_empty() {
                return;
            }
            ui.add_space(tokens::SPACE_LG);
            let button_width = width.min(240.0);
            if let Some(index) = primary {
                action_button(ui, &actions[index], button_width, true, &mut result);
                if actions.len() > 1 {
                    ui.add_space(tokens::SPACE_XS);
                }
            }
            let secondary: Vec<_> = actions
                .iter()
                .enumerate()
                .filter(|(index, _)| Some(*index) != primary)
                .map(|(_, action)| action)
                .collect();
            let columns = if width >= 2.0 * 220.0 + tokens::SPACE_SM {
                2
            } else {
                1
            };
            for row in secondary.chunks(columns) {
                let item_width = if columns == 2 { 220.0 } else { button_width };
                let row_width = row.len() as f32 * item_width
                    + row.len().saturating_sub(1) as f32 * tokens::SPACE_SM;
                ui.allocate_ui_with_layout(
                    vec2(row_width, tokens::CONTROL_HEIGHT),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = tokens::SPACE_SM;
                        for action in row {
                            action_button(ui, action, item_width, false, &mut result);
                        }
                    },
                );
            }
        },
    );
    result.rect = response.response.rect;
    result
}

fn action_button(
    ui: &mut Ui,
    action: &EmptyWorkspaceAction<'_>,
    width: f32,
    primary: bool,
    result: &mut EmptyWorkspaceResponse,
) {
    let response = ui
        .push_id(action.id, |ui| {
            let mut button = crate::button_widget(bounded(action.label, 120)).truncate();
            if primary {
                button = button.fill(theme::palette(ui.ctx()).track_selected);
            }
            ui.add_sized(vec2(width, tokens::CONTROL_HEIGHT), button)
        })
        .inner
        .on_hover_text(if action.description.is_empty() {
            bounded(action.label, 120)
        } else {
            format!(
                "{}\n{}",
                bounded(action.label, 120),
                bounded(action.description, 512)
            )
        });
    if response.clicked() {
        result.action = Some(action.id.to_owned());
    }
    result.actions.push((action.id.to_owned(), response));
}

fn bounded(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

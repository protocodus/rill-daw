//! Insert-chain structure shared by channel and device views. The caller owns
//! plug-in state, ordering and commands; these components only arrange controls.
use crate::{text, theme, tokens};
use egui::{InnerResponse, Response, Ui};

/// The chain's heading with its active count; the processing order is in the
/// count's tooltip rather than a persistent sentence.
pub fn effect_chain_header(ui: &mut Ui, count: usize, active: usize) -> Response {
    // The heading alone sets the line height, as a section heading does in
    // the sibling windows; the count shares its line at the trailing edge.
    let heading = text::section_heading(ui, "Inserts");
    let line = egui::Rect::from_x_y_ranges(
        heading.rect.right()..=ui.max_rect().right(),
        heading.rect.y_range(),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(line)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            text::note(ui, format!("{active} / {count} active")).on_hover_text(
                "Input → Inserts → Volume & pan\nEffects process the signal from top to bottom, before channel volume and pan.",
            )
        },
    )
    .inner
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectInsertSide {
    Before,
    After,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectSlotAction {
    OpenEditor,
    Parameters,
    Insert(EffectInsertSide),
    ToggleEnabled,
    MoveUp,
    MoveDown,
    Remove,
}

/// The on-card controls and its secondary-click menu share the same commands.
pub struct EffectSlotControls {
    pub index: usize,
    pub count: usize,
    pub enabled: bool,
    pub has_parameters: bool,
}

impl EffectSlotControls {
    pub fn action_row(&self, ui: &mut Ui) -> Option<EffectSlotAction> {
        let mut action = self.primary_actions(ui);
        crate::action_menu(ui, "More", |ui| {
            if let Some(selected) = self.reorder_actions(ui) {
                action = Some(selected);
                ui.close();
            }
        })
        .response
        .on_hover_text("Insert actions");
        action
    }

    pub fn context_menu(&self, ui: &mut Ui, card: &Response) -> Option<EffectSlotAction> {
        crate::selection_item_context_menu(ui, card.id.with("slot_context"), card.rect, |ui| {
            let primary = self.primary_actions(ui);
            let toggle = crate::button(
                ui,
                if self.enabled {
                    "Bypass insert"
                } else {
                    "Enable insert"
                },
            )
            .clicked();
            ui.separator();
            let reorder = self.reorder_actions(ui);
            let selected = primary
                .or(toggle.then_some(EffectSlotAction::ToggleEnabled))
                .or(reorder);
            if selected.is_some() {
                ui.close();
            }
            selected
        })
        .and_then(|popup| popup.inner)
    }

    fn primary_actions(&self, ui: &mut Ui) -> Option<EffectSlotAction> {
        let mut choice = None;
        for (label, hint, action) in [
            (
                "Open editor",
                "Open the effect editor",
                EffectSlotAction::OpenEditor,
            ),
            (
                "Parameters…",
                "Edit or automate this plug-in's parameters",
                EffectSlotAction::Parameters,
            ),
            (
                "Insert before…",
                "Choose an effect to process before this insert",
                EffectSlotAction::Insert(EffectInsertSide::Before),
            ),
            (
                "Insert after…",
                "Choose an effect to process after this insert",
                EffectSlotAction::Insert(EffectInsertSide::After),
            ),
        ] {
            // Built-ins expose automation directly on their panel controls.
            if (action != EffectSlotAction::Parameters || self.has_parameters)
                && crate::small_button(ui, label).on_hover_text(hint).clicked()
            {
                choice = Some(action);
            }
        }
        choice
    }

    fn reorder_actions(&self, ui: &mut Ui) -> Option<EffectSlotAction> {
        let up = ui.add_enabled(self.index > 0, crate::button_widget("Up"));
        let down = ui.add_enabled(self.index + 1 < self.count, crate::button_widget("Down"));
        ui.separator();
        let remove = crate::button(ui, "Remove insert");
        if up.clicked() {
            Some(EffectSlotAction::MoveUp)
        } else if down.clicked() {
            Some(EffectSlotAction::MoveDown)
        } else if remove.clicked() {
            Some(EffectSlotAction::Remove)
        } else {
            None
        }
    }
}

/// One insert: an identity row (reorder handle, enable switch with the name,
/// the plug-in model on a second line) and an action row that wraps on narrow displays.
/// Bypass reads the same everywhere: muted name, gray rail and a tooltip.
pub fn effect_slot<T: Send + Sync + 'static, R>(
    ui: &mut Ui,
    payload: T,
    index: usize,
    name: &str,
    model: &str,
    enabled: &mut bool,
    actions: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<(Response, R)> {
    let result = crate::card(ui.ctx()).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = tokens::SPACE_XS;
        let colors = theme::palette(ui.ctx());
        let switch = ui
            .horizontal(|ui| {
                crate::reorder_handle(ui, payload);
                let mut title = egui::text::LayoutJob::simple(
                    format!("{:02}  {name}", index + 1),
                    text::TextRole::Label.font(),
                    if *enabled { colors.text } else { colors.muted },
                    (ui.available_width() - tokens::SWITCH_WIDTH - tokens::SPACE_SM).max(0.0),
                );
                title.wrap.max_rows = 1;
                crate::switch(ui, enabled, title).on_hover_text(if *enabled {
                    name.to_owned()
                } else {
                    format!("{name} · Bypassed")
                })
            })
            .inner;
        text::truncated(ui, text::TextRole::Note, model).on_hover_text(model);
        let actions = crate::control_row_wrapped(ui, actions).inner;
        (switch, actions)
    });
    let colors = theme::palette(ui.ctx());
    let rect = result.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(
            rect.min + egui::vec2(0.0, tokens::SPACE_SM),
            egui::vec2(2.0, (rect.height() - 2.0 * tokens::SPACE_SM).max(0.0)),
        ),
        1.0,
        if *enabled { colors.accent } else { colors.line },
    );
    result
}

/// Empty insert chains keep their purpose and insertion action discoverable.
pub fn empty_effect_chain(ui: &mut Ui) -> Response {
    // Fill only: the inset's own outline would be a second boundary.
    crate::inset(ui.ctx())
        .stroke(egui::Stroke::NONE)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            text::label(ui, "No effects");
            text::note(ui, "Add an insert to shape this channel.")
        })
        .response
}

/// What a row inside a container lane asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneEffectAction {
    Open,
    ToggleEnabled,
    MoveUp,
    MoveDown,
    /// To the end of another lane, by index.
    MoveToLane(usize),
    Remove,
}

/// One effect inside a container lane. The lane is already a card, so the
/// row is flat: the enable switch with the name, **Open** and a **More**
/// menu on one line, the model under it. The secondary-click menu offers
/// the same actions.
#[allow(clippy::too_many_arguments)]
pub fn lane_effect_row(
    ui: &mut Ui,
    name: &str,
    model: &str,
    enabled: bool,
    index: usize,
    count: usize,
    lane: usize,
    lanes: &[String],
) -> Option<LaneEffectAction> {
    let colors = theme::palette(ui.ctx());
    let mut action = None;
    let row = ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        // The name takes the whole first line; the model and the actions
        // share the second, so a narrow lane still shows the name.
        let mut on = enabled;
        let mut title = egui::text::LayoutJob::simple_singleline(
            name.to_owned(),
            text::TextRole::Label.font(),
            if enabled { colors.text } else { colors.muted },
        );
        title.wrap = egui::text::TextWrapping::truncate_at_width(
            (ui.available_width() - tokens::SWITCH_WIDTH - tokens::SPACE_SM).max(0.0),
        );
        if crate::switch(ui, &mut on, title)
            .on_hover_text(if enabled {
                format!("{name} · Click to bypass")
            } else {
                format!("{name} · Bypassed")
            })
            .changed()
        {
            action = Some(LaneEffectAction::ToggleEnabled);
        }
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), tokens::CONTROL_HEIGHT),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                if let Some(chosen) = crate::compact_action_menu(ui, "More", |ui| {
                    lane_effect_menu(ui, index, count, lane, lanes)
                })
                .inner
                .flatten()
                {
                    action = Some(chosen);
                }
                if crate::small_button(ui, "Open")
                    .on_hover_text("Open this effect's panel")
                    .clicked()
                {
                    action = Some(LaneEffectAction::Open);
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    text::truncated(ui, text::TextRole::Note, model);
                });
            },
        );
    });
    let menu = ui.id().with(("lane_effect_menu", lane, index));
    if let Some(chosen) = crate::selection_item_context_menu(ui, menu, row.response.rect, |ui| {
        let open = crate::button(ui, "Open").clicked();
        let toggle = crate::button(ui, if enabled { "Bypass" } else { "Enable" }).clicked();
        ui.separator();
        let other = lane_effect_menu(ui, index, count, lane, lanes);
        let chosen = open
            .then_some(LaneEffectAction::Open)
            .or(toggle.then_some(LaneEffectAction::ToggleEnabled))
            .or(other);
        if chosen.is_some() {
            ui.close();
        }
        chosen
    })
    .and_then(|popup| popup.inner)
    {
        action = Some(chosen);
    }
    action
}

fn lane_effect_menu(
    ui: &mut Ui,
    index: usize,
    count: usize,
    lane: usize,
    lanes: &[String],
) -> Option<LaneEffectAction> {
    let mut action = None;
    if ui
        .add_enabled(index > 0, crate::button_widget("Move up"))
        .clicked()
    {
        action = Some(LaneEffectAction::MoveUp);
    }
    if ui
        .add_enabled(index + 1 < count, crate::button_widget("Move down"))
        .clicked()
    {
        action = Some(LaneEffectAction::MoveDown);
    }
    for (other, name) in lanes.iter().enumerate().filter(|(other, _)| *other != lane) {
        if crate::button(ui, format!("Move to {name}")).clicked() {
            action = Some(LaneEffectAction::MoveToLane(other));
        }
    }
    ui.separator();
    if crate::button(ui, "Remove").clicked() {
        action = Some(LaneEffectAction::Remove);
    }
    if action.is_some() {
        ui.close();
    }
    action
}

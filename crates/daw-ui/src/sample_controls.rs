//! Sample identity, layer selection and reusable sample actions.
//! These controls hold no file, document or playback state.
use crate::{text::TextRole, tokens};
use egui::Ui;

/// A sample operation requested by a shared toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleAction {
    Browse,
    LoadFile,
    Clear,
    TrimSilence,
    ReverseAudibleRegion,
    Export,
}

/// A concise file/library entry point and a menu of operations on a recording.
pub fn sample_actions(ui: &mut Ui, loaded: bool, audible: bool) -> Option<SampleAction> {
    let mut action = None;
    if crate::button(ui, "Browse…")
        .on_hover_text("Choose a project or library sample")
        .clicked()
    {
        action = Some(SampleAction::Browse);
    }
    crate::choice_menu(ui, "Sample actions", |ui| {
        crate::command_menu_contents(ui, |ui| {
            for (label, choice, enabled) in [
                ("Load file…", SampleAction::LoadFile, true),
                ("Trim silence", SampleAction::TrimSilence, loaded && audible),
                (
                    "Reverse audible region",
                    SampleAction::ReverseAudibleRegion,
                    loaded && audible,
                ),
                ("Export sample…", SampleAction::Export, loaded),
                ("Clear sample", SampleAction::Clear, loaded),
            ] {
                if ui
                    .add_enabled_ui(enabled, |ui| crate::menu_item(ui, label, None))
                    .inner
                    .clicked()
                {
                    action = Some(choice);
                    ui.close();
                }
            }
        });
    });
    action
}

/// Explicit naming with the shared text editor.
/// Empty names ask the owner to restore its automatic source-derived label.
pub fn sample_name_editor(ui: &mut Ui, current: &str, hint: &str) -> Option<String> {
    let button = crate::button(ui, "Rename…");
    let id = button.id.with("name");
    let mut draft = if button.clicked() {
        current.to_owned()
    } else {
        ui.data(|data| data.get_temp::<String>(id))
            .unwrap_or_default()
    };
    let mut renamed = None;
    egui::Popup::menu(&button)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let output =
                crate::inline_name_editor(ui, id, &mut draft, hint, 180.0, button.clicked());
            match output.action {
                crate::InlineNameAction::Commit => {
                    renamed = Some(draft.trim().to_owned());
                    ui.close();
                }
                crate::InlineNameAction::Cancel => ui.close(),
                crate::InlineNameAction::Editing => {}
            }
        });
    ui.data_mut(|data| data.insert_temp(id, draft));
    renamed
}

/// A bounded source-name line that never pushes sample actions out of a card.
pub fn sample_identity(ui: &mut Ui, name: &str, detail: &str) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), tokens::CONTROL_HEIGHT),
        egui::Sense::hover(),
    );
    let detail_width = (ui
        .painter()
        .layout_no_wrap(
            detail.to_owned(),
            TextRole::Note.font(),
            crate::theme::palette(ui.ctx()).muted,
        )
        .size()
        .x
        + tokens::SPACE_XS)
        .min(rect.width() * 0.5);
    let name_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(
            rect.right() - detail_width - tokens::SPACE_SM,
            rect.bottom(),
        ),
    );
    let detail_rect = egui::Rect::from_min_max(
        egui::pos2(rect.right() - detail_width, rect.top()),
        rect.max,
    );
    crate::text::paint_row(ui.painter(), name_rect, TextRole::Label, name);
    crate::text::paint_row(ui.painter(), detail_rect, TextRole::Note, detail);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            ui.is_enabled(),
            format!("{name} {detail}"),
        )
    });
    response.on_hover_text(name);
}

/// A slot name and context with actions that retain their width on narrow cards.
pub fn sample_slot_header(
    ui: &mut Ui,
    context: &str,
    name: &str,
    edit_player: bool,
) -> (Option<String>, bool) {
    let mut renamed = None;
    let mut edit = false;
    crate::control_row(ui, |ui| {
        crate::text::note(ui, context);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if edit_player {
                edit = crate::button(ui, "Edit player…").clicked();
            }
            renamed = sample_name_editor(ui, name, "Pad name");
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                crate::text::truncated(ui, TextRole::Label, name);
            });
        });
    });
    (renamed, edit)
}

#[derive(Default)]
pub struct SampleGroupOverviewOutput {
    pub edit: bool,
    pub file: Option<std::path::PathBuf>,
    pub source: Option<crate::SampleDrop>,
}

/// A composite has no single waveform. Its overview identifies the player and
/// makes adding a recording an explicit drop operation instead of replacement.
pub fn sample_group_overview(
    ui: &mut Ui,
    title: &str,
    samples: usize,
    loaded: usize,
    size: egui::Vec2,
) -> SampleGroupOverviewOutput {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let mut edit = false;
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    child.add_space(((size.y - 112.0) * 0.5).max(0.0));
    crate::text::label(&mut child, title);
    crate::text::note(
        &mut child,
        format!(
            "{samples} sample {} · {loaded} loaded",
            if samples == 1 { "player" } else { "players" }
        ),
    );
    if crate::button(&mut child, "Edit player…").clicked() {
        edit = true;
    }
    crate::text::note(&mut child, "Drop a sample here to add it");
    let dropped = response
        .dnd_release_payload::<crate::SampleDrop>()
        .map(|payload| (*payload).clone());
    let native = if ui
        .input(|input| input.pointer.hover_pos())
        .is_some_and(|at| rect.contains(at))
    {
        ui.input_mut(|input| std::mem::take(&mut input.raw.dropped_files))
            .into_iter()
            .find_map(|file| file.path)
    } else {
        None
    };
    SampleGroupOverviewOutput {
        edit,
        file: native,
        source: dropped,
    }
}

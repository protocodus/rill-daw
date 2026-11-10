//! Song markers and meter changes in the arranger: ruler flags, context menus,
//! navigation and the meter dialog. Every change is a typed song edit, so the
//! GUI, tools and Undo share one path.
use crate::app::DawApp;
use daw_core::{EditBatch, EditResult, EntityRef, SongEdit};
use eframe::egui;

const METER_DIALOG: &str = "meter_change_dialog";
const CONTEXT_BEAT: &str = "timeline_context_beat";

#[derive(Clone)]
struct MeterDraft {
    /// The existing change being edited, if any.
    original: Option<u32>,
    bar: f64,
    beats: u8,
    session: u64,
}

pub(crate) fn meter_dialog_open(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<MeterDraft>(egui::Id::new(METER_DIALOG)).is_some())
}

pub(crate) fn reset(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<MeterDraft>(egui::Id::new(METER_DIALOG)));
}

enum FlagCommand {
    GoTo(u64),
    Rename(u64),
    Delete(u64),
    EditMeter(u32),
    RemoveMeter(u32),
}

impl DawApp {
    fn timeline_edit(&mut self, label: &str, edits: Vec<SongEdit>) -> Option<EditResult> {
        let batch = EditBatch {
            label: label.into(),
            edits,
        };
        match daw_tools::SongToolHost::apply_edits(self, batch) {
            Ok(result) => Some(result),
            Err(error) => {
                self.message(Err(anyhow::anyhow!(error.message)), "");
                None
            }
        }
    }

    pub(crate) fn add_marker_at(&mut self, beat: f64) {
        let name = format!("Marker {}", self.project.markers.len() + 1);
        self.timeline_edit(
            "Add marker",
            vec![SongEdit::AddMarker {
                beat: beat.max(0.0),
                name,
                color: None,
                key: None,
            }],
        );
    }

    fn delete_marker(&mut self, id: u64) {
        self.timeline_edit(
            "Delete marker",
            vec![SongEdit::RemoveMarker {
                marker: EntityRef::Id(id),
            }],
        );
    }

    pub(crate) fn rename_marker(&mut self, id: u64, name: String) {
        self.timeline_edit(
            "Rename marker",
            vec![SongEdit::RenameMarker {
                marker: EntityRef::Id(id),
                name,
            }],
        );
    }

    pub(crate) fn go_to_adjacent_marker(&mut self, forward: bool) {
        if let Some(beat) = self
            .project
            .adjacent_marker(self.cursor, forward)
            .map(|marker| marker.beat)
        {
            self.seek(beat);
        }
    }

    /// Short meter description for the assistant context line.
    pub(crate) fn meter_summary(&self) -> String {
        let mut summary = format!("{}/4", self.project.beats_per_bar);
        for change in self.project.meter_changes.iter().take(3) {
            summary.push_str(&format!(
                ", {}/4 from bar {}",
                change.beats_per_bar, change.bar
            ));
        }
        if self.project.meter_changes.len() > 3 {
            summary.push_str(", …");
        }
        summary
    }

    pub(crate) fn open_meter_dialog(&self, ctx: &egui::Context, bar: u32) {
        let bar = bar.max(2);
        let existing = self
            .project
            .meter_changes
            .iter()
            .find(|change| change.bar == bar);
        let meter = self.project.meter_map();
        let draft = MeterDraft {
            original: existing.map(|change| change.bar),
            bar: f64::from(bar),
            beats: existing.map_or_else(
                || meter.meter_at(meter.bar_start(bar)),
                |change| change.beats_per_bar,
            ),
            session: self.song_session.id,
        };
        ctx.data_mut(|data| data.insert_temp(egui::Id::new(METER_DIALOG), draft));
    }

    pub(crate) fn meter_change_dialog(&mut self, ctx: &egui::Context) {
        let id = egui::Id::new(METER_DIALOG);
        let Some(mut draft) = ctx.data(|data| data.get_temp::<MeterDraft>(id)) else {
            return;
        };
        if draft.session != self.song_session.id
            || self.document_guard.is_some()
            || self.recording_busy()
            || draft
                .original
                .is_some_and(|bar| !self.project.meter_changes.iter().any(|c| c.bar == bar))
        {
            reset(ctx);
            return;
        }
        let (mut apply, mut remove, mut cancel) = (false, false, false);
        let title = if draft.original.is_some() {
            "Edit meter change"
        } else {
            "Insert meter change"
        };
        let response = daw_ui::modal(ctx, id.with("modal"), title, |ui| {
            daw_ui::form_grid(ui, "meter_change_fields", |ui| {
                daw_ui::form_label(ui, "Starts at bar");
                daw_ui::number_stepper_integer(
                    ui,
                    &mut draft.bar,
                    1.0,
                    2.0..=100_000.0,
                    "meter change bar",
                    |ui, value| ui.add(daw_ui::number(value).speed(0.1).range(2.0..=100_000.0)),
                );
                ui.end_row();
                daw_ui::form_label(ui, "Meter");
                daw_ui::meter_choice(ui, "meter_change_beats", &mut draft.beats);
                ui.end_row();
            });
            daw_ui::text::note(
                ui,
                "Later bars follow the new meter. Clips keep their beat positions.",
            );
            daw_ui::section_gap(ui);
            daw_ui::dialog_actions(
                ui,
                |ui| {
                    apply = daw_ui::dialog_primary(ui, "Apply meter change").clicked();
                    cancel = daw_ui::dialog_button(ui, "Cancel").clicked();
                    if draft.original.is_some() {
                        remove = daw_ui::dialog_button(ui, "Remove meter change").clicked();
                    }
                },
                |_| {},
            );
        });
        let escape =
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if cancel || (response.is_top_modal && escape) {
            reset(ctx);
            return;
        }
        let bar = draft.bar.round().clamp(2.0, f64::from(u32::MAX)) as u32;
        let edits = if remove {
            draft
                .original
                .map(|bar| vec![SongEdit::RemoveMeterChange { bar }])
        } else if apply {
            let mut edits = Vec::new();
            if let Some(original) = draft.original.filter(|original| *original != bar) {
                edits.push(SongEdit::MoveMeterChange {
                    from_bar: original,
                    to_bar: bar,
                });
            }
            edits.push(SongEdit::SetMeterChange {
                bar,
                beats_per_bar: draft.beats,
            });
            Some(edits)
        } else {
            None
        };
        match edits {
            Some(edits) => {
                let label = if remove {
                    "Remove meter change"
                } else {
                    "Meter change"
                };
                // A rejected change keeps the dialog open with its draft.
                if self.timeline_edit(label, edits).is_some() {
                    reset(ctx);
                } else {
                    ctx.data_mut(|data| data.insert_temp(id, draft));
                }
            }
            None => ctx.data_mut(|data| data.insert_temp(id, draft)),
        }
    }

    /// Context menu for the ruler and the flag lane, at the clicked beat.
    pub(crate) fn timeline_context_menu(
        &mut self,
        response: &egui::Response,
        beat_at: impl Fn(f32) -> f64,
    ) {
        let id = egui::Id::new(CONTEXT_BEAT);
        if response.secondary_clicked()
            && let Some(position) = response.interact_pointer_pos()
        {
            let beat = beat_at(position.x).max(0.0);
            response.ctx.data_mut(|data| data.insert_temp(id, beat));
        }
        response.context_menu(|ui| {
            let beat = ui.data(|data| data.get_temp::<f64>(id)).unwrap_or(self.cursor);
            let enabled = !self.recording_busy();
            if ui
                .add_enabled(enabled, daw_ui::button_widget("Add marker at playhead"))
                .clicked()
            {
                self.add_marker_at(self.cursor);
                ui.close();
            }
            if ui
                .add_enabled(enabled, daw_ui::button_widget("Add marker here"))
                .clicked()
            {
                self.add_marker_at(self.snapped(beat));
                ui.close();
            }
            let bar = self.project.meter_map().position(beat).0.max(2);
            if ui
                .add_enabled(enabled, daw_ui::button_widget("Insert meter change…"))
                .on_hover_text(format!("Change the meter from bar {bar}"))
                .clicked()
            {
                self.open_meter_dialog(ui.ctx(), bar);
                ui.close();
            }
            ui.separator();
            self.marker_navigation_items(ui);
        });
    }

    fn marker_navigation_items(&mut self, ui: &mut egui::Ui) {
        for (label, shortcut, forward) in [("Previous marker", ",", false), ("Next marker", ".", true)]
        {
            let available = self.project.adjacent_marker(self.cursor, forward).is_some();
            if daw_ui::menu_item_enabled(ui, available, label, Some(shortcut)).clicked() {
                self.go_to_adjacent_marker(forward);
                ui.close();
            }
        }
    }

    /// Edit → Markers.
    pub(crate) fn marker_menu(&mut self, ui: &mut egui::Ui) {
        daw_ui::menu_button(ui, "Markers", |ui| {
            let enabled = !self.recording_busy();
            if daw_ui::menu_item_enabled(ui, enabled, "Add marker at playhead", Some("M")).clicked()
            {
                self.add_marker_at(self.cursor);
                ui.close();
            }
            self.marker_navigation_items(ui);
            ui.separator();
            if ui
                .add_enabled(enabled, daw_ui::button_widget("Insert meter change at playhead…"))
                .clicked()
            {
                let bar = self.project.meter_map().position(self.cursor).0;
                self.open_meter_dialog(ui.ctx(), bar);
                ui.close();
            }
        });
    }

    /// M adds a marker; comma and period step between markers.
    pub(crate) fn marker_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if egui::Popup::is_any_open(ctx) {
            return false;
        }
        let (add, previous, next) = ctx.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::M),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Comma),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Period),
            )
        });
        if add && !self.recording_busy() {
            self.add_marker_at(self.cursor);
        }
        if previous {
            self.go_to_adjacent_marker(false);
        }
        if next {
            self.go_to_adjacent_marker(true);
        }
        add || previous || next
    }

    /// Paint the arranger's marker and meter lane and apply its gestures.
    /// Returns diagnostics for each flag.
    pub(crate) fn timeline_flag_lane(
        &mut self,
        ui: &mut egui::Ui,
        lane: egui::Rect,
        view: daw_ui::TimelineViewport,
        enabled: bool,
    ) -> serde_json::Value {
        let meter = self.project.meter_map();
        let mut flags: Vec<daw_ui::TimelineFlag> = self
            .project
            .meter_changes
            .iter()
            .map(|change| daw_ui::TimelineFlag {
                key: u64::from(change.bar),
                kind: daw_ui::TimelineFlagKind::Meter,
                beat: meter.bar_start(change.bar),
                label: format!("{}/4", change.beats_per_bar),
                hover: format!(
                    "{}/4 from bar {}\nClick to edit or remove",
                    change.beats_per_bar, change.bar
                ),
                color: None,
            })
            .collect();
        flags.extend(self.project.markers.iter().map(|marker| daw_ui::TimelineFlag {
            key: marker.id,
            kind: daw_ui::TimelineFlagKind::Marker,
            beat: marker.beat,
            label: marker.name.clone(),
            hover: format!(
                "{} · {}\nClick to go there · Drag to move · Double-click to rename",
                marker.name,
                daw_ui::musical_position_label(marker.beat, &meter)
            ),
            color: marker
                .color
                .map(|[r, g, b]| egui::Color32::from_rgb(r, g, b)),
        }));
        let mut command = None;
        let snap = |beat: f64, bypass: bool| if bypass { beat } else { self.snapped(beat) };
        let mut controls = ui.new_child(egui::UiBuilder::new().max_rect(lane));
        let response = daw_ui::timeline_flags(
            &mut controls,
            egui::Id::new("arrangement_flags"),
            lane,
            view,
            &flags,
            enabled && !self.recording_busy(),
            &snap,
            |ui, flag| match flag.kind {
                daw_ui::TimelineFlagKind::Marker => {
                    for (label, action) in [
                        ("Go to marker", FlagCommand::GoTo(flag.key)),
                        ("Rename marker…", FlagCommand::Rename(flag.key)),
                        ("Delete marker", FlagCommand::Delete(flag.key)),
                    ] {
                        if daw_ui::button(ui, label).clicked() {
                            command = Some(action);
                            ui.close();
                        }
                    }
                }
                daw_ui::TimelineFlagKind::Meter => {
                    let bar = flag.key as u32;
                    for (label, action) in [
                        ("Edit meter change…", FlagCommand::EditMeter(bar)),
                        ("Remove meter change", FlagCommand::RemoveMeter(bar)),
                    ] {
                        if daw_ui::button(ui, label).clicked() {
                            command = Some(action);
                            ui.close();
                        }
                    }
                }
            },
        );
        let command = command.or(match response.action {
            Some(daw_ui::TimelineFlagAction::Clicked(daw_ui::TimelineFlagKind::Marker, id)) => {
                Some(FlagCommand::GoTo(id))
            }
            Some(daw_ui::TimelineFlagAction::DoubleClicked(
                daw_ui::TimelineFlagKind::Marker,
                id,
            )) => Some(FlagCommand::Rename(id)),
            Some(
                daw_ui::TimelineFlagAction::Clicked(daw_ui::TimelineFlagKind::Meter, bar)
                | daw_ui::TimelineFlagAction::DoubleClicked(daw_ui::TimelineFlagKind::Meter, bar),
            ) => Some(FlagCommand::EditMeter(bar as u32)),
            Some(daw_ui::TimelineFlagAction::Moved(id, beat)) => {
                self.timeline_edit(
                    "Move marker",
                    vec![SongEdit::MoveMarker {
                        marker: EntityRef::Id(id),
                        beat,
                    }],
                );
                None
            }
            None => None,
        });
        match command {
            Some(FlagCommand::GoTo(id)) => {
                if let Some(beat) = self.project.marker(id).map(|marker| marker.beat) {
                    self.seek(beat);
                }
            }
            Some(FlagCommand::Rename(id)) => self.start_marker_rename(ui.ctx(), id),
            Some(FlagCommand::Delete(id)) => self.delete_marker(id),
            Some(FlagCommand::EditMeter(bar)) => self.open_meter_dialog(ui.ctx(), bar),
            Some(FlagCommand::RemoveMeter(bar)) => {
                self.timeline_edit(
                    "Remove meter change",
                    vec![SongEdit::RemoveMeterChange { bar }],
                );
            }
            None => {}
        }
        if response.lane.clicked()
            && let Some(position) = response.lane.interact_pointer_pos()
        {
            self.seek(self.snapped(view.beat_at(lane, position.x)).max(0.0));
        }
        self.timeline_context_menu(&response.lane, |x| view.beat_at(lane, x));
        let rect_json = |rect: egui::Rect| {
            serde_json::json!({"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]})
        };
        serde_json::json!({
            "lane": rect_json(lane),
            "flags": response.flags.iter().map(|(kind, key, rect)| serde_json::json!({
                "kind": match kind {
                    daw_ui::TimelineFlagKind::Marker => "marker",
                    daw_ui::TimelineFlagKind::Meter => "meter",
                },
                "key": key,
                "rect": rect_json(*rect),
            })).collect::<Vec<_>>(),
        })
    }
}

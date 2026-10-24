use crate::app::DawApp;
use daw_core::{
    AutomationParameter, AutomationTarget, Effect, Instrument, Interpolation, TrackKind,
};
use daw_ui::{CurveEdit, CurvePoint, CurveTimeline, TimelineViewport};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

type ParameterResult = Arc<Mutex<Option<Result<Vec<daw_plugins::ParameterInfo>, String>>>>;

#[derive(Clone)]
struct EditorState {
    track: Option<u64>,
    creating: bool,
    source: Option<u64>,
    parameter: usize,
    point: Option<u64>,
    viewport: TimelineViewport,
    fitted: bool,
    vst_id: u32,
    initial: f32,
    parameter_path: Option<String>,
    parameter_class_id: Option<String>,
    parameters: Vec<daw_plugins::ParameterInfo>,
    loading: Option<ParameterResult>,
    load_error: Option<String>,
    manual_id: bool,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            track: None,
            creating: false,
            source: None,
            parameter: 0,
            point: None,
            viewport: TimelineViewport::default(),
            fitted: false,
            vst_id: 0,
            initial: 0.5,
            parameter_path: None,
            parameter_class_id: None,
            parameters: Vec::new(),
            loading: None,
            load_error: None,
            manual_id: false,
        }
    }
}

pub(crate) fn reset_automation_editor(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<EditorState>(egui::Id::new("automation_editor_state"));
        data.remove::<serde_json::Value>(egui::Id::new("automation_editor_diagnostics"));
        data.remove::<egui::Rect>(egui::Id::new("automation_plot_rect"));
        data.remove::<egui::Id>(egui::Id::new("automation_curve_focus"));
    });
}

/// A parameter chooser is a page of the current editor, so dismissing it must
/// restore that editor before closing the containing utility.
pub(crate) fn cancel_automation_creation(ctx: &egui::Context) -> bool {
    ctx.data_mut(|data| {
        let key = egui::Id::new("automation_editor_state");
        let Some(mut state) = data.get_temp::<EditorState>(key) else {
            return false;
        };
        if !state.creating {
            return false;
        }
        state.creating = false;
        data.insert_temp(key, state);
        true
    })
}

impl DawApp {
    pub(crate) fn start_automation_creation(&mut self, ctx: &egui::Context) {
        let Some(lane) = self
            .selected_track
            .and_then(|id| self.project.track(id))
            .and_then(|track| track.automation.as_ref())
        else {
            return;
        };
        let key = egui::Id::new("automation_editor_state");
        let mut state = ctx
            .data_mut(|data| data.get_temp::<EditorState>(key))
            .filter(|state| state.track == self.selected_track)
            .unwrap_or_else(|| EditorState {
                track: self.selected_track,
                point: lane.points.first().map(|point| point.id),
                ..Default::default()
            });
        state.creating = true;
        state.source = Some(lane.target.track);
        state.parameter = 0;
        ctx.data_mut(|data| data.insert_temp(key, state));
    }

    pub fn automation_editor(&mut self, ctx: &egui::Context) {
        if !self.automation_open {
            return;
        }
        let key = egui::Id::new("automation_editor_state");
        let mut state = ctx
            .data_mut(|data| data.get_temp::<EditorState>(key))
            .unwrap_or_default();
        if state.track != self.selected_track {
            state = EditorState {
                track: self.selected_track,
                source: self.selected_track,
                ..Default::default()
            };
            if let Some(lane) = self
                .selected_track
                .and_then(|id| self.project.track(id))
                .and_then(|t| t.automation.as_ref())
            {
                state.point = lane.points.first().map(|p| p.id);
            }
        }
        let existing = self
            .selected_track
            .and_then(|id| self.project.track(id))
            .is_some_and(|t| t.kind == TrackKind::Automation);
        let editing = existing && !state.creating;
        let mut open = true;
        let mut cancel = false;
        let popup = if editing {
            daw_ui::workspace_popup("Automation", ctx).id(egui::Id::new("automation_editor"))
        } else {
            daw_ui::popup("Add automation", ctx)
                .id(egui::Id::new("automation_create"))
                .default_width(540.0)
                .vscroll(false)
        };
        popup.open(&mut open).show(ctx, |ui| {
            if editing {
                self.edit_automation_curve(ui, &mut state);
            } else {
                ctx.data_mut(|data| {
                    data.remove::<serde_json::Value>(egui::Id::new(
                        "automation_editor_diagnostics",
                    ));
                    data.remove::<egui::Rect>(egui::Id::new("automation_plot_rect"));
                    data.remove::<egui::Id>(egui::Id::new("automation_curve_focus"));
                });
                cancel = daw_ui::scroll_body(ui, "automation_create_body", |ui| {
                    self.create_automation_lane(ui, &mut state)
                })
                .inner;
            }
        });
        if !open || cancel {
            if existing && state.creating {
                state.creating = false;
            } else {
                self.automation_open = false;
            }
        }
        ctx.data_mut(|data| data.insert_temp(key, state));
    }

    fn create_automation_lane(&mut self, ui: &mut egui::Ui, state: &mut EditorState) -> bool {
        let back = if state.creating { "Back" } else { "Cancel" };
        let tracks: Vec<_> = self
            .project
            .tracks
            .iter()
            .filter(|t| matches!(t.kind, TrackKind::Audio | TrackKind::Midi | TrackKind::Bus))
            .map(|t| (t.id, t.name.clone()))
            .collect();
        if !tracks.iter().any(|(id, _)| Some(*id) == state.source) {
            state.source = tracks.first().map(|(id, _)| *id);
        }
        let Some(source) = state.source else {
            ui.label("Add an audio, instrument, or bus track first.");
            ui.add_space(daw_ui::tokens::SPACE_LG);
            return daw_ui::button(ui, back).clicked();
        };
        let mut selected_source = Some(source);
        let control_width =
            (ui.available_width() - daw_ui::tokens::SELECT_WIDTH - daw_ui::tokens::SPACE_LG)
                .clamp(160.0, 320.0);
        daw_ui::form_grid(ui, "automation_source", |ui| {
            daw_ui::text::label(ui, "Track");
            daw_ui::select(
                "automation_source_choice",
                tracks
                    .iter()
                    .find(|(id, _)| *id == source)
                    .unwrap()
                    .1
                    .as_str(),
            )
            .width(control_width)
            .truncate()
            .show_ui(ui, |ui| {
                ui.set_max_width(control_width);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                for (id, name) in &tracks {
                    daw_ui::selectable_value(ui, &mut selected_source, Some(*id), name);
                }
            });
            ui.end_row();
        });
        if selected_source != state.source {
            state.source = selected_source;
            state.parameter = 0;
        }
        let source = state.source.unwrap();
        let track = self.project.track(source).unwrap();
        let mut choices = vec![
            (AutomationParameter::TrackGain, "Volume".to_owned()),
            (AutomationParameter::TrackPan, "Pan".to_owned()),
        ];
        for send in &track.sends {
            let name = self
                .project
                .track(send.target_bus)
                .map(|t| t.name.as_str())
                .unwrap_or("Bus");
            choices.push((
                AutomationParameter::SendGain {
                    bus: send.target_bus,
                },
                format!("Send · {name}"),
            ));
        }
        for (index, slot) in track.inserts.iter().enumerate() {
            let prefix = format!("{:02} · {}", index + 1, slot.name);
            choices.push((
                AutomationParameter::VstParameter {
                    effect: Some(slot.id),
                    id: state.vst_id,
                },
                format!("{prefix} · Parameter"),
            ));
        }

        if matches!(track.instrument, Instrument::Vst3 { .. }) && track.kind == TrackKind::Midi {
            choices.push((
                AutomationParameter::VstParameter {
                    effect: None,
                    id: state.vst_id,
                },
                "VST instrument parameter".into(),
            ));
        }
        state.parameter = state.parameter.min(choices.len() - 1);
        daw_ui::form_grid(ui, "automation_parameter", |ui| {
            daw_ui::text::label(ui, "Parameter");
            daw_ui::select("automation_parameter_choice", &choices[state.parameter].1)
                .width(control_width)
                .truncate()
                .show_ui(ui, |ui| {
                    ui.set_max_width(control_width);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    for (index, (_, label)) in choices.iter().enumerate() {
                        daw_ui::selectable_value(ui, &mut state.parameter, index, label);
                    }
                });
            ui.end_row();
        });
        let (mut parameter, mut label) = choices[state.parameter].clone();
        if let AutomationParameter::VstParameter { effect, .. } = parameter {
            let path = effect
                .and_then(|id| track.inserts.iter().find(|slot| slot.id == id))
                .map(|slot| {
                    let Effect::Vst3 { path, class_id } = &slot.effect;
                    (path.clone(), class_id.clone())
                })
                .or_else(|| match &track.instrument {
                    Instrument::Vst3 { path, class_id } => Some((path.clone(), class_id.clone())),
                    _ => None,
                });
            if let Some((path, class_id)) = path {
                plugin_parameters(ui, state, path, class_id, effect.is_some());
            }
            parameter = AutomationParameter::VstParameter {
                effect,
                id: state.vst_id,
            };
            label = if let Some(parameter) = state.parameters.iter().find(|p| p.id == state.vst_id)
            {
                if let Some(slot) =
                    effect.and_then(|id| track.inserts.iter().find(|slot| slot.id == id))
                {
                    format!("{} · {}", slot.name, parameter.name)
                } else {
                    parameter.name.clone()
                }
            } else {
                format!("{label} · ID {}", state.vst_id)
            };
        }
        ui.add_space(daw_ui::tokens::SPACE_LG);
        let vst_parameter = matches!(parameter, AutomationParameter::VstParameter { .. });
        let ready = !vst_parameter
            || state.manual_id
            || (state.loading.is_none() && state.parameters.iter().any(|p| p.id == state.vst_id));
        let target = AutomationTarget {
            track: source,
            parameter,
        };
        let existing = self.project.tracks.iter().find_map(|track| {
            track
                .automation
                .as_ref()
                .filter(|lane| lane.target == target)
                .map(|_| track.id)
        });
        let action = if existing.is_some() {
            "Edit automation"
        } else {
            "Add automation track"
        };
        let (activate, cancel) = daw_ui::action_row(
            ui,
            |ui| {
                ui.add_enabled_ui(ready, |ui| daw_ui::primary_button(ui, action))
                    .inner
                    .clicked()
            },
            |ui| daw_ui::button(ui, back).clicked(),
        )
        .inner;
        if activate {
            if let Some(id) = existing {
                self.selected_track = Some(id);
                self.selected_clip = None;
                state.creating = false;
                return false;
            }
            let name = format!("{} · {}", track.name, label);
            self.begin_edit();
            let result = if matches!(target.parameter, AutomationParameter::VstParameter { .. }) {
                self.project
                    .add_automation_track_with_value(name, target, state.initial)
            } else {
                self.project.add_automation_track(name, target)
            };
            match result {
                Ok(id) => {
                    self.selected_track = Some(id);
                    self.selected_clip = None;
                    self.changed();
                    self.message(Ok(()), "Automation track added");
                }
                Err(error) => self.message(Err(error), ""),
            }
        }
        // Keep space for a bounded probe diagnostic while loading. Otherwise a
        // centered popup would move its footer under an in-flight pointer click.
        if vst_parameter {
            ui.allocate_ui(
                egui::vec2(ui.available_width(), daw_ui::tokens::CONTROL_HEIGHT),
                |ui| {
                    if let Some(error) = &state.load_error {
                        daw_ui::text::truncated(ui, daw_ui::text::TextRole::Note, error);
                    }
                },
            );
        }
        cancel
    }

    fn edit_automation_curve(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let Some(id) = self.selected_track else {
            return;
        };
        let Some(track) = self.project.track(id) else {
            return;
        };
        let Some(lane) = track.automation.clone() else {
            return;
        };
        daw_ui::text::truncated(ui, daw_ui::text::TextRole::Subtitle, &track.name);
        let mut interpolation = lane.interpolation;
        let mut enabled = !track.mute;
        let mut add_parameter = false;
        let mut fit = !state.fitted;
        daw_ui::toolbar(ui, |ui| {
            for (value, name) in [
                (Interpolation::Step, "Step"),
                (Interpolation::Linear, "Linear"),
                (Interpolation::Smooth, "Smooth"),
            ] {
                daw_ui::selectable_value(ui, &mut interpolation, value, name);
            }
            ui.separator();
            daw_ui::checkbox(ui, &mut enabled, "Enabled");
            fit |= daw_ui::timeline_fit_button(ui).clicked();
            add_parameter = daw_ui::button(ui, "Add parameter").clicked();
        });
        if add_parameter {
            state.creating = true;
            state.source = Some(lane.target.track);
            state.parameter = 0;
            return;
        }
        if interpolation != lane.interpolation || enabled == track.mute {
            self.begin_edit();
            let track = self.project.track_mut(id).unwrap();
            track.mute = !enabled;
            track.automation.as_mut().unwrap().interpolation = interpolation;
            self.changed();
        }
        let points: Vec<_> = lane
            .points
            .iter()
            .map(|p| CurvePoint {
                id: p.id,
                x: p.beat,
                value: p.value,
            })
            .collect();
        let response = daw_ui::curve_timeline_editor(
            ui,
            egui::Id::new(("automation_curve", id)),
            &points,
            &mut state.point,
            &mut state.viewport,
            CurveTimeline {
                snap: self.snap,
                height: (ui.available_height() - 66.0).max(160.0),
                tempo: self.project.tempo,
                beats_per_bar: self.project.beats_per_bar,
                fit,
            },
            |beat| lane.value_at(beat).unwrap_or(0.5),
        );
        state.fitted = true;
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("automation_plot_rect"), response.plot_rect);
            data.insert_temp(
                egui::Id::new("automation_curve_focus"),
                response.response.id,
            );
        });
        let mut edit = response.edit;
        let mut point_beat_rect = None;
        let mut displayed_beat = None;
        daw_ui::toolbar(ui, |ui| {
            if let Some(point) = state
                .point
                .and_then(|selected| lane.points.iter().find(|p| p.id == selected))
            {
                let mut beat = point.beat + 1.0;
                let mut value = lane.target.parameter.denormalize(point.value);
                let beat_response = ui
                    .add(
                        daw_ui::number(&mut beat)
                            .prefix("Beat ")
                            .range(1.0..=(daw_core::MAX_BEAT + 1.0))
                            .min_decimals(2)
                            .max_decimals(3)
                            .speed(self.snap.max(0.01)),
                    )
                    .on_hover_text("Musical beat number from one · drag or double-click to type");
                point_beat_rect = Some(beat_response.rect);
                displayed_beat = Some(beat);
                let range = lane.target.parameter.range();
                let speed = (*range.end() - *range.start()) as f64 / 200.0;
                let (prefix, suffix, tooltip) = match lane.target.parameter {
                    AutomationParameter::TrackGain | AutomationParameter::SendGain { .. } => (
                        "Gain ",
                        " ×",
                        "Linear gain multiplier: 1× is unity, 0× is silence",
                    ),
                    AutomationParameter::TrackPan => ("Pan ", "", "−1 left · 0 center · 1 right"),
                    AutomationParameter::VstParameter { .. } => (
                        "Value ",
                        "",
                        "Normalized plug-in parameter value from 0 to 1",
                    ),
                };
                let value_changed = ui
                    .add(
                        daw_ui::number(&mut value)
                            .range(range)
                            .speed(speed)
                            .prefix(prefix)
                            .suffix(suffix),
                    )
                    .on_hover_text(tooltip)
                    .changed();
                if beat_response.changed() || value_changed {
                    edit = Some(CurveEdit::Move {
                        id: point.id,
                        x: beat - 1.0,
                        value: lane.target.parameter.normalize(value),
                    });
                }
                if daw_ui::button(ui, "Delete point").clicked() {
                    edit = Some(CurveEdit::Delete { id: point.id });
                }
            } else {
                ui.label("Click the curve to add a point");
            }
        });
        let rect = |rect: egui::Rect| serde_json::json!({"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]});
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("automation_editor_diagnostics"), serde_json::json!({
            "plot":rect(response.plot_rect), "start_beat":state.viewport.start_beat,
            "pixels_per_beat":state.viewport.pixels_per_beat,
            "end_beat":state.viewport.beat_at(response.plot_rect,response.plot_rect.right()),
            "point_beat_rect":point_beat_rect.map(rect), "point_beat":displayed_beat,
            "navigating":response.navigation.navigating,
        }))
        });
        if let Some(edit) = edit {
            self.begin_edit();
            let result = match edit {
                CurveEdit::Add { x, value } => self
                    .project
                    .upsert_automation_point(id, x, value)
                    .map(|point| state.point = Some(point)),
                CurveEdit::Move {
                    id: point,
                    x,
                    value,
                } => self.project.move_automation_point(id, point, x, value),
                CurveEdit::Delete { id: point } => self
                    .project
                    .remove_automation_point(id, point)
                    .map(|_| state.point = None),
            };
            if result.is_ok() {
                self.changed();
            }
            self.message(result, "");
        }
    }
}

fn plugin_parameters(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    path: String,
    class_id: Option<String>,
    effect: bool,
) {
    if state.parameter_path.as_ref() != Some(&path) || state.parameter_class_id != class_id {
        state.parameter_path = Some(path.clone());
        state.parameter_class_id = class_id.clone();
        state.parameters.clear();
        state.loading = None;
        state.load_error = None;
        state.manual_id = false;
        start_parameter_read(state, path.clone(), class_id.clone(), effect);
    }
    if let Some(loading) = &state.loading {
        let ready = loading.lock().ok().and_then(|mut result| result.take());
        if let Some(result) = ready {
            state.loading = None;
            match result {
                Ok(parameters) => {
                    state.parameters = parameters.into_iter().filter(|p| !p.read_only).collect();
                    if !state.manual_id
                        && let Some(parameter) = state
                            .parameters
                            .iter()
                            .find(|p| p.id == state.vst_id)
                            .or_else(|| state.parameters.first())
                    {
                        state.vst_id = parameter.id;
                        state.initial = parameter.value as f32;
                    }
                }
                Err(error) => state.load_error = Some(error),
            }
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }
    daw_ui::toolbar(ui, |ui| {
        daw_ui::picker_button(ui, "Manual ID", |ui| {
            daw_ui::checkbox(ui, &mut state.manual_id, "Use manual ID");
            ui.label("VST parameter ID");
            ui.add_enabled(
                state.manual_id,
                daw_ui::number(&mut state.vst_id).range(0..=u32::MAX),
            );
        });
        if ui
            .add_enabled(
                state.loading.is_none(),
                daw_ui::button_widget(if state.loading.is_some() {
                    "Loading…"
                } else {
                    "Read parameters"
                }),
            )
            .clicked()
        {
            start_parameter_read(state, path, class_id.clone(), effect);
        }
    });
    if !state.manual_id && !state.parameters.is_empty() {
        let selected = state
            .parameters
            .iter()
            .find(|p| p.id == state.vst_id)
            .map(|p| p.name.as_str())
            .unwrap_or("Choose plug-in parameter");
        let width =
            (ui.available_width() - daw_ui::tokens::SELECT_WIDTH - daw_ui::tokens::SPACE_LG)
                .clamp(160.0, 320.0);
        daw_ui::form_grid(ui, "plugin_parameter", |ui| {
            daw_ui::text::label(ui, "Plug-in parameter");
            daw_ui::select("plugin_parameter_choice", selected)
                .width(width)
                .truncate()
                .show_ui(ui, |ui| {
                    ui.set_max_width(width);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    for parameter in &state.parameters {
                        if daw_ui::selectable_value(
                            ui,
                            &mut state.vst_id,
                            parameter.id,
                            &parameter.name,
                        )
                        .changed()
                        {
                            state.initial = parameter.value as f32;
                            state.manual_id = false;
                        }
                    }
                });
            ui.end_row();
        });
    }
    ui.add(daw_ui::slider(&mut state.initial, 0.0..=1.0).text("Initial value"));
}

fn start_parameter_read(
    state: &mut EditorState,
    path: String,
    class_id: Option<String>,
    effect: bool,
) {
    let result: ParameterResult = Arc::new(Mutex::new(None));
    state.loading = Some(result.clone());
    state.load_error = None;
    std::thread::spawn(move || {
        let result_value = (|| -> anyhow::Result<_> {
            if effect {
                daw_plugins::VstEffect::load_class(
                    &PathBuf::from(path),
                    class_id.as_deref(),
                    48_000.0,
                    256,
                )?
                .parameters()
            } else {
                daw_plugins::VstInstrument::load_class(
                    &PathBuf::from(path),
                    class_id.as_deref(),
                    48_000.0,
                    256,
                )?
                .parameters()
            }
        })()
        .map_err(|error| format!("{error:#}"));
        if let Ok(mut result) = result.lock() {
            *result = Some(result_value);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        app: &mut DawApp,
        state: &mut EditorState,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.create_automation_lane(ui, state));
            },
        )
    }

    fn button_position(output: &egui::FullOutput) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text() == "Add automation track"
                {
                    return Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center());
                }
                None
            })
            .expect("add lane button")
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn creation_popup_hugs_short_content_and_aligns_its_footer() {
        for appearance in [
            daw_ui::theme::Appearance::Light,
            daw_ui::theme::Appearance::Dark,
        ] {
            let ctx = egui::Context::default();
            daw_ui::theme::set_appearance(&ctx, appearance);
            let mut project = daw_core::Project::new();
            let source = project.add_audio_track("Glass keys");
            let mut app = DawApp::with_context(&ctx, project, None);
            app.selected_track = Some(source);
            app.automation_open = true;
            let mut output = egui::FullOutput::default();
            for _ in 0..4 {
                output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100.0, 720.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.automation_editor(ctx),
                );
            }
            let window = ctx
                .memory(|memory| memory.area_rect(egui::Id::new("automation_create")))
                .expect("creation window");
            assert!(
                window.height() < 300.0,
                "Short form must hug its content: {window:?}"
            );
            let label_rect = |label| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            Some(text.galley.rect.translate(text.pos.to_vec2()))
                        }
                        _ => None,
                    })
                    .unwrap()
            };
            let add = label_rect("Add automation track");
            let cancel = label_rect("Cancel");
            assert!((add.center().y - cancel.center().y).abs() < 1.0);
            assert!(cancel.right() < add.left());
            assert!(
                window.bottom() - add.bottom() < 50.0,
                "No large blank region below the footer"
            );
        }
    }

    #[test]
    fn vst_creation_waits_for_metadata_then_uses_parameter_name_and_current_value() {
        let ctx = egui::Context::default();
        let mut project = daw_core::Project::new();
        let track = project.add_midi_track("Synth");
        project.track_mut(track).unwrap().instrument = Instrument::Vst3 {
            path: "fixture.vst3".into(),
            class_id: None,
        };
        let mut app = DawApp::with_context(&ctx, project, None);
        let pending: ParameterResult = Arc::new(Mutex::new(None));
        let mut state = EditorState {
            track: Some(track),
            source: Some(track),
            parameter: 2,
            parameter_path: Some("fixture.vst3".into()),
            loading: Some(pending.clone()),
            ..Default::default()
        };
        let first = frame(&mut app, &mut state, &ctx, vec![]);
        let position = button_position(&first);
        frame(&mut app, &mut state, &ctx, pointer(position, true));
        frame(&mut app, &mut state, &ctx, pointer(position, false));
        assert_eq!(
            app.project.tracks.len(),
            1,
            "pending metadata must disable creation"
        );
        *pending.lock().unwrap() = Some(Ok(vec![
            daw_plugins::ParameterInfo {
                id: 7,
                name: "Meter".into(),
                value: 0.0,
                default: 0.0,
                unit: String::new(),
                steps: 0,
                read_only: true,
            },
            daw_plugins::ParameterInfo {
                id: 91,
                name: "Cutoff".into(),
                value: 0.25,
                default: 0.5,
                unit: "Hz".into(),
                steps: 0,
                read_only: false,
            },
        ]));
        let ready = frame(&mut app, &mut state, &ctx, vec![]);
        assert_eq!(
            state.parameters.len(),
            1,
            "read-only parameters are excluded"
        );
        assert_eq!(state.vst_id, 91);
        assert_eq!(state.initial, 0.25);
        let position = button_position(&ready);
        frame(&mut app, &mut state, &ctx, pointer(position, true));
        frame(&mut app, &mut state, &ctx, pointer(position, false));
        assert_eq!(app.project.tracks.len(), 2);
        let track = &app.project.tracks[1];
        assert_eq!(track.name, "Synth · Cutoff");
        let lane = track.automation.as_ref().unwrap();
        assert_eq!(
            lane.target.parameter,
            AutomationParameter::VstParameter {
                effect: None,
                id: 91
            }
        );
        assert!(lane.points.iter().all(|p| p.value == 0.25));
    }

    #[test]
    fn manual_parameter_add_stays_clickable_when_metadata_finishes_mid_click() {
        for result in [
            Err("The plug-in could not be loaded".to_owned()),
            Ok(vec![daw_plugins::ParameterInfo {
                id: 91,
                name: "Cutoff".into(),
                value: 0.25,
                default: 0.5,
                unit: "Hz".into(),
                steps: 0,
                read_only: false,
            }]),
        ] {
            let ctx = egui::Context::default();
            let mut project = daw_core::Project::new();
            let track = project.add_midi_track("Synth");
            project.track_mut(track).unwrap().instrument = Instrument::Vst3 {
                path: "fixture.vst3".into(),
                class_id: None,
            };
            let mut app = DawApp::with_context(&ctx, project, None);
            let pending: ParameterResult = Arc::new(Mutex::new(None));
            let mut state = EditorState {
                track: Some(track),
                source: Some(track),
                parameter: 2,
                parameter_path: Some("fixture.vst3".into()),
                loading: Some(pending.clone()),
                manual_id: true,
                vst_id: 17,
                initial: 0.75,
                ..Default::default()
            };
            let pending_frame = frame(&mut app, &mut state, &ctx, vec![]);
            let position = button_position(&pending_frame);
            frame(&mut app, &mut state, &ctx, pointer(position, true));
            *pending.lock().unwrap() = Some(result);
            let ready_frame = frame(&mut app, &mut state, &ctx, pointer(position, false));
            assert!(button_position(&ready_frame).distance(position) <= 1.0);
            assert_eq!(app.project.tracks.len(), 2);
            let lane = app.project.tracks[1].automation.as_ref().unwrap();
            assert_eq!(
                lane.target.parameter,
                AutomationParameter::VstParameter {
                    effect: None,
                    id: 17,
                }
            );
            assert!(lane.points.iter().all(|p| p.value == 0.75));
        }
    }
}

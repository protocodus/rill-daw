use crate::app::DawApp;
use daw_core::{
    AutomationParameter, AutomationTarget, Effect, Instrument, Interpolation, TrackKind,
};
use daw_ui::{CurveEdit, CurvePoint, CurveView};
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
    source: Option<u64>,
    parameter: usize,
    point: Option<u64>,
    end: f64,
    vst_id: u32,
    initial: f32,
    parameter_path: Option<String>,
    parameters: Vec<daw_plugins::ParameterInfo>,
    loading: Option<ParameterResult>,
    load_error: Option<String>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            track: None,
            source: None,
            parameter: 0,
            point: None,
            end: 8.0,
            vst_id: 0,
            initial: 0.5,
            parameter_path: None,
            parameters: Vec::new(),
            loading: None,
            load_error: None,
        }
    }
}

impl DawApp {
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
                state.end = lane.points.last().map(|p| p.beat).unwrap_or(8.0).max(8.0);
            }
        }
        let existing = self
            .selected_track
            .and_then(|id| self.project.track(id))
            .is_some_and(|t| t.kind == TrackKind::Automation);
        let mut open = true;
        let popup = if existing {
            daw_ui::workspace_popup("Automation", ctx)
        } else {
            daw_ui::popup("Automation").default_width(540.0)
        };
        popup
            .id(egui::Id::new("automation_editor"))
            .open(&mut open)
            .show(ctx, |ui| {
                if existing {
                    self.edit_automation_curve(ui, &mut state);
                } else {
                    self.create_automation_lane(ui, &mut state);
                }
            });
        if !open {
            self.automation_open = false;
        }
        ctx.data_mut(|data| data.insert_temp(key, state));
    }

    fn create_automation_lane(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
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
            return;
        };
        let mut selected_source = Some(source);
        daw_ui::select_labeled(
            "Track",
            tracks
                .iter()
                .find(|(id, _)| *id == source)
                .unwrap()
                .1
                .as_str(),
        )
        .show_ui(ui, |ui| {
            for (id, name) in &tracks {
                daw_ui::selectable_value(ui, &mut selected_source, Some(*id), name);
            }
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
            match slot.effect {
                Effect::Gain { .. } => choices.push((
                    AutomationParameter::InsertGain { effect: slot.id },
                    format!("{prefix} · Gain"),
                )),
                Effect::Delay { .. } => {
                    choices.push((
                        AutomationParameter::DelayTime { effect: slot.id },
                        format!("{prefix} · Time"),
                    ));
                    choices.push((
                        AutomationParameter::DelayFeedback { effect: slot.id },
                        format!("{prefix} · Feedback"),
                    ));
                    choices.push((
                        AutomationParameter::DelayMix { effect: slot.id },
                        format!("{prefix} · Mix"),
                    ));
                }
                Effect::Vst3 { .. } => choices.push((
                    AutomationParameter::VstParameter {
                        effect: Some(slot.id),
                        id: state.vst_id,
                    },
                    format!("{prefix} · Parameter"),
                )),
            }
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
        daw_ui::select_labeled("Parameter", &choices[state.parameter].1).show_ui(ui, |ui| {
            for (index, (_, label)) in choices.iter().enumerate() {
                daw_ui::selectable_value(ui, &mut state.parameter, index, label);
            }
        });
        let (mut parameter, label) = choices[state.parameter].clone();
        if let AutomationParameter::VstParameter { effect, .. } = parameter {
            let path = effect
                .and_then(|id| track.inserts.iter().find(|slot| slot.id == id))
                .and_then(|slot| match &slot.effect {
                    Effect::Vst3 { path } => Some(path.clone()),
                    _ => None,
                })
                .or_else(|| match &track.instrument {
                    Instrument::Vst3 { path } => Some(path.clone()),
                    _ => None,
                });
            if let Some(path) = path {
                plugin_parameters(ui, state, path, effect.is_some());
            }
            parameter = AutomationParameter::VstParameter {
                effect,
                id: state.vst_id,
            };
        }
        ui.add_space(10.0);
        if daw_ui::primary_button(ui, "Add automation track").clicked() {
            let target = AutomationTarget {
                track: source,
                parameter,
            };
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
        ui.label(&track.name);
        let mut interpolation = lane.interpolation;
        let mut enabled = !track.mute;
        let mut add_parameter = false;
        ui.horizontal(|ui| {
            for (value, name) in [
                (Interpolation::Step, "Step"),
                (Interpolation::Linear, "Linear"),
                (Interpolation::Smooth, "Smooth"),
            ] {
                daw_ui::selectable_value(ui, &mut interpolation, value, name);
            }
            ui.separator();
            daw_ui::checkbox(ui, &mut enabled, "Enabled");
            ui.label("End beat");
            ui.add(
                daw_ui::number(&mut state.end)
                    .range(1.0..=daw_core::MAX_BEAT)
                    .speed(1.0),
            );
            add_parameter = daw_ui::button(ui, "Add parameter").clicked();
        });
        if add_parameter {
            self.selected_track = Some(lane.target.track);
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
        let response = daw_ui::curve_editor(
            ui,
            egui::Id::new(("automation_curve", id)),
            &points,
            &mut state.point,
            CurveView {
                start: 0.0,
                end: state.end,
                snap: self.snap,
                height: (ui.available_height() - 66.0).max(180.0),
            },
            |beat| lane.value_at(beat).unwrap_or(0.5),
        );
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("automation_plot_rect"), response.plot_rect)
        });
        let mut edit = response.edit;
        ui.horizontal(|ui| {
            if let Some(point) = state
                .point
                .and_then(|selected| lane.points.iter().find(|p| p.id == selected))
            {
                let mut beat = point.beat;
                let mut value = lane.target.parameter.denormalize(point.value);
                ui.label("Beat");
                let beat_changed = ui
                    .add(
                        daw_ui::number(&mut beat)
                            .range(0.0..=daw_core::MAX_BEAT)
                            .speed(self.snap.max(0.01)),
                    )
                    .changed();
                ui.label("Value");
                let range = lane.target.parameter.range();
                let speed = (*range.end() - *range.start()) as f64 / 200.0;
                let value_changed = ui
                    .add(daw_ui::number(&mut value).range(range).speed(speed))
                    .changed();
                if beat_changed || value_changed {
                    edit = Some(CurveEdit::Move {
                        id: point.id,
                        x: beat,
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

fn plugin_parameters(ui: &mut egui::Ui, state: &mut EditorState, path: String, effect: bool) {
    if state.parameter_path.as_ref() != Some(&path) {
        state.parameter_path = Some(path.clone());
        state.parameters.clear();
        state.loading = None;
        state.load_error = None;
        start_parameter_read(state, path.clone(), effect);
    }
    if let Some(loading) = &state.loading {
        let ready = loading.lock().ok().and_then(|mut result| result.take());
        if let Some(result) = ready {
            state.loading = None;
            match result {
                Ok(parameters) => {
                    state.parameters = parameters.into_iter().filter(|p| !p.read_only).collect();
                    if let Some(parameter) = state
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
    ui.horizontal(|ui| {
        daw_ui::menu_button(ui, "Manual ID", |ui| {
            ui.label("VST parameter ID");
            ui.add(daw_ui::number(&mut state.vst_id).range(0..=u32::MAX));
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
            start_parameter_read(state, path, effect);
        }
    });
    if !state.parameters.is_empty() {
        let selected = state
            .parameters
            .iter()
            .find(|p| p.id == state.vst_id)
            .map(|p| p.name.as_str())
            .unwrap_or("Choose plug-in parameter");
        daw_ui::select_labeled("Plug-in parameter", selected).show_ui(ui, |ui| {
            for parameter in &state.parameters {
                if daw_ui::selectable_value(ui, &mut state.vst_id, parameter.id, &parameter.name)
                    .changed()
                {
                    state.initial = parameter.value as f32;
                }
            }
        });
    }
    ui.add(daw_ui::slider(&mut state.initial, 0.0..=1.0).text("Initial value"));
    if let Some(error) = &state.load_error {
        ui.colored_label(crate::theme::ORANGE, error);
    }
}

fn start_parameter_read(state: &mut EditorState, path: String, effect: bool) {
    let result: ParameterResult = Arc::new(Mutex::new(None));
    state.loading = Some(result.clone());
    state.load_error = None;
    std::thread::spawn(move || {
        let result_value = (|| -> anyhow::Result<_> {
            if effect {
                daw_plugins::VstEffect::load(&PathBuf::from(path), 48_000.0, 256)?.parameters()
            } else {
                daw_plugins::VstInstrument::load(&PathBuf::from(path), 48_000.0, 256)?.parameters()
            }
        })()
        .map_err(|error| format!("{error:#}"));
        if let Ok(mut result) = result.lock() {
            *result = Some(result_value);
        }
    });
}

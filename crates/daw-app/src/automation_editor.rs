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
            source: None,
            parameter: 0,
            point: None,
            end: 8.0,
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
        daw_ui::select_labeled("Parameter", &choices[state.parameter].1).show_ui(ui, |ui| {
            for (index, (_, label)) in choices.iter().enumerate() {
                daw_ui::selectable_value(ui, &mut state.parameter, index, label);
            }
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
        ui.add_space(10.0);
        let vst_parameter = matches!(parameter, AutomationParameter::VstParameter { .. });
        let ready = !vst_parameter
            || state.manual_id
            || (state.loading.is_none() && state.parameters.iter().any(|p| p.id == state.vst_id));
        if ui
            .add_enabled(ready, daw_ui::button_widget("Add automation track"))
            .clicked()
        {
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
        // Probe results must not move the Add action while a manual-ID click is
        // in progress. Keep asynchronous diagnostics below the stable controls.
        if vst_parameter && let Some(error) = &state.load_error {
            ui.colored_label(crate::theme::ORANGE, error);
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
            data.insert_temp(egui::Id::new("automation_plot_rect"), response.plot_rect);
            data.insert_temp(
                egui::Id::new("automation_curve_focus"),
                response.response.id,
            );
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
    ui.horizontal(|ui| {
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
        daw_ui::select_labeled("Plug-in parameter", selected).show_ui(ui, |ui| {
            for parameter in &state.parameters {
                if daw_ui::selectable_value(ui, &mut state.vst_id, parameter.id, &parameter.name)
                    .changed()
                {
                    state.initial = parameter.value as f32;
                    state.manual_id = false;
                }
            }
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

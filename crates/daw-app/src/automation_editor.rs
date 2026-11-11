use crate::app::DawApp;
use daw_core::{
    AutomationParameter, AutomationTarget, Effect, Instrument, Interpolation, PluginParameterValue,
    TrackKind,
};
use daw_ui::{CurveEdit, CurvePoint, CurveTimeline, TimelineViewport};
use eframe::egui;
use std::{
    collections::HashSet,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

type ParameterResult = Arc<Mutex<Option<Result<Vec<daw_plugins::ParameterInfo>, String>>>>;

// Keep the chooser order and the boundary of the larger plug-in parameter
// page together when adding another automatable channel control.
const CHANNEL_PARAMETERS: [(AutomationParameter, &str); 3] = [
    (AutomationParameter::TrackInputGain, "Input gain"),
    (AutomationParameter::TrackGain, "Volume"),
    (AutomationParameter::TrackPan, "Pan"),
];

fn default_parameter_index() -> usize {
    // Volume remains the initial workflow even when another channel control
    // precedes it in the picker.
    CHANNEL_PARAMETERS
        .iter()
        .position(|(parameter, _)| *parameter == AutomationParameter::TrackGain)
        .expect("Volume is an automatable channel control")
}

#[derive(Clone, PartialEq, Eq)]
struct ParameterSource {
    track: u64,
    effect: Option<u64>,
    path: String,
    class_id: Option<String>,
}

struct ParameterContext<'a> {
    revision: u64,
    saved_state: &'a [u8],
    overrides: &'a [PluginParameterValue],
    automated: &'a HashSet<u32>,
    last_touched: Option<u32>,
}

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
    parameter_source: Option<ParameterSource>,
    parameter_revision: Option<u64>,
    snapshot_signature: u64,
    requested_signature: u64,
    browser: daw_ui::ParameterBrowserState,
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
            parameter: default_parameter_index(),
            point: None,
            viewport: TimelineViewport::default(),
            fitted: false,
            vst_id: 0,
            initial: 0.5,
            parameter_source: None,
            parameter_revision: None,
            snapshot_signature: 0,
            requested_signature: 0,
            browser: daw_ui::ParameterBrowserState::default(),
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
        state.parameter = default_parameter_index();
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
        let parameter_page = state
            .source
            .and_then(|id| self.project.track(id))
            .is_some_and(|track| state.parameter >= CHANNEL_PARAMETERS.len() + track.sends.len());
        let mut open = true;
        let mut cancel = false;
        let popup = if editing {
            daw_ui::workspace_popup("Automation", ctx).id(egui::Id::new("automation_editor"))
        } else {
            daw_ui::popup("Add automation", ctx)
                .id(egui::Id::new(if parameter_page {
                    "automation_create_parameters"
                } else {
                    "automation_create"
                }))
                // The plug-in parameter browser needs its wide table; choosing
                // a track and channel parameter is a two-row form.
                .default_width(if parameter_page {
                    880.0
                } else {
                    daw_ui::tokens::FORM_WIDTH
                })
                .default_height(if parameter_page { 620.0 } else { 180.0 })
                .vscroll(false)
        };
        popup.open(&mut open).show(ctx, |ui| {
            daw_ui::automation_control_style(ui);
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
                cancel = self.create_automation_lane(ui, &mut state);
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
        let mut footer = daw_ui::popup_footer(ui, "automation_create_actions");
        let window_ui = ui;
        let ui = footer.body();
        let tracks: Vec<_> = self
            .project
            .channels()
            .filter(|t| matches!(t.kind, TrackKind::Audio | TrackKind::Midi | TrackKind::Bus))
            .map(|t| (t.id, t.name.clone()))
            .collect();
        if !tracks.iter().any(|(id, _)| Some(*id) == state.source) {
            state.source = tracks.first().map(|(id, _)| *id);
        }
        let Some(source) = state.source else {
            daw_ui::text::note(ui, "Add an audio, instrument, or bus track first.");
            return footer
                .show(window_ui, |ui| {
                    daw_ui::dialog_actions(
                        ui,
                        |ui| daw_ui::dialog_button(ui, back).clicked(),
                        |_| {},
                    )
                    .inner
                    .0
                })
                .inner;
        };
        let mut selected_source = Some(source);
        let control_width = daw_ui::tokens::FORM_FIELD_WIDTH;
        daw_ui::form_grid(ui, "automation_source", |ui| {
            daw_ui::form_label(ui, "Track");
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
            state.parameter = default_parameter_index();
        }
        let source = state.source.unwrap();
        let track = self.project.track(source).unwrap();
        let mut choices: Vec<_> = CHANNEL_PARAMETERS
            .iter()
            .map(|(parameter, label)| (*parameter, (*label).to_owned()))
            .collect();
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
            daw_ui::form_label(ui, "Parameter");
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
        let mut browser_activate = false;
        if let AutomationParameter::VstParameter { effect, .. } = parameter {
            let path = effect
                .and_then(|id| track.inserts.iter().find(|slot| slot.id == id))
                .map(|slot| match &slot.effect {
                    Effect::Vst3 { path, class_id } => (path.clone(), class_id.clone()),
                    Effect::Builtin { device } => crate::builtin_devices::locator(device),
                })
                .or_else(|| match &track.instrument {
                    Instrument::Vst3 { path, class_id } => Some((path.clone(), class_id.clone())),
                    Instrument::Builtin { device } => Some(crate::builtin_devices::locator(device)),
                    _ => None,
                });
            if let Some((path, class_id)) = path {
                let (saved_state, overrides) = if let Some(slot) =
                    effect.and_then(|id| track.inserts.iter().find(|slot| slot.id == id))
                {
                    (slot.state.as_slice(), slot.parameters.as_slice())
                } else {
                    (
                        track.instrument_state.as_slice(),
                        track.instrument_parameters.as_slice(),
                    )
                };
                let automated = self
                    .project
                    .tracks
                    .iter()
                    .filter_map(|track| {
                        let lane = track.automation.as_ref()?;
                        match lane.target.parameter {
                            AutomationParameter::VstParameter { effect: owner, id }
                                if lane.target.track == source && owner == effect =>
                            {
                                Some(id)
                            }
                            _ => None,
                        }
                    })
                    .collect::<HashSet<_>>();
                browser_activate = plugin_parameters(
                    ui,
                    state,
                    ParameterSource {
                        track: source,
                        effect,
                        path,
                        class_id,
                    },
                    ParameterContext {
                        revision: self.revision,
                        saved_state,
                        overrides,
                        automated: &automated,
                        last_touched: self.last_touched_plugin_parameter(source, effect),
                    },
                );
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
        let vst_parameter = matches!(parameter, AutomationParameter::VstParameter { .. });
        let ready = !vst_parameter
            || state.manual_id
            || (state.loading.is_none()
                && state.load_error.is_none()
                && state.browser.selected == Some(state.vst_id)
                && state.parameters.iter().any(|p| p.id == state.vst_id));
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
        if vst_parameter && state.manual_id {
            // Diagnostics stay in the body; the separate footer keeps action
            // targets fixed when asynchronous metadata finishes.
            ui.allocate_ui(
                egui::vec2(ui.available_width(), daw_ui::tokens::CONTROL_HEIGHT),
                |ui| {
                    if let Some(error) = &state.load_error {
                        daw_ui::text::truncated(ui, daw_ui::text::TextRole::Note, error)
                            .on_hover_text(error);
                    }
                },
            );
        }
        let (activate, cancel) = footer
            .show(window_ui, |ui| {
                // The shared footer: primary at the right edge, Cancel/Back beside it.
                daw_ui::dialog_actions(
                    ui,
                    |ui| {
                        let activate = ui
                            .add_enabled_ui(ready, |ui| daw_ui::dialog_primary(ui, action))
                            .inner
                            .clicked();
                        (activate, daw_ui::dialog_button(ui, back).clicked())
                    },
                    |_| {},
                )
                .inner
                .0
            })
            .inner;
        if ready && (activate || browser_activate) {
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
        let track_name = track.name.clone();
        let scope = daw_tools::ToolScope::Automation {
            track_id: lane.target.track,
            lane_id: id,
        };
        let mut open_ai = false;
        daw_ui::action_row(
            ui,
            |ui| {
                open_ai = daw_ui::contextual_ai_button(ui, &scope.label(&self.project)).clicked();
            },
            |ui| {
                daw_ui::text::truncated(ui, daw_ui::text::TextRole::Subtitle, &track_name);
            },
        );
        if open_ai {
            self.open_context_assistant(scope);
            return;
        }
        let mut interpolation = lane.interpolation;
        let mut enabled = !track.mute;
        let automation_state = self
            .automation_control_state(lane.target)
            .map(|(_, state, _)| state)
            .unwrap_or(daw_ui::AutomationState::Bypassed);
        let mut add_parameter = false;
        let mut fit = !state.fitted;
        let mut fit_song = false;
        let mut override_edit = None;
        let compact = ui.available_width() < 900.0;
        daw_ui::curve_toolbar(ui, compact, |ui| {
            let modes = [
                (Interpolation::Step, "Step"),
                (Interpolation::Linear, "Linear"),
                (Interpolation::Smooth, "Smooth"),
                (Interpolation::Spline, "Spline"),
            ];
            if compact {
                daw_ui::select("automation_interpolation", format!("{interpolation:?}"))
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for (value, name) in modes {
                            if daw_ui::selectable_value(ui, &mut interpolation, value, name)
                                .clicked()
                            {
                                ui.close();
                            }
                        }
                    });
            } else {
                for (value, name) in modes {
                    daw_ui::selectable_value(ui, &mut interpolation, value, name);
                }
            }
            ui.separator();
            daw_ui::checkbox(ui, &mut enabled, "Enabled");
            daw_ui::automation_badge(ui, automation_state);
            if lane.override_value.is_some() {
                if daw_ui::button(ui, "Resume automation")
                    .on_hover_text("Return this parameter to the curve at the current playhead")
                    .clicked()
                {
                    override_edit = Some(None);
                }
            } else if ui
                .add_enabled(
                    automation_state != daw_ui::AutomationState::Bypassed
                        && (!lane.points.is_empty()
                            || lane.clips.iter().any(|clip| !clip.points.is_empty())),
                    daw_ui::button_widget("Override automation"),
                )
                .on_hover_text("Hold the current curve value manually until Resume automation")
                .clicked()
            {
                override_edit = Some(Some(lane.value_at(self.cursor).unwrap_or(0.5)));
            }
            // View and lane actions trail; the gesture caption fills the
            // space between, so no separate hint band takes plot height.
            daw_ui::toolbar_trailing(ui, AUTOMATION_TRAILING_WIDTH, |ui| {
                add_parameter = daw_ui::button(ui, "Add parameter").clicked();
                fit_song = daw_ui::curve_fit_song_button(ui).clicked();
                fit |= daw_ui::curve_fit_points_button(ui).clicked();
                if let Some(caption) = daw_ui::editor_status_caption(
                    ui,
                    "Click to add · Drag to move · Drag diamonds to bend · Right-click for options",
                ) {
                    caption.on_hover_text(
                        "Click to add; drag points to move and diamonds to bend. Right-click for options.",
                    );
                }
            });
        });
        if add_parameter {
            state.creating = true;
            state.source = Some(lane.target.track);
            state.parameter = default_parameter_index();
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
        let segments: Vec<_> = lane
            .points
            .iter()
            .take(lane.points.len().saturating_sub(1))
            .map(|point| daw_ui::CurveSegment {
                start: point.id,
                curve: if point.interpolation.unwrap_or(lane.interpolation) == Interpolation::Spline
                {
                    point.curve
                } else {
                    0.0
                },
            })
            .collect();
        let response = daw_ui::curve_timeline_editor_with_fit(
            ui,
            egui::Id::new(("automation_curve", id)),
            &points,
            &segments,
            lane.override_value
                .filter(|_| automation_state == daw_ui::AutomationState::Overridden),
            &mut state.point,
            &mut state.viewport,
            CurveTimeline {
                snap: self.snap,
                // Each inspector toolbar owns its 8 px top/bottom inset. The
                // gap after the plot and between inspectors follows the theme.
                // One inspector row stays reserved under the plot, so selecting
                // a point never changes the plot height.
                height: (ui.available_height()
                    - (daw_ui::tokens::PARAMETER_ROW_HEIGHT
                        + daw_ui::tokens::SPACE_SM * 2.0
                        + ui.spacing().item_spacing.y))
                    // Keep a useful mouse canvas; compact windows scroll the
                    // point and segment inspectors below it instead of
                    // compressing the curve into overlapping percentage rows.
                    .max(
                        daw_ui::tokens::TIMELINE_RULER_HEIGHT
                            + 2.0 * daw_ui::tokens::SPACE_MD
                            + daw_ui::tokens::PARAMETER_ROW_HEIGHT,
                    ),
                tempo: self.project.tempo,
                meter: self.project.meter_map(),
                fit,
                keyboard_active: true,
                value_label: value_axis_label(lane.target.parameter),
            },
            fit_song.then_some(daw_ui::TimelineBounds {
                start_beat: 0.0,
                end_beat: self
                    .project
                    .end_beat()
                    .max(self.project.meter_map().bar_start(2)),
                start_row: 0.0,
                end_row: 1.0,
            }),
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
        let mut delete_selection = response.delete_selection;
        let mut point_beat_rect = None;
        let mut point_value_rect = None;
        let mut override_value_rect = None;
        let mut displayed_beat = None;
        let mut segment_edit = response.segment_edit;
        let mut slope_rect = None;
        let meter = self.project.meter_map();
        daw_ui::parameter_toolbar(ui, |ui| {
            if let Some(value) = lane.override_value {
                let mut manual = lane.target.parameter.denormalize(value);
                let manual_response = if matches!(
                    lane.target.parameter,
                    AutomationParameter::TrackInputGain
                        | AutomationParameter::TrackGain
                        | AutomationParameter::SendGain { .. }
                ) {
                    daw_ui::gain_value_button(
                        ui,
                        ("automation_override_gain", id),
                        &mut manual,
                        *lane.target.parameter.range().end(),
                        "Override",
                    )
                } else {
                    ui.add(
                        daw_ui::number(&mut manual)
                            .range(lane.target.parameter.range())
                            .speed(0.01)
                            .prefix("Override "),
                    )
                    .on_hover_text("Manual parameter value; Resume automation returns to the curve")
                };
                override_value_rect = Some(manual_response.rect);
                if manual_response.changed() {
                    override_edit = Some(Some(lane.target.parameter.normalize(manual)));
                }
            }
            let Some(index) = state
                .point
                .and_then(|selected| lane.points.iter().position(|p| p.id == selected))
            else {
                daw_ui::text::note(ui, "Select a point to edit its position, value and segment");
                return;
            };
            let point = &lane.points[index];
            let mut beat = point.beat;
            let mut value = lane.target.parameter.denormalize(point.value);
            let beat_response = ui
                .add(
                    daw_ui::musical_position_field(&mut beat, &meter, "Position ")
                        .range(0.0..=daw_core::MAX_BEAT)
                        .speed(self.snap_step(state.viewport.pixels_per_beat).max(0.01)),
                )
                .on_hover_text("Bar.beat.sixteenth · drag or double-click to type, e.g. 12.1");
            point_beat_rect = Some(beat_response.rect);
            displayed_beat = Some(beat + 1.0);
            let range = lane.target.parameter.range();
            let speed = (*range.end() - *range.start()) as f64 / 200.0;
            let value_response = match lane.target.parameter {
                AutomationParameter::TrackInputGain
                | AutomationParameter::TrackGain
                | AutomationParameter::SendGain { .. } => daw_ui::gain_value_button(
                    ui,
                    ("automation_point_gain", id, point.id),
                    &mut value,
                    *range.end(),
                    "Gain",
                ),
                parameter => {
                    let (prefix, tooltip) = match parameter {
                        AutomationParameter::TrackPan => ("Pan ", "−1 left · 0 center · 1 right"),
                        _ => ("Value ", "Normalized plug-in parameter value from 0 to 1"),
                    };
                    ui.add(
                        daw_ui::number(&mut value)
                            .range(range)
                            .speed(speed)
                            .prefix(prefix),
                    )
                    .on_hover_text(tooltip)
                }
            };
            point_value_rect = Some(value_response.rect);
            let value_changed = value_response.changed();
            if beat_response.changed() || value_changed {
                edit = Some(CurveEdit::Move {
                    id: point.id,
                    x: beat,
                    value: if value_changed {
                        lane.target.parameter.normalize(value)
                    } else {
                        // Position-only edits must preserve the exact stored
                        // normalized value, including quiet gains and sends.
                        point.value
                    },
                });
            }
            if index + 1 < lane.points.len() {
                let mut curve =
                    if point.interpolation.unwrap_or(lane.interpolation) == Interpolation::Spline {
                        point.curve
                    } else {
                        0.0
                    };
                let controls = daw_ui::curve_segment_controls(ui, &mut curve);
                slope_rect = Some(controls.rect);
                if controls.changed() {
                    segment_edit = Some((point.id, curve));
                }
            }
            if daw_ui::button(
                ui,
                if response.selection.len() > 1 {
                    "Delete selected points"
                } else {
                    "Delete point"
                },
            )
            .clicked()
            {
                edit = Some(CurveEdit::Delete { id: point.id });
                delete_selection = response.selection.len() > 1;
            }
            if let Some(first) = lane.points.first()
                && let Some(last) = lane.points.last()
            {
                daw_ui::curve_endpoint_summary(ui, first.value, last.value);
            }
        });
        let rect = |rect: egui::Rect| serde_json::json!({"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]});
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("automation_editor_diagnostics"), serde_json::json!({
            "plot":rect(response.plot_rect), "start_beat":state.viewport.start_beat,
            "pixels_per_beat":state.viewport.pixels_per_beat,
            "end_beat":state.viewport.beat_at(response.plot_rect,response.plot_rect.right()),
            "point_beat_rect":point_beat_rect.map(rect), "point_beat":displayed_beat,
            "point_value_rect":point_value_rect.map(rect), "override_value_rect":override_value_rect.map(rect),
            "navigating":response.navigation.navigating,
            "selected_point":state.point,"selected_points":response.selection,
            "interpolation":format!("{:?}",lane.interpolation),"override_value":lane.override_value,
            "slope_controls":slope_rect.map(rect),
            "segments":response.segment_handles.iter().map(|handle| serde_json::json!({
                "start":handle.start,"end":handle.end,"rect":rect(handle.rect),"curve":handle.curve,
            })).collect::<Vec<_>>(),
        }))
        });
        if let Some(value) = override_edit {
            self.begin_edit();
            let result = self.project.set_automation_override(id, value);
            if result.is_ok() {
                self.edit_label = Some(
                    if value.is_some() {
                        "Override automation"
                    } else {
                        "Resume automation"
                    }
                    .into(),
                );
                self.changed();
            }
            self.message(result, "");
        }
        if let Some((point, curve)) = segment_edit {
            self.begin_edit();
            let result = self.project.set_automation_point_curve(id, point, curve);
            if result.is_ok() {
                self.edit_label = Some("Automation slope".into());
                self.changed();
            }
            self.message(result, "");
        }
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
                CurveEdit::Bend { id: point, curve } => {
                    self.project.set_automation_point_curve(id, point, curve)
                }
                CurveEdit::Delete { id: point } => {
                    let selected = if delete_selection {
                        response.selection.clone()
                    } else {
                        vec![point]
                    };
                    selected
                        .into_iter()
                        .try_for_each(|point| {
                            self.project.remove_automation_point(id, point).map(|_| ())
                        })
                        .map(|_| state.point = None)
                }
                CurveEdit::Interpolation { id: point, mode } => {
                    self.project.set_automation_point_interpolation(
                        id,
                        None,
                        point,
                        match mode {
                            daw_ui::CurveInterpolation::Step => Interpolation::Step,
                            daw_ui::CurveInterpolation::Linear => Interpolation::Linear,
                            daw_ui::CurveInterpolation::Smooth => Interpolation::Smooth,
                        },
                    )
                }
            };
            if result.is_ok() {
                self.changed();
            }
            self.message(result, "");
        }
    }
}

fn parameter_signature(saved_state: &[u8], overrides: &[PluginParameterValue]) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    saved_state.hash(&mut hash);
    for parameter in overrides {
        parameter.id.hash(&mut hash);
        parameter.value.to_bits().hash(&mut hash);
    }
    hash.finish()
}

fn plugin_parameters(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    source: ParameterSource,
    context: ParameterContext<'_>,
) -> bool {
    let ParameterContext {
        revision,
        saved_state,
        overrides,
        automated,
        last_touched,
    } = context;
    let changed_source = state.parameter_source.as_ref() != Some(&source);
    if changed_source {
        state.parameter_source = Some(source.clone());
        state.parameter_revision = None;
        state.parameters.clear();
        state.loading = None;
        state.load_error = None;
        state.manual_id = false;
        state.vst_id = 0;
        state.initial = 0.5;
        state.browser = daw_ui::ParameterBrowserState {
            focus_search: true,
            ..Default::default()
        };
    }
    // Large opaque states are hashed only after a project edit, never every
    // paint. Instance identity separates two copies of the same VST class.
    if state.parameter_revision != Some(revision) {
        state.parameter_revision = Some(revision);
        state.snapshot_signature = parameter_signature(saved_state, overrides);
    }
    if let Some(loading) = &state.loading {
        let ready = loading.lock().ok().and_then(|mut result| result.take());
        if let Some(result) = ready {
            state.loading = None;
            // Values from an older snapshot must not become a new lane's start.
            if state.requested_signature == state.snapshot_signature {
                match result {
                    Ok(parameters) => {
                        state.parameters =
                            parameters.into_iter().filter(|p| !p.read_only).collect();
                        state.browser.selected = None;
                        state.load_error = None;
                    }
                    Err(error) => state.load_error = Some(error),
                }
            }
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }
    if changed_source
        || (state.loading.is_none() && state.requested_signature != state.snapshot_signature)
    {
        start_parameter_read(state, source.clone(), saved_state, overrides);
    }
    daw_ui::toolbar(ui, |ui| {
        daw_ui::picker_button(ui, "Manual ID", |ui| {
            daw_ui::checkbox(ui, &mut state.manual_id, "Use manual ID");
            daw_ui::text::label(ui, "VST parameter ID");
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
                } else if state.load_error.is_some() {
                    "Retry"
                } else {
                    "Read parameters"
                }),
            )
            .clicked()
        {
            start_parameter_read(state, source, saved_state, overrides);
        }
    });
    if state.manual_id {
        ui.add(daw_ui::slider(&mut state.initial, 0.0..=1.0).text("Initial value"));
        return false;
    }
    let items = state
        .parameters
        .iter()
        .map(|parameter| daw_ui::ParameterItem {
            id: parameter.id,
            name: &parameter.name,
            unit: &parameter.unit,
            value: parameter.value,
            automated: automated.contains(&parameter.id),
            last_touched: last_touched == Some(parameter.id),
        })
        .collect::<Vec<_>>();
    let height = ui.available_height().max(0.0);
    let mut selected_id = state.vst_id;
    let previous_selected = state.browser.selected;
    let mut initial = state.initial;
    let response = daw_ui::parameter_browser(
        ui,
        "automation_plugin_parameters",
        &mut state.browser,
        &items,
        height,
        |ui, selected| {
            if state.loading.is_some() {
                ui.spinner();
                daw_ui::text::note(ui, "Reading parameters…");
            } else if let Some(error) = &state.load_error {
                daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note, error, 4)
                    .on_hover_text(error);
            } else if let Some(parameter) = selected {
                if previous_selected != Some(parameter.id) {
                    selected_id = parameter.id;
                    initial = parameter.value as f32;
                }
                daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Title, parameter.name, 2);
                if !parameter.unit.is_empty() {
                    daw_ui::text::note(ui, parameter.unit);
                }
                if parameter.automated {
                    daw_ui::text::note(ui, "Automation already exists");
                } else {
                    ui.add(daw_ui::slider(&mut initial, 0.0..=1.0).text("Initial value"));
                }
            }
        },
    );
    state.vst_id = selected_id;
    state.initial = initial;
    response.activated.is_some_and(|id| id == state.vst_id)
        && state.loading.is_none()
        && state.load_error.is_none()
}

fn start_parameter_read(
    state: &mut EditorState,
    source: ParameterSource,
    saved_state: &[u8],
    overrides: &[PluginParameterValue],
) {
    let result: ParameterResult = Arc::new(Mutex::new(None));
    state.loading = Some(result.clone());
    state.requested_signature = state.snapshot_signature;
    state.load_error = None;
    if let Some(device) = crate::builtin_devices::device_of_locator(&source.path) {
        let parameters = crate::builtin_devices::parameters(device, overrides).ok_or_else(|| {
            format!("Built-in device \"{device}\" is not available in this version")
        });
        if let Ok(mut result) = result.lock() {
            *result = Some(parameters);
        }
        return;
    }
    let saved_state = saved_state.to_vec();
    let overrides = overrides.to_vec();
    std::thread::spawn(move || {
        let result_value = (|| -> anyhow::Result<_> {
            if source.effect.is_some() {
                let mut plugin = daw_plugins::VstEffect::load_class(
                    &PathBuf::from(source.path),
                    source.class_id.as_deref(),
                    48_000.0,
                    256,
                )?;
                if !saved_state.is_empty() {
                    plugin.load_state(&saved_state)?;
                }
                for parameter in overrides {
                    plugin.set_parameter(parameter.id, parameter.value)?;
                }
                plugin.parameters()
            } else {
                let mut plugin = daw_plugins::VstInstrument::load_class(
                    &PathBuf::from(source.path),
                    source.class_id.as_deref(),
                    48_000.0,
                    256,
                )?;
                if !saved_state.is_empty() {
                    plugin.load_state(&saved_state)?;
                }
                for parameter in overrides {
                    plugin.set_parameter(parameter.id, parameter.value)?;
                }
                plugin.parameters()
            }
        })()
        .map_err(|error| format!("{error:#}"));
        if let Ok(mut result) = result.lock() {
            *result = Some(result_value);
        }
    });
}

/// Room kept at the toolbar end for Add parameter, both Fit actions and the
/// caption's first clause; with less, the group starts its own row.
const AUTOMATION_TRAILING_WIDTH: f32 = 480.0;

/// Value-axis labels in the parameter's own units.
fn value_axis_label(parameter: AutomationParameter) -> fn(f32) -> String {
    match parameter {
        AutomationParameter::TrackInputGain => {
            |value| daw_ui::gain_axis_label(AutomationParameter::TrackInputGain.denormalize(value))
        }
        AutomationParameter::TrackGain => {
            |value| daw_ui::gain_axis_label(AutomationParameter::TrackGain.denormalize(value))
        }
        AutomationParameter::SendGain { .. } => |value| {
            daw_ui::gain_axis_label(AutomationParameter::SendGain { bus: 0 }.denormalize(value))
        },
        AutomationParameter::TrackPan => |value| {
            let pan = AutomationParameter::TrackPan.denormalize(value);
            let amount = (pan.abs() * 100.0).round();
            if amount < 0.5 {
                "C".into()
            } else if pan < 0.0 {
                format!("L {amount:.0}")
            } else {
                format!("R {amount:.0}")
            }
        },
        AutomationParameter::VstParameter { .. } => |value| format!("{value:.2}"),
    }
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
            for (parameter, (_, label)) in CHANNEL_PARAMETERS.iter().enumerate() {
                let ctx = egui::Context::default();
                daw_ui::theme::set_appearance(&ctx, appearance);
                let mut project = daw_core::Project::new();
                let source = project.add_audio_track("Glass keys");
                let mut app = DawApp::with_context(&ctx, project, None);
                app.selected_track = Some(source);
                app.automation_open = true;
                ctx.data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new("automation_editor_state"),
                        EditorState {
                            track: Some(source),
                            source: Some(source),
                            parameter,
                            ..Default::default()
                        },
                    )
                });
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
                assert!(
                    label_rect(label).is_positive(),
                    "The selected channel parameter is visible"
                );
                assert!((add.center().y - cancel.center().y).abs() < 1.0);
                assert!(cancel.right() < add.left());
                assert!(
                    window.bottom() - add.bottom() < 50.0,
                    "No large blank region below the footer"
                );
            }
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
            parameter: CHANNEL_PARAMETERS.len(),
            parameter_source: Some(ParameterSource {
                track,
                effect: None,
                path: "fixture.vst3".into(),
                class_id: None,
            }),
            parameter_revision: Some(app.revision),
            snapshot_signature: parameter_signature(&[], &[]),
            requested_signature: parameter_signature(&[], &[]),
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
                parameter: CHANNEL_PARAMETERS.len(),
                parameter_source: Some(ParameterSource {
                    track,
                    effect: None,
                    path: "fixture.vst3".into(),
                    class_id: None,
                }),
                parameter_revision: Some(app.revision),
                snapshot_signature: parameter_signature(&[], &[]),
                requested_signature: parameter_signature(&[], &[]),
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

    #[test]
    fn parameter_search_never_activates_a_hidden_selection_and_reuses_an_existing_lane() {
        let ctx = egui::Context::default();
        let mut project = daw_core::Project::new();
        let source = project.add_midi_track("Synth");
        project.track_mut(source).unwrap().instrument = Instrument::Vst3 {
            path: "fixture.vst3".into(),
            class_id: None,
        };
        let mut app = DawApp::with_context(&ctx, project, None);
        let parameters = vec![daw_plugins::ParameterInfo {
            id: 0,
            name: "Filter Cutoff".into(),
            value: 0.73,
            default: 0.5,
            unit: "Hz".into(),
            steps: 0,
            read_only: false,
        }];
        let signature = parameter_signature(&[], &[]);
        let mut state = EditorState {
            track: Some(source),
            source: Some(source),
            parameter: CHANNEL_PARAMETERS.len(),
            parameter_source: Some(ParameterSource {
                track: source,
                effect: None,
                path: "fixture.vst3".into(),
                class_id: None,
            }),
            parameter_revision: Some(app.revision),
            snapshot_signature: signature,
            requested_signature: signature,
            loading: Some(Arc::new(Mutex::new(Some(Ok(parameters))))),
            browser: daw_ui::ParameterBrowserState {
                focus_search: true,
                ..Default::default()
            },
            ..Default::default()
        };
        for _ in 0..3 {
            frame(&mut app, &mut state, &ctx, vec![]);
        }
        assert_eq!(
            state.initial, 0.73,
            "parameter ID zero uses its current value"
        );
        let enter = |pressed| {
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]
        };
        state.browser.query = "no matching control".into();
        frame(&mut app, &mut state, &ctx, vec![]);
        assert_eq!(state.browser.selected, None);
        for pressed in [true, false] {
            frame(&mut app, &mut state, &ctx, enter(pressed));
        }
        assert_eq!(app.project.tracks.len(), 1);
        state.browser.query = "cutoff hz".into();
        frame(&mut app, &mut state, &ctx, vec![]);
        for pressed in [true, false] {
            frame(&mut app, &mut state, &ctx, enter(pressed));
        }
        assert_eq!(app.project.tracks.len(), 2);
        let lane = app.project.tracks[1].automation.as_ref().unwrap();
        assert!(lane.points.iter().all(|point| point.value == 0.73));
        let id = app.project.tracks[1].id;
        for pressed in [true, false] {
            frame(&mut app, &mut state, &ctx, enter(pressed));
        }
        assert_eq!(app.project.tracks.len(), 2, "existing automation is reused");
        assert_eq!(app.selected_track, Some(id));
    }
}

use crate::app::DawApp;
use daw_core::{AutomationParameter, Interpolation, TrackKind};
use daw_ui::{CurveEdit, CurvePoint, CurveTimeline, TimelineViewport};
use eframe::egui;

#[derive(Clone, Default)]
struct EditorState {
    track: Option<u64>,
    point: Option<u64>,
    viewport: TimelineViewport,
    fitted: bool,
}

pub(crate) fn reset_automation_editor(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<EditorState>(egui::Id::new("automation_editor_state"));
        data.remove::<serde_json::Value>(egui::Id::new("automation_editor_diagnostics"));
        data.remove::<egui::Rect>(egui::Id::new("automation_plot_rect"));
        data.remove::<egui::Id>(egui::Id::new("automation_curve_focus"));
    });
}

impl DawApp {
    /// Edits an existing lane. Lanes are created from a control's
    /// right-click **Automate**, never from a parameter list here.
    pub fn automation_editor(&mut self, ctx: &egui::Context) {
        if !self.automation_open {
            return;
        }
        let editing = self
            .selected_track
            .and_then(|id| self.project.track(id))
            .is_some_and(|t| t.kind == TrackKind::Automation);
        if !editing {
            reset_automation_editor(ctx);
            self.automation_open = false;
            return;
        }
        let key = egui::Id::new("automation_editor_state");
        let mut state = ctx
            .data_mut(|data| data.get_temp::<EditorState>(key))
            .unwrap_or_default();
        if state.track != self.selected_track {
            state = EditorState {
                track: self.selected_track,
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
        let mut open = true;
        daw_ui::page("Automation")
            .id(egui::Id::new("automation_editor"))
            .open(&mut open)
            .show(ctx, |ui| {
                daw_ui::automation_control_style(ui);
                self.edit_automation_curve(ui, &mut state);
            });
        if !open {
            self.automation_open = false;
        }
        ctx.data_mut(|data| data.insert_temp(key, state));
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
        let mut fit = !state.fitted;
        let mut fit_song = false;
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
            ui.separator();
            // View and lane actions trail; the gesture caption fills the
            // space between, so no separate hint band takes plot height.
            daw_ui::toolbar_trailing(ui, AUTOMATION_TRAILING_WIDTH, |ui| {
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
        let mut displayed_beat = None;
        let mut segment_edit = response.segment_edit;
        let mut slope_rect = None;
        let meter = self.project.meter_map();
        daw_ui::parameter_toolbar(ui, |ui| {
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
                        .speed(self.snap.max(0.01)),
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
            "point_value_rect":point_value_rect.map(rect),
            "navigating":response.navigation.navigating,
            "selected_point":state.point,"selected_points":response.selection,
            "interpolation":format!("{:?}",lane.interpolation),"override_value":lane.override_value,
            "slope_controls":slope_rect.map(rect),
            "segments":response.segment_handles.iter().map(|handle| serde_json::json!({
                "start":handle.start,"end":handle.end,"rect":rect(handle.rect),"curve":handle.curve,
            })).collect::<Vec<_>>(),
        }))
        });
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

/// Room kept at the toolbar end for both Fit actions and the
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

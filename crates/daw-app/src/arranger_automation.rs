//! Arrangement automation: base points plus reusable patterns that override
//! them only inside their own bounds. All canvas controls live in daw-ui.
use crate::app::DawApp;
use daw_core::{AutomationClip, AutomationLane, Interpolation, MAX_BEAT};
use daw_ui::{AutomationCanvas, CurveEdit, CurvePoint, TimelineViewport};
use eframe::egui::{self, Pos2, Rect, Vec2};

pub(crate) enum AutomationEdit {
    Clear,
    Curve(Option<u64>, CurveEdit),
    Create(f64, f64),
    Bounds(u64, f64, f64, f64),
    Copy(u64, f64, bool),
    Paste(AutomationClip, f64),
    Delete(u64),
    Independent(u64),
}

pub(crate) struct LaneView {
    pub track: u64,
    pub rect: Rect,
    pub viewport: TimelineViewport,
    pub snap: f64,
    pub color: egui::Color32,
    pub navigating: bool,
    pub pointer_blocked: bool,
}

pub(crate) struct LaneResponse {
    pub edits: Vec<AutomationEdit>,
    pub interacted: bool,
    pub diagnostic: serde_json::Value,
}

#[derive(Clone)]
struct PatternDrag {
    original: AutomationClip,
    edge: i8,
}

fn rect_json(rect: Rect) -> serde_json::Value {
    serde_json::json!({"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]})
}
fn points(points: &[daw_core::AutomationPoint]) -> Vec<CurvePoint> {
    points
        .iter()
        .map(|point| CurvePoint {
            id: point.id,
            x: point.beat,
            value: point.value,
        })
        .collect()
}

pub(crate) fn automation_lane(
    ui: &mut egui::Ui,
    view: LaneView,
    lane: &AutomationLane,
    selected: &mut Option<u64>,
) -> LaneResponse {
    let mut result = LaneResponse {
        edits: Vec::new(),
        interacted: false,
        diagnostic: serde_json::Value::Null,
    };
    let id = egui::Id::new(("arranger_automation", view.track));
    let clip_rect = |clip: &AutomationClip| {
        Rect::from_min_max(
            Pos2::new(view.viewport.x_at(view.rect, clip.start), view.rect.top()),
            Pos2::new(
                view.viewport.x_at(view.rect, clip.end()),
                view.rect.bottom(),
            ),
        )
    };
    let over_clip = ui
        .input(|input| input.pointer.hover_pos())
        .is_some_and(|point| {
            lane.clips
                .iter()
                .any(|clip| clip_rect(clip).contains(point))
        });
    let response = daw_ui::automation_canvas_at(
        ui,
        id,
        &points(&lane.points),
        AutomationCanvas {
            rect: view.rect,
            viewport: view.viewport,
            snap: view.snap,
            color: view.color,
            editable: !view.navigating && !over_clip,
        },
        |beat| lane.arrangement_value_at(beat).unwrap_or(0.5),
    );
    if let Some(edit) = response.edit {
        result.edits.push(AutomationEdit::Curve(None, edit));
        *selected = None;
    }
    if response.response.clicked() || response.response.dragged() {
        result.interacted = true;
        *selected = None;
    }
    let bypass = ui.input(|input| input.modifiers.shift);
    let snap = |beat: f64| {
        (if view.snap > 0.0 && !bypass {
            (beat / view.snap).round() * view.snap
        } else {
            beat
        })
        .clamp(0.0, MAX_BEAT)
    };
    if response.response.secondary_clicked()
        && let Some(point) = response.response.interact_pointer_pos()
    {
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                id.with("menu_beat"),
                snap(view.viewport.beat_at(view.rect, point.x)),
            )
        });
    }
    if !view.navigating {
        response.response.context_menu(|ui| {
            let beat = ui
                .ctx()
                .data(|data| data.get_temp::<f64>(id.with("menu_beat")))
                .unwrap_or(view.viewport.start_beat);
            if daw_ui::automation_button(ui, "Create automation clip")
                .on_hover_text("Derive a four-beat pattern from the arrangement curve here")
                .clicked()
            {
                result
                    .edits
                    .push(AutomationEdit::Create(beat, 4.0_f64.min(MAX_BEAT - beat)));
                ui.close();
            }
            if daw_ui::automation_button(ui, "Clear arranger points")
                .on_hover_text("Keep patterns; use the manual value outside them")
                .clicked()
            {
                result.edits.push(AutomationEdit::Clear);
                ui.close();
            }
            let clipboard = ui.ctx().data(|data| {
                data.get_temp::<AutomationClip>(egui::Id::new("automation_clipboard"))
            });
            if ui
                .add_enabled(
                    clipboard.is_some(),
                    daw_ui::button_widget("Paste automation clip"),
                )
                .clicked()
                && let Some(clip) = clipboard
            {
                result.edits.push(AutomationEdit::Paste(clip, beat));
                ui.close();
            }
        });
    }
    let mut clips: Vec<_> = lane.clips.iter().collect();
    clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut clip_diagnostics = Vec::new();
    for clip in clips {
        let rect = clip_rect(clip);
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let clip_id = id.with(clip.id);
        let header = daw_ui::automation_pattern_header(
            ui,
            clip_id,
            rect,
            daw_ui::AutomationPatternHeader {
                name: &clip.name,
                linked: clip.link_group.is_some(),
                selected: *selected == Some(clip.id),
                color: view.color,
            },
            !view.navigating,
        );
        if header.clicked() || header.drag_started() {
            *selected = Some(clip.id);
            result.interacted = true;
        }
        if header.drag_started() {
            let position = ui
                .input(|input| input.pointer.press_origin())
                .unwrap_or(rect.center());
            let edge = if position.x < rect.left() + 8.0 {
                -1
            } else if position.x > rect.right() - 8.0 {
                1
            } else {
                0
            };
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    clip_id.with("drag"),
                    PatternDrag {
                        original: clip.clone(),
                        edge,
                    },
                )
            });
        }
        if header.dragged()
            && let Some(drag) = ui
                .ctx()
                .data(|data| data.get_temp::<PatternDrag>(clip_id.with("drag")))
        {
            let delta = header.total_drag_delta().unwrap_or_default().x as f64
                / view.viewport.pixels_per_beat as f64;
            let original = drag.original;
            let (start, length, offset) = match drag.edge {
                -1 => {
                    let start = snap(original.start + delta).clamp(
                        (original.start - original.offset).max(0.0),
                        original.end() - original.length.min(0.0625),
                    );
                    (
                        start,
                        original.end() - start,
                        original.offset + start - original.start,
                    )
                }
                1 => (
                    original.start,
                    (snap(original.end() + delta) - original.start)
                        .max(0.0625)
                        .min(MAX_BEAT - original.start),
                    original.offset,
                ),
                _ => (
                    snap(original.start + delta).min(MAX_BEAT - original.length),
                    original.length,
                    original.offset,
                ),
            };
            if (start, length, offset) != (clip.start, clip.length, clip.offset) {
                result
                    .edits
                    .push(AutomationEdit::Bounds(clip.id, start, length, offset));
            }
        }
        if !view.navigating {
            header.context_menu(|ui| {
                if daw_ui::automation_button(ui, "Copy pattern").clicked() {
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(egui::Id::new("automation_clipboard"), clip.clone())
                    });
                    ui.close();
                }
                for (label, linked) in [("Duplicate pattern", false), ("Ghost pattern", true)] {
                    if daw_ui::automation_button(ui, label).clicked() {
                        result
                            .edits
                            .push(AutomationEdit::Copy(clip.id, clip.end(), linked));
                        ui.close();
                    }
                }
                if clip.link_group.is_some()
                    && daw_ui::automation_button(ui, "Make pattern independent").clicked()
                {
                    result.edits.push(AutomationEdit::Independent(clip.id));
                    ui.close();
                }
                if daw_ui::automation_button(ui, "Delete pattern").clicked() {
                    result.edits.push(AutomationEdit::Delete(clip.id));
                    ui.close();
                }
            });
        }
        let plot = Rect::from_min_max(
            rect.min + Vec2::new(0.0, 23.0_f32.min(rect.height() * 0.4)),
            rect.max - Vec2::new(0.0, 3.0),
        );
        let response = daw_ui::automation_canvas_at(
            ui,
            clip_id.with("points"),
            &points(&clip.points),
            AutomationCanvas {
                rect: plot,
                viewport: TimelineViewport {
                    start_beat: clip.offset,
                    ..view.viewport
                },
                snap: view.snap,
                color: view.color,
                editable: !view.navigating,
            },
            |beat| clip.curve_value_at(beat).unwrap_or(0.5),
        );
        if response.response.clicked() || response.response.dragged() || response.edit.is_some() {
            *selected = Some(clip.id);
            result.interacted = true;
        }
        if let Some(edit) = response.edit {
            result
                .edits
                .push(AutomationEdit::Curve(Some(clip.id), edit));
        }
        clip_diagnostics.push(serde_json::json!({"id":clip.id,"rect":rect_json(rect),"plot":rect_json(plot),"points":response.points.into_iter().map(|(id,rect)| serde_json::json!({"id":id,"rect":rect_json(rect)})).collect::<Vec<_>>()}));
    }
    if view.pointer_blocked {
        result.edits.retain(|edit| {
            !matches!(
                edit,
                AutomationEdit::Curve(_, CurveEdit::Add { .. } | CurveEdit::Move { .. })
                    | AutomationEdit::Bounds(..)
            )
        });
        result.interacted = !result.edits.is_empty();
    }
    result.interacted |= !result.edits.is_empty();
    result.diagnostic = serde_json::json!({"track":view.track,"target":lane.target,"rect":rect_json(view.rect),"clips":clip_diagnostics,
        "points":response.points.into_iter().map(|(id,rect)| serde_json::json!({"id":id,"rect":rect_json(rect)})).collect::<Vec<_>>()});
    result
}

impl DawApp {
    pub(crate) fn apply_arranger_automation(
        &mut self,
        track: u64,
        edit: AutomationEdit,
    ) -> anyhow::Result<()> {
        self.edit_label = Some("Edit automation".into());
        match edit {
            AutomationEdit::Clear => {
                self.project
                    .automation_lane_mut(track)
                    .ok_or_else(|| anyhow::anyhow!("Automation track not found"))?
                    .points
                    .clear();
                Ok(())
            }
            AutomationEdit::Curve(clip, edit) => {
                self.project
                    .edit_automation_pattern(track, clip, |lane| match edit {
                        CurveEdit::Add { x, value } => lane.upsert_point(x, value).map(|_| ()),
                        CurveEdit::Move { id, x, value } => lane.move_point(id, x, value),
                        CurveEdit::Delete { id } => lane.remove_point(id).map(|_| ()),
                        CurveEdit::Interpolation { id, mode } => {
                            let point = lane
                                .points
                                .iter_mut()
                                .find(|p| p.id == id)
                                .ok_or_else(|| anyhow::anyhow!("Automation point not found"))?;
                            point.interpolation = Some(match mode {
                                daw_ui::CurveInterpolation::Step => Interpolation::Step,
                                daw_ui::CurveInterpolation::Linear => Interpolation::Linear,
                                daw_ui::CurveInterpolation::Smooth => Interpolation::Smooth,
                            });
                            Ok(())
                        }
                    })
            }
            AutomationEdit::Create(start, length) => self
                .project
                .create_automation_clip(track, start, length)
                .map(|id| {
                    self.selected_track = Some(track);
                    self.selected_clip = Some(id);
                }),
            AutomationEdit::Copy(id, start, linked) => self
                .project
                .copy_automation_clip(track, id, start, linked)
                .map(|id| self.selected_clip = Some(id)),
            AutomationEdit::Paste(clip, start) => self
                .project
                .paste_automation_clip(track, &clip, start)
                .map(|id| self.selected_clip = Some(id)),
            AutomationEdit::Bounds(id, start, length, offset) => self
                .project
                .set_automation_clip_bounds(track, id, start, length, offset),
            AutomationEdit::Delete(id) => self
                .project
                .remove_automation_clip(track, id)
                .map(|()| self.selected_clip = None),
            AutomationEdit::Independent(id) => {
                self.project.make_automation_clip_independent(track, id)
            }
        }
    }
}

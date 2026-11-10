//! Named song positions on a timeline lane: markers and meter changes.
//! The caller owns the values; this lane reports clicks and finished drags.
use crate::{TimelineViewport, theme, tokens};
use egui::{Color32, FontId, Id, Pos2, Rect, Response, Sense, Stroke, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimelineFlagKind {
    Marker,
    Meter,
}

#[derive(Clone, Debug)]
pub struct TimelineFlag {
    /// Stable within its kind: a marker ID or the bar of a meter change.
    pub key: u64,
    pub kind: TimelineFlagKind,
    pub beat: f64,
    pub label: String,
    pub hover: String,
    pub color: Option<Color32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimelineFlagAction {
    Clicked(TimelineFlagKind, u64),
    DoubleClicked(TimelineFlagKind, u64),
    /// A marker drag finished at a different beat.
    Moved(u64, f64),
}

pub struct TimelineFlagsResponse {
    /// The lane background, for an empty-space context menu or click.
    pub lane: Response,
    pub action: Option<TimelineFlagAction>,
    pub flags: Vec<(TimelineFlagKind, u64, Rect)>,
}

#[derive(Clone, Copy)]
struct FlagDrag {
    key: u64,
    original: f64,
    pointer: f32,
}

/// Paint the flags left to right, pushing overlapping labels apart while each
/// keeps its exact position line. Markers drag along the timeline; `snap`
/// receives the raw beat and whether snapping is bypassed (Shift).
#[allow(clippy::too_many_arguments)]
pub fn timeline_flags(
    ui: &mut Ui,
    id: Id,
    lane: Rect,
    view: TimelineViewport,
    flags: &[TimelineFlag],
    enabled: bool,
    snap: &dyn Fn(f64, bool) -> f64,
    mut menu: impl FnMut(&mut Ui, &TimelineFlag),
) -> TimelineFlagsResponse {
    let lane_response = ui.interact(lane, id, Sense::click());
    let colors = theme::palette(ui.ctx());
    let painter = ui.painter_at(lane);
    painter.rect_filled(lane, 0.0, colors.bg);
    let drag_id = id.with("drag");
    let drag = ui.data(|data| data.get_temp::<FlagDrag>(drag_id));
    let font = FontId::proportional(tokens::FONT_CAPTION);
    let inset = tokens::BORDER_WIDTH * 2.0;
    let mut placed: Vec<(usize, f64)> = flags
        .iter()
        .enumerate()
        .map(|(index, flag)| {
            let beat = match drag {
                Some(drag) if flag.kind == TimelineFlagKind::Marker && drag.key == flag.key => ui
                    .input(|input| input.pointer.interact_pos())
                    .map_or(flag.beat, |pointer| {
                        let raw = drag.original
                            + f64::from((pointer.x - drag.pointer) / view.pixels_per_beat);
                        snap(raw.max(0.0), ui.input(|input| input.modifiers.shift)).max(0.0)
                    }),
                _ => flag.beat,
            };
            (index, beat)
        })
        .collect();
    placed.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut right_edge = f32::NEG_INFINITY;
    let mut action = None;
    let mut rects = Vec::with_capacity(flags.len());
    for (index, beat) in placed {
        let flag = &flags[index];
        let x = view.x_at(lane, beat);
        let galley = painter.layout_no_wrap(flag.label.clone(), font.clone(), colors.text);
        let width = galley.size().x + 2.0 * tokens::SPACE_XS;
        let left = x.max(right_edge + tokens::BORDER_WIDTH);
        let rect = Rect::from_min_max(
            Pos2::new(left, lane.top() + inset),
            Pos2::new(left + width, lane.bottom() - inset),
        );
        right_edge = rect.right();
        if rect.right() < lane.left() || x > lane.right() {
            continue;
        }
        let fill = match flag.kind {
            TimelineFlagKind::Marker => flag.color.unwrap_or(colors.guide),
            TimelineFlagKind::Meter => colors.raised,
        };
        let ink = theme::contrast_text(fill);
        painter.vline(x, lane.y_range(), Stroke::new(1.0_f32, fill));
        painter.rect_filled(rect, 2.0, fill);
        if flag.kind == TimelineFlagKind::Meter {
            painter.rect_stroke(
                rect,
                2.0,
                Stroke::new(tokens::BORDER_WIDTH, colors.line),
                egui::StrokeKind::Inside,
            );
        }
        let galley = painter.layout_no_wrap(flag.label.clone(), font.clone(), ink);
        painter.galley(
            Pos2::new(
                rect.left() + tokens::SPACE_XS,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            ink,
        );
        let sense = match (enabled, flag.kind) {
            (false, _) => Sense::hover(),
            (true, TimelineFlagKind::Marker) => Sense::click_and_drag(),
            (true, TimelineFlagKind::Meter) => Sense::click(),
        };
        let response = ui
            .interact(rect.intersect(lane), id.with((flag.kind, flag.key)), sense)
            .on_hover_text(&flag.hover);
        // Measure from the press, not the point where the drag threshold passed.
        if response.drag_started()
            && let Some(pointer) = ui.input(|input| input.pointer.press_origin())
        {
            ui.data_mut(|data| {
                data.insert_temp(
                    drag_id,
                    FlagDrag {
                        key: flag.key,
                        original: flag.beat,
                        pointer: pointer.x,
                    },
                )
            });
        }
        let moved = response.drag_stopped()
            && flag.kind == TimelineFlagKind::Marker
            && beat.to_bits() != flag.beat.to_bits();
        if response.drag_stopped() {
            ui.data_mut(|data| data.remove::<FlagDrag>(drag_id));
        }
        if moved {
            action = Some(TimelineFlagAction::Moved(flag.key, beat));
        } else if response.clicked()
            && let Some(position) = response.interact_pointer_pos()
        {
            // The shared tracker counts the second release, independent of
            // frame timing, exactly like arranger clips.
            let tracker_id = id.with("double_click");
            let mut tracker: crate::ItemClickTracker = ui
                .data(|data| data.get_temp(tracker_id))
                .unwrap_or_default();
            let item = flag.key.wrapping_mul(2) + u64::from(flag.kind == TimelineFlagKind::Meter);
            let now = ui.input(|input| input.time);
            let plain = !ui.input(|input| input.modifiers.any());
            action = Some(if tracker.register(item, position, now, plain) {
                TimelineFlagAction::DoubleClicked(flag.kind, flag.key)
            } else {
                TimelineFlagAction::Clicked(flag.kind, flag.key)
            });
            ui.data_mut(|data| data.insert_temp(tracker_id, tracker));
        }
        if enabled {
            response.context_menu(|ui| menu(ui, flag));
        }
        rects.push((flag.kind, flag.key, rect));
    }
    if drag.is_some() && !ui.input(|input| input.pointer.any_down()) && action.is_none() {
        ui.data_mut(|data| data.remove::<FlagDrag>(drag_id));
    }
    TimelineFlagsResponse {
        lane: lane_response,
        action,
        flags: rects,
    }
}

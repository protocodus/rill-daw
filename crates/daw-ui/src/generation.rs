//! Shared, non-destructive musical generation controls and canvas previews.
//! The caller owns the draft notes, musical range, confirmation, and Undo.
use crate::{TimelineViewport, text, theme};
use egui::{Color32, Painter, Pos2, Rect, Response, Stroke, StrokeKind, Ui};

pub struct GenerationBarRangeResponse {
    pub start: Response,
    pub length: Response,
}

/// All twelve pitch classes stay visible without scrolling a long menu.
pub fn generation_root_selector(ui: &mut Ui, root: &mut u8) -> Response {
    const NAMES: [&str; 12] = [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ];
    crate::select("generation_root", NAMES[usize::from(*root % 12)])
        .show_ui(ui, |ui| {
            egui::Grid::new("generation_pitch_classes")
                .num_columns(3)
                .show(ui, |ui| {
                    for (pitch, name) in NAMES.into_iter().enumerate() {
                        crate::selectable_value(ui, root, pitch as u8, name);
                        if pitch % 3 == 2 {
                            ui.end_row();
                        }
                    }
                });
        })
        .response
}

impl GenerationBarRangeResponse {
    pub fn changed(&self) -> bool {
        self.start.changed() || self.length.changed()
    }
}

/// One-based start and duration in whole bars. The end shown below is inclusive.
/// Zero length represents a viewport with no complete bars, awaiting a range edit.
/// Native fields retain dragging and typing; visible steps and the shared value
/// keypad also allow precise entry using only the mouse.
pub fn generation_bar_range(
    ui: &mut Ui,
    start_bar: &mut u32,
    length_bars: &mut u32,
) -> GenerationBarRangeResponse {
    *start_bar = (*start_bar).clamp(1, 1_000_000);
    *length_bars = (*length_bars).min(1024);
    let wide = ui.available_width() >= 520.0;
    let response = egui::Grid::new("generation_bar_range")
        .num_columns(if wide { 4 } else { 2 })
        .min_col_width(if wide {
            // Grid labels need a real width on the first sizing pass. A zero
            // minimum wraps "Start bar" one character per line and inflates
            // the supposedly compact header before column widths stabilize.
            89.0
        } else {
            crate::tokens::SELECT_WIDTH
        })
        .spacing(egui::vec2(crate::tokens::SPACE_LG, crate::tokens::SPACE_MD))
        .min_row_height(crate::tokens::CONTROL_HEIGHT)
        .show(ui, |ui| {
            let label = text::truncated(ui, text::TextRole::Label, "Start bar");
            let mut start_value = f64::from(*start_bar);
            let start = ui
                .push_id("generation_start", |ui| {
                    crate::number_stepper_integer(
                        ui,
                        &mut start_value,
                        1.0,
                        1.0..=1_000_000.0,
                        "Start bar",
                        |ui, value| {
                            ui.add(
                                crate::number(value)
                                    .range(1.0..=1_000_000.0)
                                    .speed(0.1)
                                    .max_decimals(0),
                            )
                        },
                    )
                    .response
                })
                .inner
                .labelled_by(label.id);
            if start.changed() {
                *start_bar = start_value.round().clamp(1.0, 1_000_000.0) as u32;
            }
            if !wide {
                ui.end_row();
            }
            let label = text::truncated(ui, text::TextRole::Label, "Length");
            let mut length_value = f64::from(*length_bars);
            let length = ui
                .push_id("generation_length", |ui| {
                    crate::number_stepper_integer(
                        ui,
                        &mut length_value,
                        1.0,
                        0.0..=1024.0,
                        "Length",
                        |ui, value| {
                            ui.add(
                                crate::number(value)
                                    .range(0.0..=1024.0)
                                    .speed(0.1)
                                    .max_decimals(0)
                                    .suffix(" bars"),
                            )
                        },
                    )
                    .response
                })
                .inner
                .labelled_by(label.id);
            if length.changed() {
                *length_bars = length_value.round().clamp(0.0, 1024.0) as u32;
            }
            ui.end_row();
            GenerationBarRangeResponse { start, length }
        })
        .inner;
    let summary = if *length_bars == 0 {
        "No complete bars in view".to_owned()
    } else {
        format!("Bars {}–{}", start_bar, *start_bar + *length_bars - 1)
    };
    if wide {
        response.length.clone().on_hover_text(summary);
    } else {
        text::note(ui, summary);
    }
    response
}

pub struct GenerationFooterResponse {
    pub status: Rect,
    pub add: Response,
    pub next: Response,
    pub close: Response,
}

/// Reserve feedback space before the first addition, so errors, successful
/// additions and subsequent drafts never move the repeatable action targets.
/// Feedback stays bounded to two lines, with its full text available on hover.
pub fn generation_footer(
    ui: &mut Ui,
    status: &str,
    can_add: bool,
    can_advance: bool,
    has_added: bool,
) -> GenerationFooterResponse {
    generation_footer_inner(ui, status, can_add, can_advance, has_added, None).0
}

/// Compact utilities keep preview controls beside their bounded feedback,
/// leaving the scroll body for generation parameters. Returns whether Fit was
/// requested; the caller still owns every musical and viewport change.
pub fn generation_footer_with_preview(
    ui: &mut Ui,
    status: &str,
    can_add: bool,
    can_advance: bool,
    has_added: bool,
    preview: &mut bool,
) -> (GenerationFooterResponse, bool) {
    generation_footer_inner(ui, status, can_add, can_advance, has_added, Some(preview))
}

fn generation_footer_inner(
    ui: &mut Ui,
    status: &str,
    can_add: bool,
    can_advance: bool,
    has_added: bool,
    preview: Option<&mut bool>,
) -> (GenerationFooterResponse, bool) {
    let mut fit = false;
    let status_size = egui::vec2(ui.available_width(), crate::tokens::CONTROL_HEIGHT);
    let status = ui.allocate_ui_with_layout(
        status_size,
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_min_size(status_size);
            if let Some(preview) = preview {
                fit = crate::action_row(
                    ui,
                    |ui| {
                        let fit = crate::view_button(ui, "Fit preview").clicked();
                        crate::checkbox(ui, preview, "Preview");
                        fit
                    },
                    |ui| text::bounded_block(ui, text::TextRole::Note, status, 2),
                )
                .inner
                .0;
            } else {
                text::bounded_block(ui, text::TextRole::Note, status, 2);
            }
        },
    );
    let (add, next, close) = ui
        .allocate_ui_with_layout(
            egui::vec2(ui.available_width(), crate::tokens::CONTROL_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
            |ui| {
                let add = ui.add_enabled(can_add, crate::primary_button_widget(ui, "Add notes"));
                let next = ui
                    .add_enabled(can_advance, crate::button_widget("Next range"))
                    .on_hover_text("Move forward by this range's length, keeping your settings");
                let close = ui.add(
                    crate::button_widget(if has_added { "Done" } else { "Cancel" }).min_size(
                        egui::vec2(
                            crate::tokens::CONTROL_HEIGHT * 3.0,
                            crate::tokens::CONTROL_HEIGHT,
                        ),
                    ),
                );
                (add, next, close)
            },
        )
        .inner;
    (
        GenerationFooterResponse {
            status: status.response.rect,
            add,
            next,
            close,
        },
        fit,
    )
}

/// Violet is reserved here for draft notes, separate from blue committed notes
/// and teal action accents. Additional voices retain distinct nearby shades.
pub fn generation_preview_color(ctx: &egui::Context, voice: u8) -> Color32 {
    let colors = if ctx.style().visuals.dark_mode {
        [
            [190, 158, 248],
            [226, 157, 223],
            [149, 167, 247],
            [208, 183, 242],
        ]
    } else {
        [
            [113, 66, 181],
            [155, 61, 148],
            [75, 86, 177],
            [134, 84, 164],
        ]
    };
    let [r, g, b] = colors[usize::from(voice) % colors.len()];
    Color32::from_rgb(r, g, b)
}

/// Lightly dim the live workspace before painting the active range and draft
/// notes. Unlike a frozen utility backdrop, the musical preview stays readable
/// and responds immediately to the dialog. This creates no hit targets.
pub fn paint_generation_backdrop(painter: &Painter, bounds: Rect) {
    if !bounds.is_finite() || !bounds.is_positive() {
        return;
    }
    painter
        .with_clip_rect(bounds.intersect(painter.clip_rect()))
        .rect_filled(
            bounds,
            0.0,
            theme::palette(painter.ctx())
                .popup_scrim
                .gamma_multiply(0.35),
        );
}

/// Highlight the same time range in the ruler and roll without altering either
/// layout. Only actual start/end boundaries receive lines; clipped edges do not
/// pretend to be musical range boundaries. Returns the visible plot portion.
pub fn paint_generation_range(
    painter: &Painter,
    plot: Rect,
    ruler: Rect,
    view: TimelineViewport,
    start: f64,
    end: f64,
) -> Option<Rect> {
    if !plot.is_finite()
        || !plot.is_positive()
        || !ruler.is_finite()
        || !ruler.is_positive()
        || !start.is_finite()
        || !end.is_finite()
        || end <= start
        || !view.start_beat.is_finite()
        || !view.pixels_per_beat.is_finite()
        || view.pixels_per_beat <= 0.0
    {
        return None;
    }
    let left = view.x_at(plot, start);
    let right = view.x_at(plot, end);
    let visible = Rect::from_min_max(
        Pos2::new(left.max(plot.left()), plot.top()),
        Pos2::new(right.min(plot.right()), plot.bottom()),
    )
    .intersect(painter.clip_rect());
    if !visible.is_positive() {
        return None;
    }
    let color = generation_preview_color(painter.ctx(), 0);
    let plot_painter = painter.with_clip_rect(plot.intersect(painter.clip_rect()));
    plot_painter.rect_filled(visible, 0.0, color.gamma_multiply(0.10));
    let ruler_painter = painter.with_clip_rect(ruler.intersect(painter.clip_rect()));
    let ruler_range = Rect::from_min_max(
        Pos2::new(visible.left(), ruler.top()),
        Pos2::new(visible.right(), ruler.bottom()),
    );
    ruler_painter.rect_filled(ruler_range, 0.0, color.gamma_multiply(0.18));
    ruler_painter.line_segment(
        [ruler_range.left_bottom(), ruler_range.right_bottom()],
        Stroke::new(3.0_f32, color),
    );
    for x in [left, right] {
        if (plot.left()..=plot.right()).contains(&x) {
            plot_painter.line_segment(
                [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                Stroke::new(1.25_f32, color.gamma_multiply(0.8)),
            );
        }
    }
    Some(visible)
}

/// Paint a ghost preview above existing notes, clipped to the caller's roll.
/// This does not create a selectable note or allocate a persistent note ID.
pub fn paint_generation_note(painter: &Painter, rect: Rect, voice: u8) -> Option<Rect> {
    if !rect.is_finite() || !rect.is_positive() {
        return None;
    }
    let visible = rect.intersect(painter.clip_rect());
    if !visible.is_positive() {
        return None;
    }
    let color = generation_preview_color(painter.ctx(), voice);
    painter.rect_filled(rect, 2.0, color.gamma_multiply(0.28));
    painter.rect_stroke(rect, 2.0, Stroke::new(1.5_f32, color), StrokeKind::Inside);
    Some(visible)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, Event, Modifiers, PointerButton, RawInput, Vec2};

    fn controls(
        ctx: &Context,
        start: &mut u32,
        length: &mut u32,
        events: Vec<Event>,
    ) -> GenerationBarRangeResponse {
        let mut response = None;
        let _ = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 400.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    response = Some(generation_bar_range(ui, start, length));
                });
            },
        );
        response.unwrap()
    }

    #[test]
    fn bar_fields_are_independent_native_drag_targets_with_bounded_values() {
        let ctx = Context::default();
        theme::apply(&ctx);
        let (mut start, mut length) = (1, 4);
        let initial = controls(&ctx, &mut start, &mut length, vec![]);
        assert!(!initial.start.rect.intersects(initial.length.rect));
        let pos = initial.start.rect.center();
        controls(
            &ctx,
            &mut start,
            &mut length,
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        controls(
            &ctx,
            &mut start,
            &mut length,
            vec![Event::PointerMoved(pos + Vec2::new(20.0, 0.0))],
        );
        let edited = controls(
            &ctx,
            &mut start,
            &mut length,
            vec![Event::PointerMoved(pos + Vec2::new(60.0, 0.0))],
        );
        assert!(edited.changed());
        assert!(start > 1);
        assert_eq!(length, 4);
        start = 0;
        length = u32::MAX;
        controls(&ctx, &mut start, &mut length, vec![]);
        assert_eq!((start, length), (1, 1024));
    }

    #[test]
    fn preview_painters_clip_ranges_and_notes_in_both_themes() {
        let ctx = Context::default();
        let plot = Rect::from_min_max(Pos2::new(40.0, 40.0), Pos2::new(440.0, 300.0));
        let ruler = Rect::from_min_max(Pos2::new(40.0, 0.0), Pos2::new(440.0, 40.0));
        for appearance in [theme::Appearance::Dark, theme::Appearance::Light] {
            theme::set_appearance(&ctx, appearance);
            let output = ctx.run(RawInput::default(), |ctx| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                let view = TimelineViewport {
                    start_beat: 2.0,
                    pixels_per_beat: 50.0,
                    ..Default::default()
                };
                let active = paint_generation_range(&painter, plot, ruler, view, 0.0, 8.0).unwrap();
                assert_eq!(
                    active,
                    Rect::from_min_max(plot.min, Pos2::new(340.0, 300.0))
                );
                assert!(paint_generation_range(&painter, plot, ruler, view, 12.0, 16.0).is_none());
                assert!(
                    paint_generation_range(&painter, plot, ruler, view, f64::NAN, 8.0).is_none()
                );
                let note = Rect::from_min_max(Pos2::new(10.0, 50.0), Pos2::new(100.0, 70.0));
                assert_eq!(
                    paint_generation_note(&painter.with_clip_rect(plot), note, 1),
                    Some(note.intersect(plot))
                );
                assert_ne!(generation_preview_color(ctx, 0), theme::palette(ctx).note);
                assert_ne!(
                    generation_preview_color(ctx, 0),
                    generation_preview_color(ctx, 1)
                );
            });
            assert!(!output.shapes.is_empty());
            assert!(
                output
                    .shapes
                    .iter()
                    .all(|shape| shape.clip_rect.is_finite())
            );
        }
    }
}

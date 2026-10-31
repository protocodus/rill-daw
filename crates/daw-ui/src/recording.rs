//! Recording controls and live regions share the same theme, spacing and input
//! behavior in the transport, track headers and focused recording view.
use crate::{Icon, IconButton, text, theme};
use egui::{Color32, Painter, Rect, Response, Stroke, StrokeKind, Ui};

pub fn recording_color(ctx: &egui::Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_rgb(222, 120, 126)
    } else {
        Color32::from_rgb(164, 56, 67)
    }
}

pub fn record_button(ui: &mut Ui, active: bool, enabled: bool) -> Response {
    let label = if active { "Stop recording" } else { "Record" };
    let mut button = IconButton::new(label);
    if active {
        button = button.fill(recording_color(ui.ctx()).gamma_multiply(0.2));
    } else {
        button = button.quiet();
    }
    ui.add_enabled(enabled, button)
}

/// Shared recording options. Timing choices are distinct from the Click toggle;
/// changing the click never changes what is recorded or exported.
pub fn recording_timing_controls(
    ui: &mut Ui,
    precount: &mut u8,
    click: &mut bool,
    gain: &mut f32,
    timing_enabled: bool,
) -> bool {
    let before = (*precount, *click, *gain);
    crate::form_grid(ui, "recording_timing", |ui| {
        let label = text::label(ui, "Pre-count");
        ui.add_enabled_ui(timing_enabled, |ui| {
            crate::select("recording_precount", match *precount { 1 => "1 bar", 2 => "2 bars", _ => "No pre-count" })
                .show_ui(ui, |ui| {
                    for (value, title) in [(0, "No pre-count"), (1, "1 bar"), (2, "2 bars")] {
                        crate::selectable_value(ui, precount, value, title);
                    }
                }).response.labelled_by(label.id)
                .on_hover_text("Count in before recording from stopped. Recording during playback starts immediately.");
        });
        ui.end_row();
        text::label(ui, "Metronome");
        crate::toggle(ui, click, "Click").on_hover_text(
            "Play a metronome during playback and recording. Pre-count clicks always play.",
        );
        ui.end_row();
        let label = text::label(ui, "Click level");
        ui.add(crate::slider(gain, 0.0..=1.0)).labelled_by(label.id);
        ui.end_row();
    });
    before != (*precount, *click, *gain)
}

pub fn record_arm_at(
    ui: &mut Ui,
    rect: Rect,
    id: egui::Id,
    armed: bool,
    enabled: bool,
) -> Response {
    // The arranger owns this rectangle. A layout scope would advance the
    // parent row and move subsequent tracks and their drop targets.
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(rect));
    if !enabled {
        child.disable();
    }
    let color = if armed {
        recording_color(child.ctx())
    } else {
        theme::palette(child.ctx()).muted
    };
    let response = child.interact(rect, id, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            child.is_enabled(),
            armed,
            "Arm recording",
        )
    });
    let painter = child.painter().with_clip_rect(rect);
    if armed || response.hovered() || response.has_focus() {
        painter.rect(
            rect.shrink(1.0),
            4.0,
            color.gamma_multiply(0.14),
            Stroke::new(1.0_f32, color),
            StrokeKind::Inside,
        );
    }
    Icon::Record.paint(&painter, rect.shrink(4.0), color);
    response.on_hover_text(if armed {
        "Disarm recording"
    } else {
        "Arm recording"
    })
}

pub fn paint_recording_region(painter: &Painter, rect: Rect) {
    if !rect.is_finite() || !rect.is_positive() {
        return;
    }
    let color = recording_color(painter.ctx());
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    painter.rect(
        rect,
        3.0,
        color.gamma_multiply(0.12),
        Stroke::new(1.5_f32, color),
        StrokeKind::Inside,
    );
    text::paint_single_line(
        &painter,
        rect.shrink(6.0),
        text::TextRole::Label.font(),
        color,
        "Recording",
    );
}

/// A quiet raw-input meter. It reports level before channel gain and pan.
pub fn input_meter(ui: &mut Ui, peaks: [f32; 2]) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
    let colors = theme::palette(ui.ctx());
    let peak = peaks[0].max(peaks[1]).max(0.0);
    let fraction = ((20.0 * peak.max(1e-5).log10() + 60.0) / 60.0).clamp(0.0, 1.0);
    ui.painter().rect_filled(rect, 3.0, colors.control_fill);
    ui.painter().rect_filled(
        Rect::from_min_size(rect.min, egui::vec2(rect.width() * fraction, rect.height())),
        3.0,
        if peak >= 1.0 {
            recording_color(ui.ctx())
        } else {
            colors.accent
        },
    );
    response.on_hover_text(if peak == 0.0 {
        "Input level · Silent".into()
    } else {
        format!("Input level · {:.1} dBFS", 20.0 * peak.log10())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, vec2};

    fn frame(
        ctx: &Context,
        events: Vec<Event>,
        armed: &mut bool,
        row_clicks: &mut usize,
        enabled: bool,
    ) -> (Rect, Rect) {
        let mut bounds = (Rect::NOTHING, Rect::NOTHING);
        let _ = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 480.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let (row, row_response) =
                        ui.allocate_exact_size(vec2(248.0, 44.0), egui::Sense::click());
                    let arm = Rect::from_min_size(row.min + vec2(200.0, 8.0), vec2(22.0, 22.0));
                    let before = (ui.cursor(), ui.min_rect(), ui.available_rect_before_wrap());
                    let response = record_arm_at(ui, arm, egui::Id::new("arm"), *armed, enabled);
                    assert_eq!(
                        (ui.cursor(), ui.min_rect(), ui.available_rect_before_wrap()),
                        before,
                        "record arm must not move rows or their drop targets"
                    );
                    assert_eq!(response.rect, arm);
                    if response.clicked() {
                        *armed = !*armed;
                    }
                    *row_clicks += usize::from(row_response.clicked());
                    bounds = (arm, row);
                });
            },
        );
        bounds
    }

    fn pointer(pos: Pos2, pressed: bool) -> Vec<Event> {
        vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn record_arm_preserves_canvas_layout_and_clicks_only_its_control() {
        let ctx = Context::default();
        theme::apply(&ctx);
        let mut armed = false;
        let mut row_clicks = 0;
        let (arm, row) = frame(&ctx, vec![], &mut armed, &mut row_clicks, true);
        for pressed in [true, false] {
            frame(
                &ctx,
                pointer(arm.center(), pressed),
                &mut armed,
                &mut row_clicks,
                true,
            );
        }
        assert!(armed, "real pointer click must arm the track");
        assert_eq!(row_clicks, 0, "arming must not also activate the row");
        for pressed in [true, false] {
            frame(
                &ctx,
                pointer(arm.center(), pressed),
                &mut armed,
                &mut row_clicks,
                false,
            );
        }
        assert!(armed, "disabled arm must retain its value");
        row_clicks = 0;
        for pressed in [true, false] {
            frame(
                &ctx,
                pointer(row.min + vec2(20.0, 20.0), pressed),
                &mut armed,
                &mut row_clicks,
                true,
            );
        }
        assert!(armed);
        assert_eq!(row_clicks, 1, "the rest of the row remains interactive");
    }
}

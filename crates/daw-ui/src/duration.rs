//! Exact time targets for musical ranges, without requiring keyboard entry.
use egui::Ui;

/// Millisecond-accurate elapsed time, shared by rulers, properties and export.
pub fn format_seconds(seconds: f64) -> String {
    let milliseconds = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{}:{:02}.{:03}",
        milliseconds / 60_000,
        milliseconds / 1000 % 60,
        milliseconds % 1000,
    )
}

pub fn musical_position(beat: f64, beats_per_bar: u8) -> String {
    let beats_per_bar = f64::from(beats_per_bar.max(1));
    let beat = beat.max(0.0);
    format!(
        "Bar {} · Beat {:.2}",
        (beat / beats_per_bar).floor() as u64 + 1,
        beat % beats_per_bar + 1.0,
    )
}

/// Read-only conversions in a property grid. Timing is still edited by the
/// caller, so inspecting either unit cannot round the project or add history.
pub fn timing_summary(ui: &mut Ui, start: f64, length: f64, tempo: f64, beats_per_bar: u8) {
    let seconds_per_beat = 60.0 / tempo;
    crate::text::label(ui, "Time");
    ui.vertical(|ui| {
        // Readouts do not need full-height input rows. Keep them together so
        // the clip's editable gain and fades remain nearby on small displays.
        ui.spacing_mut().item_spacing.y = crate::tokens::SPACE_XS;
        ui.spacing_mut().interact_size.y = 0.0;
        for (label, beat) in [("Starts at", start), ("Ends at", start + length)] {
            ui.horizontal(|ui| {
                crate::text::note(ui, label);
                crate::text::note(
                    ui,
                    format!(
                        "{} · {}",
                        format_seconds(beat * seconds_per_beat),
                        musical_position(beat, beats_per_bar),
                    ),
                );
            });
        }
        ui.horizontal(|ui| {
            crate::text::note(ui, "Elapsed");
            crate::text::note(
                ui,
                format!(
                    "{} · {:.3} bars",
                    format_seconds(length * seconds_per_beat),
                    length / f64::from(beats_per_bar.max(1)),
                ),
            );
        });
    });
    ui.end_row();
}

/// Reserve a labelled loop strip below the ruler's existing clear gap. Keep
/// this space even when looping is off so toggling never moves the canvas.
pub fn loop_ruler_height(ctx: &egui::Context) -> f32 {
    let label_height = ctx
        .fonts_mut(|fonts| fonts.row_height(&egui::FontId::monospace(crate::tokens::FONT_SCALE)));
    crate::tokens::TIMELINE_RULER_HEIGHT
        + crate::tokens::SPACE_SM
        + (label_height.ceil() + crate::tokens::SPACE_XS)
            .max(crate::tokens::TIMELINE_LOOP_STRIP_HEIGHT)
}

/// Keep loop timing readable in its existing ruler lane without adding targets
/// over the timeline. A narrow range retains its colored region and hover hint.
pub fn paint_loop_region(
    painter: &egui::Painter,
    rect: egui::Rect,
    start: f64,
    end: f64,
    tempo: f64,
    beats_per_bar: u8,
) {
    let colors = crate::theme::palette(painter.ctx());
    let fill = colors.accent.gamma_multiply(0.6);
    let painter = painter.with_clip_rect(rect);
    painter.rect_filled(rect, 2.0, fill);
    let meter = f64::from(beats_per_bar.max(1));
    let position = |beat: f64| {
        format!(
            "{}:{:.2}",
            (beat / meter).floor() as u64 + 1,
            beat % meter + 1.0
        )
    };
    let label = format!(
        "Loop {}–{} · {}–{}",
        position(start),
        position(end),
        format_seconds(start * 60.0 / tempo),
        format_seconds(end * 60.0 / tempo),
    );
    let galley = painter.layout_no_wrap(
        label,
        egui::FontId::monospace(crate::tokens::FONT_SCALE),
        crate::theme::contrast_text(fill),
    );
    if galley.size().x + 2.0 * crate::tokens::SPACE_XS <= rect.width()
        && galley.size().y <= rect.height()
    {
        painter.galley(
            egui::pos2(
                rect.left() + crate::tokens::SPACE_XS,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            crate::theme::contrast_text(fill),
        );
    }
}

/// A small grip on a real loop endpoint. Clipping the range must never invent
/// an editable endpoint at the viewport boundary. Narrow ranges divide their
/// hit areas at the midpoint so both ends remain independently reachable.
pub fn loop_endpoint_handle(
    ui: &Ui,
    id: egui::Id,
    region: egui::Rect,
    lane: egui::Rect,
    start: bool,
    enabled: bool,
) -> Option<egui::Response> {
    let x = if start { region.left() } else { region.right() };
    if !region.is_positive() || x < lane.left() || x > lane.right() {
        return None;
    }
    let midpoint = region.center().x;
    let rect = egui::Rect::from_min_max(
        egui::pos2(
            if start {
                x - crate::tokens::SPACE_SM
            } else {
                (x - crate::tokens::SPACE_SM).max(midpoint)
            },
            region.top(),
        ),
        egui::pos2(
            if start {
                (x + crate::tokens::SPACE_SM).min(midpoint)
            } else {
                x + crate::tokens::SPACE_SM
            },
            region.bottom(),
        ),
    )
    .intersect(lane)
    .intersect(ui.clip_rect());
    if !rect.is_positive() {
        return None;
    }
    let response = ui.interact(
        rect,
        id,
        if enabled {
            egui::Sense::click_and_drag()
        } else {
            egui::Sense::hover()
        },
    );
    let active = enabled && (response.hovered() || response.dragged());
    if active {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let colors = crate::theme::palette(ui.ctx());
    let ink = crate::theme::contrast_text(colors.accent.gamma_multiply(0.6));
    let grip_x = x.clamp(lane.left() + 1.0, lane.right() - 1.0);
    ui.painter().with_clip_rect(lane).vline(
        grip_x,
        (region.top() + 3.0)..=(region.bottom() - 3.0),
        egui::Stroke::new(
            if active { 2.0_f32 } else { 1.0_f32 },
            ink.gamma_multiply(if active { 1.0 } else { 0.65 }),
        ),
    );
    Some(response.on_hover_text(if enabled {
        if start {
            "Drag loop start · Uses Snap; Shift bypasses Snap"
        } else {
            "Drag loop end · Uses Snap; Shift bypasses Snap"
        }
    } else {
        "Finish recording before changing the loop range"
    }))
}

/// Offer common short-form durations while keeping their interpretation with
/// the caller. Opening the menu never rounds or changes the current duration.
pub fn duration_presets(ui: &mut Ui, seconds: f64, maximum_seconds: f64) -> Option<f64> {
    let mut chosen = None;
    crate::choice_menu(ui, format!("Duration · {seconds:.3} s"), |ui| {
        for duration in [5.0, 10.0, 15.0, 30.0, 60.0, 120.0] {
            if ui
                .add_enabled(
                    duration <= maximum_seconds,
                    crate::button_widget(format!("{duration:.0} seconds")),
                )
                .clicked()
            {
                chosen = Some(duration);
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Set an exact duration from the loop start. Use Repeat to loop end to fill it with the selected phrase.");
    chosen
}

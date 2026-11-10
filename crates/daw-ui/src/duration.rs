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

/// Two-decimal one-based position. Round the complete display position before
/// splitting it so the last fractional beat carries into the following bar.
/// This is a readout only; editors retain the original beat value separately.
pub fn musical_position(beat: f64, meter: &crate::MeterMap) -> String {
    let (bar, offset) = meter.position((beat.max(0.0) * 100.0).round() / 100.0);
    let within_bar = (offset * 100.0).round() as u64 + 100;
    format!(
        "Bar {bar} · Beat {}.{:02}",
        within_bar / 100,
        within_bar % 100,
    )
}

/// Compact one-based bar:beat position. Round the whole position before
/// decomposing it so a displayed fractional beat carries into the next bar.
pub fn compact_musical_position(beat: f64, meter: &crate::MeterMap) -> String {
    let (bar, offset) = meter.position((beat.max(0.0) * 1_000.0).round() / 1_000.0);
    let within_bar = (offset * 1_000.0).round() as u64 + 1_000;
    let fractional = within_bar % 1_000;
    if fractional == 0 {
        format!("{bar}:{}", within_bar / 1_000)
    } else {
        format!("{bar}:{}.{fractional:03}", within_bar / 1_000)
            .trim_end_matches('0')
            .to_owned()
    }
}

/// Read-only conversions in a property grid. Timing is still edited by the
/// caller, so inspecting either unit cannot round the project or add history.
pub fn timing_summary(ui: &mut Ui, start: f64, length: f64, tempo: f64, meter: &crate::MeterMap) {
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
                        musical_position(beat, meter),
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
                    meter.bars(start + length) - meter.bars(start),
                ),
            );
        });
    });
    ui.end_row();
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

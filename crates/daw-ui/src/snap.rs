//! One Snap control for every timeline. The same labels appear in the
//! arranger, the audio editor and the piano roll; the caller owns the value.
use crate::tokens;
use egui::{Response, Ui};

/// Snap value that follows the grid currently drawn on the timeline, so the
/// step tracks zoom. Resolve it with [`snap_step`] before snapping a position.
pub const SNAP_GRID: f64 = -1.0;

/// Closest spacing of Grid lines in points. Between `SPACE_SM` and `SPACE_MD`
/// so the default arranger and piano-roll zooms both land on 1/16 notes.
const GRID_MIN_SPACING: f64 = 10.0;

/// Beat step of the zoom-dependent timeline grid: the finest of 1/32 to 1/4
/// notes, then whole bars doubling, whose lines are `GRID_MIN_SPACING` apart.
pub fn grid_step(pixels_per_beat: f32, beats_per_bar: u8) -> f64 {
    let minimum = GRID_MIN_SPACING / f64::from(pixels_per_beat.max(0.001));
    let mut step = 0.125;
    while step < minimum && step < 1.0 {
        step *= 2.0;
    }
    if step < minimum {
        step = f64::from(beats_per_bar.max(1));
        while step < minimum {
            step *= 2.0;
        }
    }
    step
}

/// The concrete step in beats for a snap value at this zoom; zero means off.
pub fn snap_step(snap: f64, pixels_per_beat: f32, beats_per_bar: u8) -> f64 {
    if snap == SNAP_GRID {
        grid_step(pixels_per_beat, beats_per_bar)
    } else {
        snap.max(0.0)
    }
}

/// Musical label for a snap step in beats. Zero means snapping is off.
/// `beats_per_bar` is the bar length the "1 bar" step uses.
pub fn snap_label(snap: f64, beats_per_bar: u8) -> String {
    if snap == SNAP_GRID {
        "Snap grid".into()
    } else if snap <= 0.0 {
        "Snap off".into()
    } else if snap == f64::from(beats_per_bar.max(1)) {
        "Snap 1 bar".into()
    } else if [0.125, 0.25, 0.5, 1.0].contains(&snap) {
        format!("Snap 1/{}", (4.0 / snap) as u32)
    } else {
        format!("Snap {snap} beats")
    }
}

/// A current-value chooser with the standard steps. `extra` adds editor-specific
/// snap targets below a separator; toggling them keeps the menu open.
pub fn snap_choice(
    ui: &mut Ui,
    snap: &mut f64,
    meter: &crate::MeterMap,
    extra: Option<&mut dyn FnMut(&mut Ui)>,
) -> Response {
    let before = *snap;
    // Snap steps are uniform; "1 bar" is the first bar's length. Callers that
    // place positions on the song timeline snap that step to real bar lines.
    let beats_per_bar = meter.initial();
    let bar = f64::from(beats_per_bar);
    let (response, _) = crate::quiet_picker_button(ui, snap_label(*snap, beats_per_bar), |ui| {
        ui.set_min_width(tokens::SELECT_WIDTH);
        for (name, value) in [
            ("Grid", SNAP_GRID),
            ("Off", 0.0),
            ("1 bar", bar),
            ("1/4", 1.0),
            ("1/8", 0.5),
            ("1/16", 0.25),
            ("1/32", 0.125),
        ] {
            if crate::selectable_value(ui, snap, value, name).clicked() {
                ui.close();
            }
        }
        if let Some(extra) = extra {
            ui.separator();
            extra(ui);
        }
        crate::text::note(ui, "Hold Shift to bypass snapping");
    });
    let mut response = response.on_hover_text(
        "Grid step for moving, trimming and drawing; Grid follows the lines shown at this zoom",
    );
    if *snap != before {
        response.mark_changed();
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_use_musical_fractions() {
        assert_eq!(snap_label(0.0, 4), "Snap off");
        assert_eq!(snap_label(4.0, 4), "Snap 1 bar");
        assert_eq!(snap_label(3.0, 3), "Snap 1 bar");
        assert_eq!(snap_label(1.0, 4), "Snap 1/4");
        assert_eq!(snap_label(0.25, 4), "Snap 1/16");
        assert_eq!(snap_label(0.125, 4), "Snap 1/32");
        assert_eq!(snap_label(1.5, 4), "Snap 1.5 beats");
        assert_eq!(snap_label(SNAP_GRID, 4), "Snap grid");
    }

    #[test]
    fn grid_follows_zoom_from_subdivisions_to_bars() {
        assert_eq!(grid_step(200.0, 4), 0.125);
        assert_eq!(grid_step(78.0, 4), 0.25);
        assert_eq!(grid_step(48.0, 4), 0.25);
        assert_eq!(grid_step(30.0, 4), 0.5);
        assert_eq!(grid_step(10.0, 4), 1.0);
        assert_eq!(grid_step(4.0, 4), 4.0);
        assert_eq!(grid_step(4.0, 3), 3.0);
        assert_eq!(grid_step(1.0, 4), 16.0);
        assert_eq!(snap_step(SNAP_GRID, 48.0, 4), 0.25);
        assert_eq!(snap_step(0.25, 2.0, 4), 0.25);
        assert_eq!(snap_step(0.0, 48.0, 4), 0.0);
    }
}

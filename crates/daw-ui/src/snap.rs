//! One Snap control for every timeline. The same labels appear in the
//! arranger, the audio editor and the piano roll; the caller owns the value.
use crate::tokens;
use egui::{Response, Ui};

/// Musical label for a snap step in beats. Zero means snapping is off.
/// `beats_per_bar` is the bar length the "1 bar" step uses.
pub fn snap_label(snap: f64, beats_per_bar: u8) -> String {
    if snap <= 0.0 {
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
    let mut response = response.on_hover_text("Grid step for moving, trimming and drawing");
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
    }
}

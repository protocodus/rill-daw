//! Precise, shared controls for short audio edit-boundary fades.
use egui::{Rect, Ui};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryFadeSettings {
    pub fade_in_ms: f64,
    pub fade_out_ms: f64,
}

impl Default for BoundaryFadeSettings {
    fn default() -> Self {
        Self {
            fade_in_ms: 3.0,
            fade_out_ms: 5.0,
        }
    }
}

pub fn boundary_fade_settings(ui: &mut Ui, settings: &mut BoundaryFadeSettings) -> [Rect; 2] {
    let mut controls = [Rect::NOTHING; 2];
    crate::form_grid(ui, "boundary_fade_settings", |ui| {
        for (index, (title, value)) in [
            ("Fade in", &mut settings.fade_in_ms),
            ("Fade out", &mut settings.fade_out_ms),
        ]
        .into_iter()
        .enumerate()
        {
            let label = crate::text::label(ui, title);
            controls[index] = ui
                .add(
                    crate::number(value)
                        .range(0.0..=100.0)
                        .speed(0.1)
                        .max_decimals(2)
                        .suffix(" ms"),
                )
                .labelled_by(label.id)
                .rect;
            ui.end_row();
        }
    });
    controls
}

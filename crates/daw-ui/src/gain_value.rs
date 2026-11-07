//! Exact gain editing in decibels, with the same silence floor as channel faders.
use egui::{Response, Ui};
use std::hash::Hash;

/// Format an actual linear gain for a curve axis. The caller supplies its
/// parameter's denormalized value, so track and send ranges keep their units.
pub fn gain_axis_label(gain: f32) -> String {
    if gain <= 0.0 {
        "−∞".into()
    } else {
        let db = crate::channel_gain_to_db(gain);
        if db.abs() < 0.05 {
            "0 dB".into()
        } else {
            format!("{db:+.1} dB").replace('-', "−")
        }
    }
}

/// A concise dB readout opening the shared exact-value editor and mouse keypad.
/// The stored linear value changes only after an explicit edit is committed;
/// inspecting or dismissing a rounded display never round-trips that value.
pub fn gain_value_button(
    ui: &mut Ui,
    id_salt: impl Hash,
    gain: &mut f32,
    maximum_gain: f32,
    name: &str,
) -> Response {
    gain_value_button_named(ui, id_salt, gain, maximum_gain, name, name)
}

/// Separate a bounded visible caption from the contextual exact-editor name.
pub fn gain_value_button_named(
    ui: &mut Ui,
    id_salt: impl Hash,
    gain: &mut f32,
    maximum_gain: f32,
    caption: &str,
    name: &str,
) -> Response {
    ui.push_id(id_salt, |ui| {
        let height = ui
            .spacing()
            .interact_size
            .y
            .clamp(crate::tokens::COMPACT_HEIGHT, crate::tokens::CONTROL_HEIGHT);
        let mut response = ui
            .add(
                crate::button_widget(
                    format!("{caption} {}", crate::channel_gain_label(*gain)).trim(),
                )
                .min_size(egui::vec2(0.0, height))
                .truncate(),
            )
            .on_hover_text("Edit gain in dB · 0 dB is unity · −60 dB is silence");
        let current = f64::from(crate::channel_gain_to_db(*gain));
        let maximum_db = f64::from(crate::channel_gain_to_db(maximum_gain));
        let mut silence = false;
        let popup = egui::Popup::menu(&response)
            .id(response.id.with("exact_gain_popup"))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                let next = crate::value_entry::value_entry_with_footer(
                    ui,
                    response.id,
                    current,
                    -60.0..=maximum_db,
                    "dB",
                    name,
                    crate::tokens::CONTROL_HEIGHT + crate::tokens::SPACE_SM * 2.0,
                );
                ui.separator();
                if crate::button(ui, "Silence").clicked() {
                    silence = true;
                    crate::value_entry::clear_value_entry(ui, response.id);
                    ui.close();
                }
                next
            });
        let next = if silence {
            Some(0.0)
        } else {
            popup.and_then(|popup| popup.inner).map(|db| {
                // The f32 dB conversion may land just above the exact linear
                // maximum. Keep the target's existing automation bounds.
                crate::channel_gain_from_db(db as f32).min(maximum_gain)
            })
        };
        if let Some(next) = next
            && next != *gain
        {
            *gain = next;
            response.mark_changed();
        }
        response
    })
    .inner
}

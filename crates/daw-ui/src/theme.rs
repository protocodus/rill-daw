//! The shared Rill palette and egui style. Canvas colors live here too, so
//! editors and ordinary controls share one theme source.

use crate::tokens;
use egui::{Color32, FontId, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(17, 20, 25);
pub const PANEL: Color32 = Color32::from_rgb(23, 27, 33);
pub const RAISED: Color32 = Color32::from_rgb(31, 36, 43);
pub const LINE: Color32 = Color32::from_rgb(44, 50, 58);
pub const TEXT: Color32 = Color32::from_rgb(225, 232, 239);
pub const MUTED: Color32 = Color32::from_rgb(135, 147, 160);
pub const ACCENT: Color32 = Color32::from_rgb(93, 222, 178);
pub const ORANGE: Color32 = Color32::from_rgb(245, 178, 108);
pub const TRACK_SELECTED: Color32 = Color32::from_rgb(28, 34, 40);
pub const TRACK_ALTERNATE: Color32 = Color32::from_rgb(20, 24, 29);
pub const GRID_MINOR: Color32 = Color32::from_rgb(29, 34, 40);
pub const SCALE_ROOT: Color32 = Color32::from_rgb(30, 50, 46);
pub const SCALE_MEMBER: Color32 = Color32::from_rgb(25, 34, 35);
pub const PIANO_BLACK: Color32 = Color32::from_rgb(29, 33, 39);
pub const PIANO_WHITE: Color32 = Color32::from_rgb(155, 165, 172);
pub const GRID_BAR: Color32 = Color32::from_rgb(64, 73, 78);
pub const NOTE_MUTED: Color32 = Color32::from_rgb(117, 165, 155);
pub const CONTROL_FILL: Color32 = Color32::from_rgb(48, 58, 68);
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(48, 58, 65);

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = PANEL;
    style.visuals.window_fill = PANEL;
    style.visuals.extreme_bg_color = BG;
    style.visuals.faint_bg_color = RAISED;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.3);
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    style.visuals.widgets.inactive.bg_fill = CONTROL_FILL;
    style.visuals.widgets.inactive.weak_bg_fill = RAISED;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    style.visuals.widgets.hovered.bg_fill = CONTROL_HOVER;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, MUTED);
    style.visuals.widgets.active.bg_fill = ACCENT.gamma_multiply(0.3);
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.5, ACCENT);
    style.visuals.widgets.inactive.corner_radius = tokens::RADIUS.into();
    style.visuals.widgets.hovered.corner_radius = tokens::RADIUS.into();
    style.visuals.widgets.active.corner_radius = tokens::RADIUS.into();
    style.visuals.window_corner_radius = tokens::RADIUS.into();
    style.spacing.item_spacing = egui::vec2(tokens::SPACE_SM, tokens::SPACE_SM);
    style.spacing.button_padding = egui::vec2(tokens::SPACE_MD, tokens::SPACE_XS);
    style.spacing.interact_size.y = tokens::CONTROL_HEIGHT;
    style.spacing.combo_width = tokens::SELECT_WIDTH;
    for (text_style, size) in [
        (TextStyle::Body, tokens::FONT_BODY),
        (TextStyle::Button, tokens::FONT_BUTTON),
        (TextStyle::Heading, tokens::FONT_HEADING),
        (TextStyle::Small, tokens::FONT_SMALL),
    ] {
        style
            .text_styles
            .insert(text_style, FontId::proportional(size));
    }
    ctx.set_style(style);
}

pub fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(tokens::FONT_CAPTION)
            .strong()
            .color(MUTED),
    );
}

pub fn color(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

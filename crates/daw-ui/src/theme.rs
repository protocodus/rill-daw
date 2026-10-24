//! Context-local Rill palettes. Native controls and musical canvases resolve the
//! same current egui theme, including live operating-system appearance changes.
use crate::tokens;
use egui::{Color32, FontId, Stroke, TextStyle};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}
impl Appearance {
    pub fn name(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
    pub fn preference(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub raised: Color32,
    pub line: Color32,
    pub window_border: Color32,
    pub popup_scrim: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub orange: Color32,
    pub track_selected: Color32,
    pub track_alternate: Color32,
    pub grid_minor: Color32,
    pub grid_bar: Color32,
    pub control_fill: Color32,
    pub control_hover: Color32,
    pub piano_black: Color32,
    pub piano_white: Color32,
    pub note: Color32,
    pub note_selected: Color32,
    pub note_text: Color32,
    /// Index zero: ordinary row; one: scale; two: chord. Both keys retain contrast.
    pub piano_rows_black: [Color32; 3],
    pub piano_rows_white: [Color32; 3],
}
const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}
pub const DARK: Palette = Palette {
    bg: rgb(27, 31, 33),
    panel: rgb(34, 39, 41),
    raised: rgb(41, 47, 49),
    line: rgb(58, 67, 68),
    window_border: rgb(88, 104, 104),
    popup_scrim: Color32::from_black_alpha(104),
    text: rgb(229, 234, 232),
    muted: rgb(173, 182, 182),
    accent: rgb(145, 190, 177),
    orange: rgb(221, 176, 126),
    track_selected: rgb(49, 67, 63),
    track_alternate: rgb(31, 36, 38),
    grid_minor: rgb(43, 50, 53),
    grid_bar: rgb(66, 77, 79),
    control_fill: rgb(46, 54, 56),
    control_hover: rgb(56, 66, 67),
    piano_black: rgb(27, 33, 41),
    piano_white: rgb(207, 215, 224),
    note: rgb(83, 156, 238),
    note_selected: rgb(146, 207, 255),
    note_text: rgb(12, 23, 35),
    piano_rows_black: [rgb(18, 22, 28), rgb(18, 29, 42), rgb(20, 33, 48)],
    piano_rows_white: [rgb(42, 48, 56), rgb(40, 57, 76), rgb(44, 65, 88)],
};
pub const LIGHT: Palette = Palette {
    bg: rgb(246, 247, 244),
    panel: rgb(239, 242, 238),
    raised: rgb(251, 252, 249),
    line: rgb(204, 214, 207),
    window_border: rgb(153, 172, 160),
    popup_scrim: Color32::from_black_alpha(64),
    text: rgb(37, 46, 44),
    muted: rgb(82, 96, 93),
    accent: rgb(47, 105, 90),
    orange: rgb(139, 83, 31),
    track_selected: rgb(216, 233, 225),
    track_alternate: rgb(232, 237, 232),
    grid_minor: rgb(222, 229, 223),
    grid_bar: rgb(178, 191, 183),
    control_fill: rgb(228, 234, 228),
    control_hover: rgb(218, 227, 221),
    piano_black: rgb(49, 59, 72),
    piano_white: rgb(255, 255, 255),
    note: rgb(34, 105, 183),
    note_selected: rgb(18, 78, 145),
    note_text: rgb(255, 255, 255),
    piano_rows_black: [rgb(197, 207, 220), rgb(170, 194, 221), rgb(151, 181, 215)],
    piano_rows_white: [rgb(246, 248, 251), rgb(225, 239, 252), rgb(215, 232, 250)],
};

pub fn palette(ctx: &egui::Context) -> Palette {
    match ctx.theme() {
        egui::Theme::Dark => DARK,
        egui::Theme::Light => LIGHT,
    }
}
/// Choose readable foreground text on user-defined track colors and note fills.
pub fn contrast_text(background: Color32) -> Color32 {
    let channel = |value: u8| {
        let value = f32::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * channel(background.r())
        + 0.7152 * channel(background.g())
        + 0.0722 * channel(background.b());
    if luminance > 0.179 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}
impl Palette {
    /// Quietly desaturate content outside a clip toward its surrounding canvas.
    /// Light canvases remain light instead of receiving a fixed black veil.
    pub fn inactive_clip_tint(self) -> Color32 {
        self.bg.gamma_multiply(0.32)
    }

    /// Velocity changes brightness without making the label's background transparent.
    pub fn note_fill(self, selected: bool, velocity: u8) -> Color32 {
        let base = if selected {
            self.note_selected
        } else {
            self.note
        };
        let factor = 0.45 + 0.55 * f32::from(velocity.min(127)) / 127.0;
        Color32::from_rgb(
            (f32::from(base.r()) * factor) as u8,
            (f32::from(base.g()) * factor) as u8,
            (f32::from(base.b()) * factor) as u8,
        )
    }
    pub fn piano_row(self, black: bool, highlight: u8) -> Color32 {
        let index = usize::from(highlight.min(2));
        if black {
            self.piano_rows_black[index]
        } else {
            self.piano_rows_white[index]
        }
    }
}

pub fn set_appearance(ctx: &egui::Context, appearance: Appearance) {
    ctx.set_theme(appearance.preference());
}

/// Configure both native palettes once. egui selects between them on every frame.
pub fn apply(ctx: &egui::Context) {
    crate::text::install_fonts(ctx);
    for (theme, colors) in [(egui::Theme::Dark, DARK), (egui::Theme::Light, LIGHT)] {
        let mut style = (*ctx.style_of(theme)).clone();
        style.visuals = match theme {
            egui::Theme::Dark => egui::Visuals::dark(),
            egui::Theme::Light => egui::Visuals::light(),
        };
        style.visuals.panel_fill = colors.panel;
        style.visuals.window_fill = colors.panel;
        style.visuals.extreme_bg_color = colors.bg;
        style.visuals.faint_bg_color = colors.raised;
        style.visuals.override_text_color = Some(colors.text);
        style.visuals.weak_text_color = Some(colors.muted);
        style.visuals.text_edit_bg_color = Some(colors.bg);
        style.visuals.code_bg_color = colors.bg;
        style.visuals.hyperlink_color = colors.accent;
        style.visuals.warn_fg_color = colors.orange;
        style.visuals.text_cursor.stroke = Stroke::new(1.5_f32, colors.accent);
        style.visuals.selection.bg_fill = colors.track_selected;
        style.visuals.selection.stroke = Stroke::new(1.0_f32, colors.accent);
        style.visuals.window_shadow = egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(if theme == egui::Theme::Dark { 85 } else { 36 }),
        };
        style.visuals.popup_shadow = egui::epaint::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(if theme == egui::Theme::Dark { 64 } else { 28 }),
        };
        style.visuals.window_highlight_topmost = false;
        style.visuals.window_stroke =
            Stroke::new(tokens::WINDOW_BORDER_WIDTH, colors.window_border);
        style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, colors.line);
        style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, colors.muted);
        style.visuals.widgets.noninteractive.bg_fill = colors.bg;
        style.visuals.widgets.noninteractive.weak_bg_fill = colors.panel;
        style.visuals.widgets.inactive.bg_fill = colors.control_fill;
        style.visuals.widgets.inactive.weak_bg_fill = colors.raised;
        style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, colors.line);
        style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, colors.text);
        style.visuals.widgets.hovered.bg_fill = colors.control_hover;
        style.visuals.widgets.hovered.weak_bg_fill = colors.control_hover;
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, colors.line);
        style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, colors.text);
        style.visuals.widgets.active.bg_fill = colors.track_selected;
        style.visuals.widgets.active.weak_bg_fill = colors.track_selected;
        style.visuals.widgets.active.bg_stroke = Stroke::new(1.5_f32, colors.accent);
        style.visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, colors.text);
        style.visuals.widgets.open.bg_fill = colors.track_selected;
        style.visuals.widgets.open.weak_bg_fill = colors.track_selected;
        style.visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, colors.accent);
        style.visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, colors.text);
        for visuals in [
            &mut style.visuals.widgets.noninteractive,
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
            &mut style.visuals.widgets.open,
        ] {
            visuals.corner_radius = tokens::RADIUS.into();
            visuals.expansion = 0.0;
        }
        style.visuals.window_corner_radius = tokens::SURFACE_RADIUS.into();
        style.visuals.menu_corner_radius = tokens::SURFACE_RADIUS.into();
        style.spacing.item_spacing = egui::vec2(tokens::SPACE_SM, tokens::SPACE_SM);
        style.spacing.button_padding =
            egui::vec2(tokens::BUTTON_PADDING_X, tokens::BUTTON_PADDING_Y);
        style.spacing.window_margin = tokens::WINDOW_MARGIN;
        style.spacing.menu_margin = (tokens::SPACE_SM as i8).into();
        style.spacing.icon_spacing = tokens::SPACE_SM;
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
        style
            .text_styles
            .insert(TextStyle::Heading, crate::text::TextRole::Title.font());
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(tokens::FONT_BODY));
        ctx.set_style_of(theme, style);
    }
    set_appearance(ctx, Appearance::System);
}

pub fn label(ui: &mut egui::Ui, text: &str) {
    crate::text::label(ui, text);
}
pub fn color(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// Retain a track's hue while giving its label contrast on themed panel surfaces.
pub fn readable_track_color(ctx: &egui::Context, rgb: [u8; 3]) -> Color32 {
    let light = ctx.theme() == egui::Theme::Light;
    let luminance = |rgb: Color32| {
        let linear = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(rgb.r()) + 0.7152 * linear(rgb.g()) + 0.0722 * linear(rgb.b())
    };
    let background = luminance(palette(ctx).panel);
    let mut color = color(rgb);
    for _ in 0..24 {
        let foreground = luminance(color);
        if (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05) >= 4.5 {
            break;
        }
        let adjust = |value: u8| {
            if light {
                (f32::from(value) * 0.85) as u8
            } else {
                (f32::from(value) + (255.0 - f32::from(value)) * 0.15) as u8
            }
        };
        color = Color32::from_rgb(adjust(color.r()), adjust(color.g()), adjust(color.b()));
    }
    color
}

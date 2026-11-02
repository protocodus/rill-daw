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
    /// Neutral selection and view controls.
    pub accent: Color32,
    pub action: Color32,
    pub ai: Color32,
    pub guide: Color32,
    pub warning: Color32,
    pub orange: Color32,
    /// Playback/edit position stays distinct from selection and track colors.
    pub cursor: Color32,
    pub cursor_outline: Color32,
    /// Reserved for automation curves, handles, badges and owned controls.
    pub automation: Color32,
    /// Manual takeover of automation, distinct from the saved curve.
    pub automation_override: Color32,
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
    /// Index zero: neutral row; one: scale; two: chord. Green tints stay subtle.
    pub piano_rows_black: [Color32; 3],
    pub piano_rows_white: [Color32; 3],
    /// Soft and strong velocity colors, independent of note selection.
    pub piano_notes: [Color32; 2],
    /// Optional warning hue for pitches outside the selected scale.
    pub piano_notes_outside_scale: [Color32; 2],
}
const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}
pub const DARK: Palette = Palette {
    bg: rgb(25, 27, 31),
    panel: rgb(34, 37, 42),
    raised: rgb(42, 45, 51),
    line: rgb(66, 70, 78),
    window_border: rgb(91, 96, 106),
    popup_scrim: Color32::from_black_alpha(104),
    text: rgb(225, 228, 233),
    muted: rgb(172, 180, 191),
    action: rgb(120, 182, 240),
    ai: rgb(229, 160, 202),
    guide: rgb(140, 203, 158),
    warning: rgb(239, 129, 134),
    accent: rgb(183, 190, 200),
    orange: rgb(233, 174, 114),
    cursor: rgb(140, 203, 158),
    cursor_outline: rgb(18, 32, 23),
    automation: rgb(233, 174, 114),
    automation_override: rgb(255, 207, 155),
    track_selected: rgb(52, 56, 63),
    track_alternate: rgb(29, 31, 36),
    grid_minor: rgb(45, 48, 54),
    grid_bar: rgb(65, 69, 77),
    control_fill: rgb(48, 52, 59),
    control_hover: rgb(59, 64, 72),
    piano_black: rgb(31, 34, 39),
    piano_white: rgb(41, 44, 49),
    note: rgb(145, 153, 164),
    note_selected: rgb(203, 210, 220),
    note_text: rgb(19, 23, 28),
    piano_rows_black: [rgb(31, 34, 39), rgb(31, 40, 35), rgb(31, 43, 36)],
    piano_rows_white: [rgb(41, 44, 49), rgb(40, 51, 45), rgb(40, 54, 46)],
    piano_notes: [rgb(79, 87, 99), rgb(184, 194, 207)],
    piano_notes_outside_scale: [rgb(151, 75, 79), rgb(241, 125, 129)],
};
pub const LIGHT: Palette = Palette {
    bg: rgb(236, 238, 240),
    panel: rgb(244, 245, 246),
    raised: rgb(255, 255, 255),
    line: rgb(200, 204, 209),
    window_border: rgb(147, 153, 162),
    popup_scrim: Color32::from_black_alpha(64),
    text: rgb(37, 41, 46),
    muted: rgb(85, 94, 106),
    action: rgb(27, 80, 136),
    ai: rgb(135, 38, 97),
    guide: rgb(49, 119, 80),
    warning: rgb(174, 50, 55),
    accent: rgb(78, 86, 97),
    orange: rgb(130, 65, 10),
    cursor: rgb(49, 119, 80),
    cursor_outline: rgb(255, 255, 255),
    automation: rgb(130, 65, 10),
    automation_override: rgb(184, 91, 18),
    track_selected: rgb(218, 222, 227),
    track_alternate: rgb(231, 233, 236),
    grid_minor: rgb(216, 220, 224),
    grid_bar: rgb(184, 190, 198),
    control_fill: rgb(229, 231, 234),
    control_hover: rgb(215, 219, 224),
    piano_black: rgb(235, 237, 240),
    piano_white: rgb(245, 247, 250),
    note: rgb(105, 115, 129),
    note_selected: rgb(70, 79, 93),
    note_text: rgb(255, 255, 255),
    piano_rows_black: [rgb(235, 237, 240), rgb(231, 239, 233), rgb(229, 239, 231)],
    piano_rows_white: [rgb(245, 247, 250), rgb(237, 249, 242), rgb(234, 249, 239)],
    piano_notes: [rgb(175, 183, 195), rgb(78, 88, 104)],
    piano_notes_outside_scale: [rgb(220, 170, 174), rgb(184, 65, 76)],
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
        if self.bg.r() > 128 {
            self.raised.gamma_multiply(0.32)
        } else {
            self.bg.gamma_multiply(0.32)
        }
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

    /// Keyboard and grid share the same quiet pitch colors. Sounding keys gain
    /// a stronger green tint without introducing another highlight hue.
    pub fn piano_key_fill(self, black: bool, highlight: u8, active: bool) -> Color32 {
        let base = self.piano_row(black, highlight);
        if active {
            base.lerp_to_gamma(self.guide, 0.35)
        } else {
            base
        }
    }

    /// Neutral notes strengthen with velocity; an optional red hue marks pitches
    /// outside the scale. Selection deliberately never changes this fill.
    pub fn piano_note_fill(self, velocity: u8, out_of_scale: bool) -> Color32 {
        let [soft, strong] = if out_of_scale {
            self.piano_notes_outside_scale
        } else {
            self.piano_notes
        };
        soft.lerp_to_gamma(strong, f32::from(velocity.min(127)) / 127.0)
    }

    /// A neutral, readable outline marks selection while preserving velocity.
    pub fn piano_note_stroke(self, selected: bool, fill: Color32) -> Stroke {
        if selected {
            Stroke::new(2.0_f32, contrast_text(fill))
        } else {
            Stroke::NONE
        }
    }

    /// Keep the note's velocity color visible when fitting dense or short notes.
    /// An inside selection outline leaves at least half of each dimension filled.
    pub fn piano_note_stroke_in_rect(
        self,
        selected: bool,
        fill: Color32,
        rect: egui::Rect,
    ) -> Stroke {
        let mut stroke = self.piano_note_stroke(selected, fill);
        stroke.width = stroke
            .width
            .min((rect.width().min(rect.height()) / 4.0).max(0.0));
        stroke
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
        style.override_text_valign = Some(egui::Align::Center);
        style.visuals.weak_text_color = Some(colors.muted);
        style.visuals.text_edit_bg_color = Some(colors.bg);
        style.visuals.code_bg_color = colors.bg;
        style.visuals.hyperlink_color = colors.action;
        style.visuals.warn_fg_color = colors.warning;
        style.visuals.text_cursor.stroke = Stroke::new(tokens::FOCUS_WIDTH, colors.guide);
        style.visuals.selection.bg_fill = colors.track_selected;
        style.visuals.selection.stroke = Stroke::new(1.0_f32, colors.accent);
        style.visuals.window_shadow = egui::epaint::Shadow {
            offset: [0, 8],
            blur: 21,
            spread: 0,
            color: Color32::from_black_alpha(if theme == egui::Theme::Dark { 55 } else { 34 }),
        };
        style.visuals.popup_shadow = egui::epaint::Shadow {
            offset: [0, 5],
            blur: 13,
            spread: 0,
            color: Color32::from_black_alpha(if theme == egui::Theme::Dark { 34 } else { 21 }),
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
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(
            tokens::BORDER_WIDTH,
            colors
                .line
                .lerp_to_gamma(colors.accent, tokens::GOLDEN_RATIO_INVERSE),
        );
        // Native window close controls paint only this stroke, without a fill.
        // Keep their hover/press feedback visible as well as shared buttons'.
        style.visuals.widgets.hovered.fg_stroke = Stroke::new(tokens::FOCUS_WIDTH, colors.action);
        style.visuals.widgets.active.bg_fill = colors.track_selected;
        style.visuals.widgets.active.weak_bg_fill = colors.track_selected;
        style.visuals.widgets.active.bg_stroke = Stroke::new(tokens::FOCUS_WIDTH, colors.accent);
        style.visuals.widgets.active.fg_stroke = Stroke::new(tokens::FOCUS_WIDTH, colors.action);
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
        style.spacing.item_spacing = egui::vec2(tokens::SPACE_MD, tokens::SPACE_MD);
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

/// Retain a track's hue while giving labels and foreground marks contrast on themed surfaces.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: Color32) -> f32 {
        let channel = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn piano_rows_keep_close_neutral_bases_and_quiet_green_highlights() {
        for colors in [DARK, LIGHT] {
            let black = colors.piano_row(true, 0);
            let white = colors.piano_row(false, 0);
            for (black, white) in black.to_array()[..3].iter().zip(white.to_array()) {
                assert!((6..=12).contains(&white.abs_diff(*black)));
            }
            for highlight in 0..=2 {
                assert!(
                    luminance(colors.piano_row(true, highlight))
                        < luminance(colors.piano_row(false, highlight))
                );
                for black in [false, true] {
                    let base = colors.piano_row(black, 0);
                    let fill = colors.piano_row(black, highlight);
                    assert_eq!(colors.piano_key_fill(black, highlight, false), fill);
                    assert_ne!(colors.piano_key_fill(black, highlight, true), fill);
                    for (base, fill) in base.to_array()[..3].iter().zip(fill.to_array()) {
                        assert!(base.abs_diff(fill) <= 12, "highlights stay subtle");
                    }
                    if highlight > 0 {
                        assert!(fill.g() > fill.r() && fill.g() > fill.b());
                    }
                }
            }
        }
    }

    #[test]
    fn piano_velocity_shades_and_warning_hue_keep_neutral_selection_readable() {
        for colors in [DARK, LIGHT] {
            for out_of_scale in [false, true] {
                let soft = colors.piano_note_fill(1, out_of_scale);
                let medium = colors.piano_note_fill(64, out_of_scale);
                let strong = colors.piano_note_fill(127, out_of_scale);
                assert_ne!(soft, medium);
                assert_ne!(medium, strong);
                assert!(
                    (luminance(soft) - luminance(medium)) * (luminance(medium) - luminance(strong))
                        > 0.0,
                    "velocity shading has one brightness direction in each theme"
                );
                for velocity in 0..=127 {
                    let fill = colors.piano_note_fill(velocity, out_of_scale);
                    assert_eq!(fill.a(), 255);
                    if out_of_scale {
                        assert!(fill.r() > fill.b() + 40);
                    } else {
                        let rgb = [fill.r(), fill.g(), fill.b()];
                        assert!(
                            rgb.iter().max().unwrap() - rgb.iter().min().unwrap() <= 30,
                            "default notes remain neutral"
                        );
                    }
                    let selection = colors.piano_note_stroke(true, fill);
                    assert_eq!(selection.width, 2.0);
                    assert!(matches!(selection.color, Color32::BLACK | Color32::WHITE));
                    assert!(contrast(selection.color, fill) >= 4.5);
                    assert_eq!(colors.piano_note_stroke(false, fill), Stroke::NONE);
                }
                assert_eq!(
                    colors.piano_note_fill(255, out_of_scale),
                    colors.piano_note_fill(127, out_of_scale)
                );
            }
        }
    }

    #[test]
    fn selected_note_outlines_preserve_fill_in_dense_rows_and_short_notes() {
        for colors in [DARK, LIGHT] {
            for out_of_scale in [false, true] {
                let fill = colors.piano_note_fill(96, out_of_scale);
                for (size, expected_width) in [
                    (egui::vec2(48.0, 18.0), 2.0),
                    (egui::vec2(48.0, 2.0), 0.5),
                    (egui::vec2(2.0, 18.0), 0.5),
                ] {
                    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                    let stroke = colors.piano_note_stroke_in_rect(true, fill, rect);
                    assert_eq!(stroke.width, expected_width);
                    assert_eq!(stroke.color, colors.piano_note_stroke(true, fill).color);
                    assert!(rect.width() - 2.0 * stroke.width >= rect.width() / 2.0);
                    assert!(rect.height() - 2.0 * stroke.width >= rect.height() / 2.0);
                    assert_eq!(
                        colors.piano_note_stroke_in_rect(false, fill, rect),
                        Stroke::NONE
                    );
                }
            }
        }
    }
}

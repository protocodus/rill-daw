use daw_ui::theme::{self, Appearance};
use egui::{Color32, Context, RawInput, Theme};

fn luminance(color: Color32) -> f64 {
    let channel = |value: u8| {
        let value = f64::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}
fn contrast(a: Color32, b: Color32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn both_palettes_keep_text_readable_and_every_black_pitch_row_darker() {
    for colors in [theme::DARK, theme::LIGHT] {
        for surface in [
            colors.bg,
            colors.panel,
            colors.raised,
            colors.track_selected,
        ] {
            assert!(contrast(colors.text, surface) >= 4.5);
            assert!(
                contrast(colors.muted, surface) >= 4.5,
                "muted {colors:?} {surface:?}"
            );
        }
        for black in colors.piano_rows_black {
            for white in colors.piano_rows_white {
                assert!(
                    luminance(black) < luminance(white),
                    "highlight must preserve pitch-row contrast"
                );
            }
        }
        for color in [
            colors.note,
            colors.note_selected,
            colors.piano_black,
            colors.piano_white,
        ] {
            assert!(contrast(theme::contrast_text(color), color) >= 4.5);
        }
        for velocity in 1..=127 {
            for selected in [false, true] {
                let fill = colors.note_fill(selected, velocity);
                assert_eq!(fill.a(), 255);
                assert!(contrast(theme::contrast_text(fill), fill) >= 4.5);
            }
        }
    }
}

#[test]
fn explicit_appearance_ignores_os_changes_and_system_resumes_following() {
    let context = Context::default();
    theme::apply(&context);
    for preference in [Appearance::Light, Appearance::Dark, Appearance::System] {
        theme::set_appearance(&context, preference);
        for system_theme in [Theme::Dark, Theme::Light, Theme::Dark] {
            let _ = context.run(
                RawInput {
                    system_theme: Some(system_theme),
                    ..Default::default()
                },
                |ctx| {
                    let expected = match preference {
                        Appearance::Light => Theme::Light,
                        Appearance::Dark => Theme::Dark,
                        Appearance::System => system_theme,
                    };
                    assert_eq!(ctx.theme(), expected);
                    let colors = theme::palette(ctx);
                    assert_eq!(ctx.style().visuals.panel_fill, colors.panel);
                    assert_eq!(daw_ui::panel(ctx).fill, colors.panel);
                    assert_eq!(daw_ui::card(ctx).fill, colors.raised);
                    assert_eq!(daw_ui::inset(ctx).fill, colors.bg);
                    for rgb in [[0, 0, 0], [255, 255, 255], [85, 190, 202], [160, 137, 246]] {
                        assert!(
                            contrast(theme::readable_track_color(ctx, rgb), colors.panel) >= 4.5
                        );
                    }
                },
            );
        }
    }
}

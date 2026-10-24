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
            colors.control_fill,
            colors.control_hover,
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
        assert!(contrast(colors.accent, colors.track_selected) >= 4.5);
        assert!(contrast(colors.bg, colors.accent) >= 4.5);
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
fn calm_controls_keep_readable_hints_focus_and_window_depth_separate_from_flat_panels() {
    let context = Context::default();
    theme::apply(&context);
    for appearance in [Appearance::Dark, Appearance::Light] {
        theme::set_appearance(&context, appearance);
        let _ = context.run(RawInput::default(), |ctx| {
            let style = ctx.style();
            let colors = theme::palette(ctx);
            let visuals = &style.visuals;
            assert!(visuals.window_shadow.color.a() > 0);
            assert!(visuals.window_shadow.color.a() <= 85);
            assert!(visuals.window_shadow.blur >= 20);
            assert_eq!(visuals.window_shadow.spread, 0);
            assert!(visuals.popup_shadow.color.a() > 0);
            assert!(visuals.popup_shadow.color.a() < visuals.window_shadow.color.a());
            assert!(visuals.window_stroke.width >= 1.5);
            assert!(
                contrast(visuals.window_stroke.color, colors.panel)
                    > contrast(colors.line, colors.panel),
                "floating windows must have a clearer boundary than base panels"
            );
            for surface in [daw_ui::panel(ctx), daw_ui::card(ctx), daw_ui::inset(ctx)] {
                assert_eq!(surface.shadow, egui::epaint::Shadow::NONE);
            }
            assert!(
                contrast(
                    visuals.weak_text_color.expect("readable placeholder text"),
                    visuals.text_edit_bg_color.unwrap()
                ) >= 4.5
            );
            assert!(
                contrast(visuals.widgets.active.bg_stroke.color, colors.panel) >= 3.0,
                "keyboard focus must remain distinct on calm panels"
            );
            assert_ne!(
                visuals.widgets.inactive.bg_fill, visuals.widgets.hovered.bg_fill,
                "flat controls still need visible hover feedback"
            );
            assert!(style.spacing.interact_size.y >= 34.0);
            assert!(style.spacing.button_padding.x >= 12.0);
            assert!(style.spacing.button_padding.y >= 6.0);
            for state in [
                visuals.widgets.inactive,
                visuals.widgets.hovered,
                visuals.widgets.active,
                visuals.widgets.open,
            ] {
                assert_eq!(state.expansion, 0.0, "hover must not move control edges");
                assert!(contrast(state.fg_stroke.color, state.bg_fill) >= 4.5);
            }
        });
    }
}

#[test]
fn generous_popup_frames_fit_small_viewports_even_with_a_large_minimum_body() {
    for workspace in [false, true] {
        let context = Context::default();
        theme::apply(&context);
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 720.0));
        for _ in 0..3 {
            let _ = context.run(
                RawInput {
                    screen_rect: Some(viewport),
                    ..Default::default()
                },
                |ctx| {
                    let popup = if workspace {
                        daw_ui::workspace_popup("Automation", ctx)
                    } else {
                        daw_ui::popup("Device library", ctx)
                            .default_width(940.0)
                            .min_height(550.0)
                    };
                    let mut open = true;
                    let window = popup
                        .open(&mut open)
                        .show(ctx, |ui| {
                            daw_ui::button(ui, "Select");
                            egui::ScrollArea::vertical()
                                .min_scrolled_height(420.0)
                                .max_height(420.0)
                                .show(ui, |ui| {
                                    for index in 0..100 {
                                        ui.label(format!("Device {index}"));
                                    }
                                });
                        })
                        .unwrap()
                        .response
                        .rect;
                    assert!(
                        window.top() >= daw_ui::tokens::POPUP_TOP,
                        "popup must leave the transport accessible: {window:?}"
                    );
                    assert!(
                        viewport
                            .shrink(daw_ui::tokens::POPUP_MARGIN)
                            .contains_rect(window),
                        "native title, close control, frame and body must all fit: {window:?}"
                    );
                },
            );
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

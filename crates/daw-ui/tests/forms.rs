use daw_ui::{self as controls, text, theme};
use egui::{Context, Event, Key, Modifiers, RawInput, Rect, Response};

#[test]
fn long_path_row_keeps_actions_in_bounds_and_keyboard_accessible() {
    for density in [1.0, 1.25, 2.0] {
        let context = Context::default();
        theme::apply(&context);
        context.set_pixels_per_point(density);
        let path = std::path::PathBuf::from("/音楽 🎹/Expressive instruments/".repeat(20));
        let mut action: Option<Response> = None;
        for activate in [false, true] {
            if let Some(action) = &action {
                action.request_focus();
            }
            let _ = context.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(420.0, 240.0),
                    )),
                    events: if activate {
                        vec![Event::Key {
                            key: Key::Enter,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Modifiers::NONE,
                        }]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let available = ui.available_rect_before_wrap();
                        let row = controls::path_row(ui, &path, |ui| {
                            controls::small_button(ui, "Remove")
                        });
                        assert!(available.contains_rect(row.response.rect));
                        assert!(available.contains_rect(row.inner.rect));
                        assert_eq!(row.inner.clicked(), activate);
                        action = Some(row.inner);
                    });
                },
            );
        }
    }
}

#[test]
fn form_rows_leave_clear_space_between_labels_and_adjacent_controls_at_display_scales() {
    for density in [1.0, 1.25, 2.0] {
        let context = Context::default();
        theme::apply(&context);
        context.set_pixels_per_point(density);
        for _ in 0..3 {
            let _ = context.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(620.0, 440.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut previous: Option<Rect> = None;
                        controls::form_grid(ui, "field_clearance", |ui| {
                            for label in ["Output", "Sample rate", "Sample conversion"] {
                                let label = text::label(ui, label);
                                let control = controls::select(label.id, "System default")
                                    .show_ui(ui, |_| {})
                                    .response;
                                assert!(
                                    control.rect.left() - label.rect.right() >= 20.5,
                                    "form labels need a clear gutter at {density}×"
                                );
                                let row = label.rect.union(control.rect);
                                if let Some(previous) = previous {
                                    assert!(
                                        row.top() - previous.bottom() >= 12.5,
                                        "adjacent field targets crowd at {density}×: {previous:?}, {row:?}"
                                    );
                                }
                                previous = Some(row);
                                ui.end_row();
                            }
                        });
                    });
                },
            );
        }
    }
}

#[test]
fn form_grid_keeps_labels_single_line_and_controls_in_a_shared_column() {
    let context = Context::default();
    theme::apply(&context);
    for _ in 0..3 {
        let _ = context.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(560.0, 400.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let available = ui.available_rect_before_wrap();
                    let mut control_x = None;
                    controls::form_grid(ui, "settings", |ui| {
                        for label in ["Output", "Sample rate", "Sample conversion"] {
                            let response = text::label(ui, label);
                            assert!(response.rect.height() < 24.0, "{label} wrapped");
                            let control = controls::select(label, "System default")
                                .width(if label == "Output" { 320.0 } else { 160.0 })
                                .show_ui(ui, |_| {})
                                .response;
                            assert!(available.contains_rect(control.rect));
                            assert!(response.rect.right() < control.rect.left());
                            assert_eq!(
                                *control_x.get_or_insert(control.rect.left()),
                                control.rect.left()
                            );
                            ui.end_row();
                        }
                    });
                });
            },
        );
    }
}

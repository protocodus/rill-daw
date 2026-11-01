use daw_ui::{self as controls, theme};
use egui::{Context, Event, Modifiers, RawInput, Rect};

#[test]
fn long_imported_titles_leave_room_for_decision_controls_and_actions() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 1.25, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            let bounds = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
            let title = format!("Strip silence · {} · 夜の音楽", "W".repeat(512));
            for frame in 0..4 {
                let output = ctx.run(
                    RawInput {
                        screen_rect: Some(bounds),
                        ..Default::default()
                    },
                    |ctx| {
                        let modal =
                            controls::modal(ctx, egui::Id::new("long_title"), &title, |ui| {
                                // A fixed processing form and feedback area still
                                // need useful space beneath arbitrary source names.
                                let body =
                                    ui.allocate_space(egui::vec2(ui.available_width(), 300.0)).1;
                                let actions = ui.horizontal(|ui| {
                                    [
                                        controls::button(ui, "Apply"),
                                        controls::button(ui, "Cancel"),
                                    ]
                                });
                                if frame >= 2 {
                                    assert!(bounds.contains_rect(body));
                                    for action in &actions.inner {
                                        assert!(bounds.contains_rect(action.rect));
                                        assert!(
                                            action.rect.top()
                                                >= body.bottom() + controls::tokens::SPACE_MD
                                                    - 1.0 / density
                                        );
                                    }
                                }
                            });
                        if frame >= 2 {
                            assert!(bounds.contains_rect(modal.response.rect));
                        }
                    },
                );
                if frame < 2 {
                    continue;
                }
                let heading = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == title => Some(text),
                        _ => None,
                    })
                    .expect("The complete title remains in the rendered label source");
                assert!(heading.galley.elided);
                assert!(heading.galley.rows.len() <= 2);
                assert!(bounds.contains_rect(heading.galley.rect.translate(heading.pos.to_vec2())));
            }
        }
    }
}

#[test]
fn decision_dialog_keeps_long_unicode_context_and_actions_inside_viewport() {
    for dark in [false, true] {
        for density in [1.0, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(
                &ctx,
                if dark {
                    theme::Appearance::Dark
                } else {
                    theme::Appearance::Light
                },
            );
            ctx.set_pixels_per_point(density);
            let bounds = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 720.0));
            // Apply the density change before measuring the first modal frame.
            let _ = ctx.run(
                RawInput {
                    screen_rect: Some(bounds),
                    ..Default::default()
                },
                |_| {},
            );
            let name = "夜の音楽 🎹 Long project name ".repeat(30);
            let error = "Project operation failed: /some/a-long-unbroken-path/".repeat(30);
            for frame in 0..3 {
                let output = ctx.run(
                    RawInput {
                        screen_rect: Some(bounds),
                        ..Default::default()
                    },
                    |ctx| {
                        let response = controls::modal(
                            ctx,
                            egui::Id::new("decision"),
                            "Save changes?",
                            |ui| {
                                controls::text::bounded_block(
                                    ui,
                                    controls::text::TextRole::Subtitle,
                                    &name,
                                    2,
                                );
                                ui.add_space(controls::tokens::SPACE_SM);
                                controls::text::body(
                                    ui,
                                    "Save your changes before starting a new project?",
                                );
                                controls::text::bounded_block(
                                    ui,
                                    controls::text::TextRole::Body,
                                    &error,
                                    4,
                                );
                                ui.add_space(controls::tokens::SPACE_XL);
                                controls::action_row(
                                    ui,
                                    |ui| {
                                        for label in ["Save", "Don't save", "Cancel"] {
                                            let response = controls::button(ui, label);
                                            assert!(
                                                bounds.shrink(24.0).contains_rect(response.rect), "button {:?}, bounds {:?}, density {density}, frame {frame}", response.rect, bounds
                                            );
                                        }
                                    },
                                    |_| {},
                                );
                            },
                        );
                        assert!(bounds.shrink(24.0).contains_rect(response.response.rect));
                    },
                );
                if frame < 2 {
                    continue;
                }
                for expected in [&name, &error] {
                    let text = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if &text.galley.job.text == expected => {
                                Some(text)
                            }
                            _ => None,
                        })
                        .unwrap();
                    assert!(text.galley.elided);
                    assert!(text.galley.size().x <= 460.0);
                }
            }
        }
    }
}

#[test]
fn modal_buttons_keep_keyboard_activation_and_block_background_clicks() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut background = None;
    let mut primary = None;
    let mut background_clicked = false;
    let mut accepted = false;
    let mut draw = |events| {
        let _ = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 720.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = controls::button(ui, "Background action");
                    background_clicked |= response.clicked();
                    background = Some(response);
                });
                controls::modal(ctx, egui::Id::new("decision"), "Save changes?", |ui| {
                    controls::text::body(ui, "Save this project?");
                    let response = controls::primary_button(ui, "Save");
                    accepted |= response.clicked();
                    primary = Some(response);
                });
            },
        );
        (
            background.clone().unwrap(),
            primary.clone().unwrap(),
            background_clicked,
            accepted,
        )
    };
    for _ in 0..3 {
        draw(vec![]);
    }
    let (background, primary, _, _) = draw(vec![]);
    for pressed in [true, false] {
        let (_, _, background_clicked, _) = draw(vec![
            Event::PointerMoved(background.rect.center()),
            Event::PointerButton {
                pos: background.rect.center(),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ]);
        assert!(!background_clicked);
    }
    primary.request_focus();
    let (_, _, _, accepted) = draw(vec![Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    assert!(accepted);
}

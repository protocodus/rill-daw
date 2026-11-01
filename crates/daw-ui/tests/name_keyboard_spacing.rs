use daw_ui::{name_keyboard, name_keyboard_height, set_name_keyboard_open, theme, tokens};
use egui::{Context, Event, FullOutput, Id, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

fn frame(ctx: &Context, size: Vec2, events: Vec<Event>) -> (FullOutput, Rect) {
    let mut bounds = Rect::NOTHING;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                bounds = ui
                    .allocate_ui_with_layout(
                        Vec2::new(460.0, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(460.0);
                            let field = Id::new("keyboard_spacing_field");
                            let mut value = String::new();
                            daw_ui::text_field(ui, &mut value, "Name", 220.0);
                            set_name_keyboard_open(ctx, field, true);
                            name_keyboard(ui, field, &mut value);
                        },
                    )
                    .response
                    .rect;
            });
        },
    );
    (output, bounds)
}

fn text_rect(output: &FullOutput, label: &str) -> Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing keyboard label {label}"))
}

#[test]
fn lowercase_uppercase_and_command_rows_fit_the_shared_width_and_predicted_height() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 1.25, 2.0] {
            for size in [Vec2::new(800.0, 600.0), Vec2::new(1440.0, 1000.0)] {
                let ctx = Context::default();
                theme::apply(&ctx);
                theme::set_appearance(&ctx, appearance);
                ctx.set_pixels_per_point(density);
                for _ in 0..3 {
                    frame(&ctx, size, vec![]);
                }
                let (lowercase, bounds) = frame(&ctx, size, vec![]);
                let tolerance = 1.0 / density;
                assert!(bounds.width() <= 460.0 + tolerance, "{bounds:?}");
                assert!(
                    bounds.height()
                        <= tokens::CONTROL_HEIGHT + name_keyboard_height(&ctx) + tolerance,
                    "{bounds:?}"
                );
                let caps = text_rect(&lowercase, "Caps").center();
                for pressed in [true, false] {
                    frame(
                        &ctx,
                        size,
                        vec![
                            Event::PointerMoved(caps),
                            Event::PointerButton {
                                pos: caps,
                                button: PointerButton::Primary,
                                pressed,
                                modifiers: Modifiers::NONE,
                            },
                        ],
                    );
                }
                let (uppercase, bounds) = frame(&ctx, size, vec![]);
                assert!(
                    bounds.width() <= 460.0 + tolerance,
                    "Uppercase row: {bounds:?}"
                );
                assert!(
                    bounds.height()
                        <= tokens::CONTROL_HEIGHT + name_keyboard_height(&ctx) + tolerance
                );
                for label in [
                    "Q",
                    "W",
                    "M",
                    "Caps",
                    "Space",
                    "Backspace",
                    "Select all",
                    "Clear",
                ] {
                    assert!(
                        bounds.contains_rect(text_rect(&uppercase, label)),
                        "{label}: {bounds:?}"
                    );
                }
                let q = text_rect(&uppercase, "Q");
                let w = text_rect(&uppercase, "W");
                assert!(
                    w.center().x - q.center().x
                        >= tokens::CONTROL_HEIGHT + tokens::SPACE_SM - tolerance
                );
            }
        }
    }
}

#[test]
fn inline_editor_surface_matches_its_measured_bounds_and_covers_every_row() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 1.25, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            let field = Id::new("opaque_inline_name");
            let mut value = "Long existing phrase name".to_owned();
            for keyboard in [false, true] {
                set_name_keyboard_open(&ctx, field, keyboard);
                let mut measured = Vec2::ZERO;
                let mut render = || {
                    ctx.run(
                        RawInput {
                            screen_rect: Some(Rect::from_min_size(
                                Pos2::ZERO,
                                Vec2::new(800.0, 600.0),
                            )),
                            ..Default::default()
                        },
                        |ctx| {
                            measured = daw_ui::inline_name_editor_size(ctx, 220.0, keyboard);
                            egui::Area::new(Id::new("opaque_inline_area"))
                                .fade_in(false)
                                .fixed_pos(egui::pos2(8.0, 8.0))
                                .show(ctx, |ui| {
                                    daw_ui::inline_name_editor(
                                        ui, field, &mut value, "Name", 220.0, false,
                                    );
                                });
                        },
                    )
                };
                render();
                render();
                let output = render();
                let keyboard_label = text_rect(&output, "Keyboard");
                let cancel = text_rect(&output, "Cancel");
                let surface = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(shape)
                            if shape.fill == theme::palette(&ctx).raised
                                && shape.rect.contains_rect(keyboard_label)
                                && shape.rect.contains_rect(cancel) =>
                        {
                            Some(shape.rect)
                        }
                        _ => None,
                    })
                    .expect("The inline controls share one opaque surface");
                let tolerance = 1.0 / density;
                assert!(
                    surface.width() <= measured.x + tolerance,
                    "{surface:?} > {measured:?}"
                );
                assert!(
                    surface.height() <= measured.y + tolerance,
                    "{surface:?} > {measured:?}"
                );
                assert!(
                    surface
                        .shrink(tokens::SPACE_MD)
                        .contains_rect(keyboard_label)
                );
                assert!(surface.shrink(tokens::SPACE_MD).contains_rect(cancel));
                if keyboard {
                    for label in ["q", "p", "Clear", "Caps"] {
                        assert!(
                            surface
                                .shrink(tokens::SPACE_MD)
                                .contains_rect(text_rect(&output, label))
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn inline_editor_surface_catches_pointer_gaps_without_adding_an_unnamed_tab_stop() {
    for keyboard in [false, true] {
        let ctx = Context::default();
        theme::apply(&ctx);
        let field = Id::new("inline_name_focus");
        set_name_keyboard_open(&ctx, field, keyboard);
        let mut value = "Keep this name".to_owned();
        let mut draw = |events, modifiers| {
            let mut before = None;
            let _ = ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                    events,
                    modifiers,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        before = Some(daw_ui::button(ui, "Previous control"));
                        daw_ui::inline_name_editor(ui, field, &mut value, "Name", 220.0, false);
                    });
                },
            );
            before.unwrap()
        };
        draw(vec![], Modifiers::NONE);
        let before = draw(vec![], Modifiers::NONE);
        let surface = ctx.read_response(field.with("surface")).unwrap();
        assert!(
            surface.sense.senses_click(),
            "The surface still intercepts pointer gaps"
        );
        assert!(
            !surface.sense.is_focusable(),
            "A decorative surface is not a keyboard action"
        );
        before.request_focus();
        draw(vec![], Modifiers::NONE);
        let tab = |pressed, modifiers| {
            vec![Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            }]
        };
        draw(tab(true, Modifiers::NONE), Modifiers::NONE);
        draw(tab(false, Modifiers::NONE), Modifiers::NONE);
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(field),
            "Tab goes directly to the named text field"
        );
        let shift = Modifiers {
            shift: true,
            ..Modifiers::NONE
        };
        draw(tab(true, shift), shift);
        draw(tab(false, shift), shift);
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(before.id),
            "Shift-Tab skips the unnamed surface"
        );
        assert_eq!(value, "Keep this name");
    }
}

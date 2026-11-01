use daw_ui::{self as controls, IconButton, theme};
use egui::{
    Context, Event, FullOutput, Key, Modifiers, PointerButton, RawInput, Rect, Response, Shape,
};

fn input(events: Vec<Event>, width: f32) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width, 480.0),
        )),
        events,
        ..Default::default()
    }
}

fn key(key: Key, pressed: bool) -> Vec<Event> {
    vec![Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]
}

fn pointer(position: egui::Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(position),
        Event::PointerButton {
            pos: position,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn frame(
    ctx: &Context,
    events: Vec<Event>,
    mut control: impl FnMut(&mut egui::Ui) -> Response,
) -> (Response, FullOutput) {
    let mut response = None;
    let output = ctx.run(input(events, 480.0), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            response = Some(control(ui));
        });
    });
    (response.unwrap(), output)
}

#[test]
fn tools_emit_one_labelled_radio_event_with_the_supplied_selected_state() {
    for selected in [false, true] {
        let ctx = Context::default();
        theme::apply(&ctx);
        frame(&ctx, vec![], |ui| {
            ui.add(IconButton::tool(selected, "Pencil"))
        });
        let (_, focused) = frame(&ctx, key(Key::Tab, true), |ui| {
            ui.add(IconButton::tool(selected, "Pencil"))
        });
        frame(&ctx, key(Key::Tab, false), |ui| {
            ui.add(IconButton::tool(selected, "Pencil"))
        });
        let (activated, clicked) = frame(&ctx, key(Key::Enter, true), |ui| {
            ui.add(IconButton::tool(selected, "Pencil"))
        });
        assert!(activated.clicked());
        for output in [&focused, &clicked] {
            let events = &output.platform_output.events;
            assert_eq!(
                events.len(),
                1,
                "Replacing native metadata must not duplicate events"
            );
            let info = events[0].widget_info();
            assert_eq!(info.typ, egui::WidgetType::RadioButton);
            assert_eq!(info.label.as_deref(), Some("Pencil"));
            assert_eq!(info.selected, Some(selected));
            assert!(info.enabled);
        }
        assert!(matches!(
            focused.platform_output.events[0],
            egui::output::OutputEvent::FocusGained(_)
        ));
        assert!(matches!(
            clicked.platform_output.events[0],
            egui::output::OutputEvent::Clicked(_)
        ));
    }
}

#[test]
fn tool_values_support_pointer_and_keyboard_without_reselect_changes_or_disabled_edits() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut current = 0;
    let (initial, _) = frame(&ctx, vec![], |ui| {
        controls::tool_value(ui, &mut current, 1, "Pencil")
    });
    let target = initial.rect.center();
    frame(&ctx, pointer(target, true), |ui| {
        controls::tool_value(ui, &mut current, 1, "Pencil")
    });
    let (chosen, _) = frame(&ctx, pointer(target, false), |ui| {
        controls::tool_value(ui, &mut current, 1, "Pencil")
    });
    assert_eq!(current, 1);
    assert!(chosen.changed());
    chosen.request_focus();
    let (reselected, _) = frame(&ctx, key(Key::Enter, true), |ui| {
        controls::tool_value(ui, &mut current, 1, "Pencil")
    });
    assert!(
        !reselected.changed(),
        "Selecting an active mode is not a value change"
    );
    frame(&ctx, key(Key::Enter, false), |ui| {
        controls::tool_value(ui, &mut current, 1, "Pencil")
    });

    current = 0;
    let (keyboard, _) = frame(&ctx, key(Key::Space, true), |ui| {
        controls::tool_value(ui, &mut current, 1, "Pencil")
    });
    assert_eq!(current, 1);
    assert!(keyboard.changed());
    frame(&ctx, key(Key::Space, false), |ui| {
        controls::tool_value(ui, &mut current, 1, "Pencil")
    });

    current = 0;
    let draw_disabled = |ui: &mut egui::Ui, current: &mut i32| {
        ui.add_enabled_ui(false, |ui| controls::tool_value(ui, current, 1, "Pencil"))
            .inner
    };
    let (disabled, _) = frame(&ctx, key(Key::Enter, true), |ui| {
        draw_disabled(ui, &mut current)
    });
    assert!(!disabled.enabled());
    assert!(!disabled.changed());
    assert_eq!(current, 0);
    frame(&ctx, pointer(target, true), |ui| {
        draw_disabled(ui, &mut current)
    });
    let (disabled, output) = frame(&ctx, pointer(target, false), |ui| {
        draw_disabled(ui, &mut current)
    });
    assert!(!disabled.changed());
    assert_eq!(current, 0);
    assert!(output.platform_output.events.is_empty());
}

#[test]
fn outlined_view_toggles_keep_native_keyboard_and_disabled_behavior() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut value = false;
    let (idle, output) = frame(&ctx, vec![], |ui| {
        controls::view_toggle(ui, &mut value, "Click")
    });
    let (fill, stroke) = rectangle_style(&output, idle.rect).unwrap();
    assert_eq!(fill, egui::Color32::TRANSPARENT);
    assert!(stroke.width > 0.0);
    idle.request_focus();
    let (chosen, _) = frame(&ctx, key(Key::Space, true), |ui| {
        controls::view_toggle(ui, &mut value, "Click")
    });
    assert!(chosen.changed() && value);
    frame(&ctx, key(Key::Space, false), |ui| {
        controls::view_toggle(ui, &mut value, "Click")
    });
    let (disabled, _) = frame(&ctx, key(Key::Enter, true), |ui| {
        ui.add_enabled_ui(false, |ui| controls::view_toggle(ui, &mut value, "Click"))
            .inner
    });
    assert!(!disabled.changed() && value);
    assert!(!disabled.enabled());
}

fn rectangle_style(output: &FullOutput, rect: Rect) -> Option<(egui::Color32, egui::Stroke)> {
    fn visit(shape: &Shape, rect: Rect, style: &mut Option<(egui::Color32, egui::Stroke)>) {
        match shape {
            Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, rect, style);
                }
            }
            Shape::Rect(shape)
                if shape.rect == rect && shape.blur_width == 0.0 && shape.brush.is_none() =>
            {
                // Materials keep the flat base and keyline in separate shapes.
                // The translucent gradient and blurred halo are decoration;
                // the native transparent frame must not erase the material.
                let (fill, stroke) =
                    style.get_or_insert((egui::Color32::TRANSPARENT, egui::Stroke::NONE));
                if shape.fill != egui::Color32::TRANSPARENT {
                    *fill = shape.fill;
                }
                if !shape.stroke.is_empty() {
                    *stroke = shape.stroke;
                }
            }
            _ => {}
        }
    }
    let mut style = None;
    for shape in &output.shapes {
        visit(&shape.shape, rect, &mut style);
    }
    style
}

fn selection_border(output: &FullOutput, rect: Rect, accent: egui::Color32) -> bool {
    output.shapes.iter().any(|shape| {
        matches!(&shape.shape,
            Shape::Rect(shape) if shape.rect == rect
                && shape.stroke.width == controls::tokens::FOCUS_WIDTH
                && shape.stroke.color == accent
        )
    })
}

#[test]
fn named_tool_families_keep_their_label_and_target_and_announce_the_active_mode() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(&ctx, appearance);
        let mut initial_rect = None;
        for (icon, active, expected) in [
            (controls::Icon::Duplicate, None, "Copy tools"),
            (
                controls::Icon::Duplicate,
                Some("Duplicate"),
                "Copy tools · Duplicate",
            ),
            (
                controls::Icon::Link,
                Some("Ghost copy"),
                "Copy tools · Ghost copy",
            ),
        ] {
            let draw = |ui: &mut egui::Ui| {
                ui.add(IconButton::tool_group_choice("Copy tools", icon, active))
            };
            let (idle, output) = frame(&ctx, vec![], draw);
            assert!(idle.rect.height() >= controls::tokens::CONTROL_HEIGHT);
            assert_eq!(*initial_rect.get_or_insert(idle.rect), idle.rect);
            assert!(
                output.shapes.iter().any(|shape| matches!(&shape.shape,
                    Shape::Text(text) if text.galley.text() == "Copy tools"
                )),
                "The category stays visible when its current tool changes"
            );
            assert_eq!(
                selection_border(&output, idle.rect, theme::palette(&ctx).accent),
                active.is_some()
            );
            idle.request_focus();
            let (activated, output) = frame(&ctx, key(Key::Enter, true), draw);
            assert!(activated.clicked());
            assert_eq!(output.platform_output.events.len(), 1);
            let info = output.platform_output.events[0].widget_info();
            assert_eq!(info.typ, egui::WidgetType::Button);
            assert_eq!(info.label.as_deref(), Some(expected));
            frame(&ctx, key(Key::Enter, false), draw);
            let (disabled, _) = frame(&ctx, key(Key::Space, true), |ui| {
                ui.add_enabled_ui(false, draw).inner
            });
            assert!(!disabled.enabled() && !disabled.clicked());
            frame(&ctx, key(Key::Space, false), draw);
        }
    }
}

#[test]
fn quiet_commands_and_guides_keep_native_activation_and_disabled_guards() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for toggle in [false, true] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            let mut value = false;
            let label = if toggle { "Loop" } else { "Stop" };
            let draw = |ui: &mut egui::Ui, value: &mut bool| {
                let style = ui.style().clone();
                let response = if toggle {
                    controls::quiet_toggle(ui, value, label)
                } else {
                    controls::quiet_button(ui, label)
                };
                assert_eq!(*ui.style(), style, "quiet styles must stay local");
                response
            };
            let (idle, output) = frame(&ctx, vec![], |ui| draw(ui, &mut value));
            assert!(idle.rect.height() >= controls::tokens::CONTROL_HEIGHT);
            if let Some((fill, stroke)) = rectangle_style(&output, idle.rect) {
                assert_eq!(fill, egui::Color32::TRANSPARENT);
                assert_eq!(stroke, egui::Stroke::NONE);
            }
            let target = idle.rect.center();
            frame(&ctx, pointer(target, true), |ui| draw(ui, &mut value));
            let (clicked, _) = frame(&ctx, pointer(target, false), |ui| draw(ui, &mut value));
            assert!(clicked.clicked());
            assert_eq!(value, toggle);
            let (resting, output) =
                frame(&ctx, vec![Event::PointerGone], |ui| draw(ui, &mut value));
            if toggle {
                assert_eq!(
                    rectangle_style(&output, resting.rect).unwrap().0,
                    theme::palette(&ctx).track_selected,
                    "An enabled guide stays visibly selected after the pointer leaves"
                );
            }
            resting.request_focus();
            let (activated, output) = frame(&ctx, key(Key::Enter, true), |ui| draw(ui, &mut value));
            assert!(activated.clicked());
            assert!(!value);
            assert!(output.platform_output.events.iter().any(|event| {
                let info = event.widget_info();
                info.typ == egui::WidgetType::Button && info.label.as_deref() == Some(label)
            }));
            frame(&ctx, key(Key::Enter, false), |ui| draw(ui, &mut value));
            let mut disabled_draw =
                |ui: &mut egui::Ui| ui.add_enabled_ui(false, |ui| draw(ui, &mut value)).inner;
            let (disabled, _) = frame(&ctx, key(Key::Space, true), &mut disabled_draw);
            assert!(!disabled.enabled() && !disabled.clicked());
            frame(&ctx, pointer(target, true), &mut disabled_draw);
            let (disabled, _) = frame(&ctx, pointer(target, false), &mut disabled_draw);
            assert!(!disabled.clicked() && !disabled.changed());
            assert!(!value);
        }
    }
}

#[test]
fn quiet_tabs_keep_full_targets_selection_and_native_activation() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for selected in [false, true] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            let (idle, output) = frame(&ctx, vec![], |ui| {
                controls::tab_choice(ui, selected, "Examples")
            });
            assert!(idle.rect.height() >= controls::tokens::CONTROL_HEIGHT);
            if let Some((fill, stroke)) = rectangle_style(&output, idle.rect) {
                assert_eq!(fill, egui::Color32::TRANSPARENT);
                assert_eq!(
                    stroke,
                    if selected {
                        egui::Stroke::new(
                            controls::tokens::FOCUS_WIDTH,
                            theme::palette(&ctx).accent,
                        )
                    } else {
                        egui::Stroke::NONE
                    }
                );
            }
            let target = idle.rect.center();
            frame(&ctx, pointer(target, true), |ui| {
                controls::tab_choice(ui, selected, "Examples")
            });
            let (clicked, _) = frame(&ctx, pointer(target, false), |ui| {
                controls::tab_choice(ui, selected, "Examples")
            });
            assert!(clicked.clicked());
            clicked.request_focus();
            let (activated, output) = frame(&ctx, key(Key::Enter, true), |ui| {
                controls::tab_choice(ui, selected, "Examples")
            });
            assert!(activated.clicked());
            let events = &output.platform_output.events;
            assert_eq!(events.len(), 1);
            let info = events[0].widget_info();
            assert_eq!(info.typ, egui::WidgetType::RadioButton);
            assert_eq!(info.label.as_deref(), Some("Examples"));
            assert_eq!(info.selected, Some(selected));
            frame(&ctx, key(Key::Enter, false), |ui| {
                controls::tab_choice(ui, selected, "Examples")
            });
            let (disabled, _) = frame(&ctx, key(Key::Space, true), |ui| {
                ui.add_enabled_ui(false, |ui| controls::tab_choice(ui, selected, "Examples"))
                    .inner
            });
            assert!(!disabled.enabled() && !disabled.clicked());
        }
    }
}

#[test]
fn role_styles_do_not_leak_and_active_tools_keep_a_palette_indicator_at_both_densities() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for (width, density) in [(280.0, 1.0), (800.0, 1.25), (900.0, 2.0)] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            let mut responses = None;
            let mut current = 0;
            let output = ctx.run(input(vec![], width), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let original = ui.style().clone();
                    let row = controls::toolbar(ui, |ui| {
                        [
                            controls::tool_value(ui, &mut current, 0, "Select"),
                            controls::tool_value(ui, &mut current, 1, "Pencil"),
                            controls::view_button(ui, "Fit"),
                            controls::button(ui, "Generate…"),
                        ]
                    });
                    assert_eq!(
                        *ui.style(),
                        original,
                        "role styles must stay local to the control"
                    );
                    for (index, response) in row.inner.iter().enumerate() {
                        assert!(ui.max_rect().contains_rect(response.rect));
                        assert!(response.rect.height() >= controls::tokens::CONTROL_HEIGHT);
                        for previous in &row.inner[..index] {
                            assert!(
                                !response.rect.intersects(previous.rect),
                                "wrapped targets must not overlap"
                            );
                        }
                    }
                    responses = Some(row.inner);
                });
            });
            let [active, inactive, view, action] = responses.unwrap();
            let active_style = rectangle_style(&output, active.rect).unwrap();
            assert_eq!(active_style.0, theme::palette(&ctx).track_selected);
            if let Some(inactive_style) = rectangle_style(&output, inactive.rect) {
                assert_eq!(inactive_style.0, egui::Color32::TRANSPARENT);
                assert_eq!(inactive_style.1, egui::Stroke::NONE);
            }
            let view_style = rectangle_style(&output, view.rect).unwrap();
            assert_eq!(view_style.0, egui::Color32::TRANSPARENT);
            assert!(view_style.1.width > 0.0);
            let action_style = rectangle_style(&output, action.rect).unwrap();
            assert_ne!(action_style.0, egui::Color32::TRANSPARENT);
            assert_eq!(action_style.1.width, controls::tokens::BORDER_WIDTH);
            assert_eq!(
                action_style.1.color,
                theme::palette(&ctx)
                    .line
                    .gamma_multiply(controls::tokens::GOLDEN_RATIO_INVERSE),
                "commands retain their subtle keyline after an outlined view control"
            );

            let indicators: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    Shape::Rect(shape)
                        if shape.stroke.width == controls::tokens::FOCUS_WIDTH
                            && shape.stroke.color == theme::palette(&ctx).accent =>
                    {
                        Some(shape.rect)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(indicators.len(), 1);
            assert_eq!(active.rect, indicators[0]);
            assert!(!inactive.rect.intersects(indicators[0]));
        }
    }
}

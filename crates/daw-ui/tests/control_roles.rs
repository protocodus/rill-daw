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
    output.shapes.iter().find_map(|shape| match &shape.shape {
        Shape::Rect(shape) if shape.rect == rect => Some((shape.fill, shape.stroke)),
        _ => None,
    })
}

#[test]
fn role_styles_do_not_leak_and_active_tools_keep_a_palette_indicator_at_both_densities() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for (width, density) in [(280.0, 1.0), (900.0, 2.0)] {
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
            assert_eq!(
                action_style.1,
                egui::Stroke::NONE,
                "view borders must not leak into following commands"
            );

            let indicators: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    Shape::Rect(shape)
                        if (shape.rect.height() - 3.0).abs() < 0.01
                            && shape.fill == theme::palette(&ctx).accent =>
                    {
                        Some(shape.rect)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(indicators.len(), 1);
            assert!(active.rect.contains_rect(indicators[0]));
            assert!(!inactive.rect.intersects(indicators[0]));
        }
    }
}

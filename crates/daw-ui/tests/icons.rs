use daw_ui::{Icon, IconButton, theme};
use egui::{Context, Event, Key, Modifiers, RawInput, Rect, Shape};

#[test]
fn editor_actions_use_a_window_icon_and_prefixed_actions_keep_one_plus() {
    assert_eq!(Icon::for_label("Open editor"), Some(Icon::Editor));
    assert_eq!(Icon::for_label("Show editor"), Some(Icon::Editor));
    assert_eq!(Icon::for_label("Open…"), Some(Icon::Open));
    assert_eq!(Icon::for_label("+ Insert"), None);
    assert_eq!(Icon::for_label("+ Send"), None);
    assert_eq!(Icon::for_label("Add point"), Some(Icon::Add));
}

#[test]
fn reorder_grip_is_six_vector_dots_and_does_not_depend_on_a_symbol_font() {
    for density in [1.0, 2.0] {
        let ctx = Context::default();
        theme::apply(&ctx);
        ctx.set_pixels_per_point(density);
        let output = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                daw_ui::reorder_handle(ui, 7_u64);
            });
        });
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, Shape::Text(_)))
        );
        let dots = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Circle(dot) => Some((shape.clip_rect, dot)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(dots.len(), 6);
        for (clip, dot) in dots {
            assert!(clip.contains_rect(Rect::from_center_size(
                dot.center,
                egui::Vec2::splat(dot.radius * 2.0),
            )));
            assert_ne!(dot.fill, egui::Color32::TRANSPARENT);
        }
    }
}

#[test]
fn icon_only_buttons_keep_native_keyboard_activation_and_accessible_names() {
    let ctx = Context::default();
    theme::apply(&ctx);
    for phase in 0..3 {
        let activate = phase == 2;
        let output = ctx.run(
            RawInput {
                events: if phase > 0 {
                    vec![Event::Key {
                        key: if activate { Key::Enter } else { Key::Tab },
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
                    let button = ui.add(IconButton::icon_only(Icon::Editor, "Open editor"));
                    assert_eq!(button.clicked(), activate);
                });
            },
        );
        if phase > 0 {
            let events = &output.platform_output.events;
            assert_eq!(events.len(), 1, "one labelled event per interaction");
            if activate {
                assert!(matches!(events[0], egui::output::OutputEvent::Clicked(_)));
            } else {
                assert!(matches!(
                    events[0],
                    egui::output::OutputEvent::FocusGained(_)
                ));
            }
            let info = events[0].widget_info();
            assert_eq!(info.typ, egui::WidgetType::Button);
            assert_eq!(info.label.as_deref(), Some("Open editor"));
            assert!(info.enabled);
        }
    }
}

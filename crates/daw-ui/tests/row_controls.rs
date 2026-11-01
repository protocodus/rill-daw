use daw_ui::{self as controls, theme, tokens};
use egui::{Context, Event, FullOutput, Key, Modifiers, RawInput, Rect, Response, Ui};

fn frame(
    ctx: &Context,
    width: f32,
    events: Vec<Event>,
    enabled: bool,
    mut draw: impl FnMut(&mut Ui) -> Response,
) -> (Response, FullOutput) {
    let mut response = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 800.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                response = Some(
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(width);
                            ui.add_enabled_ui(enabled, &mut draw).inner
                        },
                    )
                    .inner,
                );
            });
        },
    );
    let response = response.unwrap();
    assert!(response.rect.width() <= width + 1.0 / output.pixels_per_point);
    (response, output)
}

fn painted_text(output: &FullOutput, label: &str) -> Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape
                && text.galley.text() == label
            {
                let bounds = text.galley.rect.translate(text.pos.to_vec2());
                assert!(
                    shape
                        .clip_rect
                        .expand(1.0 / output.pixels_per_point)
                        .contains_rect(bounds),
                    "Clipped {label}: {bounds:?}"
                );
                Some(bounds)
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("Missing painted text: {label}"))
}

fn pointer(position: egui::Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(position),
        Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
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

#[test]
fn wrapped_switch_labels_keep_clear_insets_and_native_activation_at_display_scales() {
    let label = "Monitor the selected channel while recording a new performance";
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 1.25, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            let mut value = false;
            for _ in 0..3 {
                frame(&ctx, 230.0, vec![], true, |ui| {
                    controls::switch(ui, &mut value, label)
                });
            }
            let (idle, output) = frame(&ctx, 230.0, vec![], true, |ui| {
                controls::switch(ui, &mut value, label)
            });
            let text = painted_text(&output, label);
            let tolerance = 1.0 / density;
            assert!(
                text.height() > tokens::CONTROL_HEIGHT,
                "The fixture must wrap"
            );
            assert!(text.top() - idle.rect.top() >= tokens::BUTTON_PADDING_Y - tolerance);
            assert!(idle.rect.bottom() - text.bottom() >= tokens::BUTTON_PADDING_Y - tolerance);
            let target = egui::pos2(text.center().x, text.bottom() - 2.0);
            frame(&ctx, 230.0, pointer(target, true), true, |ui| {
                controls::switch(ui, &mut value, label)
            });
            let (clicked, output) = frame(&ctx, 230.0, pointer(target, false), true, |ui| {
                controls::switch(ui, &mut value, label)
            });
            assert!(clicked.clicked() && clicked.changed() && value);
            assert_eq!(clicked.id, idle.id);
            assert_eq!(clicked.rect, idle.rect);
            assert!(output.platform_output.events.iter().any(|event| {
                let info = event.widget_info();
                info.typ == egui::WidgetType::Checkbox
                    && info.label.as_deref() == Some(label)
                    && info.selected == Some(true)
            }));
            clicked.request_focus();
            let (keyboard, _) = frame(&ctx, 230.0, key(Key::Space, true), true, |ui| {
                controls::switch(ui, &mut value, label)
            });
            assert!(keyboard.changed() && !value);
            frame(&ctx, 230.0, key(Key::Space, false), true, |ui| {
                controls::switch(ui, &mut value, label)
            });
            let (disabled, _) = frame(&ctx, 230.0, key(Key::Enter, true), false, |ui| {
                controls::switch(ui, &mut value, label)
            });
            assert!(!disabled.enabled() && !disabled.changed() && !value);
        }
    }
}

#[test]
fn catalog_rows_measure_the_text_stack_and_keep_preview_arrival_and_input_stable() {
    let title = "Expressive piano 🎹 日本語 — A complete accessible device name";
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 1.25, 2.0] {
            for detail in ["", "Instrument · Piano · Acoustic and electric"] {
                let ctx = Context::default();
                theme::apply(&ctx);
                theme::set_appearance(&ctx, appearance);
                ctx.set_pixels_per_point(density);
                let preview = ctx.load_texture(
                    "row-preview",
                    egui::ColorImage::filled([80, 40], egui::Color32::WHITE),
                    Default::default(),
                );
                let draw = |ui: &mut Ui, image: bool| {
                    controls::catalog_item(ui, true, title, detail, image.then_some(&preview))
                };
                for _ in 0..3 {
                    frame(&ctx, 290.0, vec![], true, |ui| draw(ui, false));
                }
                let (idle, output) = frame(&ctx, 290.0, vec![], true, |ui| draw(ui, false));
                let title_bounds = painted_text(&output, title);
                let tolerance = 1.0 / density;
                assert!(title_bounds.top() - idle.rect.top() >= tokens::SPACE_MD - tolerance);
                assert!(idle.rect.right() - title_bounds.right() >= tokens::SPACE_MD - tolerance);
                let last_bounds = if detail.is_empty() {
                    title_bounds
                } else {
                    let detail_bounds = painted_text(&output, detail);
                    assert!(
                        detail_bounds.top() - title_bounds.bottom() >= tokens::SPACE_XS - tolerance
                    );
                    detail_bounds
                };
                assert!(idle.rect.bottom() - last_bounds.bottom() >= tokens::SPACE_MD - tolerance);
                let (loaded, output) = frame(&ctx, 290.0, vec![], true, |ui| draw(ui, true));
                assert_eq!(loaded.id, idle.id);
                assert_eq!(loaded.rect, idle.rect);
                assert_eq!(painted_text(&output, title), title_bounds);
                frame(&ctx, 290.0, pointer(idle.rect.center(), true), true, |ui| {
                    draw(ui, true)
                });
                let (clicked, output) = frame(
                    &ctx,
                    290.0,
                    pointer(idle.rect.center(), false),
                    true,
                    |ui| draw(ui, true),
                );
                assert!(clicked.clicked());
                assert!(output.platform_output.events.iter().any(|event| {
                    let info = event.widget_info();
                    info.typ == egui::WidgetType::SelectableLabel
                        && info.label.as_deref() == Some(title)
                        && info.selected == Some(true)
                }));
                clicked.request_focus();
                let (keyboard, _) = frame(&ctx, 290.0, key(Key::Enter, true), true, |ui| {
                    draw(ui, true)
                });
                assert!(keyboard.clicked());
                frame(&ctx, 290.0, key(Key::Enter, false), true, |ui| {
                    draw(ui, true)
                });
                let (disabled, _) = frame(&ctx, 290.0, key(Key::Space, true), false, |ui| {
                    draw(ui, true)
                });
                assert!(!disabled.enabled() && !disabled.clicked());
            }
        }
    }
}

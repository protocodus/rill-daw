use daw_ui::{EditorTool, ToolHintContext, editor_tool_hint, theme};
use egui::{Context, Event, Key, Modifiers, RawInput, Rect, Shape};

fn input(width: f32, events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width, 480.0),
        )),
        events,
        ..Default::default()
    }
}

#[test]
fn every_tool_hint_wraps_inside_the_read_only_panel_in_both_themes_and_densities() {
    for dark in [false, true] {
        for density in [1.0, 2.0] {
            for width in [240.0, 800.0] {
                let ctx = Context::default();
                ctx.set_theme(if dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                ctx.set_pixels_per_point(density);
                theme::apply(&ctx);
                let _ = ctx.run(input(width, vec![]), |_| {});
                for tool in EditorTool::ALL {
                    for context in [
                        ToolHintContext::Arrangement,
                        ToolHintContext::Midi,
                        ToolHintContext::Audio,
                    ] {
                        let mut panel = Rect::NOTHING;
                        let output = ctx.run(input(width, vec![]), |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                let response = editor_tool_hint(ui, tool, context);
                                panel = response.rect;
                                assert_eq!(response.sense, egui::Sense::hover());
                            });
                        });
                        let text = output
                            .shapes
                            .iter()
                            .find_map(|shape| {
                                if let Shape::Text(text) = &shape.shape {
                                    Some(text)
                                } else {
                                    None
                                }
                            })
                            .unwrap();
                        let full = text.galley.text();
                        assert!(full.starts_with(context.label(tool)));
                        assert!(full.contains(tool.key().name()));
                        assert!(full.ends_with(context.hint(tool)));
                        let bounds = text.galley.rect.translate(text.pos.to_vec2());
                        assert!(
                            panel.contains_rect(bounds),
                            "{tool:?} {context:?}: {panel:?} / {bounds:?}"
                        );
                        assert!(
                            panel.right() <= width && panel.left() >= 0.0,
                            "{tool:?} {context:?} width {width}: {panel:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn hint_is_skipped_by_keyboard_focus_and_does_not_consume_tool_keys() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut after_has_focus = false;
    for (index, events) in [
        vec![],
        vec![Event::Key {
            key: Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        vec![Event::Key {
            key: Key::Q,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    ]
    .into_iter()
    .enumerate()
    {
        let _ = ctx.run(input(800.0, events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let before = ui.button("Before");
                if index == 0 {
                    before.request_focus();
                }
                let hint = editor_tool_hint(ui, EditorTool::Select, ToolHintContext::Midi);
                let after = ui.button("After");
                after_has_focus = after.has_focus();
                assert!(!hint.has_focus());
                if index == 2 {
                    assert!(ui.input(|input| input.key_pressed(Key::Q)));
                }
            });
        });
        if index > 0 {
            assert!(after_has_focus, "Hint must not become a Tab stop");
        }
    }
}

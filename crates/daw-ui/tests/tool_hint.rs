use daw_ui::{EditorTool, ToolHintContext, editor_tool_hint, theme, tokens};
use egui::{Context, Event, FullOutput, Key, Modifiers, PointerButton, RawInput, Rect, Shape};

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

fn draw(
    ctx: &Context,
    width: f32,
    tool: EditorTool,
    context: ToolHintContext,
    events: Vec<Event>,
) -> (FullOutput, Rect) {
    let mut panel = Rect::NOTHING;
    let output = ctx.run(input(width, events), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let response = editor_tool_hint(ui, tool, context);
            panel = response.rect;
            assert_eq!(response.sense, egui::Sense::hover());
            assert!(!response.has_focus());
        });
    });
    (output, panel)
}

fn text_rect(output: &FullOutput, value: &str) -> Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            Shape::Text(text) if text.galley.text() == value => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing text {value:?}"))
}

fn click_help(
    ctx: &Context,
    width: f32,
    tool: EditorTool,
    context: ToolHintContext,
) -> (FullOutput, Rect) {
    let output = draw(ctx, width, tool, context, vec![]).0;
    let position = text_rect(&output, "Tool help").center();
    let pointer = |pressed| Event::PointerButton {
        pos: position,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    draw(
        ctx,
        width,
        tool,
        context,
        vec![Event::PointerMoved(position), pointer(true)],
    );
    draw(ctx, width, tool, context, vec![pointer(false)]);
    draw(
        ctx,
        width,
        tool,
        context,
        vec![Event::PointerMoved(egui::pos2(2.0, 2.0))],
    )
}

#[test]
fn compact_hints_keep_one_row_and_full_help_wraps_in_both_themes_and_densities() {
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
                for context in [
                    ToolHintContext::Arrangement,
                    ToolHintContext::Midi,
                    ToolHintContext::Audio,
                ] {
                    for tool in EditorTool::ALL {
                        let (output, panel) = draw(&ctx, width, tool, context, vec![]);
                        assert!(
                            panel.height() <= tokens::COMPACT_HEIGHT + tokens::SPACE_XS * 2.0 + 1.0,
                            "{tool:?} {context:?}: {panel:?}"
                        );
                        assert!(panel.right() <= width && panel.left() >= 0.0);
                        assert!(panel.contains_rect(text_rect(&output, "Tool help")));
                        let title = format!("{} · {}", context.label(tool), tool.key().name());
                        let text = output
                            .shapes
                            .iter()
                            .find_map(|shape| match &shape.shape {
                                Shape::Text(text) if text.galley.text().starts_with(&title) => {
                                    Some(text)
                                }
                                _ => None,
                            })
                            .unwrap();
                        assert_eq!(text.galley.rows.len(), 1);
                        assert!(
                            panel.contains_rect(text.galley.rect.translate(text.pos.to_vec2()))
                        );
                    }
                    click_help(&ctx, width, EditorTool::Select, context);
                    for tool in EditorTool::ALL {
                        let (output, panel) = draw(&ctx, width, tool, context, vec![]);
                        assert!(
                            panel.contains_rect(text_rect(&output, context.hint(tool))),
                            "{tool:?} {context:?}: {panel:?}"
                        );
                        assert!(panel.right() <= width && panel.left() >= 0.0);
                    }
                    let (_, collapsed) = click_help(&ctx, width, EditorTool::Select, context);
                    assert!(
                        collapsed.height() <= tokens::COMPACT_HEIGHT + tokens::SPACE_XS * 2.0 + 1.0
                    );
                }
            }
        }
    }
}

#[test]
fn disclosure_is_context_specific_and_retains_help_when_the_tool_changes() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let (expanded, _) = click_help(
        &ctx,
        800.0,
        EditorTool::Select,
        ToolHintContext::Arrangement,
    );
    text_rect(
        &expanded,
        ToolHintContext::Arrangement.hint(EditorTool::Select),
    );
    let (changed, _) = draw(
        &ctx,
        800.0,
        EditorTool::Ghost,
        ToolHintContext::Arrangement,
        vec![],
    );
    text_rect(
        &changed,
        ToolHintContext::Arrangement.hint(EditorTool::Ghost),
    );
    let (_, midi) = draw(
        &ctx,
        800.0,
        EditorTool::Select,
        ToolHintContext::Midi,
        vec![],
    );
    assert!(midi.height() <= tokens::COMPACT_HEIGHT + tokens::SPACE_XS * 2.0 + 1.0);
    let (returned, _) = draw(
        &ctx,
        800.0,
        EditorTool::Select,
        ToolHintContext::Arrangement,
        vec![],
    );
    text_rect(
        &returned,
        ToolHintContext::Arrangement.hint(EditorTool::Select),
    );
}

#[test]
fn tool_help_is_keyboard_accessible_without_stealing_focus_or_consuming_tool_keys() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let key = |key| Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    let mut before_focused = false;
    let mut after_focused = false;
    for (index, events) in [
        vec![],
        vec![],
        vec![key(Key::Tab)],
        vec![key(Key::Tab)],
        vec![key(Key::Q)],
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
                editor_tool_hint(ui, EditorTool::Select, ToolHintContext::Midi);
                let after = ui.button("After");
                before_focused = before.has_focus();
                after_focused = after.has_focus();
                if index == 4 {
                    assert!(ui.input(|input| input.key_pressed(Key::Q)));
                }
            });
        });
        match index {
            1 => assert!(before_focused, "Passive guidance must not take focus"),
            2 => assert!(
                !before_focused && !after_focused,
                "Tool help is a reachable view control"
            ),
            3 | 4 => assert!(after_focused, "Only the disclosure is an added Tab stop"),
            _ => {}
        }
    }
}

#[test]
fn help_stays_reachable_when_a_previous_control_widens_the_parent() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut panel = Rect::NOTHING;
    let output = ctx.run(input(800.0, vec![]), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.set_width(1200.0);
            panel = editor_tool_hint(ui, EditorTool::Cursor, ToolHintContext::Audio).rect;
        });
    });
    assert!(panel.right() <= 800.0);
    assert!(text_rect(&output, "Tool help").right() <= 800.0);
}

#[test]
fn toolbar_help_has_a_full_target_and_native_keyboard_open_and_dismiss() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(&ctx, appearance);
        let draw = |events: Vec<Event>| {
            let mut response = None;
            let output = ctx.run(input(800.0, events), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    response = Some(daw_ui::editor_tool_help(
                        ui,
                        EditorTool::Pencil,
                        ToolHintContext::Midi,
                    ));
                });
            });
            (output, response.unwrap())
        };
        let (closed, response) = draw(vec![]);
        assert!(response.rect.height() >= tokens::CONTROL_HEIGHT);
        assert!(!egui::Popup::is_any_open(&ctx));
        assert!(!closed.shapes.iter().any(|shape| matches!(&shape.shape,
            Shape::Text(text) if text.galley.text() == ToolHintContext::Midi.hint(EditorTool::Pencil)
        )));
        response.request_focus();
        let key = |key, pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        draw(vec![key(Key::Enter, true)]);
        let (opened, _) = draw(vec![key(Key::Enter, false)]);
        assert!(egui::Popup::is_any_open(&ctx));
        text_rect(&opened, "Pencil · W");
        text_rect(&opened, ToolHintContext::Midi.hint(EditorTool::Pencil));
        draw(vec![key(Key::Escape, true)]);
        draw(vec![key(Key::Escape, false)]);
        assert!(!egui::Popup::is_any_open(&ctx));
    }
}

#[test]
fn toolbar_caption_uses_only_remaining_space_without_moving_controls_or_canvas() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for width in [160.0, 240.0, 800.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            let draw = |tool: Option<EditorTool>| {
                let mut controls = Vec::new();
                let mut caption = None;
                let mut toolbar = Rect::NOTHING;
                let mut canvas = Rect::NOTHING;
                let output = ctx.run(input(width, vec![]), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        toolbar = daw_ui::toolbar(ui, |ui| {
                            daw_ui::toolbar_trailing(ui, 120.0, |ui| {
                                controls.push(daw_ui::quiet_button(ui, "Tool help").rect);
                                controls.push(daw_ui::quiet_button(ui, "Fit").rect);
                                if let Some(tool) = tool {
                                    caption = daw_ui::editor_tool_caption(
                                        ui,
                                        tool,
                                        ToolHintContext::Arrangement,
                                    );
                                }
                            });
                        })
                        .response
                        .rect;
                        canvas = ui.label("Canvas").rect;
                    });
                });
                (output, controls, caption, toolbar, canvas)
            };
            draw(None);
            let (_, controls, _, toolbar, canvas) = draw(None);
            for tool in [
                EditorTool::Select,
                EditorTool::Duplicate,
                EditorTool::Cursor,
            ] {
                let (output, actual_controls, caption, actual_toolbar, actual_canvas) =
                    draw(Some(tool));
                assert_eq!(actual_controls, controls);
                assert_eq!(actual_toolbar, toolbar);
                assert_eq!(actual_canvas, canvas);
                if let Some(caption) = caption {
                    assert_eq!(caption.sense, egui::Sense::hover());
                    assert!(!caption.has_focus());
                    assert!(toolbar.contains_rect(caption.rect));
                    for control in &controls {
                        assert!(!caption.rect.intersects(*control));
                    }
                    let painted = text_rect(&output, ToolHintContext::Arrangement.gesture(tool));
                    assert!(toolbar.contains_rect(painted));
                    assert!(painted.height() <= tokens::CONTROL_HEIGHT);
                }
            }
        }
    }
}

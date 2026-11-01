use daw_ui::{EditorTool, ToolHintContext, theme};
use egui::{Context, Event, FullOutput, Modifiers, PointerButton, RawInput, Rect, Response, Shape};

fn frame(
    ctx: &Context,
    tool: &mut EditorTool,
    width: f32,
    enabled: bool,
    events: Vec<Event>,
) -> (FullOutput, Vec<Response>) {
    let mut responses = Vec::new();
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    daw_ui::toolbar(ui, |ui| {
                        responses = daw_ui::editor_tools_with_context(
                            ui,
                            tool,
                            &EditorTool::ALL,
                            ToolHintContext::Arrangement,
                        );
                    });
                });
            });
        },
    );
    (output, responses)
}

fn label(output: &FullOutput, name: &str) -> Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            Shape::Text(text) if text.galley.text() == name => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing {name}"))
}

fn click(ctx: &Context, tool: &mut EditorTool, width: f32, enabled: bool, at: egui::Pos2) {
    for pressed in [true, false] {
        frame(
            ctx,
            tool,
            width,
            enabled,
            vec![
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    }
}

#[test]
fn every_tool_is_visible_and_one_click_away_without_moving_adjacent_controls() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 1.25, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            let mut tool = EditorTool::Select;
            frame(&ctx, &mut tool, 800.0, true, vec![]);
            for width in [800.0, 1100.0] {
                let (output, controls) = frame(&ctx, &mut tool, width, true, vec![]);
                let initial: Vec<_> = controls.iter().map(|response| response.rect).collect();
                assert_eq!(controls.len(), EditorTool::ALL.len());
                let viewport = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.0));
                for (index, response) in controls.iter().enumerate() {
                    assert!(viewport.contains_rect(response.rect));
                    assert!(response.rect.height() >= daw_ui::tokens::CONTROL_HEIGHT);
                    for other in &controls[..index] {
                        assert!(!other.rect.intersects(response.rect));
                    }
                }
                assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                    Shape::Text(text) if text.galley.text() == "More tools"
                )));
                for candidate in EditorTool::ALL {
                    let name = ToolHintContext::Arrangement.label(candidate);
                    let target = label(&output, name);
                    let response = controls
                        .iter()
                        .find(|response| response.rect.contains_rect(target))
                        .unwrap();
                    click(&ctx, &mut tool, width, true, response.rect.center());
                    assert_eq!(tool, candidate);
                    assert!(!egui::Popup::is_any_open(&ctx));
                    let (selected, current) = frame(&ctx, &mut tool, width, true, vec![]);
                    assert_eq!(
                        current
                            .iter()
                            .map(|response| response.rect)
                            .collect::<Vec<_>>(),
                        initial
                    );
                    assert!(
                        selected.shapes.iter().any(|shape| matches!(&shape.shape,
                            Shape::Rect(shape) if shape.rect == response.rect
                                && shape.stroke.width == daw_ui::tokens::FOCUS_WIDTH
                                && shape.stroke.color == theme::palette(&ctx).accent
                        )),
                        "The active {name} mode outlines its complete target"
                    );
                    assert!(selected.shapes.iter().any(|shape| matches!(&shape.shape,
                        Shape::Mesh(mesh) if mesh.vertices.len() == 128
                            && (mesh.calc_bounds().min - response.rect.expand(daw_ui::tokens::SELECTION_GLOW_EXTENT).min).length() < 0.1
                            && (mesh.calc_bounds().max - response.rect.expand(daw_ui::tokens::SELECTION_GLOW_EXTENT).max).length() < 0.1
                    )), "The active {name} border has a bounded soft halo");
                }
            }
        }
    }
}

#[test]
fn separators_only_divide_adjacent_families_on_the_same_row() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut tool = EditorTool::Select;
    for width in [800.0, 1100.0] {
        let (output, controls) = frame(&ctx, &mut tool, width, true, vec![]);
        let separators: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match shape.shape {
                Shape::LineSegment { points, stroke }
                    if points[0].x == points[1].x
                        && (points[1].y - points[0].y - daw_ui::tokens::COMPACT_HEIGHT).abs()
                            < 0.1
                        && stroke.width == daw_ui::tokens::BORDER_WIDTH =>
                {
                    Some(points)
                }
                _ => None,
            })
            .collect();
        assert!(
            !separators.is_empty(),
            "Related tool families have visible separators"
        );
        for [top, bottom] in separators {
            let center = (top.y + bottom.y) * 0.5;
            let on_row = |response: &&Response| (response.rect.center().y - center).abs() < 1.0;
            assert!(
                controls
                    .iter()
                    .filter(on_row)
                    .any(|response| response.rect.right() < top.x)
            );
            assert!(
                controls
                    .iter()
                    .filter(on_row)
                    .any(|response| response.rect.left() > top.x)
            );
            assert!(!controls.iter().any(|response| response.rect.contains(top)));
        }
    }
}

#[test]
fn individual_tools_keep_keyboard_radio_semantics_and_disabled_state() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut tool = EditorTool::Select;
    let (output, controls) = frame(&ctx, &mut tool, 800.0, true, vec![]);
    let stretch = label(&output, "Stretch");
    let target = controls
        .iter()
        .find(|response| response.rect.contains_rect(stretch))
        .unwrap();
    target.request_focus();
    frame(&ctx, &mut tool, 800.0, true, vec![]);
    let (activated, _) = frame(
        &ctx,
        &mut tool,
        800.0,
        true,
        vec![Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(tool, EditorTool::Stretch);
    assert!(!egui::Popup::is_any_open(&ctx));
    let events = activated.platform_output.events;
    assert_eq!(events.len(), 1);
    let info = events[0].widget_info();
    assert_eq!(info.typ, egui::WidgetType::RadioButton);
    assert_eq!(info.label.as_deref(), Some("Stretch"));
    assert!(info.enabled);
    frame(&ctx, &mut tool, 800.0, false, vec![]);
    click(
        &ctx,
        &mut tool,
        800.0,
        false,
        label(&output, "Erase").center(),
    );
    assert_eq!(tool, EditorTool::Stretch);
    assert!(!egui::Popup::is_any_open(&ctx));
}

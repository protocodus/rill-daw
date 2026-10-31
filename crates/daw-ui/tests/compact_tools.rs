use daw_ui::{EditorTool, ToolHintContext, theme};
use egui::{Context, Event, FullOutput, Modifiers, PointerButton, RawInput, Rect, Response, Shape};

const PRIMARY: &[EditorTool] = &[
    EditorTool::Select,
    EditorTool::Pencil,
    EditorTool::Cursor,
    EditorTool::Cut,
];
const SUPPORTED: &[EditorTool] = &[
    EditorTool::Select,
    EditorTool::Pencil,
    EditorTool::Cursor,
    EditorTool::Cut,
    EditorTool::Duplicate,
    EditorTool::Glue,
    EditorTool::Ghost,
    EditorTool::Stretch,
    EditorTool::Slip,
    EditorTool::Erase,
];

fn frame(ctx: &Context, tool: &mut EditorTool, events: Vec<Event>) -> (FullOutput, Response) {
    let mut response = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                daw_ui::toolbar(ui, |ui| {
                    response = daw_ui::compact_editor_tools(
                        ui,
                        tool,
                        SUPPORTED,
                        PRIMARY,
                        ToolHintContext::Arrangement,
                    );
                });
            });
        },
    );
    (output, response.unwrap())
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

fn click(ctx: &Context, tool: &mut EditorTool, at: egui::Pos2) {
    for pressed in [true, false] {
        frame(
            ctx,
            tool,
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
    frame(ctx, tool, vec![]);
}

#[test]
fn secondary_choice_is_mouse_accessible_and_keeps_its_width_and_active_indicator() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(&ctx, appearance);
        let mut tool = EditorTool::Select;
        let (_, initial) = frame(&ctx, &mut tool, vec![]);
        let width = initial.rect.width();
        for candidate in SUPPORTED.iter().filter(|tool| !PRIMARY.contains(tool)) {
            let (_, chooser) = frame(&ctx, &mut tool, vec![]);
            click(&ctx, &mut tool, chooser.rect.center());
            assert!(egui::Popup::is_any_open(&ctx));
            let (menu, _) = frame(&ctx, &mut tool, vec![]);
            click(
                &ctx,
                &mut tool,
                label(&menu, ToolHintContext::Arrangement.label(*candidate)).center(),
            );
            assert_eq!(tool, *candidate);
            assert!(!egui::Popup::is_any_open(&ctx));
            let (selected, chooser) = frame(&ctx, &mut tool, vec![]);
            assert!((chooser.rect.width() - width).abs() < 0.1);
            assert!(
                chooser
                    .rect
                    .contains_rect(label(&selected, ToolHintContext::Arrangement.label(tool)))
            );
            assert!(selected.shapes.iter().any(|shape| matches!(&shape.shape,
                Shape::Rect(shape) if shape.rect.height() == 3.0 && chooser.rect.contains_rect(shape.rect)
            )), "Selected secondary mode keeps the tool underline");
        }
        let (selected, _) = frame(&ctx, &mut tool, vec![]);
        click(&ctx, &mut tool, label(&selected, "Select").center());
        assert_eq!(tool, EditorTool::Select);
        let (reset, chooser) = frame(&ctx, &mut tool, vec![]);
        assert!(chooser.rect.contains_rect(label(&reset, "More tools")));
        assert!((chooser.rect.width() - width).abs() < 0.1);
        click(&ctx, &mut tool, chooser.rect.center());
        click(&ctx, &mut tool, egui::pos2(760.0, 550.0));
        assert_eq!(
            tool,
            EditorTool::Select,
            "Dismissing choices never changes mode"
        );
        assert!(!egui::Popup::is_any_open(&ctx));
    }
}

#[test]
fn chooser_supports_keyboard_open_and_escape_without_selecting_a_tool() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut tool = EditorTool::Stretch;
    let (_, chooser) = frame(&ctx, &mut tool, vec![]);
    chooser.request_focus();
    for pressed in [true, false] {
        frame(
            &ctx,
            &mut tool,
            vec![Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
    }
    assert!(egui::Popup::is_any_open(&ctx));
    assert_eq!(tool, EditorTool::Stretch);
    for pressed in [true, false] {
        frame(
            &ctx,
            &mut tool,
            vec![Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
    }
    assert!(!egui::Popup::is_any_open(&ctx));
    assert_eq!(tool, EditorTool::Stretch);
}

#[test]
fn chooser_emits_one_labelled_button_event_instead_of_a_mode_selection() {
    for (initial, name) in [
        (EditorTool::Select, "More tools"),
        (EditorTool::Stretch, "Stretch"),
    ] {
        let ctx = Context::default();
        theme::apply(&ctx);
        let mut tool = initial;
        frame(&ctx, &mut tool, vec![]);
        let mut focused = None;
        for _ in 0..5 {
            for pressed in [true, false] {
                let output = frame(
                    &ctx,
                    &mut tool,
                    vec![Event::Key {
                        key: egui::Key::Tab,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    }],
                )
                .0;
                if pressed {
                    focused = Some(output);
                }
            }
        }
        let activated = frame(
            &ctx,
            &mut tool,
            vec![Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        )
        .0;
        for output in [focused.unwrap(), activated] {
            let events = output.platform_output.events;
            assert_eq!(
                events.len(),
                1,
                "Chooser must expose exactly one focus or click event"
            );
            let info = events[0].widget_info();
            assert_eq!(info.typ, egui::WidgetType::Button);
            assert_eq!(info.label.as_deref(), Some(name));
            assert_eq!(info.selected, None, "Opening a menu does not select a tool");
        }
        assert_eq!(tool, initial);
        assert!(egui::Popup::is_any_open(&ctx));
    }
}

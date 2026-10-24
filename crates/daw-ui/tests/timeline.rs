use daw_ui::{TimelineBounds, TimelineNavigation, TimelinePane, TimelineViewport};
use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect};

const BOUNDS: TimelineBounds = TimelineBounds {
    start_beat: 32.0,
    end_beat: 64.0,
    start_row: 10.0,
    end_row: 30.0,
};

fn frame(
    ctx: &Context,
    view: &mut TimelineViewport,
    events: Vec<Event>,
    modifiers: Modifiers,
    text_focus: bool,
) -> TimelineNavigation {
    let mut navigation = TimelineNavigation::default();
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            modifiers,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if text_focus {
                    let mut text = String::new();
                    daw_ui::text_field(ui, &mut text, "Name", 180.0).request_focus();
                }
                navigation = TimelinePane::new(
                    "test",
                    Rect::from_min_max(egui::pos2(100.0, 100.0), egui::pos2(700.0, 500.0)),
                )
                .navigate(ui, view, BOUNDS);
            });
        },
    );
    navigation
}

#[test]
fn pan_press_has_no_jump_and_motion_is_incremental_until_release() {
    let ctx = Context::default();
    let mut view = TimelineViewport {
        start_beat: 16.0,
        row_offset: 10.0,
        ..Default::default()
    };
    frame(&ctx, &mut view, vec![], Modifiers::NONE, false);
    let before = view;
    let start = egui::pos2(400.0, 300.0);
    let down = frame(
        &ctx,
        &mut view,
        vec![
            Event::PointerMoved(start),
            Event::PointerButton {
                pos: start,
                button: PointerButton::Middle,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
        Modifiers::NONE,
        false,
    );
    assert!(down.navigating);
    assert_eq!(
        view, before,
        "Initial press must not pan by distance from previous pointer"
    );
    let moved = start - egui::vec2(48.0, 24.0);
    frame(
        &ctx,
        &mut view,
        vec![Event::PointerMoved(moved)],
        Modifiers::NONE,
        false,
    );
    assert!((view.start_beat - 17.0).abs() < 1e-6);
    assert!((view.row_offset - 11.0).abs() < 1e-6);
    frame(
        &ctx,
        &mut view,
        vec![Event::PointerButton {
            pos: moved,
            button: PointerButton::Middle,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
        false,
    );
    let settled = view;
    let result = frame(
        &ctx,
        &mut view,
        vec![Event::PointerMoved(start)],
        Modifiers::NONE,
        false,
    );
    assert!(!result.navigating);
    assert_eq!(view, settled);
}

#[test]
fn text_field_focus_prevents_fit_and_zoom_keyboard_shortcuts() {
    let ctx = Context::default();
    let mut view = TimelineViewport::default();
    frame(
        &ctx,
        &mut view,
        vec![Event::PointerMoved(egui::pos2(400.0, 300.0))],
        Modifiers::NONE,
        true,
    );
    let before = view;
    for key in [egui::Key::F, egui::Key::Plus, egui::Key::Minus] {
        let result = frame(
            &ctx,
            &mut view,
            vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            Modifiers::NONE,
            true,
        );
        assert!(!result.changed);
        assert_eq!(view, before);
    }
}

#[test]
fn disabled_background_pane_does_not_consume_active_editor_shortcuts() {
    let ctx = Context::default();
    let mut view = TimelineViewport::default();
    let before = view;
    let mut tool = daw_ui::EditorTool::Select;
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events: vec![
                Event::PointerMoved(egui::pos2(400.0, 300.0)),
                Event::Key {
                    key: egui::Key::W,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                },
                Event::Key {
                    key: egui::Key::F,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_enabled_ui(false, |ui| {
                    daw_ui::editor_tool_shortcuts(ui, &mut tool, &daw_ui::EditorTool::ALL);
                    TimelinePane::new("disabled", ui.max_rect()).navigate(ui, &mut view, BOUNDS);
                });
                assert!(ui.input(|input| input.key_pressed(egui::Key::W)));
                assert!(ui.input(|input| input.key_pressed(egui::Key::F)));
            });
        },
    );
    assert_eq!(view, before);
    assert_eq!(tool, daw_ui::EditorTool::Select);
}

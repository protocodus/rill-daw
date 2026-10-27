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

#[test]
fn foreground_keyboard_routing_is_explicit_and_respects_input_guards() {
    for (name, active, enabled, text_focus, popup) in [
        ("default", false, true, false, false),
        ("foreground", true, true, false, false),
        ("disabled", true, false, false, false),
        ("text entry", true, true, true, false),
        ("open menu", true, true, false, true),
    ] {
        for key in [egui::Key::F, egui::Key::Plus, egui::Key::Minus] {
            let ctx = Context::default();
            let mut view = TimelineViewport::default();
            let before = view;
            let accepts = active && enabled && !text_focus && !popup;
            let mut result = TimelineNavigation::default();
            let mut retained = false;
            for pressed in [false, true] {
                let mut events = vec![Event::PointerMoved(egui::pos2(20.0, 20.0))];
                if pressed {
                    events.push(Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    });
                }
                let _ = ctx.run(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(800.0, 600.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            if text_focus {
                                let mut text = String::new();
                                daw_ui::text_field(ui, &mut text, "Name", 180.0).request_focus();
                            }
                            if popup {
                                egui::Popup::open_id(ctx, egui::Id::new("menu"));
                            }
                            ui.add_enabled_ui(enabled, |ui| {
                                let pane = TimelinePane::new(
                                    "foreground",
                                    Rect::from_min_max(
                                        egui::pos2(100.0, 100.0),
                                        egui::pos2(700.0, 500.0),
                                    ),
                                );
                                result = if active { pane.keyboard_active() } else { pane }
                                    .navigate(ui, &mut view, BOUNDS);
                                retained = ui.input(|input| input.key_pressed(key));
                            });
                        });
                    },
                );
            }
            assert_eq!(result.changed, accepts, "{name}, {key:?}");
            assert_eq!(retained, !accepts, "{name}, {key:?}");
            if !accepts {
                assert_eq!(view, before, "{name}, {key:?}");
            }
        }
    }
}

#[test]
fn horizontal_canvas_leaves_vertical_wheel_to_its_scrollable_parent() {
    let ctx = Context::default();
    let mut view = TimelineViewport {
        start_beat: 16.0,
        row_offset: 10.0,
        ..Default::default()
    };
    let mut step = |delta: egui::Vec2, modifiers: Modifiers| {
        let mut output = None;
        let _ = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
                events: vec![
                    Event::PointerMoved(egui::pos2(400.0, 300.0)),
                    Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta,
                        modifiers,
                    },
                ],
                modifiers,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let parent = egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_min_height(1200.0);
                            let navigation = TimelinePane::new(
                                "horizontal_child",
                                Rect::from_min_max(
                                    egui::pos2(100.0, 100.0),
                                    egui::pos2(700.0, 500.0),
                                ),
                            )
                            .horizontal_only()
                            .navigate(ui, &mut view, BOUNDS);
                            (navigation, ui.input(|input| input.raw_scroll_delta))
                        });
                    output = Some((view, parent.inner.0, parent.inner.1, parent.state.offset.y));
                });
            },
        );
        output.unwrap()
    };
    let before = step(egui::Vec2::ZERO, Modifiers::NONE).0;
    let (scrolled, navigation, remaining, parent_offset) =
        step(egui::vec2(-48.0, -90.0), Modifiers::NONE);
    assert!(navigation.changed);
    assert_eq!(scrolled.start_beat, before.start_beat + 1.0);
    assert_eq!(
        (scrolled.row_offset, scrolled.row_height),
        (before.row_offset, before.row_height)
    );
    assert_eq!(remaining, egui::vec2(0.0, -90.0));
    assert!(
        parent_offset > 0.0,
        "The parent must receive vertical scrolling over a horizontal canvas"
    );
    let (shifted, _, remaining, _) = step(egui::vec2(0.0, -48.0), Modifiers::SHIFT);
    assert_eq!(shifted.start_beat, scrolled.start_beat + 1.0);
    assert_eq!(
        remaining,
        egui::Vec2::ZERO,
        "Shift-wheel belongs to horizontal navigation"
    );
    let (zoomed, _, remaining, _) = step(egui::vec2(0.0, 90.0), Modifiers::CTRL);
    assert!(zoomed.pixels_per_beat > shifted.pixels_per_beat);
    assert_eq!(
        remaining,
        egui::Vec2::ZERO,
        "Modifier zoom must not also scroll the parent"
    );
}

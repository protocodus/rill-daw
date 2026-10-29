use daw_ui::{CurvePoint, CurveResponse, CurveSegment, CurveTimeline, TimelineViewport};
use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect};

struct Editor {
    ctx: Context,
    viewport: TimelineViewport,
    selected: Option<u64>,
    points: [CurvePoint; 2],
    curve: f32,
}

impl Editor {
    fn new(values: [f32; 2]) -> Self {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        Self {
            ctx,
            viewport: TimelineViewport::default(),
            selected: None,
            points: [
                CurvePoint {
                    id: 1,
                    x: 2.0,
                    value: values[0],
                },
                CurvePoint {
                    id: 2,
                    x: 6.0,
                    value: values[1],
                },
            ],
            curve: 0.0,
        }
    }

    fn frame(&mut self, events: Vec<Event>, modifiers: Modifiers, fit: bool) -> CurveResponse {
        let mut response = None;
        let points = self.points;
        let curve = self.curve;
        let _ = self.ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    response = Some(daw_ui::curve_timeline_editor_with_segments(
                        ui,
                        egui::Id::new("segment-test"),
                        &points,
                        &[CurveSegment { start: 1, curve }],
                        None,
                        &mut self.selected,
                        &mut self.viewport,
                        CurveTimeline {
                            snap: 0.25,
                            height: 400.0,
                            tempo: 120.0,
                            beats_per_bar: 4,
                            fit,
                            keyboard_active: false,
                        },
                        |beat| {
                            let t = ((beat - 2.0) / 4.0).clamp(0.0, 1.0) as f32;
                            let bent = if curve < 0.0 {
                                1.0 - (1.0 - t).powi(3)
                            } else {
                                t.powi(3)
                            };
                            points[0].value
                                + (points[1].value - points[0].value)
                                    * (t + (bent - t) * curve.abs())
                        },
                    ));
                });
            },
        );
        response.unwrap()
    }
}

fn pointer(pos: Pos2, button: PointerButton, pressed: bool, modifiers: Modifiers) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers,
        },
    ]
}

#[test]
fn spline_handle_selects_one_segment_and_drag_uses_total_distance_without_moving_points() {
    for values in [[0.2, 0.8], [0.8, 0.2], [0.5, 0.5]] {
        let mut editor = Editor::new(values);
        let before = editor.points;
        let initial = editor.frame(vec![], Modifiers::NONE, true);
        assert_eq!(initial.segment_handles.len(), 1);
        let origin = initial.segment_handles[0].rect.center();
        editor.frame(
            pointer(origin, PointerButton::Primary, true, Modifiers::NONE),
            Modifiers::NONE,
            false,
        );
        let first = editor.frame(
            vec![Event::PointerMoved(origin + egui::vec2(0.0, 12.0))],
            Modifiers::NONE,
            false,
        );
        assert_eq!(first.edit, None);
        let (point, value) = first.segment_edit.expect("diamond bends the segment");
        assert_eq!(point, 1);
        assert_eq!(editor.selected, Some(1));
        assert!(value.abs() > 0.0);
        if values[1] != values[0] {
            assert_eq!(value.signum(), (values[1] - values[0]).signum());
        }
        editor.curve = value;
        let second = editor.frame(
            vec![Event::PointerMoved(origin + egui::vec2(0.0, 24.0))],
            Modifiers::NONE,
            false,
        );
        assert_eq!(second.edit, None);
        let (_, second_value) = second.segment_edit.unwrap();
        assert!(
            (second_value - 2.0 * value).abs() < 0.001,
            "drag must not accumulate prior frames"
        );
        let released = editor.frame(
            pointer(
                origin + egui::vec2(0.0, 24.0),
                PointerButton::Primary,
                false,
                Modifiers::NONE,
            ),
            Modifiers::NONE,
            false,
        );
        assert_eq!(
            released.edit, None,
            "release must not add a point beneath the handle"
        );
        assert_eq!(editor.points, before);
    }
}

#[test]
fn navigation_gestures_starting_on_spline_handles_never_change_the_curve() {
    for (button, modifiers) in [
        (PointerButton::Middle, Modifiers::NONE),
        (
            PointerButton::Primary,
            Modifiers {
                alt: true,
                ..Modifiers::NONE
            },
        ),
        (
            PointerButton::Secondary,
            Modifiers {
                ctrl: true,
                ..Modifiers::NONE
            },
        ),
    ] {
        let mut editor = Editor::new([0.2, 0.8]);
        let first = editor.frame(vec![], Modifiers::NONE, true);
        let origin = first.segment_handles[0].rect.center();
        editor.frame(pointer(origin, button, true, modifiers), modifiers, false);
        let moved = editor.frame(
            vec![Event::PointerMoved(origin + egui::vec2(80.0, -30.0))],
            modifiers,
            false,
        );
        assert!(moved.navigation.navigating);
        assert!(moved.segment_edit.is_none());
        assert!(moved.edit.is_none());
        assert!(editor.selected.is_none());
    }
}

//! Small automation lanes must retain precise and coalesced native gestures.
use daw_ui::{AutomationCanvas, CurveEdit, CurvePoint, TimelineViewport};
use egui::{Context, Event, PointerButton, Pos2, RawInput, Rect};

struct InlineCurve {
    ctx: Context,
    point: CurvePoint,
    time: f64,
}

impl InlineCurve {
    fn new() -> Self {
        let mut curve = Self {
            ctx: Context::default(),
            point: CurvePoint {
                id: 1,
                x: 2.1,
                value: 0.5,
            },
            time: 0.0,
        };
        for _ in 0..3 {
            curve.frame(Vec::new());
        }
        curve
    }

    fn position(&self) -> Pos2 {
        Pos2::new(
            40.0 + self.point.x as f32 * 50.0,
            64.0 - self.point.value * 24.0,
        )
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<CurveEdit> {
        self.time += 1.0 / 60.0;
        let mut edit = None;
        let _ = self.ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 120.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    edit = daw_ui::automation_canvas_at(
                        ui,
                        egui::Id::new("precise-inline"),
                        &[self.point],
                        AutomationCanvas {
                            rect: Rect::from_min_size(
                                Pos2::new(40.0, 40.0),
                                egui::vec2(200.0, 24.0),
                            ),
                            viewport: TimelineViewport {
                                pixels_per_beat: 50.0,
                                ..Default::default()
                            },
                            snap: 0.25,
                            color: egui::Color32::WHITE,
                            editable: true,
                        },
                        |_| self.point.value,
                    )
                    .edit;
                });
            },
        );
        if let Some(CurveEdit::Move { x, value, .. }) = edit {
            self.point.x = x;
            self.point.value = value;
        }
        edit
    }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}

#[test]
fn sub_threshold_motion_and_return_restore_the_exact_off_grid_origin() {
    let mut curve = InlineCurve::new();
    let original = curve.point;
    let at = curve.position();
    assert!(
        curve
            .frame(vec![Event::PointerMoved(at), button(at, true)])
            .is_none()
    );
    let moved = at + egui::vec2(0.0, -3.0);
    assert!(matches!(
        curve.frame(vec![Event::PointerMoved(moved)]),
        Some(CurveEdit::Move { .. })
    ));
    assert_eq!(curve.point.value, 0.625);
    curve.frame(vec![Event::PointerMoved(at)]);
    assert_eq!(curve.point.x, original.x);
    assert_eq!(curve.point.value, original.value);
    assert!(curve.frame(vec![button(at, false)]).is_none());
}

#[test]
fn release_frame_includes_the_final_movement() {
    let mut curve = InlineCurve::new();
    let at = curve.position();
    curve.frame(vec![Event::PointerMoved(at), button(at, true)]);
    curve.frame(vec![Event::PointerMoved(at + egui::vec2(0.0, 2.0))]);
    let released = at + egui::vec2(0.0, 4.0);
    curve.frame(vec![Event::PointerMoved(released), button(released, false)]);
    assert!((curve.point.value - (0.5 - 4.0 / 24.0)).abs() < 1e-6);
}

#[test]
fn coalesced_press_move_release_keeps_the_press_position() {
    let mut curve = InlineCurve::new();
    let at = curve.position();
    let end = at + egui::vec2(0.0, -3.0);
    curve.frame(vec![
        Event::PointerMoved(at),
        button(at, true),
        Event::PointerMoved(end),
        button(end, false),
    ]);
    assert_eq!(curve.point.value, 0.625);
}

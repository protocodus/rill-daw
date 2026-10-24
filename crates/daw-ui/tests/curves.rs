use daw_ui::{CurveEdit, CurvePoint, CurveResponse, CurveView};
use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput};

fn frame(
    ctx: &Context,
    events: Vec<Event>,
    points: &[CurvePoint],
    selected: &mut Option<u64>,
) -> CurveResponse {
    let mut result = None;
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = Some(daw_ui::curve_editor(
                    ui,
                    egui::Id::new("test_curve"),
                    points,
                    selected,
                    CurveView {
                        start: 0.0,
                        end: 8.0,
                        snap: 0.25,
                        height: 300.0,
                    },
                    |_| 0.5,
                ));
            });
        },
    );
    result.unwrap()
}

fn pointer(pos: Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

#[test]
fn click_adds_snapped_normalized_point() {
    let ctx = Context::default();
    let mut selected = None;
    let plot = frame(&ctx, vec![], &[], &mut selected).plot_rect;
    let at = egui::pos2(
        plot.left() + plot.width() * 0.26,
        plot.top() + plot.height() * 0.3,
    );
    frame(&ctx, pointer(at, true), &[], &mut selected);
    let response = frame(&ctx, pointer(at, false), &[], &mut selected);
    assert!(response.response.changed());
    let Some(CurveEdit::Add { x, value }) = response.edit else {
        panic!("missing added point")
    };
    assert_eq!(x, 2.0);
    assert!((value - 0.7).abs() < 1e-5);
}

#[test]
fn dragging_uses_original_point_and_total_pointer_distance_then_delete_uses_focus() {
    let ctx = Context::default();
    let mut selected = None;
    let mut points = [CurvePoint {
        id: 7,
        x: 2.0,
        value: 0.5,
    }];
    let plot = frame(&ctx, vec![], &points, &mut selected).plot_rect;
    let start = egui::pos2(plot.left() + plot.width() * 0.25, plot.center().y);
    frame(&ctx, pointer(start, true), &points, &mut selected);
    let half = start + egui::vec2(plot.width() * 0.125, -plot.height() * 0.1);
    let first = frame(
        &ctx,
        vec![Event::PointerMoved(half)],
        &points,
        &mut selected,
    );
    let Some(CurveEdit::Move { x, value, .. }) = first.edit else {
        panic!("missing first drag")
    };
    points[0].x = x;
    points[0].value = value;
    let finish = start + egui::vec2(plot.width() * 0.25, -plot.height() * 0.2);
    let second = frame(
        &ctx,
        vec![Event::PointerMoved(finish)],
        &points,
        &mut selected,
    );
    let Some(CurveEdit::Move { x, value, .. }) = second.edit else {
        panic!("missing second drag")
    };
    assert_eq!(x, 4.0, "each drag frame must use the original point");
    assert!((value - 0.7).abs() < 1e-5);
    points[0].x = x;
    points[0].value = value;
    frame(&ctx, pointer(finish, false), &points, &mut selected);
    let deleted = frame(
        &ctx,
        vec![Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        &points,
        &mut selected,
    );
    assert_eq!(deleted.edit, Some(CurveEdit::Delete { id: 7 }));
}

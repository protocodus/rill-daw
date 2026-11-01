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
fn short_timeline_preserves_a_usable_plot_and_room_for_inspector_actions() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut selected = None;
    let mut viewport = daw_ui::TimelineViewport::default();
    let bounds = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(640.0, 180.0));
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(bounds),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = daw_ui::curve_timeline_editor(
                    ui,
                    egui::Id::new("short_curve"),
                    &[],
                    &mut selected,
                    &mut viewport,
                    daw_ui::CurveTimeline {
                        snap: 0.25,
                        height: 96.0,
                        tempo: 120.0,
                        beats_per_bar: 4,
                        fit: true,
                        keyboard_active: false,
                    },
                    |_| 0.5,
                );
                assert_eq!(response.response.rect.height(), 96.0);
                assert!(response.plot_rect.height() >= daw_ui::tokens::SPACE_LG);
                assert!(response.response.rect.contains_rect(response.plot_rect));
                let action = daw_ui::button(ui, "Edit selected point");
                assert!(
                    bounds.contains_rect(action.rect),
                    "Inspector was displaced: {:?}",
                    action.rect
                );
                assert!(
                    action.rect.top() - response.response.rect.bottom() >= daw_ui::tokens::SPACE_MD
                );
            });
        },
    );
}

#[test]
fn percent_ticks_follow_available_height_without_overlapping_at_display_scales() {
    for density in [1.0, 1.25, 2.0] {
        for (height, expected_ticks) in [(96.0, 2), (120.0, 3), (210.0, 5)] {
            let ctx = Context::default();
            daw_ui::theme::apply(&ctx);
            ctx.set_pixels_per_point(density);
            let screen = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(640.0, 480.0));
            let _ = ctx.run(
                RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |_| {},
            );
            let mut viewport = daw_ui::TimelineViewport::default();
            let mut selected = None;
            let output = ctx.run(
                RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let curve = daw_ui::curve_timeline_editor(
                            ui,
                            egui::Id::new("tick_spacing"),
                            &[],
                            &mut selected,
                            &mut viewport,
                            daw_ui::CurveTimeline {
                                snap: 0.25,
                                height,
                                tempo: 120.0,
                                beats_per_bar: 4,
                                fit: true,
                                keyboard_active: false,
                            },
                            |_| 0.5,
                        );
                        assert_eq!(curve.response.rect.height(), height);
                    });
                },
            );
            let mut labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && ["0%", "25%", "50%", "75%", "100%"].contains(&text.galley.text())
                    {
                        Some((
                            text.galley.text(),
                            text.galley.rect.translate(text.pos.to_vec2()),
                        ))
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(labels.len(), expected_ticks);
            labels.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
            assert_eq!(labels.first().unwrap().0, "100%");
            assert_eq!(labels.last().unwrap().0, "0%");
            for pair in labels.windows(2) {
                assert!(
                    pair[1].1.top() - pair[0].1.bottom()
                        >= daw_ui::tokens::SPACE_XS - 1.0 / density,
                    "Percent labels crowded at {height}pt and {density}×: {pair:?}"
                );
            }
        }
    }
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

struct TimelineCurve {
    ctx: Context,
    view: daw_ui::TimelineViewport,
    selected: Option<u64>,
    points: Vec<CurvePoint>,
}

impl TimelineCurve {
    fn new() -> Self {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        Self {
            ctx,
            view: daw_ui::TimelineViewport::default(),
            selected: None,
            points: vec![
                CurvePoint {
                    id: 1,
                    x: 32.0,
                    value: 0.25,
                },
                CurvePoint {
                    id: 2,
                    x: 40.0,
                    value: 0.5,
                },
                CurvePoint {
                    id: 3,
                    x: 64.0,
                    value: 0.75,
                },
            ],
        }
    }

    fn frame(&mut self, events: Vec<Event>, modifiers: Modifiers, fit: bool) -> CurveResponse {
        let mut result = None;
        let _ = self.ctx.run(
            RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    result = Some(daw_ui::curve_timeline_editor(
                        ui,
                        egui::Id::new("timeline_curve"),
                        &self.points,
                        &mut self.selected,
                        &mut self.view,
                        daw_ui::CurveTimeline {
                            snap: 0.25,
                            height: 400.0,
                            tempo: 120.0,
                            beats_per_bar: 4,
                            fit,
                            keyboard_active: false,
                        },
                        |_| 0.5,
                    ));
                });
            },
        );
        result.unwrap()
    }

    fn at(&self, plot: egui::Rect, beat: f64, value: f32) -> Pos2 {
        egui::pos2(
            self.view.x_at(plot, beat),
            plot.bottom() - value * plot.height(),
        )
    }
}

fn gesture_events(
    pos: Pos2,
    button: PointerButton,
    pressed: bool,
    modifiers: Modifiers,
) -> Vec<Event> {
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
fn timeline_pan_and_zoom_starting_on_points_never_edit_or_open_context_menus() {
    for (button, modifiers, zoom) in [
        (PointerButton::Middle, Modifiers::NONE, false),
        (
            PointerButton::Primary,
            Modifiers {
                alt: true,
                ..Modifiers::NONE
            },
            false,
        ),
        (
            PointerButton::Secondary,
            Modifiers {
                ctrl: true,
                ..Modifiers::NONE
            },
            true,
        ),
    ] {
        let mut curve = TimelineCurve::new();
        let plot = curve.frame(vec![], Modifiers::NONE, true).plot_rect;
        let start = curve.at(plot, 40.0, 0.5);
        let before = curve.view;
        let down = curve.frame(
            gesture_events(start, button, true, modifiers),
            modifiers,
            false,
        );
        assert!(down.navigation.navigating);
        assert!(down.edit.is_none());
        let finish = start + egui::vec2(if zoom { 70.0 } else { -70.0 }, 30.0);
        let moving = curve.frame(vec![Event::PointerMoved(finish)], modifiers, false);
        assert!(moving.navigation.navigating);
        assert!(moving.edit.is_none());
        assert_ne!(curve.view.start_beat, before.start_beat);
        assert_eq!(curve.view.row_offset, 0.0);
        assert_eq!(curve.view.row_height, plot.height());
        let released = curve.frame(
            gesture_events(finish, button, false, modifiers),
            modifiers,
            false,
        );
        assert!(released.edit.is_none());
        assert!(curve.selected.is_none());
        assert!(!egui::Popup::is_any_open(&curve.ctx));
        let settled = curve.frame(vec![], Modifiers::NONE, false);
        assert!(!settled.navigation.navigating);
        assert!(settled.edit.is_none());
    }
}

#[test]
fn singleton_timeline_fit_leaves_one_bar_to_draw_with_the_full_parameter_range() {
    let mut curve = TimelineCurve::new();
    curve.points = vec![CurvePoint {
        id: 1,
        x: 96.0,
        value: 0.4,
    }];
    let fitted = curve.frame(vec![], Modifiers::NONE, true);
    let plot = fitted.plot_rect;
    let visible_end = curve.view.beat_at(plot, plot.right());
    assert!(curve.view.start_beat <= 96.0);
    assert!(curve.view.start_beat >= 95.0);
    assert!(visible_end >= 100.0);
    assert!(visible_end - curve.view.start_beat < 5.0);
    assert_eq!(curve.view.row_offset, 0.0);
    assert_eq!(curve.view.row_height, plot.height());
    assert!(plot.contains(curve.at(plot, 96.0, 0.0)));
    assert!(plot.contains(curve.at(plot, 96.0, 1.0)));
    assert!(fitted.edit.is_none());
}

#[test]
fn timeline_wheel_preserves_anchor_full_value_range_and_focused_fit_then_point_editing() {
    let mut curve = TimelineCurve::new();
    let plot = curve.frame(vec![], Modifiers::NONE, true).plot_rect;
    let anchor = plot.center();
    let before = curve.view;
    let modifiers = Modifiers {
        ctrl: true,
        ..Modifiers::NONE
    };
    let wheel = |modifiers| {
        vec![
            Event::PointerMoved(anchor),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 120.0),
                modifiers,
            },
        ]
    };
    let zoomed = curve.frame(wheel(modifiers), modifiers, false);
    assert!(zoomed.edit.is_none());
    assert!(curve.view.pixels_per_beat > before.pixels_per_beat);
    assert!((curve.view.beat_at(plot, anchor.x) - before.beat_at(plot, anchor.x)).abs() < 1e-5);
    let after_zoom = curve.view;
    let modifiers = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::NONE
    };
    curve.frame(wheel(modifiers), modifiers, false);
    assert_eq!(
        curve.view, after_zoom,
        "Vertical zoom must not hide parameter extrema"
    );

    let fit = curve.frame(vec![], Modifiers::NONE, true);
    for point in &curve.points {
        assert!(fit.plot_rect.contains(curve.at(plot, point.x, point.value)));
    }
    let point = curve.at(plot, 40.0, 0.5);
    curve.frame(pointer(point, true), Modifiers::NONE, false);
    let clicked = curve.frame(pointer(point, false), Modifiers::NONE, false);
    assert!(clicked.edit.is_none());
    assert_eq!(curve.selected, Some(2));
    assert!(clicked.response.has_focus());
    let key = |key| {
        vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]
    };
    let prior = curve.view;
    curve.frame(key(egui::Key::Equals), Modifiers::NONE, false);
    assert!(curve.view.pixels_per_beat > prior.pixels_per_beat);
    let fitted = curve.frame(key(egui::Key::F), Modifiers::NONE, false);
    assert!(fitted.navigation.fit);
    assert!((curve.view.pixels_per_beat - prior.pixels_per_beat).abs() < 1e-5);
    let from = curve.at(plot, 40.0, 0.5);
    let to = curve.at(plot, 44.0, 0.8);
    curve.frame(pointer(from, true), Modifiers::NONE, false);
    let dragged = curve.frame(vec![Event::PointerMoved(to)], Modifiers::NONE, false);
    let Some(CurveEdit::Move { id, x, value }) = dragged.edit else {
        panic!("point drag lost after navigation");
    };
    assert_eq!(id, 2);
    assert_eq!(x, 44.0);
    assert!((value - 0.8).abs() < 1e-5);
    curve.points[1] = CurvePoint { id, x, value };
    curve.frame(pointer(to, false), Modifiers::NONE, false);
    assert_eq!(
        curve
            .frame(key(egui::Key::Delete), Modifiers::NONE, false)
            .edit,
        Some(CurveEdit::Delete { id: 2 })
    );
}

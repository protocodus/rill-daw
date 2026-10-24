use daw_ui::{CrossfadeCurve, CrossfadeResponse, theme};
use egui::{Context, Event, FullOutput, Modifiers, PointerButton, Pos2, RawInput};

fn frame(
    ctx: &Context,
    events: Vec<Event>,
    curve: &mut CrossfadeCurve,
    enabled: bool,
) -> (CrossfadeResponse, FullOutput) {
    let mut response = None;
    let modifiers = events
        .iter()
        .find_map(|event| match event {
            Event::Key { modifiers, .. } | Event::PointerButton { modifiers, .. } => {
                Some(*modifiers)
            }
            _ => None,
        })
        .unwrap_or_default();
    let output = ctx.run(
        RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
            events,
            modifiers,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                response = Some(
                    ui.add_enabled_ui(enabled, |ui| {
                        daw_ui::crossfade_editor(ui, egui::Id::new("crossfade"), curve)
                    })
                    .inner,
                );
            });
        },
    );
    (response.unwrap(), output)
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
fn key(key: egui::Key, modifiers: Modifiers) -> Vec<Event> {
    vec![Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }]
}

#[test]
fn handle_drag_uses_total_displacement_and_keeps_normalized_ordered_time() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut curve = CrossfadeCurve::default();
    let original = curve;
    let (initial, _) = frame(&ctx, vec![], &mut curve, true);
    let start = initial.handles[0].center();
    let plot = initial.plot_rect;
    frame(&ctx, pointer(start, true), &mut curve, true);
    let half = start + egui::vec2(plot.width() * 0.05, -plot.height() * 0.1);
    let (first, _) = frame(&ctx, vec![Event::PointerMoved(half)], &mut curve, true);
    assert!(first.drag_started && first.response.changed());
    let finish = start + egui::vec2(plot.width() * 0.1, -plot.height() * 0.2);
    let (second, _) = frame(&ctx, vec![Event::PointerMoved(finish)], &mut curve, true);
    assert!(!second.drag_started && second.response.changed());
    assert!((curve.control1[0] - original.control1[0] - 0.1).abs() < 1e-5);
    assert!((curve.control1[1] - original.control1[1] - 0.2).abs() < 1e-5);
    assert_eq!(curve.control2, original.control2);
    let outside = start + egui::vec2(plot.width() * 2.0, -plot.height() * 2.0);
    frame(&ctx, vec![Event::PointerMoved(outside)], &mut curve, true);
    assert_eq!(curve.control1[0], curve.control2[0]);
    assert_eq!(
        curve.control1[1], 1.0,
        "gain handles may cross each other vertically"
    );
    let (release, _) = frame(&ctx, pointer(outside, false), &mut curve, true);
    assert!(release.drag_stopped);
    let previous = curve;
    frame(&ctx, vec![], &mut curve, true);
    assert_eq!(curve, previous, "idle rendering must not create edits");
}

#[test]
fn focused_handles_support_keyboard_fine_adjustment_accessibility_and_disabled_state() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut curve = CrossfadeCurve::default();
    let initial = frame(&ctx, vec![], &mut curve, true).0;
    let point = initial.handles[1].center();
    frame(&ctx, pointer(point, true), &mut curve, true);
    frame(&ctx, pointer(point, false), &mut curve, true);
    let before = curve;
    let (edited, output) = frame(
        &ctx,
        key(egui::Key::ArrowDown, Modifiers::NONE),
        &mut curve,
        true,
    );
    assert!(edited.response.changed());
    assert!((curve.control2[1] - before.control2[1] + 0.02).abs() < 1e-5);
    assert!(output.platform_output.events.iter().any(|event| {
        let info = event.widget_info();
        info.typ == egui::WidgetType::Slider
            && info
                .label
                .as_ref()
                .is_some_and(|label| label.starts_with("Crossfade handle 2"))
            && info.value == Some(f64::from(curve.control2[1]))
    }));
    let before = curve;
    frame(
        &ctx,
        key(egui::Key::ArrowRight, Modifiers::SHIFT),
        &mut curve,
        true,
    );
    assert!((curve.control2[0] - before.control2[0] - 0.002).abs() < 1e-5);
    let before = curve;
    let (disabled, _) = frame(
        &ctx,
        key(egui::Key::ArrowUp, Modifiers::NONE),
        &mut curve,
        false,
    );
    assert_eq!(curve, before);
    assert!(!disabled.response.changed());
    let point = frame(&ctx, vec![], &mut curve, true).0.handles[1].center();
    frame(&ctx, pointer(point, true), &mut curve, true);
    frame(&ctx, pointer(point, false), &mut curve, true);
    frame(
        &ctx,
        key(egui::Key::Home, Modifiers::NONE),
        &mut curve,
        true,
    );
    assert_eq!(curve, CrossfadeCurve::default());
}

#[test]
fn rendered_spline_gains_are_complementary_in_both_themes() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(&ctx, appearance);
        let mut curve = CrossfadeCurve {
            control1: [0.1, 0.8],
            control2: [0.85, 0.2],
        };
        let (response, output) = frame(&ctx, vec![], &mut curve, true);
        let lines: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Path(path) if path.points.len() > 40 => Some(path),
                _ => None,
            })
            .collect();
        assert_eq!(lines.len(), 2);
        let [outgoing, incoming] = [lines[0], lines[1]];
        assert_eq!(outgoing.points.len(), incoming.points.len());
        for (out, input) in outgoing.points.iter().zip(&incoming.points) {
            assert_eq!(out.x, input.x);
            assert!(
                (out.y + input.y - response.plot_rect.top() - response.plot_rect.bottom()).abs()
                    < 1e-4
            );
            assert!(response.plot_rect.expand(0.001).contains(*out));
            assert!(response.plot_rect.expand(0.001).contains(*input));
        }
        assert_eq!(
            incoming.points.first().unwrap(),
            &response.plot_rect.left_bottom()
        );
        assert_eq!(
            incoming.points.last().unwrap(),
            &response.plot_rect.right_top()
        );
    }
}

#[test]
fn timeline_overlay_preserves_layout_and_clips_to_the_supplied_bounds() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let rect = egui::Rect::from_min_size(egui::pos2(40.0, 30.0), egui::vec2(140.0, 54.0));
    let output = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let before = ui.next_widget_position();
            daw_ui::paint_crossfade(ui.painter(), rect, CrossfadeCurve::default());
            assert_eq!(ui.next_widget_position(), before);
        });
    });
    let paths: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Path(path) => Some((shape.clip_rect, path)),
            _ => None,
        })
        .collect();
    assert_eq!(paths.len(), 2);
    for (clip, path) in paths {
        assert_eq!(clip, rect);
        assert!(path.points.iter().all(|point| rect.contains(*point)));
    }
}

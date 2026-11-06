use daw_ui::{ClipFadeBounds, ClipFadeResponse, CrossfadeHandleResponse, TimelineViewport, theme};
use egui::{
    Context, Event, FullOutput, Id, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, pos2,
    vec2,
};

fn frame(
    ctx: &Context,
    events: Vec<Event>,
) -> (ClipFadeResponse, CrossfadeHandleResponse, FullOutput) {
    frame_at(ctx, events, 1.0, false)
}

fn frame_at(
    ctx: &Context,
    events: Vec<Event>,
    fade_in_end: f64,
    narrow: bool,
) -> (ClipFadeResponse, CrossfadeHandleResponse, FullOutput) {
    let mut result = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 400.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if narrow {
                    ui.set_clip_rect(ui.clip_rect().with_max_x(320.0));
                }
                let body = Rect::from_min_size(pos2(80.0, 80.0), vec2(400.0, 160.0));
                let clip = body.with_min_y(60.0);
                let view = TimelineViewport {
                    pixels_per_beat: 100.0,
                    ..Default::default()
                };
                let fades = daw_ui::clip_fade_handles_with_hover_bounds(
                    ui,
                    Id::new("fade"),
                    body,
                    clip,
                    view,
                    ClipFadeBounds {
                        start: 0.0,
                        end: 4.0,
                        fade_in_end,
                        fade_out_start: 3.0,
                    },
                    true,
                );
                let blend = daw_ui::crossfade_duration_handle(
                    ui,
                    Id::new("blend"),
                    body,
                    view,
                    daw_ui::CrossfadeBounds {
                        start: 0.0,
                        end: 4.0,
                        duration: 2.0,
                    },
                    true,
                );
                result = Some((fades, blend));
            });
        },
    );
    let (fades, blend) = result.unwrap();
    (fades, blend, output)
}

#[test]
fn active_fade_keeps_ownership_after_its_endpoint_moves_outside_the_viewport() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let (idle, _, _) = frame_at(&ctx, vec![], 1.0, true);
    let at = idle.handles[0].unwrap().center();
    frame_at(&ctx, pointer(at, true), 1.0, true);
    let (out, _, _) = frame_at(
        &ctx,
        vec![Event::PointerMoved(pos2(500.0, at.y))],
        1.0,
        true,
    );
    assert_eq!(out.edit, Some((daw_ui::ClipFadeEdge::In, 4.0)));
    // The caller applied the prior edit; the endpoint is now outside the pane.
    let (returning, _, _) = frame_at(
        &ctx,
        vec![Event::PointerMoved(pos2(280.0, at.y))],
        4.0,
        true,
    );
    assert_eq!(returning.edit, Some((daw_ui::ClipFadeEdge::In, 2.0)));
    assert!(returning.interacting && returning.visible[0]);
    let (released, _, _) = frame_at(&ctx, pointer(pos2(300.0, at.y), false), 2.0, true);
    assert_eq!(released.edit, Some((daw_ui::ClipFadeEdge::In, 2.2)));
    let (idle, _, _) = frame_at(
        &ctx,
        vec![Event::PointerMoved(pos2(600.0, 350.0))],
        4.0,
        true,
    );
    assert!(idle.edit.is_none() && !idle.interacting);
    assert!(idle.handles[0].is_none());
}

#[test]
fn named_collapsed_clips_keep_fade_targets_inside_the_short_waveform_body() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let _ = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let body = Rect::from_min_size(pos2(80.0, 80.0), vec2(400.0, 11.5));
            let result = daw_ui::clip_fade_handles(
                ui,
                Id::new("collapsed"),
                body,
                TimelineViewport {
                    pixels_per_beat: 100.0,
                    ..Default::default()
                },
                ClipFadeBounds {
                    start: 0.0,
                    end: 4.0,
                    fade_in_end: 0.0,
                    fade_out_start: 4.0,
                },
                true,
            );
            for target in result.handles {
                let target = target.expect("Short bodies must remain directly editable");
                assert!(body.contains_rect(target) && target.height() >= 8.0);
            }
        });
    });
}

fn pointer(at: Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(at),
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

#[test]
fn fade_and_crossfade_handles_hide_without_losing_their_hit_targets() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(&ctx, appearance);
        let (idle, blend, output) = frame(&ctx, vec![Event::PointerMoved(pos2(600.0, 350.0))]);
        assert_eq!(idle.visible, [false, false]);
        assert!(!blend.visible);
        assert!(idle.handles.iter().all(Option::is_some));
        assert!(blend.handle.is_some());
        let colors = theme::palette(&ctx);
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape,Shape::Rect(rect) if rect.fill==colors.orange))
        );
        let target = idle.handles[0].unwrap().center();
        let (hover, _, _) = frame(&ctx, vec![Event::PointerMoved(target)]);
        assert_eq!(hover.visible, [true, false]);
        assert_eq!(hover.handles, idle.handles);
        let (corner, _, _) = frame(&ctx, vec![Event::PointerMoved(pos2(85.0, 65.0))]);
        assert_eq!(
            corner.visible,
            [true, false],
            "A clip title corner reveals its fade endpoint"
        );
        frame(&ctx, pointer(target, true));
        let (drag, _, _) = frame(&ctx, vec![Event::PointerMoved(pos2(580.0, 320.0))]);
        assert!(
            drag.visible[0] && drag.interacting,
            "Active drag stays visible outside its hover zone"
        );
        frame(&ctx, pointer(pos2(580.0, 320.0), false));
        let (away, _, _) = frame(&ctx, vec![Event::PointerMoved(pos2(600.0, 350.0))]);
        assert_eq!(away.visible, [false, false]);
        let (_, blend, _) = frame(
            &ctx,
            vec![Event::PointerMoved(blend.handle.unwrap().center())],
        );
        assert!(
            blend.visible,
            "Crossfade duration follows the same reveal rule"
        );
    }
}

#[test]
fn preview_shades_only_actual_attenuation_and_is_bounded_and_subtle() {
    for dark in [false, true] {
        let ctx = Context::default();
        theme::apply(&ctx);
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        let rect = Rect::from_min_size(pos2(40.0, 60.0), vec2(400.0, 160.0));
        for active in [false, true] {
            let mut preview = None;
            let output = ctx.run(RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    preview = daw_ui::paint_clip_fades(
                        ui.painter(),
                        rect,
                        TimelineViewport {
                            pixels_per_beat: 100.0,
                            ..Default::default()
                        },
                        0.0,
                        4.0,
                        |beat| {
                            if active {
                                (beat as f32).clamp(0.0, 1.0)
                            } else {
                                1.0
                            }
                        },
                    );
                });
            });
            let meshes: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    Shape::Mesh(mesh) => Some(mesh),
                    _ => None,
                })
                .collect();
            if active {
                let preview = preview.unwrap();
                assert!(rect.contains_rect(preview));
                assert!(
                    preview.right() <= rect.left() + 102.0,
                    "Unity plateau should not be shaded"
                );
                assert_eq!(meshes.len(), 1);
                let mesh = meshes[0];
                assert!(mesh.is_valid());
                assert!(mesh.vertices.len() <= 4100);
                assert!(mesh.vertices.iter().all(|vertex| rect.contains(vertex.pos)
                    && vertex.color.a() > 0
                    && vertex.color.a() <= 32));
            } else {
                assert!(preview.is_none());
                assert!(meshes.is_empty(), "Zero fades leave no veil or unity line");
                assert!(
                    !output
                        .shapes
                        .iter()
                        .any(|shape| matches!(&shape.shape, Shape::Path(_)))
                );
            }
        }
    }
}

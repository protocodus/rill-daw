use daw_ui::{mixer_route_lane, theme};
use egui::{Context, RawInput, Rect, Sense, Shape, Vec2};

#[test]
fn compact_channel_actions_leave_readable_source_kinds_without_smaller_tap_targets() {
    for dark in [false, true] {
        let ctx = Context::default();
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        theme::apply(&ctx);
        for kind in ["Audio", "MIDI", "Bus"] {
            let mut source = Rect::NOTHING;
            let mut menu = Rect::NOTHING;
            let output = ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(200.0, 200.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.set_width(156.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_SM;
                            menu = daw_ui::compact_action_menu(ui, "Channel", |_| {})
                                .response
                                .rect;
                            source = daw_ui::text::note(ui, kind).rect;
                        });
                    });
                },
            );
            assert!(menu.height() >= daw_ui::tokens::CONTROL_HEIGHT);
            assert!(source.left() >= menu.right() + daw_ui::tokens::SPACE_SM - 0.1);
            assert!(source.right() <= 164.1);
            let label = output
                .shapes
                .iter()
                .find_map(|shape| {
                    if let Shape::Text(text) = &shape.shape {
                        (text.galley.text() == kind).then_some(text)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert_eq!(
                label.galley.rows.len(),
                1,
                "{kind} must never split across lines"
            );
            assert!(source.contains_rect(label.galley.rect.translate(label.pos.to_vec2())));
        }
    }
}

#[test]
fn route_lanes_submit_separate_arrowheads_and_keep_clipped_destinations_named() {
    for dark in [false, true] {
        let ctx = Context::default();
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        theme::apply(&ctx);
        let mut lanes = Vec::new();
        let output = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 300.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    for (target, caption) in [
                        (690.0, "Output · Source → Master"),
                        (360.0, "Pre send · Source → Bus"),
                        (1600.0, "Post send · Source → Return"),
                    ] {
                        let route =
                            mixer_route_lane(ui, 90.0, target, caption, theme::palette(ctx).accent);
                        assert_eq!(route.response.sense, Sense::hover());
                        lanes.push(route.painted.unwrap());
                    }
                });
            },
        );
        assert!(
            lanes
                .windows(2)
                .all(|pair| pair[0].lane.bottom() <= pair[1].lane.top())
        );
        assert!(
            lanes
                .iter()
                .all(|lane| lane.lane.contains_rect(lane.head)
                    && lane.lane.contains_rect(lane.shaft))
        );
        assert!(!lanes[0].target_clipped && lanes[2].target_clipped);
        let heads: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let Shape::Mesh(mesh) = &shape.shape {
                    (mesh.vertices.len() == 3 && mesh.indices.len() == 3).then_some(mesh)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            heads.len(),
            3,
            "each lane must actually submit its arrowhead"
        );
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| if let Shape::Text(text) = &shape.shape {
                    text.galley.text().contains("target offscreen")
                } else {
                    false
                })
        );
        assert!(ctx.memory(|memory| memory.focused().is_none()));
    }
}

#[test]
fn a_route_to_a_bus_on_the_left_points_left_without_crossing_another_lane() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let _ = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let left = mixer_route_lane(
                ui,
                600.0,
                90.0,
                "Output · Bus → Earlier bus",
                theme::palette(ctx).accent,
            )
            .painted
            .unwrap();
            assert!(left.from.x > left.to.x);
            assert!(left.head.left() >= left.to.x);
            let clipped = mixer_route_lane(
                ui,
                -90.0,
                -160.0,
                "Output · Offscreen → Earlier bus",
                theme::palette(ctx).accent,
            )
            .painted
            .unwrap();
            assert!(clipped.source_clipped && clipped.target_clipped);
            assert!(clipped.from.x > clipped.to.x);
            assert!(clipped.head.right() < clipped.lane.left() + 32.0);
        });
    });
}

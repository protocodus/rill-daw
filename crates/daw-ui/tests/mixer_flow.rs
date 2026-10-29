use daw_ui::{MixerRouteEndpoint, mixer_route_lane, theme};
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
        for state in [
            None,
            Some(daw_ui::AutomationState::Read),
            Some(daw_ui::AutomationState::Overridden),
            Some(daw_ui::AutomationState::Bypassed),
        ] {
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
                                menu = daw_ui::automation_channel_menu(ui, state, |_| {})
                                    .response
                                    .rect;
                                source = daw_ui::text::note(ui, kind).rect;
                            });
                        });
                    },
                );
                assert!(menu.height() >= daw_ui::tokens::CONTROL_HEIGHT);
                let gap = if state.is_some() {
                    daw_ui::tokens::SPACE_XS
                } else {
                    daw_ui::tokens::SPACE_SM
                };
                assert!(source.left() >= menu.right() + gap - 0.1);
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
}

#[test]
fn route_lanes_connect_channels_with_bounded_gradient_ribbons_in_both_directions() {
    for dark in [false, true] {
        let ctx = Context::default();
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        theme::apply(&ctx);
        let source_color = egui::Color32::from_rgb(60, 150, 210);
        let target_color = egui::Color32::from_rgb(230, 160, 70);
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
                    ui.add_space(36.0);
                    let clip = ui.clip_rect();
                    for (source, target) in [(90.0, 690.0), (690.0, 360.0), (90.0, 1600.0)] {
                        let route = mixer_route_lane(
                            ui,
                            MixerRouteEndpoint {
                                position: egui::pos2(source, 40.0),
                                color: source_color,
                            },
                            MixerRouteEndpoint {
                                position: egui::pos2(target, 40.0),
                                color: target_color,
                            },
                            "Output · Source → Destination",
                            clip,
                        );
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
        assert!(!lanes[0].target_clipped && lanes[2].target_clipped);
        let ribbons: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let Shape::Mesh(mesh) = &shape.shape {
                    (mesh.vertices.len() == 8 && mesh.indices.len() == 18)
                        .then_some((mesh, shape.clip_rect))
                } else {
                    None
                }
            })
            .collect();
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
        assert_eq!(ribbons.len(), 3);
        assert_eq!(heads.len(), 3);
        for (index, route) in lanes.iter().enumerate() {
            assert_eq!(route.path[0], route.from);
            assert_eq!(route.path[3], route.to);
            assert_eq!(route.path[0].x, route.path[1].x);
            assert_eq!(route.path[1].y, route.path[2].y);
            assert_eq!(route.path[2].x, route.path[3].x);
            assert!(route.path[1].y > route.from.y && route.path[2].y > route.to.y);
            assert_eq!(route.stroke_width, 3.0);
            let (ribbon, clip) = ribbons[index];
            assert!(ribbon.is_valid());
            assert!(
                clip.top() <= route.from.y,
                "clip must include channel attachment above lane"
            );
            assert_eq!(
                (ribbon.vertices[0].pos + ribbon.vertices[1].pos.to_vec2()) * 0.5,
                route.from
            );
            assert_eq!(ribbon.vertices[0].color, source_color);
            assert_eq!(ribbon.vertices[1].color, source_color);
            assert_ne!(ribbon.vertices[2].color, source_color);
            assert_ne!(ribbon.vertices[2].color, target_color);
            assert_eq!(heads[index].vertices[0].pos, route.to);
            assert_eq!(heads[index].vertices[0].color, target_color);
            assert_eq!(heads[index].vertices[1].color, ribbon.vertices[6].color);
            assert_eq!(ribbon.vertices[6].pos.y, heads[index].vertices[1].pos.y);
            assert_eq!(route.source_color, source_color);
            assert_eq!(route.target_color, target_color);
        }
        assert!(ctx.memory(|memory| memory.focused().is_none()));
    }
}

#[test]
fn clipped_routes_keep_their_true_endpoints_and_reject_invalid_geometry() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let _ = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let clip = ui.clip_rect();
            ui.add_space(30.0);
            let endpoint = |x| MixerRouteEndpoint {
                position: egui::pos2(x, 30.0),
                color: theme::palette(ctx).accent,
            };
            let left = mixer_route_lane(
                ui,
                endpoint(600.0),
                endpoint(90.0),
                "Output · Later → Earlier",
                clip,
            )
            .painted
            .unwrap();
            assert!(left.from.x > left.to.x);
            assert_eq!(left.to, egui::pos2(90.0, 30.0));
            assert_eq!(left.head.top(), left.to.y);
            let clipped = mixer_route_lane(
                ui,
                endpoint(-90.0),
                endpoint(-160.0),
                "Output · Offscreen",
                clip,
            )
            .painted
            .unwrap();
            assert!(clipped.source_clipped && clipped.target_clipped);
            assert_eq!(clipped.from, egui::pos2(-90.0, 30.0));
            assert_eq!(clipped.to, egui::pos2(-160.0, 30.0));
            for target in [f32::NAN, f32::INFINITY, 90.0] {
                assert!(
                    mixer_route_lane(ui, endpoint(90.0), endpoint(target), "Invalid", clip)
                        .painted
                        .is_none()
                );
            }
        });
    });
}

#[test]
fn scrolling_route_lanes_keeps_stems_attached_to_channel_bottoms() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut expected = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(800.0, 400.0),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_space(60.0);
                let channel_bottom = ui.cursor().top();
                ui.add_space(10.0);
                let connection_clip = ui.clip_rect();
                egui::ScrollArea::vertical()
                    .max_height(90.0)
                    .show(ui, |ui| {
                        expected = mixer_route_lane(
                            ui,
                            MixerRouteEndpoint {
                                position: egui::pos2(90.0, channel_bottom),
                                color: egui::Color32::BLUE,
                            },
                            MixerRouteEndpoint {
                                position: egui::pos2(690.0, channel_bottom),
                                color: egui::Color32::RED,
                            },
                            "Output · Source → Bus",
                            connection_clip,
                        )
                        .painted;
                    });
            });
        },
    );
    let route = expected.unwrap();
    assert!(route.lane.top() > route.from.y);
    let ribbon = output
        .shapes
        .iter()
        .find(|shape| {
            matches!(&shape.shape,
        Shape::Mesh(mesh) if mesh.vertices.len() == 8 && mesh.indices.len() == 18)
        })
        .unwrap();
    assert!(
        ribbon.clip_rect.top() <= route.from.y,
        "lane scroll clipping must not cut off the connections to their channels"
    );
    assert!(ribbon.clip_rect.bottom() <= route.lane.bottom());
}

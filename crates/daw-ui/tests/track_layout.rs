use daw_ui::{ResizeAxis, resize_value_at};
use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect};

fn frame(ctx: &Context, width: &mut f32, events: Vec<Event>, enabled: bool) {
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    let rect = Rect::from_min_max(
                        egui::pos2(*width - 4.0, 30.0),
                        egui::pos2(*width + 4.0, 500.0),
                    );
                    resize_value_at(
                        ui,
                        egui::Id::new("labels"),
                        rect,
                        width,
                        224.0..=600.0,
                        ResizeAxis::Horizontal,
                        "Resize labels",
                    );
                });
            });
        },
    );
}

fn button(x: f32, pressed: bool, button: PointerButton) -> Event {
    Event::PointerButton {
        pos: egui::pos2(x, 200.0),
        button,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn resize_uses_the_original_value_as_its_gutter_moves_and_clamps_the_view() {
    let ctx = Context::default();
    let mut width = 248.0;
    frame(&ctx, &mut width, vec![], true);
    frame(
        &ctx,
        &mut width,
        vec![
            Event::PointerMoved(egui::pos2(248.0, 200.0)),
            button(248.0, true, PointerButton::Primary),
        ],
        true,
    );
    for target in [300.0, 350.0, 700.0] {
        frame(
            &ctx,
            &mut width,
            vec![Event::PointerMoved(egui::pos2(target, 200.0))],
            true,
        );
        assert_eq!(width, target.min(600.0));
    }
    frame(
        &ctx,
        &mut width,
        vec![button(700.0, false, PointerButton::Primary)],
        true,
    );
    assert_eq!(width, 600.0);
    assert!(ctx.memory(|memory| memory.focused()).is_none());
}

#[test]
fn middle_drag_and_disabled_gutters_do_not_resize() {
    for (button_kind, enabled) in [
        (PointerButton::Middle, true),
        (PointerButton::Primary, false),
    ] {
        let ctx = Context::default();
        let mut width = 248.0;
        frame(&ctx, &mut width, vec![], enabled);
        frame(
            &ctx,
            &mut width,
            vec![
                Event::PointerMoved(egui::pos2(248.0, 200.0)),
                button(248.0, true, button_kind),
            ],
            enabled,
        );
        frame(
            &ctx,
            &mut width,
            vec![Event::PointerMoved(egui::pos2(400.0, 200.0))],
            enabled,
        );
        frame(
            &ctx,
            &mut width,
            vec![button(400.0, false, button_kind)],
            enabled,
        );
        assert_eq!(width, 248.0);
    }
}

#[test]
fn deep_folder_names_retain_readable_space_inside_a_narrow_header() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut name = Rect::NOTHING;
    let heading = Rect::from_min_max(egui::pos2(14.0, 20.0), egui::pos2(216.0, 110.0));
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                name = daw_ui::track_hierarchy(
                    ui,
                    heading,
                    20,
                    "Deep folder",
                    daw_ui::Icon::Folder,
                    false,
                    egui::Color32::LIGHT_BLUE,
                );
            });
        },
    );
    assert!(heading.contains_rect(name));
    assert!(name.width() >= 80.0);
    let text = output
        .shapes
        .iter()
        .find_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape {
                Some(text)
            } else {
                None
            }
        })
        .unwrap();
    assert!(name.contains_rect(text.galley.rect.translate(text.pos.to_vec2())));
}

#[test]
fn recursive_containers_enclose_whole_lanes_and_keep_each_bottom_separate() {
    let ctx = Context::default();
    let colors = [
        egui::Color32::RED,
        egui::Color32::BLUE,
        egui::Color32::GREEN,
    ];
    let rows: Vec<_> = [(0, true), (1, false), (1, true), (2, true), (3, false)]
        .into_iter()
        .enumerate()
        .map(|(index, (depth, folder))| daw_ui::FolderLane {
            id: index as u64,
            depth,
            folder,
            color: colors[index.min(2)],
            rect: Rect::from_min_size(
                egui::pos2(0.0, index as f32 * 80.0),
                egui::vec2(600.0, 80.0),
            ),
        })
        .collect();
    let closures = daw_ui::folder_closures(&rows);
    assert_eq!(closures[4], vec![3, 2, 0]);
    let mut groups = Vec::new();
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 900.0))),
            ..Default::default()
        },
        |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("groups"),
            ));
            groups = daw_ui::paint_folder_containers(
                &painter,
                &rows,
                Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 500.0)),
                6.0,
            );
        },
    );
    assert_eq!(groups.len(), 3);
    for pair in groups.windows(2) {
        assert!(pair[0].rect.contains_rect(pair[1].rect));
        assert!(pair[0].rect.bottom() > pair[1].rect.bottom() + 4.0);
    }
    assert!(groups[2].rect.contains_rect(Rect::from_min_max(
        egui::pos2(248.0, 328.0),
        egui::pos2(578.0, 392.0)
    )));
    assert_eq!(groups[0].color, rows[0].color);
    assert_eq!(groups[1].color, rows[2].color);
    assert_eq!(groups[2].color, rows[3].color);
}

#[test]
fn deep_narrow_containers_keep_content_inside_and_do_not_invent_scrolled_caps() {
    let ctx = Context::default();
    let mut rows: Vec<_> = (0..32)
        .map(|depth| daw_ui::FolderLane {
            id: depth as u64,
            depth,
            folder: true,
            color: egui::Color32::LIGHT_BLUE,
            rect: Rect::from_min_size(
                egui::pos2(0.0, depth as f32 * 20.0),
                egui::vec2(800.0, 20.0),
            ),
        })
        .collect();
    rows.push(daw_ui::FolderLane {
        id: 32,
        depth: 32,
        folder: false,
        color: egui::Color32::RED,
        rect: Rect::from_min_size(egui::pos2(0.0, 640.0), egui::vec2(800.0, 20.0)),
    });
    let (step, padding) = daw_ui::folder_lane_padding(&rows, 224.0);
    let viewport = Rect::from_min_max(egui::pos2(0.0, 650.0), egui::pos2(800.0, 700.0));
    let mut groups = Vec::new();
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 900.0))),
            ..Default::default()
        },
        |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("groups"),
            ));
            groups = daw_ui::paint_folder_containers(&painter, &rows, viewport, step);
        },
    );
    assert_eq!(groups.len(), 32);
    let content = daw_ui::folder_content_inset(32, false, step);
    let inner = groups.last().unwrap();
    assert!(content >= inner.rect.left() + 4.0);
    assert!(800.0 - padding - 4.0 < inner.rect.right());
    assert!(inner.rect.bottom() >= rows[32].rect.bottom());
    for group in groups {
        assert!(
            group.borders[0].is_none(),
            "The real group top is above the scroll viewport"
        );
        if group.rect.bottom() > viewport.bottom() + 1.0 {
            assert!(group.borders[3].is_none());
        }
        for border in group.borders.iter().flatten() {
            assert!(viewport.contains_rect(*border));
        }
    }
}

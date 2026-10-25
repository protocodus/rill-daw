use daw_ui::{ViewBackdrop, ViewLink};
use egui::{Context, Event, PointerButton, Pos2, RawInput, Rect, Vec2};

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
        events,
        ..Default::default()
    }
}

#[test]
fn retained_view_paints_without_recreating_text_controls_or_handling_clicks() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut backdrop = ViewBackdrop::default();
    let mut button = Rect::NOTHING;
    for _ in 0..3 {
        let _ = ctx.run(input(vec![]), |ctx| {
            let panel = egui::CentralPanel::default().show(ctx, |ui| {
                button = daw_ui::button(ui, "Previous view action").rect;
                daw_ui::text::title(ui, "Previous view");
            });
            backdrop = ViewBackdrop::capture(ctx, panel.response.rect);
        });
    }
    assert!(backdrop.shape_count() > 2);
    for pressed in [true, false] {
        let output = ctx.run(
            input(vec![
                Event::PointerMoved(button.center()),
                Event::PointerButton {
                    pos: button.center(),
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]),
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| backdrop.paint(ui));
            },
        );
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Mesh(_)))
        );
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Text(_)))
        );
        assert!(output.platform_output.events.is_empty());
    }
}

#[test]
fn dialog_context_keeps_parent_modal_without_live_controls_or_a_second_scrim() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let scrim = daw_ui::theme::palette(&ctx).popup_scrim;
    let mut backdrop = ViewBackdrop::default();
    let mut parent_button = Rect::NOTHING;
    for _ in 0..3 {
        let _ = ctx.run(input(vec![]), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                daw_ui::text::title(ui, "Arrangement");
            });
            daw_ui::modal(ctx, egui::Id::new("parent"), "Save changes?", |ui| {
                daw_ui::text::body(ui, "This is the previous foreground dialog");
                parent_button = daw_ui::button(ui, "Save project").rect;
            });
            let ordinary = ViewBackdrop::capture(ctx, ctx.content_rect());
            backdrop = ViewBackdrop::capture_dialog_context(ctx, ctx.content_rect());
            assert!(backdrop.shape_count() > ordinary.shape_count());
        });
    }
    for pressed in [true, false] {
        let output = ctx.run(
            input(vec![
                Event::PointerMoved(parent_button.center()),
                Event::PointerButton {
                    pos: parent_button.center(),
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]),
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| backdrop.paint(ui));
            },
        );
        assert!(output.platform_output.events.is_empty());
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| { matches!(shape.shape, egui::Shape::Text(_)) })
        );
        let meshes = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) => Some(mesh),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!meshes.is_empty());
        assert!(
            meshes
                .iter()
                .all(|mesh| mesh.vertices.iter().all(|v| v.color != scrim))
        );
    }
}

#[test]
fn retained_view_keeps_preview_texture_alive_until_last_snapshot_is_dropped() {
    let ctx = Context::default();
    let image = ctx.load_texture(
        "preview",
        egui::ColorImage::filled([8, 8], egui::Color32::BLUE),
        Default::default(),
    );
    let id = image.id();
    let mut backdrop = ViewBackdrop::default();
    let _ = ctx.run(input(vec![]), |ctx| {
        let panel = egui::CentralPanel::default().show(ctx, |ui| {
            ui.image((id, Vec2::splat(80.0)));
        });
        backdrop = ViewBackdrop::capture(ctx, panel.response.rect);
    });
    let nested = backdrop.clone();
    drop(image);
    drop(backdrop);
    assert!(ctx.tex_manager().read().meta(id).is_some());
    drop(nested);
    assert!(ctx.tex_manager().read().meta(id).is_none());
}

#[test]
fn active_view_link_can_be_reselected_to_dismiss_an_overlay() {
    let ctx = Context::default();
    let links = [
        ViewLink {
            value: 0,
            label: "Arranger",
            enabled: true,
        },
        ViewLink {
            value: 1,
            label: "Mixer",
            enabled: true,
        },
    ];
    let mut point = Pos2::ZERO;
    for _ in 0..3 {
        let output = ctx.run(input(vec![]), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                daw_ui::view_links(ui, 0, &links);
            });
        });
        for shape in output.shapes {
            if let egui::Shape::Text(text) = shape.shape
                && text.galley.job.text == "Arranger"
            {
                point = text.pos + text.galley.size() * 0.5;
            }
        }
    }
    assert_ne!(point, Pos2::ZERO);
    let mut selected = None;
    for pressed in [true, false] {
        let _ = ctx.run(
            input(vec![
                Event::PointerMoved(point),
                Event::PointerButton {
                    pos: point,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]),
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    selected = daw_ui::view_links(ui, 0, &links);
                });
            },
        );
    }
    assert_eq!(selected, Some(0));
}

#[test]
fn related_view_buttons_wrap_as_whole_controls_at_narrow_widths() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let links = ["Channel", "Instrument", "Effects", "Routing", "Mixer"]
        .into_iter()
        .enumerate()
        .map(|(value, label)| ViewLink {
            value,
            label,
            enabled: true,
        })
        .collect::<Vec<_>>();
    for width in [260.0, 520.0, 800.0] {
        for _ in 0..3 {
            let mut raw = input(vec![]);
            raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 600.0)));
            let output = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    daw_ui::view_links(ui, 0, &links);
                });
            });
            let mut seen = 0;
            for shape in output.shapes {
                if let egui::Shape::Text(text) = shape.shape
                    && links.iter().any(|link| link.label == text.galley.job.text)
                {
                    assert_eq!(
                        text.galley.rows.len(),
                        1,
                        "label must stay on one line at {width}: {}",
                        text.galley.job.text
                    );
                    assert!(
                        text.pos.x >= 0.0 && text.pos.x + text.galley.size().x <= width,
                        "whole label must remain visible at {width}: {}",
                        text.galley.job.text
                    );
                    seen += 1;
                }
            }
            assert_eq!(seen, links.len());
        }
    }
}

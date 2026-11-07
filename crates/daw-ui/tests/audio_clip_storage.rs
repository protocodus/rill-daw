use daw_ui::{
    DISK_STREAMED_AUDIO, Icon, audio_streaming_status, clip_surface_named,
    paint_audio_streaming_badge, paint_audio_streaming_badge_avoiding, theme,
};
use egui::{Context, Event, Id, Modifiers, PointerButton, RawInput, Rect, Sense, Shape};

#[test]
fn disk_badges_keep_clip_names_thin_headers_and_existing_controls_in_both_themes() {
    assert_eq!(Icon::for_label(DISK_STREAMED_AUDIO), Some(Icon::Disk));
    assert_ne!(Icon::Disk, Icon::Save);
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for scale in [1.0, 1.25, 2.0] {
            let ctx = Context::default();
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(scale);
            for name in ["Long audio source", ""] {
                let mut original = None;
                let mut placed = None;
                let output = ctx.run(RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let clip =
                            Rect::from_min_size(egui::pos2(24.0, 24.0), egui::vec2(240.0, 48.0));
                        let surface = clip_surface_named(
                            ui.painter(),
                            clip,
                            egui::Color32::from_rgb(100, 145, 185),
                            false,
                            name,
                        );
                        let first = paint_audio_streaming_badge(ui.painter(), clip).unwrap();
                        let protected_fade = first.expand(2.0);
                        let badge = paint_audio_streaming_badge_avoiding(
                            ui.painter(),
                            clip,
                            &[protected_fade, surface.name_hit],
                        )
                        .unwrap();
                        assert!(!badge.intersects(protected_fade));
                        assert!(!badge.intersects(surface.name_hit));
                        assert!(badge.left() >= clip.left() + 10.0);
                        assert!(badge.right() <= clip.right() - 10.0);
                        assert!(clip.contains_rect(badge));
                        assert!(ui.painter().clip_rect().contains_rect(badge));
                        for edge in [badge.left(), badge.top(), badge.right(), badge.bottom()] {
                            assert!((edge * scale - (edge * scale).round()).abs() < 1e-4);
                        }
                        if name.is_empty() {
                            assert!(
                                surface.title.height() <= 3.5,
                                "A streaming marker cannot introduce a title band for a blank name"
                            );
                        }
                        let after = clip_surface_named(
                            ui.painter(),
                            clip,
                            egui::Color32::from_rgb(100, 145, 185),
                            false,
                            name,
                        );
                        assert_eq!(surface.title, after.title);
                        assert_eq!(surface.content, after.content);
                        assert_eq!(surface.name_hit, after.name_hit);
                        original = Some(clip);
                        placed = Some(badge);
                    });
                });
                assert!(original.unwrap().contains_rect(placed.unwrap()));
                assert!(
                    !output
                        .shapes
                        .iter()
                        .any(|shape| matches!(shape.shape, Shape::Text(_))),
                    "Canvas badges are vector paint, with no replacement clip title"
                );
            }
        }
    }
}

#[test]
fn badges_do_not_take_mouse_input_and_skip_unpaintable_regions() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let clip = Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(200.0, 64.0));
    let mut position = None;
    for phase in 0..3 {
        let pointer = position.unwrap_or(clip.center());
        let mut events = vec![Event::PointerMoved(pointer)];
        if phase > 0 {
            events.push(Event::PointerButton {
                pos: pointer,
                button: PointerButton::Primary,
                pressed: phase == 1,
                modifiers: Modifiers::NONE,
            });
        }
        let _ = ctx.run(
            RawInput {
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let body =
                        ui.interact(clip, Id::new("original_clip_body"), Sense::click_and_drag());
                    let badge = paint_audio_streaming_badge(ui.painter(), clip).unwrap();
                    position = Some(badge.center());
                    if phase == 2 {
                        assert!(
                            body.clicked(),
                            "The original body owns clicks through the badge"
                        );
                    }
                    for tiny in [
                        Rect::NOTHING,
                        Rect::from_min_size(clip.min, egui::vec2(20.0, 10.0)),
                    ] {
                        assert!(paint_audio_streaming_badge(ui.painter(), tiny).is_none());
                    }
                    let clipped = ui
                        .painter()
                        .with_clip_rect(Rect::from_min_size(clip.min, egui::vec2(20.0, 20.0)));
                    assert!(paint_audio_streaming_badge(&clipped, clip).is_none());
                    assert!(
                        paint_audio_streaming_badge_avoiding(ui.painter(), clip, &[clip]).is_none()
                    );
                    let long = Rect::from_min_size(clip.min, egui::vec2(100_000_000.0, 64.0));
                    let marker = paint_audio_streaming_badge(ui.painter(), long).unwrap();
                    assert!(
                        ui.painter().clip_rect().contains_rect(marker),
                        "A long offscreen source places its badge within visible bounded work"
                    );
                });
            },
        );
    }
}

#[test]
fn properties_storage_status_has_a_plain_accessible_label() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let output = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            audio_streaming_status(ui);
        });
    });
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
        Shape::Text(text) if text.galley.text()==DISK_STREAMED_AUDIO)));
}

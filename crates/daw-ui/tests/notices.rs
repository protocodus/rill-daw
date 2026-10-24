use daw_ui::{NoticeKind, notice, theme};
use egui::{Context, RawInput, Rect};

#[test]
fn long_notice_text_keeps_actions_and_readable_text_inside_a_small_viewport() {
    for dark in [false, true] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(
            &ctx,
            if dark {
                theme::Appearance::Dark
            } else {
                theme::Appearance::Light
            },
        );
        for kind in [
            NoticeKind::Busy,
            NoticeKind::Success,
            NoticeKind::Warning,
            NoticeKind::Error,
        ] {
            let mut actions = Vec::new();
            let message = "Exported · 音楽 🎹/A long descriptive filename/".repeat(20);
            let output = ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(480.0, 100.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let bounds = ui.available_rect_before_wrap();
                        let response = notice(ui, kind, &message, |ui| {
                            actions.push(daw_ui::button(ui, "Dismiss").rect);
                            actions.push(daw_ui::button(ui, "Copy path").rect);
                        });
                        assert!(bounds.contains_rect(response.response.rect));
                        assert!(actions.iter().all(|rect| bounds.contains_rect(*rect)));
                    });
                },
            );
            let text = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == message => Some(text),
                    _ => None,
                })
                .unwrap();
            assert!(text.galley.elided);
            assert!(text.galley.size().y >= daw_ui::tokens::FONT_BODY);
        }
    }
}

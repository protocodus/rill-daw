use daw_ui::{ErrorDetailsResponse, error_details, theme};
use egui::{Context, Event, FullOutput, Modifiers, Pos2, RawInput, Rect, Vec2};

fn frame(
    ctx: &Context,
    size: Vec2,
    summary: &str,
    details: &str,
    events: Vec<Event>,
) -> (FullOutput, ErrorDetailsResponse) {
    let mut response = None;
    let title = "Error · 夜の音楽 🎹 Extremely long instrument name ".repeat(15);
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                daw_ui::text::body(ui, "Arrangement");
            });
            response = Some(error_details(ctx, &title, summary, details));
        },
    );
    (output, response.unwrap())
}

#[test]
fn long_unicode_diagnostics_scroll_without_hiding_actions_or_eliding_details() {
    let summary = "Unable to load 夜の音楽 🎹 — instrument ".repeat(12);
    let details = format!(
        "Operation: Load instrument\nPath: /Plugins/{}.vst3\n{}\nFinal cause: signal 11",
        "Long_plugin_夜の音楽_🎹_".repeat(20),
        "Plugin initialization failed: nested cause αβγ\n".repeat(80)
    );
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 2.0] {
            for size in [Vec2::new(360.0, 280.0), Vec2::new(1100.0, 720.0)] {
                let ctx = Context::default();
                theme::apply(&ctx);
                theme::set_appearance(&ctx, appearance);
                ctx.set_pixels_per_point(density);
                for _ in 0..3 {
                    frame(&ctx, size, &summary, &details, vec![]);
                }
                let (output, response) = frame(&ctx, size, &summary, &details, vec![]);
                let bounds = Rect::from_min_size(Pos2::ZERO, size);
                assert!(
                    bounds.contains_rect(response.response.rect),
                    "Dialog {:?} exceeds {bounds:?}; details {:?}, copy {:?}, close {:?}",
                    response.response.rect,
                    response.details_viewport,
                    response.copy.rect,
                    response.close.rect
                );
                for action in [&response.copy, &response.close] {
                    assert!(bounds.contains_rect(action.rect));
                    assert!(!action.rect.intersects(response.details_viewport));
                }
                assert!(!response.closed);
                assert!(!response.copied);
                let text = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == details => Some(text),
                        _ => None,
                    })
                    .expect("Complete diagnostic text remains available for rendering/selection");
                assert!(!text.galley.elided);
                assert!(text.galley.size().x <= response.details_viewport.width() + 1.0);
                assert!(text.galley.size().y > response.details_viewport.height());
            }
        }
    }
}

#[test]
fn copy_preserves_the_full_snapshot_and_close_retains_keyboard_behavior() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let size = Vec2::new(420.0, 320.0);
    let summary = "Plugin initialization failed";
    let details = format!(
        "Instrument: 夜の音楽 🎹\n{}\nCaused by: signal 11\n",
        "long diagnostic line ✓\n".repeat(120)
    );
    for _ in 0..3 {
        frame(&ctx, size, summary, &details, vec![]);
    }
    let (_, response) = frame(&ctx, size, summary, &details, vec![]);
    let pos = response.copy.rect.center();
    for pressed in [true, false] {
        let (output, response) = frame(
            &ctx,
            size,
            summary,
            &details,
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        if !pressed {
            assert!(response.copied);
            assert!(!response.closed);
            assert!(output.platform_output.commands.iter().any(|command| {
                matches!(command, egui::OutputCommand::CopyText(text) if text == &details)
            }));
        }
    }
    let (_, response) = frame(&ctx, size, summary, &details, vec![]);
    response.close.request_focus();
    let (_, response) = frame(
        &ctx,
        size,
        summary,
        &details,
        vec![Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(response.closed);
    let (_, response) = frame(
        &ctx,
        size,
        summary,
        &details,
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(response.closed);
}

use daw_ui::{AssistantMessage, AssistantOptions, AssistantResponse, AssistantRole, theme};
use egui::{Context, Event, FullOutput, Key, Modifiers, RawInput, Rect};

fn frame(
    context: &Context,
    prompt: &mut String,
    messages: &[AssistantMessage],
    busy: bool,
    events: Vec<Event>,
    modifiers: Modifiers,
) -> (AssistantResponse, FullOutput) {
    let mut response = None;
    let output = context.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(440.0, 360.0),
            )),
            modifiers,
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                response = Some(daw_ui::assistant(
                    ui,
                    prompt,
                    AssistantOptions {
                        messages,
                        status: busy.then_some("Creating a variation"),
                        busy,
                        can_submit: true,
                        can_undo: true,
                        context: "Bass · Bars 9–16",
                    },
                ));
            });
        },
    );
    (response.unwrap(), output)
}

fn key(modifiers: Modifiers) -> Vec<Event> {
    vec![Event::Key {
        key: Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }]
}

#[test]
fn submit_shortcut_requires_prompt_focus_and_respects_busy_state() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let command = Modifiers {
        ctrl: true,
        command: true,
        ..Modifiers::NONE
    };
    let mut prompt = "Add a counter melody".to_owned();
    let (initial, _) = frame(&ctx, &mut prompt, &[], false, vec![], Modifiers::NONE);
    let (unfocused, _) = frame(&ctx, &mut prompt, &[], false, key(command), command);
    assert!(!unfocused.send);
    initial.prompt.request_focus();
    let (submitted, _) = frame(&ctx, &mut prompt, &[], false, key(command), command);
    assert!(submitted.send);
    assert_eq!(prompt, "Add a counter melody");
    let (busy, _) = frame(&ctx, &mut prompt, &[], true, key(command), command);
    assert!(!busy.send);
    assert!(!busy.send_button.enabled());
    assert!(busy.stop_button.enabled());
    assert!(!busy.undo_button.enabled());
}

#[test]
fn enter_edits_multiline_prompt_and_blank_prompt_cannot_submit() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut prompt = "A phrase".to_owned();
    let (initial, _) = frame(&ctx, &mut prompt, &[], false, vec![], Modifiers::NONE);
    initial.prompt.request_focus();
    let (edited, _) = frame(
        &ctx,
        &mut prompt,
        &[],
        false,
        key(Modifiers::NONE),
        Modifiers::NONE,
    );
    assert!(!edited.send);
    assert!(prompt.contains('\n'));
    prompt = " \n \t".to_owned();
    let (empty, _) = frame(&ctx, &mut prompt, &[], false, vec![], Modifiers::NONE);
    assert!(!empty.send_button.enabled());
    assert!(!empty.stop_button.enabled());
    assert!(empty.undo_button.enabled());
}

#[test]
fn long_history_and_unicode_prompt_keep_actions_in_viewport() {
    for dark in [false, true] {
        for density in [1.0, 2.0] {
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
            ctx.set_pixels_per_point(density);
            // egui applies a changed density on the first frame before accepting
            // the next viewport rectangle. Warm it up before testing geometry.
            let mut warmup = String::new();
            frame(&ctx, &mut warmup, &[], false, vec![], Modifiers::NONE);
            let mut messages = vec![AssistantMessage {
                role: AssistantRole::Assistant,
                text: "old-transcript-marker".into(),
            }];
            messages.extend((0..80).map(|index| AssistantMessage {
                role: AssistantRole::Tool,
                text: format!("Action {index}: {}", "音楽 phrase ".repeat(100)),
            }));
            let mut prompt = "音楽\n".repeat(daw_ui::ASSISTANT_PROMPT_LIMIT);
            for _ in 0..2 {
                let (response, output) =
                    frame(&ctx, &mut prompt, &messages, true, vec![], Modifiers::NONE);
                let viewport = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(440.0, 360.0));
                for action in [
                    response.send_button,
                    response.stop_button,
                    response.undo_button,
                ] {
                    assert!(
                        viewport.contains_rect(action.rect),
                        "action {:?}, prompt {:?}",
                        action.rect,
                        response.prompt.rect
                    );
                }
                assert_eq!(prompt.chars().count(), daw_ui::ASSISTANT_PROMPT_LIMIT);
                assert!(!output.shapes.iter().any(|shape| match &shape.shape {
                    egui::Shape::Text(text) => {
                        text.galley.job.text.contains("old-transcript-marker")
                    }
                    _ => false,
                }));
            }
        }
    }
}

#[test]
fn secret_field_paints_masked_text_only() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let key = "sk-secret-credential-12345";
    let mut value = key.to_owned();
    let output = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            daw_ui::secret_field(ui, &mut value, "API key", 240.0);
        });
    });
    assert_eq!(value, key);
    assert!(!output.shapes.iter().any(|shape| match &shape.shape {
        egui::Shape::Text(text) => text.galley.job.text.contains(key),
        _ => false,
    }));
}

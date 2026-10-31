use daw_ui::{AssistantOptions, AssistantResponse, AssistantState, AssistantSubmitState};
use egui::{Context, Event, FullOutput, Key, Modifiers, RawInput, Rect};

#[derive(Clone, Copy)]
struct Scenario<'a> {
    messages: &'a [daw_ui::AssistantMessage],
    submit: AssistantSubmitState<'a>,
    failure: Option<&'a str>,
    recover: bool,
    busy: bool,
}

impl Default for Scenario<'_> {
    fn default() -> Self {
        Self {
            messages: &[],
            submit: AssistantSubmitState::Ready,
            failure: None,
            recover: false,
            busy: false,
        }
    }
}

fn frame(
    ctx: &Context,
    prompt: &mut String,
    scenario: Scenario<'_>,
    events: Vec<Event>,
    modifiers: Modifiers,
) -> (AssistantResponse, FullOutput) {
    let mut response = None;
    let state_id = egui::Id::new("readiness_fixture_state");
    let mut state = ctx
        .data(|data| data.get_temp::<AssistantState>(state_id))
        .unwrap_or_default();
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(440.0, 360.0),
            )),
            events,
            modifiers,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                response = Some(daw_ui::assistant(
                    ui,
                    prompt,
                    &mut state,
                    AssistantOptions {
                        messages: scenario.messages,
                        status: scenario.busy.then_some("Working…"),
                        busy: scenario.busy,
                        submit_state: scenario.submit,
                        failure: scenario.failure,
                        recover_prompt: scenario.recover,
                        can_undo: false,
                        context: "112 BPM · 4/4",
                        scope: "Song",
                        provider: "Example provider",
                        configured: scenario.submit != AssistantSubmitState::SetupRequired,
                        can_reuse: true,
                        examples: &[],
                        tips: &[],
                        tools: &[],
                    },
                ));
            });
        },
    );
    ctx.data_mut(|data| data.insert_temp(state_id, state));
    (response.unwrap(), output)
}

fn click(
    ctx: &Context,
    prompt: &mut String,
    scenario: Scenario<'_>,
    rect: Rect,
) -> AssistantResponse {
    let pos = rect.center();
    let pointer = |pressed| Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(
        ctx,
        prompt,
        scenario,
        vec![Event::PointerMoved(pos), pointer(true)],
        Modifiers::NONE,
    );
    frame(ctx, prompt, scenario, vec![pointer(false)], Modifiers::NONE).0
}

fn painted(output: &FullOutput, text: &str) -> bool {
    output.shapes.iter().any(|shape| match &shape.shape {
        egui::Shape::Text(galley) => galley.galley.text().contains(text),
        _ => false,
    })
}

#[test]
fn connect_is_actionable_without_a_draft_and_blocked_shortcut_cannot_send() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut prompt = String::new();
    let setup = Scenario {
        submit: AssistantSubmitState::SetupRequired,
        ..Default::default()
    };
    frame(&ctx, &mut prompt, setup, vec![], Modifiers::NONE);
    let (response, output) = frame(&ctx, &mut prompt, setup, vec![], Modifiers::NONE);
    assert!(painted(&output, "Connect AI"));
    assert!(response.send_button.enabled());
    let connected = click(&ctx, &mut prompt, setup, response.send_button.rect);
    assert!(connected.settings);
    assert!(!connected.send);
    assert!(prompt.is_empty());

    prompt = "Make a gentler variation".into();
    let blocked = Scenario {
        submit: AssistantSubmitState::Blocked("Finish recording before sending a request."),
        ..Default::default()
    };
    let (response, output) = frame(&ctx, &mut prompt, blocked, vec![], Modifiers::NONE);
    assert!(painted(&output, "Finish recording"));
    assert!(!response.send_button.enabled());
    response.prompt.request_focus();
    let command = Modifiers {
        ctrl: true,
        command: true,
        ..Modifiers::NONE
    };
    let (response, _) = frame(
        &ctx,
        &mut prompt,
        blocked,
        vec![Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: command,
        }],
        command,
    );
    assert!(!response.send && !response.settings);
    assert_eq!(prompt, "Make a gentler variation");
}

#[test]
fn recovery_only_requests_editing_and_never_replaces_a_draft_or_stops_a_run() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let mut prompt = String::new();
    let recovery = Scenario {
        failure: Some("The provider is unavailable."),
        recover: true,
        ..Default::default()
    };
    frame(&ctx, &mut prompt, recovery, vec![], Modifiers::NONE);
    let (response, output) = frame(&ctx, &mut prompt, recovery, vec![], Modifiers::NONE);
    assert!(painted(&output, "Edit last prompt"));
    let edited = click(&ctx, &mut prompt, recovery, response.stop_button.rect);
    assert!(edited.reuse_last);
    assert!(!edited.send && !edited.stop && !edited.settings);
    assert!(
        prompt.is_empty(),
        "The host owns restoring the previous prompt"
    );

    prompt = "Keep this new idea".into();
    let (response, output) = frame(&ctx, &mut prompt, recovery, vec![], Modifiers::NONE);
    assert!(!painted(&output, "Edit last prompt"));
    assert!(!response.stop_button.enabled());
    assert_eq!(prompt, "Keep this new idea");

    prompt.clear();
    let busy = Scenario {
        busy: true,
        ..recovery
    };
    let (response, output) = frame(&ctx, &mut prompt, busy, vec![], Modifiers::NONE);
    assert!(!painted(&output, "Edit last prompt"));
    let stopped = click(&ctx, &mut prompt, busy, response.stop_button.rect);
    assert!(stopped.stop);
    assert!(!stopped.reuse_last && !stopped.send);
}

#[test]
fn long_failure_details_leave_prompt_and_recovery_visible_in_both_themes() {
    let failure = format!(
        "Connection details: {} End of diagnostic.",
        "音楽 provider detail. ".repeat(100)
    );
    for appearance in [
        daw_ui::theme::Appearance::Light,
        daw_ui::theme::Appearance::Dark,
    ] {
        for density in [1.0, 2.0] {
            let ctx = Context::default();
            daw_ui::theme::apply(&ctx);
            daw_ui::theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            let mut prompt = String::new();
            let recovery = Scenario {
                failure: Some(&failure),
                recover: true,
                ..Default::default()
            };
            for _ in 0..2 {
                frame(&ctx, &mut prompt, recovery, vec![], Modifiers::NONE);
            }
            let (response, output) = frame(&ctx, &mut prompt, recovery, vec![], Modifiers::NONE);
            let viewport = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(440.0, 360.0));
            for rect in [
                response.send_button.rect,
                response.stop_button.rect,
                response.undo_button.rect,
                response.prompt.interact_rect,
            ] {
                assert!(
                    viewport.contains_rect(rect),
                    "Control escaped at {appearance:?}/{density}: {rect:?}"
                );
            }
            assert!(
                painted(&output, "End of diagnostic."),
                "The body must retain readable diagnostic text, not truncate it to one line"
            );
            assert!(painted(&output, "Edit last prompt"));
        }
    }
}

#[test]
fn new_failure_reveals_once_and_manual_scrolling_survives_until_another_attempt() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    ctx.style_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
    let messages: Vec<_> = (0..8)
        .map(|index| daw_ui::AssistantMessage {
            role: daw_ui::AssistantRole::User,
            text: format!("Older thought {index} about this phrase."),
        })
        .collect();
    let failure = "The provider lost its connection.";
    let scenario = Scenario {
        messages: &messages,
        failure: Some(failure),
        recover: true,
        ..Default::default()
    };
    let mut prompt = String::new();
    let visible_rect = |output: &FullOutput, label: &str| {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                let rect = text
                    .galley
                    .rect
                    .translate(text.pos.to_vec2())
                    .intersect(shape.clip_rect);
                rect.is_positive().then_some(rect)
            }
            _ => None,
        })
    };
    for _ in 0..3 {
        frame(&ctx, &mut prompt, scenario, vec![], Modifiers::NONE);
    }
    let (_, revealed) = frame(&ctx, &mut prompt, scenario, vec![], Modifiers::NONE);
    let detail = visible_rect(&revealed, failure).expect("New failure is revealed");
    frame(
        &ctx,
        &mut prompt,
        scenario,
        vec![
            Event::PointerMoved(detail.center()),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 1000.0),
                modifiers: Modifiers::NONE,
            },
        ],
        Modifiers::NONE,
    );
    for _ in 0..12 {
        frame(&ctx, &mut prompt, scenario, vec![], Modifiers::NONE);
    }
    let (_, scrolled) = frame(&ctx, &mut prompt, scenario, vec![], Modifiers::NONE);
    assert!(visible_rect(&scrolled, "Older thought 0 about this phrase.").is_some());
    assert!(
        visible_rect(&scrolled, failure).is_none(),
        "Steady frames must respect manual scrolling"
    );

    ctx.data_mut(|data| {
        let id = egui::Id::new("readiness_fixture_state");
        let mut state = data.get_temp::<AssistantState>(id).unwrap();
        state.reset_failure_reveal();
        data.insert_temp(id, state);
    });
    for _ in 0..3 {
        frame(&ctx, &mut prompt, scenario, vec![], Modifiers::NONE);
    }
    let (_, repeated) = frame(&ctx, &mut prompt, scenario, vec![], Modifiers::NONE);
    assert!(
        visible_rect(&repeated, failure).is_some(),
        "An identical failure from a new attempt is revealed again"
    );
}

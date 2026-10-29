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
                    &mut daw_ui::AssistantState::default(),
                    AssistantOptions {
                        messages,
                        status: busy.then_some("Creating a variation"),
                        busy,
                        can_submit: true,
                        can_undo: true,
                        context: "Bass · Bars 9–16",
                        scope: "Global · Entire song",
                        provider: "Example provider",
                        configured: true,
                        can_reuse: false,
                        examples: &[],
                        tips: &[],
                        tools: &[],
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

const EXAMPLES: [daw_ui::AssistantExample<'static>; 2] = [
    daw_ui::AssistantExample {
        id: "progression",
        category: "Compose",
        title: "Start a progression",
        description: "Build a four-bar musical idea with chords and bass.",
        prompt: "Create four bars in C minor, keeping existing material unchanged.",
    },
    daw_ui::AssistantExample {
        id: "space",
        category: "Mix",
        title: "Make space",
        description: "Inspect the current mix and suggest a focused next step.",
        prompt: "Inspect the current mix and suggest one improvement without editing it.",
    },
];

const TOOLS: [daw_ui::AssistantTool<'static>; 2] = [
    daw_ui::AssistantTool {
        name: "read_notes",
        title: "Inspect notes",
        description: "Read the notes in this piano roll before suggesting an edit.",
        contexts: "Global, Piano roll",
        prompt: "Inspect the notes in this piano roll and suggest one improvement.",
    },
    daw_ui::AssistantTool {
        name: "transpose_notes",
        title: "Transpose notes",
        description: "Move notes by a musical interval within this piano roll.",
        contexts: "Global, Piano roll",
        prompt: "Transpose this piano roll up one octave. Keep its timing unchanged.",
    },
];

fn guided_frame(
    context: &Context,
    prompt: &mut String,
    state: &mut daw_ui::AssistantState,
    size: egui::Vec2,
    busy: bool,
    events: Vec<Event>,
) -> (AssistantResponse, FullOutput) {
    let mut response = None;
    let output = context.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                response = Some(daw_ui::assistant(
                    ui,
                    prompt,
                    state,
                    AssistantOptions {
                        messages: &[],
                        status: busy.then_some("Building a progression"),
                        busy,
                        can_submit: true,
                        can_undo: true,
                        context: "Glass Current · 112 BPM · 4/4 · Bass · Phrase · 8 notes · Beats 8–16",
                        scope: "Piano roll · Bass / Phrase",
                        provider: "Example provider",
                        configured: false,
                        can_reuse: true,
                        examples: &EXAMPLES,
                        tools: &TOOLS,
                        tips: &[
                            daw_ui::AssistantTip {
                                title: "Give a musical brief",
                                body: "Include the style, key, length, and the part you want to develop. Describe what should stay unchanged. Choose an example and edit the draft before sending it.",
                            },
                            daw_ui::AssistantTip {
                                title: "Listen, then refine",
                                body: "Audition a result and describe one focused change at a time. Use Undo to reverse ordinary song edits. A measurement of audio is not the same as a model hearing it.",
                            },
                        ],
                    },
                ));
            });
        },
    );
    (response.unwrap(), output)
}

fn label_position(output: &FullOutput, label: &str) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                let rect = text.galley.rect.translate(text.pos.to_vec2());
                shape
                    .clip_rect
                    .contains(rect.center())
                    .then_some(rect.center())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("No visible label {label:?}"))
}

fn click_guide(
    ctx: &Context,
    prompt: &mut String,
    state: &mut daw_ui::AssistantState,
    label: &str,
) -> AssistantResponse {
    let size = egui::vec2(620.0, 520.0);
    let (_, output) = guided_frame(ctx, prompt, state, size, false, vec![]);
    let position = label_position(&output, label);
    let pointer = |pressed| Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    guided_frame(
        ctx,
        prompt,
        state,
        size,
        false,
        vec![Event::PointerMoved(position), pointer(true)],
    );
    guided_frame(ctx, prompt, state, size, false, vec![pointer(false)]).0
}

#[test]
fn browsing_and_inserting_an_example_preserves_draft_and_requires_send() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut state = daw_ui::AssistantState::default();
    let mut prompt = "Keep the melody unchanged.".to_owned();
    let response = click_guide(&ctx, &mut prompt, &mut state, "Browse examples");
    assert!(!response.send);
    assert_eq!(state.page, daw_ui::AssistantPage::Examples);
    assert_eq!(prompt, "Keep the melody unchanged.");
    click_guide(&ctx, &mut prompt, &mut state, "Mix");
    assert_eq!(state.category, "Mix");
    let inserted = click_guide(&ctx, &mut prompt, &mut state, "Make space");
    assert_eq!(inserted.inserted_example.as_deref(), Some("space"));
    assert!(!inserted.send);
    assert_eq!(
        prompt,
        format!("Keep the melody unchanged.\n\n{}", EXAMPLES[1].prompt)
    );
    assert_eq!(state.page, daw_ui::AssistantPage::Conversation);
    assert!(ctx.memory(|memory| memory.has_focus(inserted.prompt.id)));
}

#[test]
fn oversized_example_never_replaces_or_truncates_an_existing_draft() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut state = daw_ui::AssistantState {
        page: daw_ui::AssistantPage::Examples,
        category: "Compose".into(),
        ..Default::default()
    };
    let original = "音".repeat(daw_ui::ASSISTANT_PROMPT_LIMIT - 1);
    let mut prompt = original.clone();
    let response = click_guide(&ctx, &mut prompt, &mut state, "Start a progression");
    assert!(response.inserted_example.is_none());
    assert_eq!(prompt, original);
    assert!(!response.send);
    assert_eq!(state.page, daw_ui::AssistantPage::Examples);
}

#[test]
fn scoped_tool_browser_selects_a_tool_and_preserves_an_unsent_draft() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut state = daw_ui::AssistantState::default();
    let mut prompt = "Keep the rhythm.".to_owned();
    let (_, initial) = guided_frame(
        &ctx,
        &mut prompt,
        &mut state,
        egui::vec2(620.0, 520.0),
        false,
        vec![],
    );
    let scope = label_position(&initial, "Scope · Piano roll · Bass / Phrase");
    let selection = label_position(
        &initial,
        "Glass Current · 112 BPM · 4/4 · Bass · Phrase · 8 notes · Beats 8–16",
    );
    assert!(scope.y < selection.y);
    assert!(!click_guide(&ctx, &mut prompt, &mut state, "Tools").send);
    assert_eq!(state.page, daw_ui::AssistantPage::Tools);
    assert_eq!(state.selected_tool.as_deref(), Some("read_notes"));
    click_guide(&ctx, &mut prompt, &mut state, "Transpose notes");
    assert_eq!(state.selected_tool.as_deref(), Some("transpose_notes"));
    let inserted = click_guide(&ctx, &mut prompt, &mut state, "Use tool");
    assert_eq!(inserted.inserted_tool.as_deref(), Some("transpose_notes"));
    assert_eq!(prompt, format!("Keep the rhythm.\n\n{}", TOOLS[1].prompt));
    assert!(!inserted.send);
    assert_eq!(state.page, daw_ui::AssistantPage::Conversation);
    assert!(ctx.memory(|memory| memory.has_focus(inserted.prompt.id)));
}

#[test]
fn tool_search_filters_supplied_tools_and_clears_stale_selection() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut state = daw_ui::AssistantState {
        page: daw_ui::AssistantPage::Tools,
        ..Default::default()
    };
    let mut prompt = "Keep the draft.".to_owned();
    click_guide(&ctx, &mut prompt, &mut state, "Search tools…");
    let (_, filtered) = guided_frame(
        &ctx,
        &mut prompt,
        &mut state,
        egui::vec2(620.0, 520.0),
        false,
        vec![Event::Text("TRANSPOSE piano".into())],
    );
    assert_eq!(state.selected_tool.as_deref(), Some("transpose_notes"));
    label_position(&filtered, "1 tool");
    assert!(!filtered.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.text() == "read_notes")));
    let (response, empty) = guided_frame(
        &ctx,
        &mut prompt,
        &mut state,
        egui::vec2(620.0, 520.0),
        false,
        vec![Event::Text(" unavailable".into())],
    );
    assert!(state.selected_tool.is_none());
    assert!(response.inserted_tool.is_none());
    label_position(
        &empty,
        "No matching tools. Clear the search to browse this scope.",
    );
    click_guide(&ctx, &mut prompt, &mut state, "Clear");
    assert!(state.tool_query.is_empty());
    assert_eq!(state.selected_tool.as_deref(), Some("read_notes"));
    assert_eq!(prompt, "Keep the draft.");
}

#[test]
fn oversized_tool_prompt_never_clobbers_a_draft() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let mut state = daw_ui::AssistantState {
        page: daw_ui::AssistantPage::Tools,
        ..Default::default()
    };
    let original = "音".repeat(daw_ui::ASSISTANT_PROMPT_LIMIT - 1);
    let mut prompt = original.clone();
    let response = click_guide(&ctx, &mut prompt, &mut state, "Use tool");
    assert!(response.inserted_tool.is_none());
    assert_eq!(prompt, original);
    assert!(!response.send);
    assert_eq!(state.page, daw_ui::AssistantPage::Tools);
}

#[test]
fn all_guidance_pages_keep_composer_and_actions_inside_compact_viewport() {
    for appearance in [theme::Appearance::Dark, theme::Appearance::Light] {
        for density in [1.0, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            let size = egui::vec2(440.0, 360.0);
            let viewport = Rect::from_min_size(egui::Pos2::ZERO, size);
            let mut prompt = "Keep my draft.".to_owned();
            let mut state = daw_ui::AssistantState::default();
            guided_frame(&ctx, &mut prompt, &mut state, size, false, vec![]);
            for page in [
                daw_ui::AssistantPage::Conversation,
                daw_ui::AssistantPage::Examples,
                daw_ui::AssistantPage::Tools,
                daw_ui::AssistantPage::Tips,
            ] {
                state.page = page;
                for busy in [false, true] {
                    let (response, output) =
                        guided_frame(&ctx, &mut prompt, &mut state, size, busy, vec![]);
                    for action in [
                        response.send_button,
                        response.stop_button,
                        response.undo_button,
                    ] {
                        assert!(
                            viewport.contains_rect(action.rect),
                            "{appearance:?}, {density}, {page:?}: action {:?}",
                            action.rect
                        );
                    }
                    assert!(viewport.contains_rect(response.prompt.interact_rect));
                    label_position(&output, "AI settings");
                    label_position(&output, "Conversation");
                    label_position(&output, "Examples");
                    label_position(&output, "Tools");
                    label_position(&output, "Tips");
                    assert_eq!(prompt, "Keep my draft.");
                }
            }
        }
    }
}

#[test]
fn popup_browsing_actions_fit_below_reserved_transport_at_minimum_app_size() {
    for appearance in [theme::Appearance::Dark, theme::Appearance::Light] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(&ctx, appearance);
        let mut prompt = String::new();
        let mut state = daw_ui::AssistantState::default();
        for page in [
            daw_ui::AssistantPage::Conversation,
            daw_ui::AssistantPage::Tools,
        ] {
            state.page = page;
            let mut painted = false;
            for _ in 0..3 {
                let output = ctx.run(RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                    ..Default::default()
                }, |ctx| {
                    egui::TopBottomPanel::top("transport").exact_height(160.0).show(ctx, |_| {});
                    daw_ui::popup("Assistant", ctx)
                        .default_width(620.0)
                        .default_height(520.0)
                        .vscroll(false)
                        .show(ctx, |ui| {
                            daw_ui::assistant(ui, &mut prompt, &mut state, AssistantOptions {
                                messages: &[], status: None, busy: false,
                                can_submit: false, can_undo: false,
                                scope: "Piano roll · Keys / Phrase",
                                context: "120 BPM · 4/4 · Context stays fixed when selection changes",
                                provider: "Example provider", configured: false, can_reuse: false,
                                examples: &EXAMPLES, tools: &TOOLS, tips: &[],
                            });
                        });
                });
                // The native popup has an opening frame before it paints.
                if output.shapes.iter().any(|shape| {
                    matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == "AI prompt")
                }) {
                    painted = true;
                    label_position(
                        &output,
                        if page == daw_ui::AssistantPage::Conversation {
                            "Browse examples"
                        } else {
                            "Use tool"
                        },
                    );
                    label_position(&output, "Send");
                }
            }
            assert!(painted, "Assistant popup never painted {page:?}");
        }
    }
}

//! A bounded assistant workspace, independent of agent providers and song state.
use crate::{text, tokens};
use egui::{Response, Ui};

/// Bound interactive text layout independently of provider response limits.
pub const ASSISTANT_PROMPT_LIMIT: usize = 16_384;
const MESSAGE_LIMIT: usize = 64;
const MESSAGE_CHAR_LIMIT: usize = 8_192;
const GUIDANCE_LIMIT: usize = 64;
const TOOL_LIMIT: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistantRole {
    User,
    Assistant,
    Tool,
}

impl AssistantRole {
    fn label(self) -> &'static str {
        match self {
            Self::User => "You",
            Self::Assistant => "Assistant",
            Self::Tool => "Activity",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantMessage {
    pub role: AssistantRole,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AssistantPage {
    #[default]
    Conversation,
    Examples,
    Tools,
    Tips,
}

/// Presentation state stays separate from the caller's conversation and song.
#[derive(Clone, Debug, Default)]
pub struct AssistantState {
    pub page: AssistantPage,
    pub category: String,
    pub tool_query: String,
    pub selected_tool: Option<String>,
    last_revealed_failure: Option<String>,
}

impl AssistantState {
    /// A new failed attempt should reveal its explanation even if it has the
    /// same message as the previous attempt or finished while the window closed.
    pub fn reset_failure_reveal(&mut self) {
        self.last_revealed_failure = None;
    }
}

/// Provider-independent example content supplied by the host.
pub struct AssistantExample<'a> {
    pub id: &'a str,
    pub category: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    pub prompt: &'a str,
}

pub struct AssistantTip<'a> {
    pub title: &'a str,
    pub body: &'a str,
}

/// Tool discovery is owned by the host. Supply only tools authorized for the
/// pinned scope; this presentation never expands that capability set.
pub struct AssistantTool<'a> {
    pub name: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    pub contexts: &'a str,
    pub prompt: &'a str,
}

/// The next step belongs to the host's current musical context. Missing setup
/// offers an action; a temporary block explains why a request cannot run yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistantSubmitState<'a> {
    Ready,
    SetupRequired,
    Blocked(&'a str),
}

/// The application owns history, execution state, and the captured song context.
pub struct AssistantOptions<'a> {
    pub messages: &'a [AssistantMessage],
    pub status: Option<&'a str>,
    pub busy: bool,
    pub submit_state: AssistantSubmitState<'a>,
    /// Failed requests stay readable in the scrolling conversation, leaving the
    /// composer and its recovery action available even in a short window.
    pub failure: Option<&'a str>,
    pub recover_prompt: bool,
    pub can_undo: bool,
    pub context: &'a str,
    /// A pinned edit boundary, distinct from the changing musical selection.
    pub scope: &'a str,
    pub provider: &'a str,
    pub configured: bool,
    /// Reusing a prompt must not silently replace an unsent draft.
    pub can_reuse: bool,
    pub examples: &'a [AssistantExample<'a>],
    pub tips: &'a [AssistantTip<'a>],
    pub tools: &'a [AssistantTool<'a>],
}

pub struct AssistantResponse {
    pub send: bool,
    pub stop: bool,
    pub undo: bool,
    pub settings: bool,
    pub new_chat: bool,
    pub reuse_last: bool,
    /// Insertion edits only the draft. The host may use this ID for diagnostics.
    pub inserted_example: Option<String>,
    pub inserted_tool: Option<String>,
    pub prompt: Response,
    pub send_button: Response,
    pub stop_button: Response,
    pub undo_button: Response,
}

/// Draw a scrollable conversation, examples, tools, or tips above a fixed composer.
/// Examples append complete prompts without replacing a draft or sending it.
/// Enter inserts a newline; Ctrl/Cmd+Enter submits only while the prompt owns
/// focus. The caller handles execution, settings, reuse, and conversation reset.
pub fn assistant(
    ui: &mut Ui,
    prompt: &mut String,
    state: &mut AssistantState,
    options: AssistantOptions<'_>,
) -> AssistantResponse {
    truncate_chars(prompt, ASSISTANT_PROMPT_LIMIT);
    let mut settings = false;
    let mut new_chat = false;
    let mut reuse_last = false;
    let mut inserted_example = None;
    let mut inserted_tool = None;
    let compact_layout = ui.available_height() < tokens::CONTROL_HEIGHT * 13.0;
    let blocked = match options.submit_state {
        AssistantSubmitState::Blocked(reason) if !options.busy => Some(reason),
        _ => None,
    };
    let show_status = options.busy
        || blocked.is_some()
        || options.failure.is_some()
        || options.status.is_some_and(|status| !status.is_empty());
    if options.failure.is_none() {
        state.reset_failure_reveal();
    }
    crate::action_row(
        ui,
        |ui| {
            crate::menu_button(ui, "More", |ui| {
                let reuse = ui
                    .add_enabled_ui(
                        options.can_reuse && prompt.is_empty() && !options.busy,
                        |ui| crate::button(ui, "Edit last prompt"),
                    )
                    .inner;
                if crate::hover_text(
                    reuse,
                    "Bring the last prompt back for editing; it is not sent.",
                )
                .clicked()
                {
                    reuse_last = true;
                    state.page = AssistantPage::Conversation;
                    ui.close();
                }
                let can_reset = !options.busy
                    && (!options.messages.is_empty()
                        || options.status.is_some_and(|status| !status.is_empty()));
                let reset = ui
                    .add_enabled_ui(can_reset, |ui| crate::button(ui, "New chat"))
                    .inner;
                if crate::hover_text(
                    reset,
                    "Clear the conversation. Keep the song and unsent draft.",
                )
                .clicked()
                {
                    new_chat = true;
                    ui.close();
                }
            });
            settings = ui
                .add_enabled_ui(!options.busy, |ui| crate::ai_button(ui, "AI settings"))
                .inner
                .on_hover_text(if options.configured {
                    format!("{} · Connected", bounded(options.provider, 120))
                } else {
                    "Choose a provider. Your draft stays here.".into()
                })
                .clicked();
        },
        |ui| {
            let scope = format!("Scope · {}", bounded(options.scope, 2_048));
            text::truncated(ui, text::TextRole::Label, scope)
                .on_hover_text(bounded(options.context, 2_048));
        },
    );
    // Status shares the header's context line. Keeping it out of the fixed
    // composer leaves room for readable diagnostics on a short display.
    // The complete musical context remains available on the scope label.
    if show_status {
        ui.horizontal(|ui| {
            if options.busy {
                ui.spinner();
            }
            let message = if let Some(reason) = blocked {
                reason
            } else if options.failure.is_some() {
                "Request interrupted · details in Conversation"
            } else {
                options
                    .status
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Working…")
            };
            text::truncated(ui, text::TextRole::Note, bounded(message, 512));
        });
    } else if !compact_layout && !options.context.is_empty() {
        text::truncated(ui, text::TextRole::Note, bounded(options.context, 2_048));
    }
    ui.horizontal_wrapped(|ui| {
        for (page, label) in [
            (AssistantPage::Conversation, "Conversation"),
            (AssistantPage::Examples, "Examples"),
            (AssistantPage::Tools, "Tools"),
            (AssistantPage::Tips, "Tips"),
        ] {
            if crate::tab_choice(ui, state.page == page, label).clicked() {
                state.page = page;
            }
        }
    });
    if !compact_layout {
        ui.separator();
    }
    ui.add_space(tokens::SPACE_SM);

    // Browsing needs room for choices, while the conversation benefits from a
    // taller composer. A short popup keeps one editable line and scrolls the
    // complete draft without displacing the scope or fixed actions.
    let compact_composer = compact_layout
        || state.page != AssistantPage::Conversation
        || ui.available_height() < tokens::CONTROL_HEIGHT * 9.0;
    let composer_height = tokens::CONTROL_HEIGHT * if compact_composer { 1.0 } else { 2.0 };
    let prompt_id = ui.make_persistent_id("assistant_prompt");
    let mut footer = crate::popup_footer(ui, "assistant_composer");
    let window_ui = ui;
    let ui = footer.body();
    let body_height = ui.available_height().max(0.0);
    if body_height > 0.0 {
        let page = state.page;
        egui::ScrollArea::vertical()
            .id_salt(("assistant_body", page))
            .max_height(body_height)
            .min_scrolled_height(0.0)
            .auto_shrink([false, false])
            .stick_to_bottom(page == AssistantPage::Conversation && !options.messages.is_empty() && options.failure.is_none())
            .show(ui, |ui| match page {
                AssistantPage::Conversation => {
                    if options.messages.is_empty() && options.failure.is_none() {
                        inserted_example = welcome(ui, prompt, options.examples, body_height);
                    } else {
                        transcript(ui, options.messages);
                    }
                    if let Some(failure) = options.failure {
                        crate::notice(ui, crate::NoticeKind::Error, "Couldn’t finish this request", |_| {});
                        let failure = bounded(failure, MESSAGE_CHAR_LIMIT);
                        let detail = text::body(ui, failure);
                        if state.last_revealed_failure.as_deref() != Some(failure) {
                            detail.scroll_to_me(Some(egui::Align::TOP));
                            state.last_revealed_failure = Some(failure.to_owned());
                        }
                        text::note(ui, "Edits already made are kept. Review them before sending another request.");
                    }
                }
                AssistantPage::Examples => {
                    inserted_example = examples(ui, prompt, state, options.examples);
                }
                AssistantPage::Tools => {
                    inserted_tool = tool_browser(ui, prompt, state, options.tools);
                }
                AssistantPage::Tips => {
                    text::subtitle(ui, "Better prompts, better music");
                    for (index, tip) in options.tips.iter().take(GUIDANCE_LIMIT).enumerate() {
                        ui.push_id(index, |ui| {
                            text::label(ui, bounded(tip.title, 160));
                            text::body(ui, bounded(tip.body, 2_048));
                            ui.add_space(tokens::SPACE_SM);
                        });
                    }
                }
            });
    }
    footer
        .show(window_ui, |ui| {
            let can_send = options.submit_state == AssistantSubmitState::Ready
                && !options.busy
                && !prompt.trim().is_empty();
            let shortcut = can_send
                && ui.is_enabled()
                && ui.memory(|memory| memory.has_focus(prompt_id))
                && ui.input_mut(|input| {
                    input.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
                        || input.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)
                });
            let mut composer = |ui: &mut Ui, label_id| {
                egui::ScrollArea::vertical()
                    .id_salt("assistant_composer_scroll")
                    .max_height(composer_height)
                    .min_scrolled_height(composer_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(prompt)
                                .id(prompt_id)
                                .hint_text("Describe the music or edit…")
                                .desired_width(ui.available_width())
                                .desired_rows(if compact_composer { 1 } else { 3 })
                                .char_limit(ASSISTANT_PROMPT_LIMIT)
                                .font(text::TextRole::Body.font())
                                .margin(egui::vec2(
                                    tokens::BUTTON_PADDING_X,
                                    tokens::BUTTON_PADDING_Y,
                                )),
                        )
                        .labelled_by(label_id)
                    })
                    .inner
            };
            let prompt_response = if compact_layout {
                ui.horizontal(|ui| {
                    let label = text::label(ui, "AI prompt");
                    composer(ui, label.id)
                })
                .inner
            } else {
                let label = text::label(ui, "AI prompt");
                composer(ui, label.id)
            };
            if inserted_example.is_some() || inserted_tool.is_some() || reuse_last {
                prompt_response.request_focus();
                if let Some(mut edit_state) = egui::TextEdit::load_state(ui.ctx(), prompt_id) {
                    edit_state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::one(
                            egui::text::CCursor::new(prompt.chars().count()),
                        )));
                    edit_state.store(ui.ctx(), prompt_id);
                }
            }
            ui.add_space(tokens::SPACE_SM);
            let setup_required =
                options.submit_state == AssistantSubmitState::SetupRequired && !options.busy;
            let recover_prompt =
                options.recover_prompt && options.can_reuse && prompt.is_empty() && !options.busy;
            let (send_button, (stop_button, undo_button)) = crate::action_row(
                ui,
                |ui| {
                    let can_send = options.submit_state == AssistantSubmitState::Ready
                        && !options.busy
                        && !prompt.trim().is_empty();
                    let send = ui
                        .add_enabled_ui(can_send || setup_required, |ui| {
                            crate::ai_button(ui, if setup_required { "Connect AI" } else { "Send" })
                        })
                        .inner;
                    crate::hover_text(
                        send,
                        if setup_required {
                            "Choose a provider in AI settings. Your draft stays here."
                        } else {
                            "Send prompt · Ctrl/Cmd+Enter"
                        },
                    )
                },
                |ui| {
                    let stop = if options.busy || recover_prompt {
                        crate::button(
                            ui,
                            if recover_prompt {
                                "Edit last prompt"
                            } else {
                                "Stop"
                            },
                        )
                    } else {
                        hidden_action(ui)
                    };
                    let stop = crate::hover_text(
                        stop,
                        if recover_prompt {
                            "Restore the last request for editing. Nothing is sent."
                        } else {
                            "Stop further assistant work. Keep edits already made."
                        },
                    );
                    let undo = if options.can_undo && !options.busy {
                        crate::button(ui, "Undo")
                    } else {
                        hidden_action(ui)
                    };
                    (stop, undo)
                },
            )
            .inner;
            if setup_required && send_button.clicked() {
                settings = true;
            }
            if recover_prompt && stop_button.clicked() {
                reuse_last = true;
                state.page = AssistantPage::Conversation;
            }
            AssistantResponse {
                send: shortcut || (!setup_required && send_button.clicked()),
                stop: !recover_prompt && stop_button.clicked(),
                undo: undo_button.clicked(),
                settings,
                new_chat,
                reuse_last,
                inserted_example,
                inserted_tool,
                prompt: prompt_response,
                send_button,
                stop_button,
                undo_button,
            }
        })
        .inner
}

// Keep response IDs available to hosts without showing unavailable actions or
// registering a focusable target in the footer.
fn hidden_action(ui: &mut Ui) -> Response {
    ui.add_enabled_ui(false, |ui| {
        ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover())
    })
    .inner
}

fn welcome(
    ui: &mut Ui,
    prompt: &mut String,
    examples: &[AssistantExample<'_>],
    body_height: f32,
) -> Option<String> {
    let examples = &examples[..examples.len().min(GUIDANCE_LIMIT)];
    let mut starters: Vec<&AssistantExample<'_>> = Vec::new();
    // Offer distinct musical starting points, preserving the host's scope and
    // ordering. A smaller catalog may still supply up to three useful choices.
    for example in examples {
        if !starters
            .iter()
            .any(|chosen| chosen.category == example.category)
        {
            starters.push(example);
        }
        if starters.len() == 3 {
            break;
        }
    }
    for example in examples {
        if starters.len() == 3 {
            break;
        }
        if !starters.iter().any(|chosen| chosen.id == example.id) {
            starters.push(example);
        }
    }
    let compact = body_height < 200.0 && ui.available_width() >= 480.0;
    let starter_rows = if compact { 1 } else { starters.len() };
    let content_height = tokens::FONT_HEADING
        + tokens::FONT_BODY * 1.5
        + tokens::SPACE_LG * 2.0
        + starter_rows as f32 * (tokens::CONTROL_HEIGHT + tokens::SPACE_XS);
    ui.add_space(((body_height - content_height) * 0.5).clamp(0.0, tokens::SPACE_XL * 2.0));
    let mut inserted = None;
    ui.vertical_centered(|ui| {
        text::title(ui, "Start with an idea");
        text::note(ui, "Describe what you hear, or try a starting point.");
        ui.add_space(tokens::SPACE_SM);
        let starter_width = if compact {
            let count = starters.len().max(1) as f32;
            (ui.available_width() - ui.spacing().item_spacing.x * (count - 1.0)) / count
        } else {
            ui.available_width().min(360.0)
        };
        let draw_starters = |ui: &mut Ui| {
            for example in starters {
                ui.push_id(example.id, |ui| {
                    let separator = if prompt.is_empty() { "" } else { "\n\n" };
                    let fits =
                        prompt.chars().count() + separator.len() + example.prompt.chars().count()
                            <= ASSISTANT_PROMPT_LIMIT;
                    let response = ui
                        .add_enabled_ui(fits, |ui| {
                            ui.add_sized(
                                egui::vec2(starter_width, tokens::CONTROL_HEIGHT),
                                crate::button_widget(bounded(example.title, 160)).truncate(),
                            )
                        })
                        .inner;
                    let hint = if fits {
                        format!(
                            "{}\n\nAdd to your draft, then edit and Send.",
                            bounded(example.description, 1_024)
                        )
                    } else {
                        "This example would exceed the prompt limit. Shorten the draft first."
                            .to_owned()
                    };
                    if crate::hover_text(response, hint).clicked() {
                        prompt.push_str(separator);
                        prompt.push_str(example.prompt);
                        inserted = Some(example.id.to_owned());
                    }
                });
            }
        };
        if compact {
            ui.horizontal(draw_starters);
        } else {
            ui.vertical_centered(draw_starters);
        }
    });
    inserted
}

/// Consistent local AI entry point for a musical editor's toolbar. The host
/// captures the target identity and opens the corresponding assistant scope.
pub fn contextual_ai_button(ui: &mut Ui, scope: &str) -> Response {
    crate::hover_text(
        crate::ai_button(ui, "AI for this context"),
        format!("Open AI scoped to {}", bounded(scope, 2_048)),
    )
}

/// A compact version for bounded editor headers. Keep a readable AI label and
/// the same scope explanation without taking space from the musical identity.
pub fn contextual_ai_button_compact(ui: &mut Ui, scope: &str) -> Response {
    crate::hover_text(
        ui.add(crate::ai_button_widget("AI")),
        format!(
            "AI for this context · Open AI scoped to {}",
            bounded(scope, 2_048)
        ),
    )
}

fn tool_browser(
    ui: &mut Ui,
    prompt: &mut String,
    state: &mut AssistantState,
    tools: &[AssistantTool<'_>],
) -> Option<String> {
    let tools = &tools[..tools.len().min(TOOL_LIMIT)];
    truncate_chars(&mut state.tool_query, 256);
    let clear = crate::action_row(
        ui,
        |ui| crate::small_button(ui, "Clear").clicked(),
        |ui| {
            ui.horizontal(|ui| {
                let label = text::label(ui, "Find a tool");
                crate::text_field(
                    ui,
                    &mut state.tool_query,
                    "Search tools…",
                    ui.available_width().max(0.0),
                )
                .labelled_by(label.id);
            })
        },
    )
    .inner
    .0;
    if clear {
        state.tool_query.clear();
    }
    truncate_chars(&mut state.tool_query, 256);
    let words: Vec<_> = state
        .tool_query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let filtered: Vec<_> = tools
        .iter()
        .filter(|tool| {
            words.is_empty() || {
                let haystack = format!(
                    "{} {} {} {}",
                    bounded(tool.name, 128),
                    bounded(tool.title, 160),
                    bounded(tool.description, 4_096),
                    bounded(tool.contexts, 512),
                )
                .to_lowercase();
                words.iter().all(|word| haystack.contains(word))
            }
        })
        .collect();
    if !filtered
        .iter()
        .any(|tool| state.selected_tool.as_deref() == Some(tool.name))
    {
        state.selected_tool = filtered.first().map(|tool| tool.name.to_owned());
    }
    if filtered.is_empty() {
        text::body(
            ui,
            if tools.is_empty() {
                "No tools are available in this scope."
            } else {
                "No matching tools. Clear the search to browse this scope."
            },
        );
        return None;
    }
    let mut inserted = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_LG;
        ui.columns(2, |columns| {
            let list = &mut columns[0];
            let noun = if filtered.len() == 1 { "tool" } else { "tools" };
            crate::hover_text(
                text::note(list, format!("{} {noun}", filtered.len())),
                "Available in this scope",
            );
            for tool in &filtered {
                list.push_id(tool.name, |ui| {
                    ui.spacing_mut().item_spacing.y = tokens::SPACE_SM;
                    if crate::selectable(
                        ui,
                        state.selected_tool.as_deref() == Some(tool.name),
                        bounded(tool.title, 160),
                    )
                    .clicked()
                    {
                        state.selected_tool = Some(tool.name.to_owned());
                    }
                    text::truncated(ui, text::TextRole::Note, bounded(tool.name, 128));
                });
            }
            let detail = &mut columns[1];
            if let Some(tool) = filtered
                .iter()
                .find(|tool| state.selected_tool.as_deref() == Some(tool.name))
            {
                let separator = if prompt.is_empty() { "" } else { "\n\n" };
                let fits = !tool.prompt.is_empty()
                    && prompt.chars().count() + separator.len() + tool.prompt.chars().count()
                        <= ASSISTANT_PROMPT_LIMIT;
                let use_tool = crate::action_row(
                    detail,
                    |ui| {
                        ui.add_enabled_ui(fits, |ui| crate::button(ui, "Use tool"))
                            .inner
                    },
                    |ui| text::truncated(ui, text::TextRole::Label, bounded(tool.title, 160)),
                )
                .inner
                .0;
                let hint = if fits {
                    "Add an editable request to the draft. Review it, then Send."
                } else if tool.prompt.is_empty() {
                    "This tool has no suggested prompt. Describe its use in the draft."
                } else {
                    "This request would exceed the prompt limit. Shorten the draft first."
                };
                if crate::hover_text(use_tool, hint).clicked() {
                    prompt.push_str(separator);
                    prompt.push_str(tool.prompt);
                    inserted = Some(tool.name.to_owned());
                    state.page = AssistantPage::Conversation;
                }
                text::body(detail, bounded(tool.description, 4_096));
                if !tool.contexts.is_empty() {
                    text::note(
                        detail,
                        format!("Available in · {}", bounded(tool.contexts, 512)),
                    );
                }
            }
        });
    });
    inserted
}

fn transcript(ui: &mut Ui, messages: &[AssistantMessage]) {
    for (index, message) in messages
        .iter()
        .enumerate()
        .skip(messages.len().saturating_sub(MESSAGE_LIMIT))
    {
        ui.push_id(index, |ui| {
            if message.role == AssistantRole::Tool {
                text::truncated(
                    ui,
                    text::TextRole::Note,
                    format!("Activity · {}", bounded(&message.text, 512)),
                );
            } else {
                text::label(ui, message.role.label());
                text::selectable_block(
                    ui,
                    text::TextRole::Body,
                    bounded(&message.text, MESSAGE_CHAR_LIMIT),
                );
                ui.add_space(tokens::SPACE_SM);
            }
        });
    }
}

fn examples(
    ui: &mut Ui,
    prompt: &mut String,
    state: &mut AssistantState,
    examples: &[AssistantExample<'_>],
) -> Option<String> {
    let examples = &examples[..examples.len().min(GUIDANCE_LIMIT)];
    let mut categories = Vec::new();
    for example in examples {
        if !categories.contains(&example.category) {
            categories.push(example.category);
        }
    }
    if !categories.contains(&state.category.as_str()) {
        state.category = categories.first().copied().unwrap_or_default().to_owned();
    }
    ui.horizontal_wrapped(|ui| {
        for category in categories {
            if crate::view_choice(ui, state.category == category, bounded(category, 80)).clicked() {
                state.category = category.to_owned();
            }
        }
    });
    text::note(ui, "Choose an example, edit the draft, then Send.");
    let mut inserted = None;
    for example in examples
        .iter()
        .filter(|example| example.category == state.category)
    {
        ui.push_id(example.id, |ui| {
            ui.add_space(tokens::SPACE_SM);
            let separator = if prompt.is_empty() { "" } else { "\n\n" };
            let fits = prompt.chars().count() + separator.len() + example.prompt.chars().count()
                <= ASSISTANT_PROMPT_LIMIT;
            let response = ui
                .add_enabled_ui(fits, |ui| crate::button(ui, bounded(example.title, 160)))
                .inner;
            let hint = if fits {
                "Insert example into your draft. Review it, then Send."
            } else {
                "This example would exceed the prompt limit. Shorten the draft first."
            };
            if crate::hover_text(response, hint).clicked() {
                prompt.push_str(separator);
                prompt.push_str(example.prompt);
                inserted = Some(example.id.to_owned());
            }
            text::body(ui, bounded(example.description, 1_024));
        });
    }
    if inserted.is_some() {
        state.page = AssistantPage::Conversation;
    }
    inserted
}

/// A masked credential input using the same dimensions as ordinary text fields.
/// The application must redact and protect the backing value separately.
pub fn secret_field(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .password(true)
            .hint_text(hint)
            .desired_width(width.max(0.0))
            .char_limit(4_096)
            .margin(egui::vec2(
                tokens::BUTTON_PADDING_X,
                tokens::BUTTON_PADDING_Y,
            ))
            .vertical_align(egui::Align::Center)
            .min_size(egui::vec2(0.0, tokens::CONTROL_HEIGHT)),
    )
}

fn bounded(value: &str, characters: usize) -> &str {
    let end = value
        .char_indices()
        .nth(characters)
        .map_or(value.len(), |(index, _)| index);
    &value[..end]
}

fn truncate_chars(value: &mut String, characters: usize) {
    if let Some((end, _)) = value.char_indices().nth(characters) {
        value.truncate(end);
    }
}

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

/// The application owns history, execution state, and the captured song context.
pub struct AssistantOptions<'a> {
    pub messages: &'a [AssistantMessage],
    pub status: Option<&'a str>,
    pub busy: bool,
    pub can_submit: bool,
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
                .add_enabled_ui(!options.busy, |ui| crate::view_button(ui, "AI settings"))
                .inner
                .clicked();
        },
        |ui| {
            let readiness = if options.configured {
                format!("{} · Configured", bounded(options.provider, 120))
            } else {
                "Connect a provider to start".into()
            };
            text::truncated(ui, text::TextRole::Note, readiness);
        },
    );
    let scope = format!("Scope · {}", bounded(options.scope, 2_048));
    text::truncated(ui, text::TextRole::Label, &scope);
    if !options.context.is_empty() {
        text::truncated(ui, text::TextRole::Note, bounded(options.context, 2_048));
    }
    ui.horizontal_wrapped(|ui| {
        for (page, label) in [
            (AssistantPage::Conversation, "Conversation"),
            (AssistantPage::Examples, "Examples"),
            (AssistantPage::Tools, "Tools"),
            (AssistantPage::Tips, "Tips"),
        ] {
            if crate::view_choice(ui, state.page == page, label).clicked() {
                state.page = page;
            }
        }
    });
    ui.add_space(tokens::SPACE_XS);

    // Reserve every fixed control after measuring the actual header rows. All
    // pages share this bounded viewport, so long tips, tools and examples cannot
    // push the composer or transport actions below the window.
    // Browsing needs room for choices, while the conversation benefits from a
    // taller composer. A short popup keeps one editable line and scrolls the
    // complete draft without displacing the scope or fixed actions.
    let compact_composer = state.page != AssistantPage::Conversation
        || ui.available_height() < tokens::CONTROL_HEIGHT * 9.0;
    let composer_height = tokens::CONTROL_HEIGHT * if compact_composer { 1.0 } else { 2.0 };
    let show_status = options.busy || options.status.is_some_and(|status| !status.is_empty());
    let status_height = if show_status {
        tokens::CONTROL_HEIGHT
    } else {
        0.0
    };
    let body_height = (ui.available_height()
        - composer_height
        - status_height
        - tokens::CONTROL_HEIGHT
        - tokens::FONT_BODY * 1.5
        - tokens::SPACE_SM * 5.0)
        .clamp(0.0, 460.0);
    if body_height > 0.0 {
        let page = state.page;
        egui::ScrollArea::vertical()
            .id_salt(("assistant_body", page))
            .max_height(body_height)
            .min_scrolled_height(0.0)
            .auto_shrink([false, false])
            .stick_to_bottom(page == AssistantPage::Conversation && !options.messages.is_empty())
            .show(ui, |ui| match page {
                AssistantPage::Conversation => {
                    if options.messages.is_empty() {
                        crate::action_row(
                            ui,
                            |ui| {
                                if ui
                                    .add_enabled_ui(!options.examples.is_empty(), |ui| {
                                        crate::view_button(ui, "Browse examples")
                                    })
                                    .inner
                                    .clicked()
                                {
                                    state.page = AssistantPage::Examples;
                                }
                            },
                            |ui| {
                                text::truncated(ui, text::TextRole::Subtitle, "Make music with AI");
                            },
                        );
                        text::body(
                            ui,
                            "Describe a musical goal, choose an example, or ask for advice.",
                        );
                        text::note(ui, "Tip: include the style, key, length, and what to keep.");
                    } else {
                        transcript(ui, options.messages);
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
        ui.add_space(tokens::SPACE_SM);
    }
    if show_status {
        ui.horizontal(|ui| {
            if options.busy {
                ui.spinner();
            }
            text::truncated(
                ui,
                text::TextRole::Note,
                bounded(
                    options
                        .status
                        .filter(|s| !s.is_empty())
                        .unwrap_or("Working…"),
                    512,
                ),
            );
        });
        ui.add_space(tokens::SPACE_XS);
    }

    let label = text::label(ui, "AI prompt");
    let prompt_id = ui.make_persistent_id("assistant_prompt");
    let can_send = options.can_submit && !options.busy && !prompt.trim().is_empty();
    let shortcut = can_send
        && ui.is_enabled()
        && ui.memory(|memory| memory.has_focus(prompt_id))
        && ui.input_mut(|input| {
            input.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
                || input.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)
        });
    let prompt_response = egui::ScrollArea::vertical()
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
            .labelled_by(label.id)
        })
        .inner;
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
    let (send_button, stop_button, undo_button) = ui
        .horizontal_wrapped(|ui| {
            let can_send = options.can_submit && !options.busy && !prompt.trim().is_empty();
            let send = ui
                .add_enabled_ui(can_send, |ui| crate::primary_button(ui, "Send"))
                .inner;
            let send = crate::hover_text(send, "Send prompt · Ctrl/Cmd+Enter");
            let stop = ui
                .add_enabled_ui(options.busy, |ui| crate::button(ui, "Stop"))
                .inner;
            let undo = ui
                .add_enabled_ui(options.can_undo && !options.busy, |ui| {
                    crate::button(ui, "Undo")
                })
                .inner;
            (send, stop, undo)
        })
        .inner;
    AssistantResponse {
        send: shortcut || send_button.clicked(),
        stop: stop_button.clicked(),
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
}

/// Consistent local AI entry point for a musical editor's toolbar. The host
/// captures the target identity and opens the corresponding assistant scope.
pub fn contextual_ai_button(ui: &mut Ui, scope: &str) -> Response {
    crate::hover_text(
        crate::view_button(ui, "AI for this context"),
        format!("Open AI scoped to {}", bounded(scope, 2_048)),
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
    let filtered: Vec<_> = ui
        .horizontal(|ui| {
            let label = text::label(ui, "Find a tool");
            crate::text_field(
                ui,
                &mut state.tool_query,
                "Search tools…",
                (ui.available_width() - 148.0).max(60.0),
            )
            .labelled_by(label.id);
            if crate::small_button(ui, "Clear").clicked() {
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
            let noun = if filtered.len() == 1 { "tool" } else { "tools" };
            crate::hover_text(
                text::note(ui, format!("{} {noun}", filtered.len())),
                "Available in this scope",
            );
            filtered
        })
        .inner;
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
    ui.columns(2, |columns| {
        let list = &mut columns[0];
        for tool in &filtered {
            list.push_id(tool.name, |ui| {
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

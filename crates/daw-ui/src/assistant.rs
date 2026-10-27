//! A bounded assistant transcript and composer, independent of agent providers.
use crate::{text, tokens};
use egui::{Response, Ui};

/// Bound interactive text layout independently of provider response limits.
pub const ASSISTANT_PROMPT_LIMIT: usize = 16_384;
const MESSAGE_LIMIT: usize = 64;
const MESSAGE_CHAR_LIMIT: usize = 8_192;

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
            Self::Tool => "Action",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantMessage {
    pub role: AssistantRole,
    pub text: String,
}

/// The application owns history, execution state, and the captured song context.
pub struct AssistantOptions<'a> {
    pub messages: &'a [AssistantMessage],
    pub status: Option<&'a str>,
    pub busy: bool,
    pub can_submit: bool,
    pub can_undo: bool,
    pub context: &'a str,
}

pub struct AssistantResponse {
    pub send: bool,
    pub stop: bool,
    pub undo: bool,
    pub prompt: Response,
    pub send_button: Response,
    pub stop_button: Response,
    pub undo_button: Response,
}

/// Draw a transcript with a fixed composer and footer. Enter inserts a newline;
/// Ctrl/Cmd+Enter submits only when this prompt owns keyboard focus. The caller
/// starts/stops work and clears the prompt after accepting a submission.
pub fn assistant(
    ui: &mut Ui,
    prompt: &mut String,
    options: AssistantOptions<'_>,
) -> AssistantResponse {
    truncate_chars(prompt, ASSISTANT_PROMPT_LIMIT);
    if !options.context.is_empty() {
        text::truncated(ui, text::TextRole::Note, bounded(options.context, 512));
        ui.add_space(tokens::SPACE_XS);
    }

    // Reserve every non-scrolling control before assigning transcript space.
    let composer_height = tokens::CONTROL_HEIGHT * 2.0;
    let status_height = if options.status.is_some() || options.busy {
        tokens::CONTROL_HEIGHT
    } else {
        0.0
    };
    let footer_height = tokens::CONTROL_HEIGHT;
    let transcript_height = (ui.available_height()
        - composer_height
        - status_height
        - footer_height
        - tokens::FONT_BODY * 1.5
        - tokens::SPACE_SM * 5.0)
        .clamp(0.0, 360.0);
    if transcript_height > 0.0 && !options.messages.is_empty() {
        egui::ScrollArea::vertical()
            .id_salt("assistant_transcript")
            .max_height(transcript_height)
            .min_scrolled_height(0.0)
            .auto_shrink([false, true])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (index, message) in options
                    .messages
                    .iter()
                    .enumerate()
                    .skip(options.messages.len().saturating_sub(MESSAGE_LIMIT))
                {
                    ui.push_id(index, |ui| {
                        text::label(ui, message.role.label());
                        text::selectable_block(
                            ui,
                            text::TextRole::Body,
                            bounded(&message.text, MESSAGE_CHAR_LIMIT),
                        );
                        ui.add_space(tokens::SPACE_SM);
                    });
                }
            });
        ui.add_space(tokens::SPACE_SM);
    }
    if options.status.is_some() || options.busy {
        ui.horizontal(|ui| {
            if options.busy {
                ui.spinner();
            }
            text::truncated(
                ui,
                text::TextRole::Note,
                bounded(options.status.unwrap_or("Working…"), 512),
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
                    .desired_rows(3)
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
    ui.add_space(tokens::SPACE_SM);
    let (send_button, stop_button, undo_button) = ui
        .horizontal_wrapped(|ui| {
            let can_send = options.can_submit && !options.busy && !prompt.trim().is_empty();
            let send = ui
                .add_enabled_ui(can_send, |ui| crate::primary_button(ui, "Send"))
                .inner
                .on_hover_text("Send prompt · Ctrl/Cmd+Enter");
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
        prompt: prompt_response,
        send_button,
        stop_button,
        undo_button,
    }
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

use daw_ui::{AssistantMessage, AssistantOptions, AssistantRole, NoticeKind};
use egui::Ui;

pub struct State {
    prompt: String,
    messages: Vec<AssistantMessage>,
    assistant: daw_ui::AssistantState,
}

impl Default for State {
    fn default() -> Self {
        Self {
            assistant: daw_ui::AssistantState::default(),
            prompt: "Lower the pads by 2 dB and keep the melody unchanged.".into(),
            messages: vec![
                AssistantMessage {
                    role: AssistantRole::User,
                    text: "Write a four-bar arpeggio on the selected track.".into(),
                },
                AssistantMessage {
                    role: AssistantRole::Tool,
                    text: "Example action · 32 notes added to Glass arpeggio.".into(),
                },
                AssistantMessage {
                    role: AssistantRole::Assistant,
                    text: "A new phrase is ready. Edit its notes or use Undo.".into(),
                },
            ],
        }
    }
}

pub fn show(ui: &mut Ui, state: &mut State) {
    ui.columns(2, |columns| {
        let left = &mut columns[0];
        daw_ui::text::subtitle(left, "Non-modal notices");
        daw_ui::text::note(left, "notice · actions stay beside bounded messages");
        left.add_space(daw_ui::tokens::SPACE_MD);
        for (kind, message, action) in [
            (NoticeKind::Busy, "Exporting Glass Current.wav…", "Cancel"),
            (NoticeKind::Success, "Saved Glass Current.rill", "Copy path"),
            (
                NoticeKind::Warning,
                "Choose an instrument for this MIDI track.",
                "Choose…",
            ),
            (
                NoticeKind::Error,
                "The example output device is unavailable.",
                "Details",
            ),
        ] {
            left.push_id(message, |ui| {
                daw_ui::notice(ui, kind, message, |ui| {
                    daw_ui::small_button(ui, action);
                });
            });
            left.add_space(daw_ui::tokens::SPACE_MD);
        }
        left.add_space(daw_ui::tokens::SPACE_LG);
        daw_ui::text::body(
            left,
            "Progress, completion, warnings, and recovery use the same quiet row.",
        );
        daw_ui::text::note(
            left,
            "Messages and transcript are static gallery sample data.",
        );

        let right = &mut columns[1];
        daw_ui::text::subtitle(right, "Assistant");
        daw_ui::text::note(right, "assistant · sample conversation and prompt");
        right.add_space(daw_ui::tokens::SPACE_MD);
        let can_reuse = state.prompt.is_empty();
        let response = daw_ui::assistant(
            right,
            &mut state.prompt,
            &mut state.assistant,
            AssistantOptions {
                messages: &state.messages,
                status: Some("Sample conversation"),
                busy: false,
                can_submit: true,
                can_undo: true,
                context: "Selected track · Glass arpeggio",
                scope: "Global · Entire song",
                provider: "Example provider",
                configured: true,
                can_reuse,
                tools: &[
                    daw_ui::AssistantTool {
                        name: "add_notes",
                        title: "Add notes",
                        description: "Write a musical phrase into a MIDI clip.",
                        contexts: "Global, Piano roll, MIDI clip",
                        prompt: "Add a four-bar arpeggio to the selected MIDI clip. Preserve its existing notes.",
                    },
                    daw_ui::AssistantTool {
                        name: "set_parameter",
                        title: "Adjust a parameter",
                        description: "Adjust a track, instrument, or effect control.",
                        contexts: "Global, Instrument, Effect, Effect chain",
                        prompt: "Inspect the available parameters and suggest one adjustment before editing.",
                    },
                ],
                examples: &[
                    daw_ui::AssistantExample {
                        id: "progression",
                        category: "Compose",
                        title: "A four-bar progression",
                        description: "Start with harmony, a bass line, and an arpeggio.",
                        prompt: "Create four bars in D minor with chords, bass, and an arpeggio. Keep the existing tracks unchanged.",
                    },
                    daw_ui::AssistantExample {
                        id: "variation",
                        category: "Develop",
                        title: "A gentler variation",
                        description: "Develop the selected phrase while preserving the original.",
                        prompt: "Duplicate the selected phrase and make a quieter variation with fewer notes. Keep its key and length.",
                    },
                ],
                tips: &[
                    daw_ui::AssistantTip {
                        title: "Give a musical brief",
                        body: "Include style, key, length, and the part you want to develop.",
                    },
                    daw_ui::AssistantTip {
                        title: "Listen, then refine",
                        body: "Audition the result and describe one change at a time. Undo reverses normal song edits.",
                    },
                ],
            },
        );
        if response.new_chat {
            state.messages.clear();
            state.assistant = daw_ui::AssistantState::default();
        }
        if response.reuse_last
            && let Some(message) = state.messages.iter().rev().find(|m| m.role == AssistantRole::User)
        {
            state.prompt.clone_from(&message.text);
        }
        if response.send {
            state.prompt.clear();
        }
        if response.undo {
            state.messages.pop();
        }
    });
}

use daw_ui::{AssistantMessage, AssistantOptions, AssistantRole, NoticeKind};
use egui::Ui;

pub struct State {
    prompt: String,
    messages: Vec<AssistantMessage>,
}

impl Default for State {
    fn default() -> Self {
        Self {
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
        let response = daw_ui::assistant(
            right,
            &mut state.prompt,
            AssistantOptions {
                messages: &state.messages,
                status: Some("Sample conversation"),
                busy: false,
                can_submit: true,
                can_undo: true,
                context: "Selected track · Glass arpeggio",
            },
        );
        if response.send {
            state.prompt.clear();
        }
        if response.undo {
            state.messages.pop();
        }
    });
}

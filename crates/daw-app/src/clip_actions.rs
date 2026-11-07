//! Whole-clip musical actions share the same validation and history path.
use crate::app::DawApp;
use daw_core::{Clip, ClipContent, MAX_BEAT, MAX_SOURCE_RATE, MIN_SOURCE_RATE};

pub(crate) fn midi_clip_action_availability(clip: &Clip) -> daw_ui::MidiClipActionAvailability {
    let ClipContent::Midi { notes } = &clip.content else {
        return Default::default();
    };
    let low = notes.iter().map(|note| note.pitch).min();
    let high = notes.iter().map(|note| note.pitch).max();
    daw_ui::MidiClipActionAvailability {
        has_notes: notes.iter().any(|note| {
            note.start < clip.offset + clip.source_length()
                && note.start + note.length > clip.offset
        }),
        half_time: clip.source_rate * 0.5 >= MIN_SOURCE_RATE
            && clip.start + clip.length * 2.0 <= MAX_BEAT,
        double_time: clip.source_rate * 2.0 <= MAX_SOURCE_RATE && clip.length * 0.5 > 0.0,
        semitone_up: high.is_some_and(|pitch| pitch < 127),
        semitone_down: low.is_some_and(|pitch| pitch > 0),
        octave_up: high.is_some_and(|pitch| pitch <= 115),
        octave_down: low.is_some_and(|pitch| pitch >= 12),
    }
}

impl DawApp {
    pub(crate) fn apply_midi_clip_action(
        &mut self,
        track: u64,
        clip: u64,
        action: daw_ui::MidiClipAction,
    ) {
        if self.recording_busy() {
            return;
        }
        if matches!(
            action,
            daw_ui::MidiClipAction::Quantize | daw_ui::MidiClipAction::Humanize
        ) {
            self.request_midi_clip_transform(track, clip, action);
            return;
        }
        self.flush_pending_edit();
        let mut candidate = self.project.clone();
        let result = (|| -> anyhow::Result<&'static str> {
            let original = candidate
                .clip(track, clip)
                .ok_or_else(|| anyhow::anyhow!("Clip not found"))?;
            anyhow::ensure!(
                matches!(original.content, ClipContent::Midi { .. }),
                "Choose a MIDI clip"
            );
            let label = match action {
                daw_ui::MidiClipAction::HalfTime | daw_ui::MidiClipAction::DoubleTime => {
                    let factor = if action == daw_ui::MidiClipAction::HalfTime {
                        2.0
                    } else {
                        0.5
                    };
                    let length = original.length * factor;
                    candidate.stretch_clip(track, clip, length)?;
                    if factor > 1.0 {
                        "MIDI clip half time"
                    } else {
                        "MIDI clip double time"
                    }
                }
                daw_ui::MidiClipAction::Transpose(semitones) => {
                    let ClipContent::Midi { notes } = &original.content else {
                        unreachable!()
                    };
                    let selected = notes.iter().map(|note| note.id).collect();
                    let notes = daw_core::move_midi_notes(notes, &selected, 0.0, semitones)?;
                    candidate
                        .clip_mut(track, clip)
                        .expect("validated clip")
                        .content = ClipContent::Midi { notes };
                    candidate.synchronize_linked_clip(track, clip)?;
                    "Transpose MIDI clip"
                }
                daw_ui::MidiClipAction::Quantize | daw_ui::MidiClipAction::Humanize => {
                    unreachable!("Preview requests return before mutation")
                }
            };
            candidate.validate()?;
            Ok(label)
        })();
        match result {
            Ok(label) if candidate != self.project => {
                self.begin_edit();
                self.project = candidate;
                self.edit_label = Some(label.into());
                self.changed();
                self.flush_pending_edit();
            }
            Ok(_) => {}
            Err(error) => self.message(Err(error), ""),
        }
    }
}

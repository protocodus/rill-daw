//! Persistent track identity colors, independent of appearance and object IDs.
use crate::{Project, Track};

// Alternate hue families so neighboring tracks are easy to tell apart. These
// moderate colors work as markers and tinted clip surfaces in both UI themes.
const TRACK_COLORS: [[u8; 3]; 12] = [
    [96, 177, 166],  // teal
    [219, 159, 99],  // orange
    [155, 137, 216], // violet
    [158, 185, 108], // green
    [106, 159, 214], // blue
    [207, 129, 166], // rose
    [207, 185, 104], // gold
    [114, 182, 204], // cyan
    [208, 132, 112], // coral
    [181, 146, 205], // lavender
    [122, 172, 127], // sage
    [136, 148, 190], // slate
];

impl Project {
    /// Assign color only when a genuinely new track enters the project. Loading,
    /// undo/redo, existing-track imports, and manual recoloring retain their colors.
    pub(crate) fn insert_new_track(&mut self, mut track: Track) -> u64 {
        let mut uses = [0_usize; TRACK_COLORS.len()];
        let mut next = 0;
        for existing in &self.tracks {
            if let Some(index) = TRACK_COLORS
                .iter()
                .position(|color| *color == existing.color)
            {
                uses[index] += 1;
                next = (index + 1) % TRACK_COLORS.len();
            }
        }
        // Prefer unused colors, then cycle evenly once the palette is exhausted.
        // Deriving this from saved colors avoids a second state to synchronize
        // through project loading, deleted tracks, and undo history.
        let index = (0..TRACK_COLORS.len())
            .map(|offset| (next + offset) % TRACK_COLORS.len())
            .min_by_key(|index| uses[*index])
            .unwrap_or(0);
        track.color = TRACK_COLORS[index];
        let id = track.id;
        self.tracks.push(track);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AutomationParameter, AutomationTarget, Clip, Note};
    use std::collections::HashMap;

    #[test]
    fn all_track_creation_paths_share_the_palette_without_recoloring_existing_tracks() {
        let mut project = Project::new();
        let audio = project.add_audio_track("Audio");
        project.add_midi_track("MIDI");
        project.add_bus("Bus");
        project.add_folder("Folder");
        project
            .add_automation_track(
                "Volume",
                AutomationTarget {
                    track: audio,
                    parameter: AutomationParameter::TrackGain,
                },
            )
            .unwrap();
        let original = project.tracks.clone();
        assert_eq!(
            original
                .iter()
                .map(|track| track.color)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            original.len()
        );
        let custom = [12, 34, 56];
        project.track_mut(audio).unwrap().color = custom;
        project.add_audio_track("Another audio");
        assert_eq!(project.track(audio).unwrap().color, custom);
        assert_eq!(&project.tracks[1..original.len()], &original[1..]);
    }

    #[test]
    fn colors_cycle_evenly_and_do_not_depend_on_note_clip_or_process_ids() {
        let mut project = Project::new();
        let mut control = Project::new();
        let mut counts = HashMap::new();
        for index in 0..TRACK_COLORS.len() * 3 {
            let id = project.add_audio_track(format!("Track {index}"));
            // Other musical objects must not consume a track color.
            let _ = Clip::midi("Unused clip", 0.0, 4.0, vec![Note::new(60, 0.0, 1.0, 100)]);
            let control_id = control.add_midi_track("Other kind");
            let color = project.track(id).unwrap().color;
            assert_eq!(color, control.track(control_id).unwrap().color);
            *counts.entry(color).or_insert(0) += 1;
            if index < TRACK_COLORS.len() {
                assert_eq!(counts.len(), index + 1);
            }
        }
        assert!(counts.values().all(|count| *count == 3));
        assert!(
            project
                .tracks
                .windows(2)
                .all(|pair| pair[0].color != pair[1].color)
        );
    }

    #[test]
    fn midi_batch_import_assigns_colors_against_the_destination_project() {
        let directory = tempfile::tempdir().unwrap();
        let mut source = Project::new();
        for pitch in [60, 64, 67] {
            let id = source.add_midi_track("Imported MIDI");
            source.track_mut(id).unwrap().clips.push(Clip::midi(
                "Notes",
                0.0,
                4.0,
                vec![Note::new(pitch, 0.0, 1.0, 100)],
            ));
        }
        let midi = directory.path().join("tracks.mid");
        source.export_midi(&midi).unwrap();
        let mut destination = Project::new();
        destination.add_audio_track("Existing");
        let before = destination.tracks[0].clone();
        let ids = destination.import_midi(&midi, 4.0).unwrap();
        assert_eq!(ids.len(), 3);
        assert_eq!(destination.tracks[0], before);
        assert_eq!(
            destination
                .tracks
                .iter()
                .map(|track| track.color)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            4
        );
    }
}

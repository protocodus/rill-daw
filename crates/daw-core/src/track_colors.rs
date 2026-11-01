//! Persistent track identity colors, independent of appearance and object IDs.
use crate::{Project, Track};

pub const DEFAULT_TRACK_COLOR: [u8; 3] = [148, 153, 162];

impl Project {
    /// New tracks start neutral; retain any color supplied by an import or copy.
    pub(crate) fn insert_new_track(&mut self, track: Track) -> u64 {
        let id = track.id;
        self.tracks.push(track);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AutomationParameter, AutomationTarget, Clip, Note};

    #[test]
    fn all_track_creation_paths_start_neutral_without_recoloring_existing_tracks() {
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
            1
        );
        let custom = [12, 34, 56];
        project.track_mut(audio).unwrap().color = custom;
        project.add_audio_track("Another audio");
        assert_eq!(project.track(audio).unwrap().color, custom);
        assert_eq!(&project.tracks[1..original.len()], &original[1..]);
    }

    #[test]
    fn new_track_colors_are_neutral_and_independent_of_object_ids() {
        let mut project = Project::new();
        for index in 0..24 {
            let id = project.add_audio_track(format!("Track {index}"));
            let _ = Clip::midi("Unused", 0.0, 4.0, vec![Note::new(60, 0.0, 1.0, 100)]);
            assert_eq!(project.track(id).unwrap().color, DEFAULT_TRACK_COLOR);
        }
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
            1
        );
    }
}

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{BufReader, BufWriter, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A generous finite timeline limit keeps beat-to-sample conversion exact and
/// prevents malformed project files from saturating integer sample positions.
pub const MAX_BEAT: f64 = 1_000_000.0;

/// IDs are unique within a process; loading a project reserves its existing IDs.
pub fn new_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub tempo: f64,
    pub beats_per_bar: u8,
    pub tracks: Vec<Track>,
    pub loop_enabled: bool,
    pub loop_start: f64,
    pub loop_end: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: u64,
    pub name: String,
    pub kind: TrackKind,
    pub color: [u8; 3],
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub instrument: Instrument,
    pub clips: Vec<Clip>,
    /// Additional path latency, added to latency reported by the instrument.
    pub latency_samples: u32,
    #[serde(default)]
    pub inserts: Vec<EffectSlot>,
    #[serde(default)]
    pub sends: Vec<Send>,
    /// Main audio output; `None` routes to the master output.
    #[serde(default)]
    pub output_bus: Option<u64>,
    /// Organizational parent. Folders do not sum or process audio.
    #[serde(default)]
    pub parent_folder: Option<u64>,
    #[serde(default)]
    pub collapsed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Audio,
    Midi,
    Bus,
    Folder,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectSlot {
    pub id: u64,
    pub name: String,
    pub enabled: bool,
    pub effect: Effect,
}

impl EffectSlot {
    pub fn new(name: impl Into<String>, effect: Effect) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            enabled: true,
            effect,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    Gain {
        gain: f32,
    },
    Delay {
        time_ms: f32,
        feedback: f32,
        mix: f32,
    },
    Vst3 {
        path: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Send {
    pub target_bus: u64,
    pub gain: f32,
    pub pre_fader: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Instrument {
    Sine,
    Triangle,
    Saw,
    Vst3 { path: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: u64,
    pub name: String,
    pub start: f64,
    pub length: f64,
    /// Source beat at the clip's left edge. Trimming never destroys source notes.
    pub offset: f64,
    pub content: ClipContent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClipContent {
    Midi {
        notes: Vec<Note>,
    },
    Audio {
        path: String,
        gain: f32,
        fade_in: f64,
        fade_out: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub pitch: u8,
    /// Source position, not arrangement position; see `Clip::offset`.
    pub start: f64,
    pub length: f64,
    pub velocity: u8,
}

impl Note {
    pub fn new(pitch: u8, start: f64, length: f64, velocity: u8) -> Self {
        Self {
            id: new_id(),
            pitch,
            start,
            length,
            velocity,
        }
    }
}

impl Clip {
    pub fn midi(name: impl Into<String>, start: f64, length: f64, notes: Vec<Note>) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            start,
            length,
            offset: 0.0,
            content: ClipContent::Midi { notes },
        }
    }

    pub fn audio(
        name: impl Into<String>,
        start: f64,
        length: f64,
        path: impl Into<String>,
    ) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            start,
            length,
            offset: 0.0,
            content: ClipContent::Audio {
                path: path.into(),
                gain: 1.0,
                fade_in: 0.0,
                fade_out: 0.0,
            },
        }
    }

    pub fn end(&self) -> f64 {
        self.start + self.length
    }

    fn refresh_ids(&mut self) {
        self.id = new_id();
        if let ClipContent::Midi { notes } = &mut self.content {
            for note in notes {
                note.id = new_id();
            }
        }
    }

    fn constrain_fades(&mut self) {
        if let ClipContent::Audio {
            fade_in, fade_out, ..
        } = &mut self.content
        {
            *fade_in = fade_in.min(self.length);
            *fade_out = fade_out.min(self.length);
        }
    }
}

impl Track {
    pub fn new(name: impl Into<String>, kind: TrackKind) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            kind,
            color: match kind {
                TrackKind::Audio => [80, 184, 195],
                TrackKind::Midi => [169, 132, 255],
                TrackKind::Bus => [227, 164, 95],
                TrackKind::Folder => [131, 143, 166],
            },
            gain: if matches!(kind, TrackKind::Bus | TrackKind::Folder) {
                1.0
            } else {
                0.75
            },
            pan: 0.0,
            mute: false,
            solo: false,
            instrument: Instrument::Triangle,
            clips: Vec::new(),
            latency_samples: 0,
            inserts: Vec::new(),
            sends: Vec::new(),
            output_bus: None,
            parent_folder: None,
            collapsed: false,
        }
    }
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

impl Project {
    pub fn new() -> Self {
        Self {
            name: "Untitled session".into(),
            tempo: 120.0,
            beats_per_bar: 4,
            tracks: Vec::new(),
            loop_enabled: false,
            loop_start: 0.0,
            loop_end: 16.0,
        }
    }

    pub fn add_audio_track(&mut self, name: impl Into<String>) -> u64 {
        let track = Track::new(name, TrackKind::Audio);
        let id = track.id;
        self.tracks.push(track);
        id
    }

    pub fn add_midi_track(&mut self, name: impl Into<String>) -> u64 {
        let track = Track::new(name, TrackKind::Midi);
        let id = track.id;
        self.tracks.push(track);
        id
    }

    pub fn track(&self, id: u64) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == id)
    }
    pub fn track_mut(&mut self, id: u64) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|track| track.id == id)
    }

    pub fn clip(&self, track_id: u64, clip_id: u64) -> Option<&Clip> {
        self.track(track_id)?
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
    }

    pub fn clip_mut(&mut self, track_id: u64, clip_id: u64) -> Option<&mut Clip> {
        self.track_mut(track_id)?
            .clips
            .iter_mut()
            .find(|clip| clip.id == clip_id)
    }

    pub fn midi_notes(&self, track_id: u64, clip_id: u64) -> Option<&[Note]> {
        match &self.clip(track_id, clip_id)?.content {
            ClipContent::Midi { notes } => Some(notes),
            _ => None,
        }
    }

    pub fn midi_notes_mut(&mut self, track_id: u64, clip_id: u64) -> Option<&mut Vec<Note>> {
        match &mut self.clip_mut(track_id, clip_id)?.content {
            ClipContent::Midi { notes } => Some(notes),
            _ => None,
        }
    }

    pub fn split_note(
        &mut self,
        track_id: u64,
        clip_id: u64,
        note_id: u64,
        at_source_beat: f64,
    ) -> Result<u64> {
        let notes = self
            .midi_notes_mut(track_id, clip_id)
            .context("MIDI clip not found")?;
        let index = notes
            .iter()
            .position(|note| note.id == note_id)
            .context("Note not found")?;
        let note = &mut notes[index];
        ensure!(
            at_source_beat.is_finite()
                && at_source_beat > note.start
                && at_source_beat < note.start + note.length,
            "Split must be inside the note"
        );
        let right = Note::new(
            note.pitch,
            at_source_beat,
            note.start + note.length - at_source_beat,
            note.velocity,
        );
        note.length = at_source_beat - note.start;
        let id = right.id;
        notes.insert(index + 1, right);
        Ok(id)
    }

    pub fn duplicate_note(&mut self, track_id: u64, clip_id: u64, note_id: u64) -> Result<u64> {
        let notes = self
            .midi_notes_mut(track_id, clip_id)
            .context("MIDI clip not found")?;
        let mut duplicate = notes
            .iter()
            .find(|note| note.id == note_id)
            .context("Note not found")?
            .clone();
        duplicate.start += duplicate.length;
        validate_note(&duplicate)?;
        duplicate.id = new_id();
        let id = duplicate.id;
        notes.push(duplicate);
        Ok(id)
    }

    pub fn delete_note(&mut self, track_id: u64, clip_id: u64, note_id: u64) -> Result<Note> {
        let notes = self
            .midi_notes_mut(track_id, clip_id)
            .context("MIDI clip not found")?;
        let index = notes
            .iter()
            .position(|note| note.id == note_id)
            .context("Note not found")?;
        Ok(notes.remove(index))
    }

    /// Glue same-pitch notes into one sustained note, including gaps between them.
    /// The earliest selected note supplies pitch and velocity.
    pub fn glue_notes(&mut self, track_id: u64, clip_id: u64, note_ids: &[u64]) -> Result<u64> {
        let selected: HashSet<_> = note_ids.iter().copied().collect();
        ensure!(selected.len() >= 2, "Select at least two notes to glue");
        let notes = self
            .midi_notes_mut(track_id, clip_id)
            .context("MIDI clip not found")?;
        let selected_notes: Vec<_> = notes
            .iter()
            .filter(|note| selected.contains(&note.id))
            .collect();
        ensure!(
            selected_notes.len() == selected.len(),
            "A selected note was not found"
        );
        let first = selected_notes
            .iter()
            .min_by(|a, b| a.start.total_cmp(&b.start))
            .expect("selection has at least two notes");
        ensure!(
            selected_notes.iter().all(|note| note.pitch == first.pitch),
            "Glue requires notes with the same pitch"
        );
        let end = selected_notes
            .iter()
            .map(|note| note.start + note.length)
            .fold(first.start, f64::max);
        let glued = Note::new(first.pitch, first.start, end - first.start, first.velocity);
        validate_note(&glued)?;
        let id = glued.id;
        notes.retain(|note| !selected.contains(&note.id));
        notes.push(glued);
        notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
        Ok(id)
    }

    pub fn end_beat(&self) -> f64 {
        self.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(Clip::end)
            .fold(0.0, f64::max)
    }

    pub fn move_clip(&mut self, track_id: u64, clip_id: u64, new_start: f64) -> Result<()> {
        ensure!(
            nonnegative(new_start),
            "Clip position must be finite and nonnegative"
        );
        let clip = self.clip_mut(track_id, clip_id).context("Clip not found")?;
        ensure!(
            valid_range(new_start, clip.length),
            "Clip exceeds the supported timeline"
        );
        clip.start = new_start;
        Ok(())
    }

    /// Split at an absolute arrangement beat, keeping the source intact on both sides.
    pub fn split_clip(&mut self, track_id: u64, clip_id: u64, at_beat: f64) -> Result<u64> {
        let track = self.track_mut(track_id).context("Track not found")?;
        let index = track
            .clips
            .iter()
            .position(|clip| clip.id == clip_id)
            .context("Clip not found")?;
        let clip = &mut track.clips[index];
        ensure!(
            at_beat.is_finite() && at_beat > clip.start && at_beat < clip.end(),
            "Split must be inside the clip"
        );
        let mut right = clip.clone();
        let delta = at_beat - clip.start;
        right.refresh_ids();
        right.start = at_beat;
        right.offset += delta;
        right.length -= delta;
        right.constrain_fades();
        clip.length = delta;
        clip.constrain_fades();
        let id = right.id;
        track.clips.insert(index + 1, right);
        Ok(id)
    }

    pub fn duplicate_clip(&mut self, track_id: u64, clip_id: u64) -> Result<u64> {
        let track = self.track_mut(track_id).context("Track not found")?;
        let mut clip = track
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .context("Clip not found")?
            .clone();
        clip.start = clip.end();
        ensure!(
            valid_range(clip.start, clip.length),
            "Duplicate clip exceeds the supported timeline"
        );
        clip.refresh_ids();
        let id = clip.id;
        track.clips.push(clip);
        Ok(id)
    }

    pub fn delete_clip(&mut self, track_id: u64, clip_id: u64) -> Result<Clip> {
        let track = self.track_mut(track_id).context("Track not found")?;
        let index = track
            .clips
            .iter()
            .position(|clip| clip.id == clip_id)
            .context("Clip not found")?;
        Ok(track.clips.remove(index))
    }

    /// Change clip bounds without changing source events. Extending left cannot
    /// expose time before the source begins; extending right may reveal silence.
    pub fn trim_clip(
        &mut self,
        track_id: u64,
        clip_id: u64,
        new_start: f64,
        new_length: f64,
    ) -> Result<()> {
        ensure!(valid_range(new_start, new_length), "Invalid clip bounds");
        let clip = self.clip_mut(track_id, clip_id).context("Clip not found")?;
        let new_offset = clip.offset + new_start - clip.start;
        ensure!(
            valid_range(new_offset, new_length),
            "Trim extends before the source or overflows its range"
        );
        clip.offset = new_offset;
        clip.start = new_start;
        clip.length = new_length;
        clip.constrain_fades();
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.tempo.is_finite() && (20.0..=400.0).contains(&self.tempo),
            "Tempo must be between 20 and 400 BPM"
        );
        ensure!(
            (1..=32).contains(&self.beats_per_bar),
            "Invalid time signature numerator"
        );
        ensure!(
            nonnegative(self.loop_start)
                && nonnegative(self.loop_end)
                && self.loop_end > self.loop_start,
            "Invalid loop range"
        );
        let mut ids = HashSet::new();
        let mut check_id = |id| -> Result<()> {
            ensure!(
                id > 0 && id < u64::MAX && ids.insert(id),
                "Duplicate or invalid object ID: {id}"
            );
            Ok(())
        };
        for track in &self.tracks {
            check_id(track.id)?;
            ensure!(
                track.gain.is_finite() && track.gain >= 0.0,
                "Invalid track gain"
            );
            ensure!(
                track.pan.is_finite() && (-1.0..=1.0).contains(&track.pan),
                "Invalid track pan"
            );
            if let Instrument::Vst3 { path } = &track.instrument {
                ensure!(!path.is_empty(), "Missing VST3 path");
            }
            for slot in &track.inserts {
                check_id(slot.id)?;
                slot.effect.validate()?;
            }
            for clip in &track.clips {
                check_id(clip.id)?;
                ensure!(valid_range(clip.start, clip.length), "Invalid clip bounds");
                ensure!(
                    valid_range(clip.offset, clip.length),
                    "Invalid source offset"
                );
                match &clip.content {
                    ClipContent::Midi { notes } => {
                        ensure!(
                            track.kind == TrackKind::Midi,
                            "MIDI clip is on an audio track"
                        );
                        for note in notes {
                            check_id(note.id)?;
                            ensure!(
                                note.pitch <= 127 && (1..=127).contains(&note.velocity),
                                "Invalid MIDI pitch or velocity"
                            );
                            ensure!(
                                valid_range(note.start, note.length),
                                "Invalid MIDI note bounds"
                            );
                        }
                    }
                    ClipContent::Audio {
                        path,
                        gain,
                        fade_in,
                        fade_out,
                    } => {
                        ensure!(
                            track.kind == TrackKind::Audio,
                            "Audio clip is on a MIDI track"
                        );
                        ensure!(!path.is_empty(), "Missing audio source path");
                        ensure!(gain.is_finite() && *gain >= 0.0, "Invalid audio gain");
                        ensure!(
                            nonnegative(*fade_in)
                                && nonnegative(*fade_out)
                                && *fade_in <= clip.length
                                && *fade_out <= clip.length,
                            "Invalid audio fades"
                        );
                    }
                }
            }
        }
        self.routing_order()?;
        Ok(())
    }

    /// Write a versioned project atomically beside its destination. Audio remains
    /// referenced externally; relative paths are resolved against the project on load.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate()?;
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let filename = path
            .file_name()
            .context("Project destination needs a filename")?
            .to_string_lossy();
        let temporary = parent.join(format!(
            ".{filename}.{}.{}.tmp",
            std::process::id(),
            new_id()
        ));
        let result = (|| -> Result<()> {
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .context("Create temporary project")?;
            let mut writer = BufWriter::new(file);
            serde_json::to_writer_pretty(
                &mut writer,
                &ProjectFile {
                    version: 1,
                    project: self.clone(),
                },
            )?;
            writer.write_all(b"\n")?;
            writer.flush()?;
            writer.get_ref().sync_all()?;
            fs::rename(&temporary, path).context("Replace project file")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file =
            fs::File::open(path).with_context(|| format!("Open project {}", path.display()))?;
        let envelope: ProjectFile =
            serde_json::from_reader(BufReader::new(file)).context("Read project JSON")?;
        ensure!(
            envelope.version == 1,
            "Unsupported project version {}",
            envelope.version
        );
        let mut project = envelope.project;
        project.validate()?;
        let base = fs::canonicalize(path)?
            .parent()
            .context("Project has no parent directory")?
            .to_path_buf();
        let mut max_id = 0;
        for track in &mut project.tracks {
            max_id = max_id.max(track.id);
            if let Instrument::Vst3 { path } = &mut track.instrument {
                absolutize(path, &base);
            }
            for slot in &mut track.inserts {
                max_id = max_id.max(slot.id);
                if let Effect::Vst3 { path } = &mut slot.effect {
                    absolutize(path, &base);
                }
            }
            for clip in &mut track.clips {
                max_id = max_id.max(clip.id);
                match &mut clip.content {
                    ClipContent::Midi { notes } => {
                        for note in notes {
                            max_id = max_id.max(note.id);
                        }
                    }
                    ClipContent::Audio { path, .. } => absolutize(path, &base),
                }
            }
        }
        NEXT_ID.fetch_max(max_id + 1, Ordering::Relaxed);
        Ok(project)
    }

    pub fn demo() -> Self {
        let mut project = Self {
            name: "Midnight circuit".into(),
            tempo: 112.0,
            loop_enabled: true,
            ..Self::new()
        };
        let mut chords = Track::new("Glass chords", TrackKind::Midi);
        chords.color = [160, 137, 246];
        chords.instrument = Instrument::Triangle;
        chords.gain = 0.34;
        let mut chord_notes = Vec::new();
        for (bar, pitches) in [[57, 60, 64], [53, 57, 60], [48, 52, 55], [55, 59, 62]]
            .iter()
            .enumerate()
        {
            for pitch in pitches {
                chord_notes.push(Note::new(*pitch, bar as f64 * 4.0, 3.5, 84));
            }
        }
        chords
            .clips
            .push(Clip::midi("Am · F · C · G", 0.0, 16.0, chord_notes));
        let mut bass = Track::new("Sub motion", TrackKind::Midi);
        bass.color = [86, 190, 170];
        bass.instrument = Instrument::Sine;
        bass.gain = 0.48;
        let mut bass_notes = Vec::new();
        for (bar, pitch) in [33, 29, 24, 31].iter().enumerate() {
            for beat in [0.0, 1.5, 2.5, 3.0] {
                bass_notes.push(Note::new(*pitch, bar as f64 * 4.0 + beat, 0.45, 103));
            }
        }
        bass.clips
            .push(Clip::midi("Pulse bass", 0.0, 16.0, bass_notes));
        let mut melody = Track::new("Signal lead", TrackKind::Midi);
        melody.color = [234, 176, 100];
        melody.instrument = Instrument::Triangle;
        melody.gain = 0.24;
        let pitches = [
            69, 72, 76, 72, 69, 67, 64, 67, 69, 72, 79, 76, 74, 71, 67, 64,
        ];
        let notes = pitches
            .iter()
            .enumerate()
            .map(|(i, p)| Note::new(*p, i as f64 * 0.5, 0.3, 90))
            .collect();
        melody
            .clips
            .push(Clip::midi("Night sequence", 4.0, 8.0, notes));
        project.tracks = vec![
            chords,
            bass,
            melody,
            Track::new("Audio textures", TrackKind::Audio),
        ];
        project
    }
}

fn absolutize(path: &mut String, base: &Path) {
    if Path::new(path).is_relative() {
        *path = base.join(&*path).to_string_lossy().into_owned();
    }
}

fn nonnegative(value: f64) -> bool {
    value.is_finite() && (0.0..=MAX_BEAT).contains(&value)
}
fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0 && value <= MAX_BEAT
}
fn valid_range(start: f64, length: f64) -> bool {
    nonnegative(start) && positive(length) && start + length <= MAX_BEAT && start + length > start
}

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    version: u32,
    project: Project,
}

/// Bounded undo/redo snapshots; call `commit` immediately BEFORE an edit (once
/// per drag/gesture, not every pointer frame). This type never runs on audio threads.
#[derive(Default)]
pub struct History {
    undo: Vec<Project>,
    redo: Vec<Project>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn commit(&mut self, project: &Project) {
        if self.undo.last() != Some(project) {
            self.undo.push(project.clone());
            if self.undo.len() > 128 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
    }
    pub fn undo(&mut self, project: &mut Project) -> bool {
        if let Some(previous) = self.undo.pop() {
            self.redo.push(std::mem::replace(project, previous));
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self, project: &mut Project) -> bool {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(project, next));
            true
        } else {
            false
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scale {
    Chromatic,
    Major,
    Minor,
    Dorian,
    Pentatonic,
}

impl Scale {
    pub const ALL: [Self; 5] = [
        Self::Chromatic,
        Self::Major,
        Self::Minor,
        Self::Dorian,
        Self::Pentatonic,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Chromatic => "Chromatic",
            Self::Major => "Major",
            Self::Minor => "Natural minor",
            Self::Dorian => "Dorian",
            Self::Pentatonic => "Major pentatonic",
        }
    }
    pub fn contains(self, root: u8, pitch: u8) -> bool {
        let interval = ((u16::from(pitch) + 12 - u16::from(root % 12)) % 12) as u8;
        match self {
            Self::Chromatic => true,
            Self::Major => [0, 2, 4, 5, 7, 9, 11].contains(&interval),
            Self::Minor => [0, 2, 3, 5, 7, 8, 10].contains(&interval),
            Self::Dorian => [0, 2, 3, 5, 7, 9, 10].contains(&interval),
            Self::Pentatonic => [0, 2, 4, 7, 9].contains(&interval),
        }
    }
}

pub fn note_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ];
    format!(
        "{}{}",
        NAMES[usize::from(pitch % 12)],
        i16::from(pitch) / 12 - 1
    )
}

/// Validate note edits before accepting them into a project.
pub fn validate_note(note: &Note) -> Result<()> {
    if note.pitch > 127
        || note.velocity == 0
        || note.velocity > 127
        || !valid_range(note.start, note.length)
    {
        bail!("Invalid MIDI note");
    }
    Ok(())
}

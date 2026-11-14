//! Optional note labels supplied by an instrument, never inferred from its category.
//! Native inspection is asynchronous and isolated; a missing map means piano keys.
use daw_core::{Instrument, SongSession, Track};
use daw_plugins::{NoteName, PluginInfo};
use eframe::egui;
use std::sync::{Arc, Mutex, OnceLock, Weak, mpsc};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NamedNote {
    pub pitch: u8,
    pub name: String,
}

#[derive(Clone)]
struct Configuration {
    instrument: Instrument,
    state: Vec<u8>,
    parameters: Vec<(u32, f64)>,
}

impl Configuration {
    fn matches(&self, track: &Track) -> bool {
        self.instrument == track.instrument
            && self.state == track.instrument_state
            && self.parameters.len() == track.instrument_parameters.len()
            && self
                .parameters
                .iter()
                .zip(&track.instrument_parameters)
                .all(|((id, value), parameter)| *id == parameter.id && *value == parameter.value)
    }
}

type InspectionResult = Mutex<Option<Vec<NamedNote>>>;

struct Request {
    configuration: Arc<Configuration>,
    result: Weak<InspectionResult>,
    ctx: egui::Context,
}

fn worker() -> Option<&'static mpsc::SyncSender<Request>> {
    static WORKER: OnceLock<Option<mpsc::SyncSender<Request>>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<Request>(1);
            std::thread::Builder::new()
                .name("instrument-note-names".into())
                .spawn(move || {
                    for request in receiver {
                        // Moving to another instrument makes queued work unnecessary.
                        let Some(result) = request.result.upgrade() else {
                            continue;
                        };
                        let configuration = &request.configuration;
                        let Instrument::Vst3 { path, class_id } = &configuration.instrument else {
                            continue;
                        };
                        let info = PluginInfo {
                            path: path.into(),
                            class_id: class_id.clone(),
                            ..Default::default()
                        };
                        let names = std::panic::catch_unwind(|| {
                            daw_plugins::inspect_note_names_with_configuration(
                                &info,
                                &configuration.state,
                                &configuration.parameters,
                            )
                        })
                        .ok()
                        .and_then(Result::ok)
                        .map(sanitize)
                        .unwrap_or_default();
                        *result
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(names);
                        request.ctx.request_repaint();
                    }
                })
                .ok()?;
            Some(sender)
        })
        .as_ref()
}

#[derive(Default)]
pub(crate) struct InstrumentNoteNames {
    checked: Option<(SongSession, u64, u64)>,
    owner: Option<(u64, u64)>,
    configuration: Option<Arc<Configuration>>,
    pending: Option<Arc<InspectionResult>>,
    submitted: bool,
    names: Vec<NamedNote>,
}

impl InstrumentNoteNames {
    pub(crate) fn is_loading(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn get(
        &mut self,
        ctx: &egui::Context,
        session: SongSession,
        revision: u64,
        track: &Track,
    ) -> Vec<NamedNote> {
        let checked = (session, revision, track.id);
        if self.checked != Some(checked) {
            self.checked = Some(checked);
            let owner = (session.id, track.id);
            if self.owner != Some(owner)
                || !self
                    .configuration
                    .as_ref()
                    .is_some_and(|saved| saved.matches(track))
            {
                self.owner = Some(owner);
                self.pending = None;
                self.submitted = false;
                self.names = builtin_names(&track.instrument);
                self.configuration = None;
                // Bound the clone before handing the native state to a worker.
                if track.instrument_state.len() <= daw_core::MAX_PLUGIN_STATE_BYTES
                    && track.instrument_parameters.len() <= 4096
                {
                    self.configuration = Some(Arc::new(Configuration {
                        instrument: track.instrument.clone(),
                        state: track.instrument_state.clone(),
                        parameters: track
                            .instrument_parameters
                            .iter()
                            .map(|parameter| (parameter.id, parameter.value))
                            .collect(),
                    }));
                    if matches!(track.instrument, Instrument::Vst3 { .. }) {
                        self.pending = Some(Arc::new(Mutex::new(None)));
                    }
                }
            }
        }
        if let Some(pending) = self.pending.as_ref() {
            let ready = pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            if let Some(names) = ready {
                self.names = names;
                self.pending = None;
            }
        }
        if !self.submitted
            && let Some(pending) = self.pending.as_ref()
        {
            if let Some(worker) = worker() {
                let request = Request {
                    configuration: self.configuration.as_ref().unwrap().clone(),
                    result: Arc::downgrade(pending),
                    ctx: ctx.clone(),
                };
                match worker.try_send(request) {
                    Ok(()) => self.submitted = true,
                    Err(mpsc::TrySendError::Full(_)) => {
                        ctx.request_repaint_after(Duration::from_millis(100))
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => self.pending = None,
                }
            } else {
                self.pending = None;
            }
        }
        self.names.clone()
    }
}

fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn builtin_names(instrument: &Instrument) -> Vec<NamedNote> {
    if instrument.builtin_device() != Some("r1-drums") {
        return Vec::new();
    }
    use daw_devices::devices::drums::{FIRST_NOTE, PADS};
    (0..PADS)
        .map(|pad| {
            let name = instrument
                .sample(pad as u32)
                .map(|sample| {
                    let name = clean_name(&sample.name);
                    if !name.is_empty() {
                        return name;
                    }
                    std::path::Path::new(&sample.path)
                        .file_stem()
                        .map(|stem| clean_name(&stem.to_string_lossy()))
                        .unwrap_or_default()
                })
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("Pad {} · Empty", pad + 1));
            NamedNote {
                pitch: FIRST_NOTE + pad as u8,
                name,
            }
        })
        .collect()
}

/// Normal MIDI notes target input bus 0, channel 0. Other channel-specific names
/// must not relabel those notes; an exact address wins over a wildcard entry.
fn sanitize(names: Vec<NoteName>) -> Vec<NamedNote> {
    let mut pitches: [Option<(u8, String)>; 128] = std::array::from_fn(|_| None);
    for note in names.into_iter().take(4096) {
        if !(-1..=0).contains(&note.port)
            || !(-1..=0).contains(&note.channel)
            || !(0..=127).contains(&note.pitch)
        {
            continue;
        }
        let name = clean_name(&note.name);
        if name.is_empty() {
            continue;
        }
        let rank = u8::from(note.port == 0) + u8::from(note.channel == 0);
        let current = &mut pitches[note.pitch as usize];
        if current
            .as_ref()
            .is_none_or(|(previous, _)| rank > *previous)
        {
            *current = Some((rank, name));
        }
    }
    pitches
        .into_iter()
        .enumerate()
        .filter_map(|(pitch, entry)| {
            entry.map(|(_, name)| NamedNote {
                pitch: pitch as u8,
                name,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_notes_use_only_the_destination_and_prefer_specific_names() {
        let note = |pitch, port, channel, name: &str| NoteName {
            pitch,
            port,
            channel,
            name: name.into(),
        };
        let names = sanitize(vec![
            note(36, -1, -1, "Generic"),
            note(36, 0, 0, "  Kick\n "),
            note(36, 0, 0, "Duplicate"),
            note(38, 0, 1, "Wrong channel"),
            note(39, 1, 0, "Wrong port"),
            note(-1, -1, -1, "All keys"),
            note(128, 0, 0, "Out of range"),
            note(42, 0, 0, "\n\t"),
            note(40, -1, -1, "Snare"),
        ]);
        assert_eq!(
            names,
            vec![
                NamedNote {
                    pitch: 36,
                    name: "Kick".into()
                },
                NamedNote {
                    pitch: 40,
                    name: "Snare".into()
                }
            ]
        );
        assert!(builtin_names(&Instrument::builtin("r1-mini")).is_empty());
    }

    #[test]
    fn drum_names_follow_sample_assignment_and_keep_empty_pads_explicit() {
        let instrument = Instrument::Builtin {
            device: "r1-drums".into(),
            samples: vec![
                daw_core::DeviceSample {
                    slot: 0,
                    path: "/bundled/123.wav".into(),
                    name: "My Kick".into(),
                },
                daw_core::DeviceSample {
                    slot: 2,
                    path: "/samples/Clap.wav".into(),
                    name: String::new(),
                },
            ],
        };
        let names = builtin_names(&instrument);
        assert_eq!(names.len(), 16);
        assert_eq!((names[0].pitch, names[0].name.as_str()), (36, "My Kick"));
        assert_eq!(names[1].name, "Pad 2 · Empty");
        assert_eq!(names[2].name, "Clap");
        assert_eq!(names[15].pitch, 51);
    }

    #[test]
    fn cache_invalidates_on_sample_undo_track_or_document_changes() {
        let ctx = egui::Context::default();
        let mut track = Track::new("Drums", daw_core::TrackKind::Midi);
        track.instrument = Instrument::builtin("r1-drums");
        let mut session = SongSession::default();
        let mut cache = InstrumentNoteNames::default();
        let empty = cache.get(&ctx, session, 0, &track);
        track.instrument = Instrument::Builtin {
            device: "r1-drums".into(),
            samples: vec![daw_core::DeviceSample {
                slot: 0,
                path: "kick.wav".into(),
                name: "Kick".into(),
            }],
        };
        session.bump();
        assert_eq!(cache.get(&ctx, session, 1, &track)[0].name, "Kick");
        track.instrument = Instrument::builtin("r1-drums");
        session.bump();
        assert_eq!(cache.get(&ctx, session, 2, &track), empty);
        track.instrument = Instrument::builtin("r1-mini");
        track.id += 1;
        assert!(cache.get(&ctx, session, 2, &track).is_empty());
        track.instrument = Instrument::builtin("r1-drums");
        session.reset();
        assert_eq!(cache.get(&ctx, session, 2, &track), empty);
        assert!(!cache.is_loading());
    }

    #[test]
    fn late_native_response_cannot_replace_a_new_instruments_names() {
        let ctx = egui::Context::default();
        let mut track = Track::new("Drums", daw_core::TrackKind::Midi);
        track.instrument = Instrument::builtin("r1-drums");
        let mut session = SongSession::default();
        let mut cache = InstrumentNoteNames::default();
        cache.get(&ctx, session, 0, &track);
        let old_result = Arc::new(Mutex::new(None));
        cache.pending = Some(old_result.clone());
        cache.submitted = true;
        track.instrument = Instrument::builtin("r1-mini");
        session.bump();
        assert!(cache.get(&ctx, session, 1, &track).is_empty());
        *old_result.lock().unwrap() = Some(vec![NamedNote {
            pitch: 36,
            name: "Stale Kick".into(),
        }]);
        assert!(cache.get(&ctx, session, 1, &track).is_empty());
        assert!(!cache.is_loading());
    }
}

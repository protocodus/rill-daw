use crate::{settings::Settings, theme::*};
use daw_core::*;
use daw_engine::{AudioEngine, DeviceInfo};
use eframe::egui::{self, RichText};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub use daw_ui::EditorTool as Tool;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorkspaceView {
    Arrangement,
    Clip,
    Mixer,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    Library,
    Setup,
    Inspector,
    Instrument,
    Effects,
    Routing,
    Automation,
    History,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusedView {
    Workspace(WorkspaceView),
    Popup(PopupKind),
}

struct NavigationSnapshot {
    view: FocusedView,
    selected_track: Option<u64>,
    selected_clip: Option<u64>,
    selected_notes: HashSet<u64>,
    last_editor_clip: Option<u64>,
    effect_editor_target: Option<(u64, u64)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SetupPage {
    Audio,
    Appearance,
    Storage,
    Plugins,
}

#[derive(Clone)]
pub enum Drag {
    Clip {
        track: u64,
        clip: u64,
        start: f64,
        length: f64,
        resize: bool,
    },
}

pub struct DawApp {
    pub project: Project,
    pub history: History,
    pub selected_track: Option<u64>,
    pub selected_clip: Option<u64>,
    pub selected_notes: HashSet<u64>,
    pub engine: Option<AudioEngine>,
    pub audio_device_enabled: bool,
    pub(crate) audition_request: Option<(u64, u8, u8)>,
    pub loading: Option<std::sync::mpsc::Receiver<(u64, anyhow::Result<AudioEngine>)>>,
    loading_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    settings_rollback: Option<Settings>,
    retiring: Option<std::sync::mpsc::Receiver<()>>,
    pub(crate) project_io: crate::project_io::ProjectIo,
    pub export_job: Option<std::sync::mpsc::Receiver<anyhow::Result<PathBuf>>>,
    pub revision: u64,
    pub desired_playing: bool,
    pub cursor: f64,
    pub settings: Settings,
    pub setup_draft: Settings,
    pub setup_page: SetupPage,
    pub view: WorkspaceView,
    pub library_open: bool,
    pub inspector_open: bool,
    pub setup_open: bool,
    pub instrument_open: bool,
    pub effect_editor_target: Option<(u64, u64)>,
    navigation: Vec<NavigationSnapshot>,
    pub close_ready: bool,
    screenshot_requested: bool,
    reset_views: bool,
    pub instrument_ui: crate::plugin_ui::InstrumentUi,
    pub effects_open: bool,
    pub routing_open: bool,
    pub automation_open: bool,
    pub history_open: bool,
    pub last_render_errors: u64,
    pub devices: Vec<DeviceInfo>,
    pub status: String,
    pub error: bool,
    pub project_path: Option<PathBuf>,
    pub modified: bool,
    pub edit_dirty: bool,
    pub before_edit: Option<Project>,
    pub(crate) edit_label: Option<String>,
    pub(crate) edit_group: Option<String>,
    pub zoom: f32,
    pub note_zoom: f32,
    pub midi_editor: crate::midi_editor::MidiEditorState,
    pub snap: f64,
    pub root: u8,
    pub scale: Scale,
    pub tool: Tool,
    pub drag: Option<Drag>,
    pub waveforms: HashMap<String, AudioData>,
    pub(crate) waveform_warning: Option<String>,
    pub drop_target: Option<(Option<u64>, f64)>,
    pub vst_paths: Vec<PathBuf>,
    pub vst_catalog: Vec<daw_plugins::catalog::CatalogEntry>,
    pub vst_scan:
        Option<std::sync::mpsc::Receiver<anyhow::Result<Vec<daw_plugins::catalog::CatalogEntry>>>>,
    pub plugin_filter: String,
    pub plugin_search_focus: bool,
    pub plugin_assign_target: Option<u64>,
    pub(crate) library_target: crate::library::LibraryTarget,
    pub instrument_autolaunch: Option<u64>,
    pub last_editor_clip: Option<u64>,
    pub frame_count: usize,
    pub screenshot: Option<PathBuf>,
    pub last_recovery: Instant,
}

impl DawApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        project: Project,
        screenshot: Option<PathBuf>,
    ) -> Self {
        Self::with_context(&cc.egui_ctx, project, screenshot)
    }
    pub fn with_context(
        ctx: &egui::Context,
        project: Project,
        screenshot: Option<PathBuf>,
    ) -> Self {
        crate::theme::apply(ctx);
        let settings = Settings::load();
        crate::theme::set_appearance(ctx, settings.appearance);
        let selected_track = project.tracks.first().map(|t| t.id);
        let selected_clip = project
            .tracks
            .first()
            .and_then(|t| t.clips.first())
            .map(|c| c.id);
        let mut app = Self {
            project,
            history: History::new(),
            selected_track,
            selected_clip,
            selected_notes: HashSet::new(),
            engine: None,
            audio_device_enabled: true,
            audition_request: None,
            loading: None,
            loading_cancel: None,
            settings_rollback: None,
            retiring: None,
            project_io: Default::default(),
            export_job: None,
            revision: 0,
            desired_playing: false,
            cursor: 0.0,
            setup_draft: settings.clone(),
            settings,
            setup_page: SetupPage::Audio,
            view: WorkspaceView::Arrangement,
            library_open: false,
            inspector_open: false,
            setup_open: false,
            instrument_open: false,
            effect_editor_target: None,
            navigation: Vec::new(),
            close_ready: false,
            screenshot_requested: false,
            reset_views: false,
            instrument_ui: Default::default(),
            effects_open: false,
            routing_open: false,
            automation_open: false,
            history_open: false,
            last_render_errors: 0,
            devices: Vec::new(),
            status: "Ready · Space to play · Drop audio anywhere in the arrangement".into(),
            error: false,
            project_path: None,
            modified: false,
            edit_dirty: false,
            before_edit: None,
            edit_label: None,
            edit_group: None,
            zoom: 48.0,
            note_zoom: 78.0,
            midi_editor: Default::default(),
            snap: 0.25,
            root: 0,
            scale: Scale::Minor,
            tool: Tool::Select,
            drag: None,
            waveforms: HashMap::new(),
            waveform_warning: None,
            drop_target: None,
            vst_paths: Vec::new(),
            vst_catalog: Vec::new(),
            vst_scan: None,
            plugin_filter: String::new(),
            plugin_search_focus: false,
            plugin_assign_target: None,
            library_target: crate::library::LibraryTarget::Browse,
            instrument_autolaunch: None,
            last_editor_clip: None,
            frame_count: 0,
            screenshot,
            last_recovery: Instant::now(),
        };
        app.cache_audio();
        app
    }

    fn focused_view(&self) -> FocusedView {
        for (open, kind) in [
            (self.library_open, PopupKind::Library),
            (self.setup_open, PopupKind::Setup),
            (self.inspector_open, PopupKind::Inspector),
            (self.instrument_open, PopupKind::Instrument),
            (self.effects_open, PopupKind::Effects),
            (self.routing_open, PopupKind::Routing),
            (self.automation_open, PopupKind::Automation),
            (self.history_open, PopupKind::History),
        ] {
            if open {
                return FocusedView::Popup(kind);
            }
        }
        FocusedView::Workspace(self.view)
    }

    fn hide_focused_view(&mut self) {
        self.library_open = false;
        self.setup_open = false;
        self.inspector_open = false;
        self.instrument_open = false;
        self.effects_open = false;
        self.routing_open = false;
        self.automation_open = false;
        self.history_open = false;
        self.view = WorkspaceView::Arrangement;
    }

    /// Return to Arrangement deliberately, discarding all utility return paths.
    /// Document changes also use this to prevent restoring stale selections.
    pub fn close_popups(&mut self) {
        self.hide_focused_view();
        self.navigation.clear();
        self.selected_notes.clear();
        self.effect_editor_target = None;
    }

    /// Dismiss only the focused utility, restoring its caller when still valid.
    pub fn close_popup(&mut self) {
        self.hide_focused_view();
        self.effect_editor_target = None;
        while let Some(previous) = self.navigation.pop() {
            if previous
                .selected_track
                .is_some_and(|id| self.project.track(id).is_none())
                || previous
                    .effect_editor_target
                    .is_some_and(|(track, effect)| {
                        self.project
                            .track(track)
                            .is_none_or(|track| !track.inserts.iter().any(|slot| slot.id == effect))
                    })
            {
                continue;
            }
            let clip = previous
                .selected_track
                .zip(previous.selected_clip)
                .and_then(|(track, clip)| self.project.clip(track, clip));
            if previous.view == FocusedView::Workspace(WorkspaceView::Clip) && clip.is_none() {
                continue;
            }
            self.selected_notes = match clip.map(|clip| &clip.content) {
                Some(ClipContent::Midi { notes }) => previous
                    .selected_notes
                    .into_iter()
                    .filter(|id| notes.iter().any(|note| note.id == *id))
                    .collect(),
                _ => HashSet::new(),
            };
            self.selected_track = previous.selected_track;
            self.selected_clip = clip.map(|clip| clip.id);
            self.last_editor_clip = previous.last_editor_clip;
            self.effect_editor_target = previous.effect_editor_target;
            match previous.view {
                FocusedView::Workspace(view) => self.view = view,
                FocusedView::Popup(kind) => self.show_popup(kind),
            }
            return;
        }
        self.selected_notes.clear();
    }

    fn show_popup(&mut self, kind: PopupKind) {
        match kind {
            PopupKind::Library => self.library_open = true,
            PopupKind::Setup => self.setup_open = true,
            PopupKind::Inspector => self.inspector_open = true,
            PopupKind::Instrument => self.instrument_open = true,
            PopupKind::Effects => {
                self.effects_open = true;
                if self.vst_paths.is_empty() && self.vst_scan.is_none() {
                    self.scan_plugins();
                }
            }
            PopupKind::Routing => self.routing_open = true,
            PopupKind::Automation => self.automation_open = true,
            PopupKind::History => self.history_open = true,
        }
    }

    pub fn open_popup(&mut self, kind: PopupKind) {
        let current = self.focused_view();
        if current != FocusedView::Workspace(WorkspaceView::Arrangement)
            && current != FocusedView::Popup(kind)
        {
            self.navigation.push(NavigationSnapshot {
                view: current,
                selected_track: self.selected_track,
                selected_clip: self.selected_clip,
                selected_notes: self.selected_notes.clone(),
                last_editor_clip: self.last_editor_clip,
                effect_editor_target: self.effect_editor_target,
            });
        }
        self.replace_popup(kind);
    }

    /// Complete a chooser flow without returning to that chooser on dismissal.
    pub fn replace_popup(&mut self, kind: PopupKind) {
        self.hide_focused_view();
        if self
            .navigation
            .last()
            .is_some_and(|previous| previous.view == FocusedView::Popup(kind))
        {
            self.navigation.pop();
        }
        self.show_popup(kind);
        match kind {
            PopupKind::Library => {
                self.plugin_search_focus = true;
                self.plugin_assign_target = None;
                self.library_target = crate::library::LibraryTarget::Browse;
                if self.vst_paths.is_empty() && self.vst_scan.is_none() {
                    self.scan_plugins();
                }
            }
            PopupKind::Setup => {
                self.setup_draft = self.settings.clone();
                self.refresh_devices();
            }
            PopupKind::Instrument => {
                self.effect_editor_target = None;
                if self.vst_paths.is_empty() && self.vst_scan.is_none() {
                    self.scan_plugins();
                }
            }
            PopupKind::Inspector
            | PopupKind::Effects
            | PopupKind::Routing
            | PopupKind::Automation
            | PopupKind::History => {}
        }
    }

    pub fn open_workspace(&mut self, view: WorkspaceView) {
        if self.view != view {
            self.selected_notes.clear();
        }
        self.close_popups();
        self.view = view;
    }

    fn refresh_devices(&mut self) {
        match daw_engine::list_output_devices() {
            Ok(devices) => self.devices = devices,
            Err(error) => self.message(Err(error), ""),
        }
    }

    pub fn message(&mut self, result: anyhow::Result<()>, success: &str) {
        match result {
            Ok(()) => {
                self.status = success.into();
                self.error = false;
            }
            Err(e) => {
                self.status = format!("{e:#}");
                self.error = true;
            }
        }
    }
    pub fn begin_edit(&mut self) {
        if self.before_edit.is_none() {
            self.before_edit = Some(self.project.clone());
        }
    }
    pub fn changed(&mut self) {
        self.project.prune_automation();
        self.modified = true;
        self.edit_dirty = true;
    }
    pub fn selected(&self) -> Option<&Clip> {
        self.project.clip(self.selected_track?, self.selected_clip?)
    }
    pub fn selected_mut(&mut self) -> Option<&mut Clip> {
        self.project
            .clip_mut(self.selected_track?, self.selected_clip?)
    }
    pub fn snapped(&self, beat: f64) -> f64 {
        if self.snap > 0.0 {
            (beat / self.snap).round() * self.snap
        } else {
            beat
        }
    }
    pub fn playing(&self) -> bool {
        self.engine.as_ref().is_some_and(|e| e.snapshot().playing)
    }
    pub fn play(&mut self) {
        if self.loading.is_some() {
            self.desired_playing = !self.desired_playing;
            return;
        }
        if let Some(e) = &self.engine {
            self.desired_playing = !e.snapshot().playing;
            e.set_playing(self.desired_playing);
        } else {
            self.prepare_engine(true);
        }
    }

    fn prepare_engine(&mut self, playing: bool) {
        self.desired_playing = playing;
        let previous = self.engine.take();
        if let Some(engine) = &previous {
            engine.set_playing(false);
            let _ = engine.audition_note(None);
        }
        let preparing = self.loading.take();
        let retiring = self.retiring.take();
        if let Some(cancel) = self.loading_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.loading_cancel = Some(std::sync::Arc::clone(&cancel));
        let project = self.project.clone();
        let settings = self.settings.audio();
        let revision = self.revision;
        let (tx, rx) = std::sync::mpsc::channel();
        self.loading = Some(rx);
        std::thread::spawn(move || {
            // Device changes are serialized: release every old stream and sandbox
            // before constructing the replacement. The GUI remains responsive.
            crate::engine_lifecycle::retire(previous, preparing, retiring);
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            let result = AudioEngine::start_cancellable(&project, settings, cancel);
            let _ = tx.send((revision, result));
        });
        self.status = "Preparing audio and isolated plug-ins…".into();
        self.error = false;
    }

    fn update_note_audition(&mut self, ctx: &egui::Context) {
        let note = self.midi_audition(ctx).filter(|(track, _, _)| {
            self.project
                .track(*track)
                .is_some_and(|track| matches!(track.instrument, Instrument::Vst3 { .. }))
                && !self.library_open
                && !self.instrument_open
                && !self.effects_open
                && !self.setup_open
                && !self.routing_open
                && !self.inspector_open
                && !self.automation_open
                && !self.history_open
        });
        if note != self.audition_request {
            self.audition_request = note;
            if note.is_some()
                && self.engine.is_none()
                && self.loading.is_none()
                && self.audio_device_enabled
            {
                self.prepare_engine(false);
            }
        }
        if let Some(engine) = &self.engine
            && let Err(error) = engine.audition_note(note)
        {
            self.message(Err(error), "");
        }
    }
    pub fn retire_engine(&mut self) {
        let previous = self.engine.take();
        if let Some(engine) = &previous {
            engine.set_playing(false);
            let _ = engine.audition_note(None);
        }
        let preparing = self.loading.take();
        if let Some(cancel) = self.loading_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        let retiring = self.retiring.take();
        let (tx, rx) = std::sync::mpsc::channel();
        self.retiring = Some(rx);
        std::thread::spawn(move || {
            crate::engine_lifecycle::retire(previous, preparing, retiring);
            let _ = tx.send(());
        });
    }
    fn poll_engine(&mut self) {
        let ready = match self.loading.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(value)) => Some(value),
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => Some((
                self.revision,
                Err(anyhow::anyhow!(
                    "Audio preparation stopped unexpectedly; press Play to retry"
                )),
            )),
            _ => None,
        };
        if let Some((revision, result)) = ready {
            self.loading = None;
            if revision != self.revision {
                // A stale preparation still owns a stream. Send it through the
                // same retirement chain before opening the current graph, even
                // when the transport is paused during a settings change.
                self.engine = result.ok();
                self.prepare_engine(self.desired_playing);
                return;
            }
            match result {
                Ok(engine) => {
                    self.settings_rollback = None;
                    self.loading_cancel = None;
                    self.last_render_errors = 0;
                    engine.seek(self.cursor);
                    engine.set_playing(self.desired_playing);
                    self.engine = Some(engine);
                    if !self.error {
                        self.status = "Audio ready".into();
                    }
                }
                Err(error) => {
                    self.desired_playing = false;
                    if let Some(previous) = self.settings_rollback.take() {
                        self.settings = previous;
                        self.setup_draft = self.settings.clone();
                        let _ = self.settings.save();
                        self.prepare_engine(false);
                        self.message(Err(anyhow::anyhow!("Audio setup failed; restoring the previous device settings: {error:#}")), "");
                    } else {
                        self.message(Err(error), "");
                    }
                }
            }
        }
    }
    fn stop(&mut self) {
        self.desired_playing = false;
        self.audition_request = None;
        self.retire_engine();
        self.revision = self.revision.wrapping_add(1);
        self.seek(0.0);
    }

    pub fn seek(&mut self, beat: f64) {
        self.cursor = beat.max(0.0);
        if let Some(e) = &self.engine {
            e.seek(self.cursor);
        }
    }
    pub fn rebuild(&mut self) {
        let playing = self.playing() || self.desired_playing;
        if let Some(engine) = &self.engine {
            self.cursor = engine.position_beats();
        }
        if self.engine.is_some() || self.loading.is_some() {
            self.prepare_engine(playing);
        }
    }
    pub(crate) fn finish_settings_apply(&mut self, settings: Settings) {
        if let Err(error) = settings.save() {
            self.message(Err(error), "");
            return;
        }
        let restart = self.engine.is_some() || self.loading.is_some();
        let previous = std::mem::replace(&mut self.settings, settings);
        if restart {
            // Several Apply actions can arrive before preparation completes.
            // Keep the last verified settings, not an unverified intermediate.
            self.settings_rollback.get_or_insert(previous);
        }
        self.revision = self.revision.wrapping_add(1);
        self.rebuild();
        self.scan_plugins();
        self.setup_open = false;
        if !restart {
            self.message(Ok(()), "Setup saved");
        }
    }

    pub(crate) fn apply_appearance_settings(&mut self, settings: Settings) {
        match settings.save() {
            Ok(()) => {
                // A device preparation already in progress may still roll back
                // its audio settings. Preserve the independent appearance choice.
                if let Some(previous) = &mut self.settings_rollback {
                    previous.appearance = settings.appearance;
                }
                self.settings = settings;
                self.setup_open = false;
                self.message(Ok(()), "Setup saved");
            }
            Err(error) => self.message(Err(error), ""),
        }
    }

    pub fn cache_audio(&mut self) {
        use crate::project_io::{WAVEFORM_CACHE_BYTES, WAVEFORM_FILE_BYTES};
        let mut used = self
            .waveforms
            .values()
            .map(|audio| audio.samples.len() * std::mem::size_of::<f32>())
            .sum::<usize>();
        for t in &self.project.tracks {
            for c in &t.clips {
                if let ClipContent::Audio { path, .. } = &c.content
                    && !self.waveforms.contains_key(path)
                    && let Ok(data) = AudioData::load_wav_with_limit(
                        path,
                        WAVEFORM_FILE_BYTES.min(WAVEFORM_CACHE_BYTES.saturating_sub(used)),
                    )
                {
                    used += data.samples.len() * std::mem::size_of::<f32>();
                    self.waveforms.insert(path.clone(), data);
                }
            }
        }
    }
    pub fn import_audio(&mut self, path: &Path, track_id: Option<u64>, beat: f64) {
        let mut preview_limited = false;
        self.begin_edit();
        let result = (|| -> anyhow::Result<()> {
            let source = AudioData::load_wav(path)?;
            let length = source.duration_seconds() * self.project.tempo / 60.0;
            anyhow::ensure!(length > 0.0, "Audio file is empty");
            let name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let id = track_id
                .filter(|id| {
                    self.project
                        .track(*id)
                        .is_some_and(|t| t.kind == TrackKind::Audio)
                })
                .unwrap_or_else(|| self.project.add_audio_track(&name));
            let path = std::fs::canonicalize(path)?.to_string_lossy().to_string();
            let clip = Clip::audio(name, beat.max(0.0), length, path.clone());
            self.selected_track = Some(id);
            self.selected_clip = Some(clip.id);
            self.selected_notes.clear();
            let bytes = source.samples.len() * std::mem::size_of::<f32>();
            let used = self
                .waveforms
                .values()
                .map(|audio| audio.samples.len() * std::mem::size_of::<f32>())
                .sum::<usize>();
            let previous = self
                .waveforms
                .get(&path)
                .map_or(0, |audio| audio.samples.len() * std::mem::size_of::<f32>());
            if bytes <= crate::project_io::WAVEFORM_FILE_BYTES
                && bytes
                    <= crate::project_io::WAVEFORM_CACHE_BYTES
                        .saturating_sub(used.saturating_sub(previous))
            {
                self.waveforms.insert(path, source);
            } else {
                self.waveforms.remove(&path);
                preview_limited = true;
            }
            self.project.track_mut(id).unwrap().clips.push(clip);
            self.changed();
            Ok(())
        })();
        self.message(
            result,
            if preview_limited {
                "Audio imported · Waveform preview exceeds cache limit"
            } else {
                "Audio imported · Drag clip to move, drag its right edge to trim"
            },
        );
    }
    pub(crate) fn autosave_recovery(&mut self) {
        if !self.modified {
            return;
        }
        let result = std::fs::create_dir_all(&self.settings.scratch_dir)
            .map_err(anyhow::Error::from)
            .and_then(|_| {
                self.project
                    .save(self.settings.scratch_dir.join("recovery.daw.json"))
            });
        if let Err(e) = result {
            self.message(Err(e), "");
        }
    }
    fn import_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Audio / MIDI", &["wav", "mid", "midi"])
            .pick_file()
        {
            if path.extension().is_some_and(|e| e == "mid" || e == "midi") {
                self.begin_edit();
                let result = self.project.import_midi(&path, self.cursor).map(|_| ());
                if result.is_ok() {
                    self.changed();
                }
                self.message(result, "MIDI imported");
            } else {
                self.import_audio(&path, self.selected_track, self.cursor);
            }
        }
    }
    fn save(&mut self, save_as: bool) {
        let path = if save_as {
            None
        } else {
            self.project_path.clone()
        }
        .or_else(|| {
            rfd::FileDialog::new()
                .set_file_name(format!("{}.rill", self.project.name))
                .add_filter("Rill project", &["rill"])
                .add_filter("Legacy Rill session", &["json"])
                .save_file()
        });
        if let Some(path) = path {
            self.save_to(path);
        }
    }
    pub(crate) fn finish_new_session(&mut self) {
        self.autosave_recovery();
        self.reset_instrument_ui();
        self.retire_engine();
        self.loading = None;
        self.desired_playing = false;
        self.audition_request = None;
        self.settings_rollback = None;
        self.midi_editor = Default::default();
        self.reset_views = true;
        self.project = Project::new();
        self.waveform_warning = None;
        self.project_path = None;
        self.history.clear();
        self.before_edit = None;
        self.edit_label = None;
        self.edit_group = None;
        self.modified = false;
        self.edit_dirty = false;
        self.revision = self.revision.wrapping_add(1);
        self.cursor = 0.0;
        self.selected_track = None;
        self.selected_clip = None;
        self.selected_notes.clear();
        self.close_popups();
        self.status = "New session · Add a track or drop audio into the arrangement".into();
        self.error = false;
    }
    pub fn extend_selected_clip(&mut self) {
        let selected = self.selected_notes.clone();
        if let Some(clip) = self.selected_mut()
            && let ClipContent::Midi { notes } = &clip.content
        {
            let end = notes
                .iter()
                .filter(|n| selected.contains(&n.id))
                .map(|n| n.start + n.length - clip.offset)
                .fold(clip.length, f64::max);
            clip.length = end;
        }
    }
    fn open(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Rill project", &["rill"])
            .add_filter("Legacy Rill session", &["json"])
            .pick_file()
        {
            self.request_open_project(path, false);
        }
    }
    pub(crate) fn finish_open_project(
        &mut self,
        loaded: crate::project_io::LoadedProject,
        path: PathBuf,
        recovery: bool,
    ) {
        self.autosave_recovery();
        self.reset_instrument_ui();
        self.retire_engine();
        self.loading = None;
        self.desired_playing = false;
        self.audition_request = None;
        self.settings_rollback = None;
        self.midi_editor = Default::default();
        self.reset_views = true;
        self.history.clear();
        self.before_edit = None;
        self.edit_label = None;
        self.edit_group = None;
        self.project = loaded.project;
        self.waveform_warning = loaded.waveform_warning;
        let old_waveforms = std::mem::replace(&mut self.waveforms, loaded.waveforms);
        std::thread::spawn(move || drop(old_waveforms));
        self.project_path = if recovery { None } else { Some(path) };
        self.cursor = 0.0;
        self.selected_track = self.project.tracks.first().map(|t| t.id);
        self.selected_clip = None;
        self.selected_notes.clear();
        self.revision = self.revision.wrapping_add(1);
        self.modified = recovery;
        if recovery {
            self.history.mark_unsaved();
        }
        self.edit_dirty = false;
        self.close_popups();
        self.status = if recovery {
            "Recovery session opened"
        } else {
            "Session opened"
        }
        .into();
        self.error = false;
    }
    fn export(&mut self) {
        if self.export_job.is_some() {
            self.status = "A mixdown is already rendering".into();
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name("mixdown.wav")
            .add_filter("Wave audio", &["wav"])
            .save_file()
        {
            self.export_to(path);
        }
    }

    pub(crate) fn start_export(&mut self, path: PathBuf) {
        if self.export_job.is_some() {
            self.message(Err(anyhow::anyhow!("An export is already running")), "");
            return;
        }
        let project = self.project.clone();
        let rate = self.settings.sample_rate;
        let options = daw_engine::RenderOptions {
            resample_quality: self.settings.resample_quality,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        self.export_job = Some(rx);
        self.status = "Rendering stereo mixdown in background…".into();
        self.error = false;
        std::thread::spawn(move || {
            let result =
                daw_engine::export_project_wav_with_options(&project, &path, rate, options)
                    .map(|_| path);
            let _ = tx.send(result);
        });
    }
    fn poll_export(&mut self) {
        match self.export_job.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(result)) => {
                self.export_job = None;
                match result {
                    Ok(path) => self.message(Ok(()), &format!("Exported {}", path.display())),
                    Err(e) => self.message(Err(e), ""),
                }
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                self.export_job = None;
                self.message(
                    Err(anyhow::anyhow!("Export worker stopped unexpectedly")),
                    "",
                );
            }
            _ => {}
        }
    }
    pub fn delete_selection(&mut self) {
        self.begin_edit();
        if !self.selected_notes.is_empty() {
            let selected = self.selected_notes.clone();
            if let Some(Clip {
                content: ClipContent::Midi { notes },
                ..
            }) = self.selected_mut()
            {
                notes.retain(|n| !selected.contains(&n.id));
                self.changed();
            }
            self.selected_notes.clear();
        } else if let (Some(t), Some(c)) = (self.selected_track, self.selected_clip) {
            let result = self.project.delete_clip(t, c).map(|_| ());
            if result.is_ok() {
                self.selected_clip = None;
                self.changed();
            }
            self.message(result, "Clip deleted");
        }
    }
    pub fn duplicate_selection(&mut self) {
        self.begin_edit();
        if !self.selected_notes.is_empty() {
            let selected = self.selected_notes.clone();
            if let Some(Clip {
                content: ClipContent::Midi { notes },
                ..
            }) = self.selected_mut()
            {
                let chosen: Vec<_> = notes
                    .iter()
                    .filter(|n| selected.contains(&n.id))
                    .cloned()
                    .collect();
                let start = chosen.iter().map(|n| n.start).fold(f64::INFINITY, f64::min);
                let end = chosen
                    .iter()
                    .map(|n| n.start + n.length)
                    .fold(0.0, f64::max);
                let mut ids = HashSet::new();
                for mut n in chosen {
                    n.id = new_id();
                    n.start += end - start;
                    ids.insert(n.id);
                    notes.push(n);
                }
                self.selected_notes = ids;
                self.extend_selected_clip();
                self.changed();
            }
        } else if let (Some(t), Some(c)) = (self.selected_track, self.selected_clip) {
            match self.project.duplicate_clip(t, c) {
                Ok(id) => {
                    self.selected_clip = Some(id);
                    self.changed();
                }
                Err(e) => self.message(Err(e), ""),
            }
        }
    }
    fn keyboard(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            // Native menus and dropdowns process Escape during their own draw.
            // Leave the containing workspace intact until that inner popup closes.
            if !egui::Popup::is_any_open(ctx) {
                if self.midi_editor.cancel_quantize() {
                    ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                    return;
                }
                self.close_popup();
            }
            return;
        }
        let curve_focused = self.automation_open
            && ctx
                .data(|data| data.get_temp::<egui::Id>(egui::Id::new("automation_curve_focus")))
                .is_some_and(|id| ctx.memory(|memory| memory.has_focus(id)));
        if (!ctx.wants_keyboard_input() || curve_focused)
            && ctx.input(|i| {
                i.modifiers.command && (i.key_pressed(egui::Key::Z) || i.key_pressed(egui::Key::Y))
            })
        {
            self.undo_redo(ctx.input(|i| i.modifiers.shift || i.key_pressed(egui::Key::Y)));
            return;
        }
        if self.library_open
            && !egui::Popup::is_any_open(ctx)
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
        {
            ctx.data_mut(|data| data.insert_temp(egui::Id::new("instrument_chooser_enter"), true));
        }
        if self.library_open
            || self.setup_open
            || self.inspector_open
            || self.instrument_open
            || self.effects_open
            || self.routing_open
            || self.automation_open
            || self.history_open
        {
            return;
        }
        if ctx.wants_keyboard_input() {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.play();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Home)) {
            self.seek(0.0);
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            self.save(ctx.input(|i| i.modifiers.shift));
        }
        if self.view == WorkspaceView::Clip
            && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A))
            && let Some(Clip {
                content: ClipContent::Midi { notes },
                ..
            }) = self.selected()
        {
            self.selected_notes = notes.iter().map(|n| n.id).collect();
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O)) {
            self.open();
        }
        if !self.midi_editor.quantize_active()
            && ctx
                .input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            self.delete_selection();
        }
        if !self.midi_editor.quantize_active()
            && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::D))
        {
            self.duplicate_selection();
        }
        for (key, tool) in [
            (egui::Key::Num1, Tool::Select),
            (egui::Key::Num2, Tool::Pencil),
            (egui::Key::Num3, Tool::Duplicate),
            (egui::Key::Num4, Tool::Cut),
            (egui::Key::Num5, Tool::Glue),
        ] {
            if self.view == WorkspaceView::Clip
                && ctx.input(|i| !i.modifiers.any() && i.key_pressed(key))
            {
                self.tool = tool;
            }
        }
    }
    fn transport(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("transport")
            .frame(daw_ui::panel(ctx).inner_margin(12))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("R I L L")
                            .size(18.0)
                            .strong()
                            .color(palette(ui.ctx()).accent),
                    );
                    ui.separator();
                    ui.add_space(12.0);
                    if daw_ui::button(ui, "Start")
                        .on_hover_text("Return to start · Home")
                        .clicked()
                    {
                        self.seek(0.0);
                    }
                    let playing =
                        self.playing() || (self.loading.is_some() && self.desired_playing);
                    if daw_ui::primary_button(ui, if playing { "Pause" } else { "Play" })
                        .on_hover_text("Space")
                        .clicked()
                    {
                        self.play();
                    }
                    if daw_ui::button(ui, "Stop")
                        .on_hover_text("Stop and reset instruments")
                        .clicked()
                    {
                        self.stop();
                    }
                    let mut looping = self.project.loop_enabled;
                    if daw_ui::toggle(ui, &mut looping, "Loop").changed() {
                        self.begin_edit();
                        self.project.loop_enabled = looping;
                        self.changed();
                    }
                    ui.separator();
                    let bar = (self.cursor / self.project.beats_per_bar as f64).floor() as u64 + 1;
                    let beat = (self.cursor % self.project.beats_per_bar as f64).floor() as u64 + 1;
                    ui.label(
                        RichText::new(format!(
                            "{bar:03} . {beat:02} . {:02}",
                            (self.cursor.fract() * 100.0) as u64
                        ))
                        .monospace()
                        .size(20.0)
                        .color(palette(ui.ctx()).accent),
                    );
                    ui.separator();
                    let mut tempo = self.project.tempo;
                    if ui
                        .add(
                            daw_ui::number(&mut tempo)
                                .speed(0.1)
                                .range(20.0..=400.0)
                                .suffix(" BPM"),
                        )
                        .changed()
                    {
                        self.begin_edit();
                        self.project.tempo = tempo;
                        self.changed();
                    }
                    ui.label(
                        RichText::new(format!("{}/4", self.project.beats_per_bar))
                            .color(palette(ui.ctx()).muted),
                    );
                });
            });
    }

    fn menus(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("application-menu")
            .frame(daw_ui::panel(ctx).inner_margin(6))
            .show(ctx, |ui| {
                daw_ui::menu_bar(ui, |ui| {
                    daw_ui::menu_button(ui, "File", |ui| {
                        if daw_ui::button(ui, "New").clicked() {
                            self.new_session();
                            ui.close();
                        }
                        if daw_ui::button(ui, "Open…").on_hover_text("⌘O").clicked() {
                            self.open();
                            ui.close();
                        }
                        if daw_ui::button(ui, "Save").on_hover_text("⌘S").clicked() {
                            self.save(false);
                            ui.close();
                        }
                        if daw_ui::button(ui, "Save as…").clicked() {
                            self.save(true);
                            ui.close();
                        }
                        ui.separator();
                        if daw_ui::button(ui, "Import…").clicked() {
                            self.import_dialog();
                            ui.close();
                        }
                        if daw_ui::button(ui, "Export WAV…").clicked() {
                            self.export();
                            ui.close();
                        }
                        if daw_ui::button(ui, "Export MIDI…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .set_file_name("session.mid")
                                .save_file()
                            {
                                let result = self.project.export_midi(path);
                                self.message(result, "MIDI exported");
                            }
                            ui.close();
                        }
                        ui.separator();
                        if daw_ui::button(ui, "Quit").clicked() {
                            self.request_quit(ctx);
                            ui.close();
                        }
                    });
                    daw_ui::menu_button(ui, "Edit", |ui| {
                        if ui
                            .add_enabled(
                                self.history.can_undo() && !self.state_sync_pending(),
                                daw_ui::button_widget("Undo"),
                            )
                            .on_hover_text(format!(
                                "Undo {} · Ctrl/Cmd+Z",
                                self.history.undo_labels().next().unwrap_or("")
                            ))
                            .clicked()
                        {
                            self.undo_redo(false);
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                self.history.can_redo() && !self.state_sync_pending(),
                                daw_ui::button_widget("Redo"),
                            )
                            .on_hover_text(format!(
                                "Redo {} · Ctrl/Cmd+Shift+Z",
                                self.history.redo_labels().next().unwrap_or("")
                            ))
                            .clicked()
                        {
                            self.undo_redo(true);
                            ui.close();
                        }
                        ui.separator();
                        if daw_ui::button(ui, "History…").clicked() {
                            self.open_popup(PopupKind::History);
                            ui.close();
                        }
                        let has_clip =
                            self.selected().is_some() && !self.midi_editor.quantize_active();
                        if ui
                            .add_enabled(has_clip, daw_ui::button_widget("Duplicate"))
                            .clicked()
                        {
                            self.duplicate_selection();
                            ui.close();
                        }
                        if ui
                            .add_enabled(has_clip, daw_ui::button_widget("Delete"))
                            .clicked()
                        {
                            self.delete_selection();
                            ui.close();
                        }
                        if ui
                            .add_enabled(has_clip, daw_ui::button_widget("Split at cursor"))
                            .clicked()
                        {
                            if let (Some(track), Some(clip)) =
                                (self.selected_track, self.selected_clip)
                            {
                                self.begin_edit();
                                match self.project.split_clip(track, clip, self.cursor) {
                                    Ok(id) => {
                                        self.selected_clip = Some(id);
                                        self.changed();
                                    }
                                    Err(error) => self.message(Err(error), ""),
                                }
                            }
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                self.view == WorkspaceView::Clip && has_clip,
                                daw_ui::button_widget("Select all notes"),
                            )
                            .clicked()
                        {
                            if let Some(Clip {
                                content: ClipContent::Midi { notes },
                                ..
                            }) = self.selected()
                            {
                                self.selected_notes = notes.iter().map(|note| note.id).collect();
                            }
                            ui.close();
                        }
                    });
                    self.track_menu(ui);
                    daw_ui::menu_button(ui, "Window", |ui| {
                        if daw_ui::button(ui, "Arrangement").clicked() {
                            self.open_workspace(WorkspaceView::Arrangement);
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                self.selected().is_some(),
                                daw_ui::button_widget("Clip Editor"),
                            )
                            .clicked()
                        {
                            self.open_workspace(WorkspaceView::Clip);
                            ui.close();
                        }
                        if daw_ui::button(ui, "Mixer").clicked() {
                            self.open_workspace(WorkspaceView::Mixer);
                            ui.close();
                        }
                        ui.separator();
                        let track = self.selected_track.and_then(|id| self.project.track(id));
                        let midi = track.is_some_and(|track| track.kind == TrackKind::Midi);
                        let audio_channel = track.is_some_and(|track| {
                            matches!(
                                track.kind,
                                TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                            )
                        });
                        for (name, kind, enabled) in [
                            ("Instrument", PopupKind::Instrument, midi),
                            ("Effects", PopupKind::Effects, audio_channel),
                            ("Routing", PopupKind::Routing, audio_channel),
                            (
                                "Automation",
                                PopupKind::Automation,
                                track.is_some_and(|track| track.kind != TrackKind::Folder),
                            ),
                            (
                                "Properties",
                                PopupKind::Inspector,
                                self.selected_track.is_some(),
                            ),
                        ] {
                            if ui
                                .add_enabled(enabled, daw_ui::button_widget(name))
                                .clicked()
                            {
                                self.open_popup(kind);
                                ui.close();
                            }
                        }
                        ui.separator();
                        if daw_ui::button(ui, "Library").clicked() {
                            self.open_popup(PopupKind::Library);
                            ui.close();
                        }
                        if daw_ui::button(ui, "Setup").clicked() {
                            self.open_popup(PopupKind::Setup);
                            ui.close();
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{}{}",
                                self.project.name,
                                if self.modified { " •" } else { "" }
                            ))
                            .color(palette(ui.ctx()).muted),
                        );
                    });
                });
            });
    }

    pub(crate) fn undo_redo(&mut self, redo: bool) {
        self.undo_redo_steps(redo, 1);
    }

    pub(crate) fn undo_redo_steps(&mut self, redo: bool, steps: usize) {
        self.midi_editor.cancel_quantize();
        if self.state_sync_pending() {
            return;
        }
        self.flush_pending_edit();
        let editor_state = self.editor_state_signature();
        let selected_start = self
            .selected()
            .map(|clip| clip.start)
            .unwrap_or(self.cursor);
        let mut changed = false;
        for _ in 0..steps {
            let step = if redo {
                self.history.redo(&mut self.project)
            } else {
                self.history.undo(&mut self.project)
            };
            changed |= step;
            if !step {
                break;
            }
        }
        if changed {
            self.reconcile_editor_history(editor_state);
            if self
                .selected_track
                .is_some_and(|id| self.project.track(id).is_none())
            {
                self.selected_track = self
                    .project
                    .tracks
                    .iter()
                    .find(|track| !matches!(track.kind, TrackKind::Folder | TrackKind::Automation))
                    .map(|track| track.id);
            }
            if self.selected_clip.is_some() && self.selected().is_none() {
                self.selected_clip = self
                    .selected_track
                    .and_then(|id| self.project.track(id))
                    .and_then(|track| {
                        track
                            .clips
                            .iter()
                            .min_by(|a, b| {
                                (a.start - selected_start)
                                    .abs()
                                    .total_cmp(&(b.start - selected_start).abs())
                            })
                            .map(|clip| clip.id)
                    });
            }
            let valid_notes: HashSet<u64> = self
                .selected()
                .and_then(|clip| {
                    if let ClipContent::Midi { notes } = &clip.content {
                        Some(notes.iter().map(|note| note.id).collect())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            self.selected_notes.retain(|id| valid_notes.contains(id));
            self.modified = !self.history.is_saved();
            self.revision = self.revision.wrapping_add(1);
            self.rebuild();
            self.cache_audio();
        }
    }

    fn track_menu(&mut self, ui: &mut egui::Ui) {
        daw_ui::menu_button(ui, "Track", |ui| {
            if daw_ui::button(ui, "Add instrument…").clicked() {
                self.open_instrument_chooser(false);
                ui.close();
            }
            for (name, kind) in [
                ("Add MIDI track", TrackKind::Midi),
                ("Add audio track", TrackKind::Audio),
                ("Add bus", TrackKind::Bus),
                ("Add folder", TrackKind::Folder),
            ] {
                if daw_ui::button(ui, name).clicked() {
                    self.begin_edit();
                    self.selected_clip = None;
                    self.selected_track = Some(match kind {
                        TrackKind::Midi => {
                            let track = self.project.add_midi_track("New instrument");
                            let clip = Clip::midi(
                                "New pattern",
                                self.snapped(self.cursor).max(0.0),
                                4.0,
                                Vec::new(),
                            );
                            self.selected_clip = Some(clip.id);
                            self.project.track_mut(track).unwrap().clips.push(clip);
                            track
                        }
                        TrackKind::Audio => self.project.add_audio_track("New audio"),
                        TrackKind::Bus => self.project.add_bus("New bus"),
                        TrackKind::Folder => self.project.add_folder("New folder"),
                        TrackKind::Automation => {
                            unreachable!("Automation requires a parameter target")
                        }
                    });
                    self.selected_notes.clear();
                    self.close_popups();
                    self.changed();
                    if kind == TrackKind::Midi {
                        self.open_instrument_chooser(true);
                    }
                    ui.close();
                }
            }
            ui.separator();
            if daw_ui::button(ui, "Add automation track…").clicked() {
                if let Some(target) = self
                    .selected_track
                    .and_then(|id| self.project.track(id))
                    .and_then(|track| track.automation.as_ref())
                    .map(|lane| lane.target.track)
                {
                    self.selected_track = Some(target);
                }
                self.open_popup(PopupKind::Automation);
                ui.close();
            }
        });
    }

    fn browser(&mut self, ctx: &egui::Context) {
        self.plugin_library(ctx);
    }
    pub(crate) fn scan_plugins(&mut self) {
        let mut dirs = daw_plugins::default_locations();
        dirs.extend(self.settings.vst_dirs.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        self.vst_scan = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(daw_plugins::catalog::PluginCatalog::default().scan(&dirs));
        });
    }
    fn setup(&mut self, ctx: &egui::Context) {
        if !self.setup_open {
            return;
        }
        let mut open = true;
        daw_ui::popup("Setup", ctx).open(&mut open).default_width(520.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                daw_ui::selectable_value(ui, &mut self.setup_page, SetupPage::Audio, "Audio");
                daw_ui::selectable_value(ui, &mut self.setup_page, SetupPage::Appearance, "Appearance");
                daw_ui::selectable_value(ui, &mut self.setup_page, SetupPage::Storage, "Storage");
                daw_ui::selectable_value(ui, &mut self.setup_page, SetupPage::Plugins, "Plug-ins");
            });
            ui.separator();
            ui.add_space(8.0);
            match self.setup_page {
                SetupPage::Appearance => {
                    daw_ui::select_labeled("Theme", self.setup_draft.appearance.name()).show_ui(ui, |ui| {
                        for appearance in [Appearance::System, Appearance::Light, Appearance::Dark] {
                            daw_ui::selectable_value(ui, &mut self.setup_draft.appearance, appearance, appearance.name());
                        }
                    }).response.on_hover_text("System follows the operating system appearance.");
                }
                SetupPage::Audio => {
                    let previous_device = self.setup_draft.device_name.clone();
                    daw_ui::select_labeled("Output", self.setup_draft.device_name.as_deref().unwrap_or("System default")).width(320.0).show_ui(ui, |ui| {
                        daw_ui::selectable_value(ui, &mut self.setup_draft.device_name, None, "System default");
                        for device in &self.devices { daw_ui::selectable_value(ui, &mut self.setup_draft.device_name, Some(device.name.clone()), &device.name); }
                    });
                    let rates = self.devices.iter().find(|d| self.setup_draft.device_name.as_ref().map_or(d.is_default, |name| name == &d.name)).map(|d| d.supported_sample_rates.clone()).unwrap_or_else(|| vec![44_100, 48_000, 88_200, 96_000]);
                    if self.setup_draft.device_name != previous_device && !rates.is_empty() && !rates.contains(&self.setup_draft.sample_rate) {
                        self.setup_draft.sample_rate = if rates.contains(&48_000) { 48_000 } else { rates[0] };
                    }
                    daw_ui::select_labeled("Sample rate", format!("{} Hz", self.setup_draft.sample_rate)).show_ui(ui, |ui| {
                        for rate in rates { daw_ui::selectable_value(ui, &mut self.setup_draft.sample_rate, rate, format!("{rate} Hz")); }
                    });
                    daw_ui::select_labeled("Buffer", format!("{} samples", self.setup_draft.buffer_frames)).show_ui(ui, |ui| {
                        for size in [64, 128, 256, 512, 1024, 2048] { daw_ui::selectable_value(ui, &mut self.setup_draft.buffer_frames, size, format!("{size} samples")); }
                    });
                    daw_ui::select_labeled("Sample conversion", match self.setup_draft.resample_quality { daw_engine::ResampleQuality::High => "High quality", daw_engine::ResampleQuality::Fast => "Fast" }).show_ui(ui, |ui| {
                        daw_ui::selectable_value(ui, &mut self.setup_draft.resample_quality, daw_engine::ResampleQuality::High, "High quality");
                        daw_ui::selectable_value(ui, &mut self.setup_draft.resample_quality, daw_engine::ResampleQuality::Fast, "Fast");
                    }).response.on_hover_text("High quality filters sample-rate conversion; Fast prepares mixed-rate audio sooner. Audio uses 32-bit float with 64-bit summing.");
                    let rate = self.setup_draft.sample_rate.max(1) as f64;
                    ui.label(RichText::new(format!("{:.1} ms device buffer", self.setup_draft.buffer_frames as f64 * 1000.0 / rate)).small().color(palette(ui.ctx()).muted))
                        .on_hover_text(format!("The render queue adds up to {:.1} ms. Device and plug-in latency may add more.", self.setup_draft.buffer_frames.saturating_mul(2).max(512) as f64 * 1000.0 / rate));
                    ui.add_space(8.0);
                    if daw_ui::button(ui, "Refresh devices").clicked() { self.refresh_devices(); }
                }
                SetupPage::Storage => {
                    label(ui, "SCRATCH DIRECTORY");
                    ui.label(self.setup_draft.scratch_dir.display().to_string()).on_hover_text("Recovery snapshots are saved here every 30 seconds while editing.");
                    ui.horizontal(|ui| {
                        if daw_ui::button(ui, "Choose…").clicked() && let Some(path) = rfd::FileDialog::new().pick_folder() { self.setup_draft.scratch_dir = path; }
                        if daw_ui::button(ui, "Recover session").clicked() {
                            self.request_open_project(self.setup_draft.scratch_dir.join("recovery.daw.json"), true);
                        }
                    });
                }
                SetupPage::Plugins => {
                    label(ui, "VST3 SCAN DIRECTORIES");
                    ui.label(RichText::new("System folders included").small().color(palette(ui.ctx()).muted)).on_hover_text(daw_plugins::default_locations().iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
                    let mut remove = None;
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        for (index, path) in self.setup_draft.vst_dirs.iter().enumerate() {
                            ui.push_id(index, |ui| { ui.horizontal(|ui| {
                                ui.label(path.display().to_string());
                                if daw_ui::small_button(ui, "Remove").clicked() { remove = Some(index); }
                            }); });
                        }
                    });
                    if let Some(index) = remove { self.setup_draft.vst_dirs.remove(index); }
                    let path_key = egui::Id::new("plugin_scan_folder_input");
                    let mut folder = ctx.data_mut(|data| data.get_temp::<String>(path_key)).unwrap_or_default();
                    ui.horizontal(|ui| {
                        daw_ui::text_field(ui, &mut folder, "Folder path", 310.0);
                        if daw_ui::button(ui, "Add folder").clicked() {
                            let path = PathBuf::from(folder.trim());
                            if path.is_dir() {
                                if !self.setup_draft.vst_dirs.contains(&path) { self.setup_draft.vst_dirs.push(path); }
                                folder.clear();
                            } else { self.message(Err(anyhow::anyhow!("Choose an existing plug-in folder")), ""); }
                        }
                        if daw_ui::button(ui, "Browse…").clicked() && let Some(path) = rfd::FileDialog::new().pick_folder() && !self.setup_draft.vst_dirs.contains(&path) { self.setup_draft.vst_dirs.push(path); }
                    });
                    ctx.data_mut(|data| data.insert_temp(path_key, folder));
                }
            }
            ui.add_space(16.0);
            ui.separator();
            ui.horizontal(|ui| {
                if daw_ui::primary_button(ui, "Apply").clicked() {
                    self.request_settings_apply();
                }
                if daw_ui::button(ui, "Cancel").clicked() { self.setup_open = false; }
            });
        });
        self.setup_open &= open;
    }
}

impl DawApp {
    pub fn ui_frame(&mut self, ctx: &egui::Context) {
        crate::theme::set_appearance(ctx, self.settings.appearance);
        self.frame_count += 1;
        if std::mem::take(&mut self.reset_views) {
            ctx.data_mut(|data| {
                data.remove::<daw_ui::TimelineViewport>(egui::Id::new(
                    "arrangement_timeline_viewport",
                ));
                for id in ["timeline_diagnostics", "midi_editor_diagnostics"] {
                    data.remove::<serde_json::Value>(egui::Id::new(id));
                }
            });
            crate::audio_editor::reset_audio_editor(ctx);
        }
        if !self.close_ready && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_quit(ctx);
        }
        if ctx.input(|i| {
            i.pointer.any_pressed()
                || i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Key { .. }
                    )
                })
        }) {
            self.begin_edit();
            if ctx.wants_keyboard_input()
                && ctx.input(|i| {
                    i.events
                        .iter()
                        .any(|event| matches!(event, egui::Event::Text(_) | egui::Event::Paste(_)))
                })
            {
                self.edit_group = ctx
                    .memory(|memory| memory.focused())
                    .map(|id| format!("text:{id:?}"));
            }
        }
        self.keyboard(ctx);
        self.poll_engine();
        self.poll_instrument_editor(ctx);
        self.poll_project_io(ctx);
        self.poll_export();
        self.update_note_audition(ctx);
        if let Some(e) = &self.engine {
            self.cursor = e.position_beats();
            let s = e.snapshot();
            if s.render_errors > self.last_render_errors {
                self.last_render_errors = s.render_errors;
                self.status = format!(
                    "Plug-in/render error · {} failures · Stop and restart playback to retry",
                    s.render_errors
                );
                self.error = true;
            }
        }
        if let Some(rx) = &self.vst_scan
            && let Ok(result) = rx.try_recv()
        {
            match result {
                Ok(entries) => {
                    if !self.error {
                        self.status = format!("Found {} VST3 plug-ins", entries.len());
                    }
                    self.vst_paths = entries
                        .iter()
                        .map(|entry| entry.plugin.path.clone())
                        .collect();
                    self.vst_paths.sort();
                    self.vst_paths.dedup();
                    self.vst_catalog = entries;
                }
                Err(e) => self.message(Err(e), ""),
            }
            self.vst_scan = None;
        }
        self.poll_plugin_previews(ctx);
        self.menus(ctx);
        self.transport(ctx);
        if self.error
            || self.loading.is_some()
            || self.export_job.is_some()
            || self.state_sync_pending()
            || self.project_io_pending()
            || self.waveform_warning.is_some()
        {
            egui::TopBottomPanel::bottom("status")
                .frame(daw_ui::panel(ctx).inner_margin(8))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if !self.error && self.waveform_warning.is_none() {
                            ui.spinner();
                        }
                        let status = if !self.error
                            && self.loading.is_none()
                            && self.export_job.is_none()
                            && !self.state_sync_pending()
                            && !self.project_io_pending()
                        {
                            self.waveform_warning.as_deref().unwrap_or(&self.status)
                        } else {
                            &self.status
                        };
                        ui.label(RichText::new(status).size(12.0).color(if self.error {
                            palette(ui.ctx()).orange
                        } else {
                            palette(ui.ctx()).muted
                        }));
                        if self.error && daw_ui::small_button(ui, "Dismiss").clicked() {
                            self.error = false;
                        }
                    });
                });
        }
        if self.view == WorkspaceView::Clip && self.selected().is_none() {
            self.close_popups();
        }
        let utility_open = self.library_open
            || self.setup_open
            || self.inspector_open
            || self.instrument_open
            || self.effects_open
            || self.routing_open
            || self.automation_open
            || self.history_open;
        egui::CentralPanel::default()
            .frame(daw_ui::inset(ctx).inner_margin(0))
            .show(ctx, |ui| {
                ui.add_enabled_ui(
                    self.view == WorkspaceView::Arrangement && !utility_open,
                    |ui| self.arrangement(ui),
                );
            });
        let focused_before_draw = self.focused_view();
        let workspace = self.view;
        if workspace != WorkspaceView::Arrangement {
            let mut open = true;
            daw_ui::workspace_popup(
                if workspace == WorkspaceView::Clip {
                    "Clip Editor"
                } else {
                    "Mixer"
                },
                ctx,
            )
            .id(egui::Id::new("workspace-tool"))
            .open(&mut open)
            .show(ctx, |ui| {
                if workspace == WorkspaceView::Clip {
                    self.editor(ui);
                } else {
                    self.mixer(ui);
                }
            });
            if !open && self.view == workspace {
                self.close_popup();
            }
        }
        self.browser(ctx);
        self.setup(ctx);
        self.inspector(ctx);
        self.instrument_editor(ctx);
        self.channel_rack(ctx);
        self.routing_popup(ctx);
        self.automation_editor(ctx);
        self.history_popup(ctx);
        self.update_note_audition(ctx);
        // Utility windows own their close button and Apply/Cancel flags. Restore
        // their caller only on dismissal, never on a transition to another tool.
        if matches!(focused_before_draw, FocusedView::Popup(_))
            && self.focused_view() == FocusedView::Workspace(WorkspaceView::Arrangement)
        {
            self.close_popup();
        }
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        for file in files {
            if let Some(path) = file.path {
                let (track, beat) = if self.view == WorkspaceView::Arrangement {
                    self.drop_target.unwrap_or((None, self.cursor))
                } else {
                    (None, self.cursor)
                };
                self.close_popups();
                self.import_audio(&path, track, beat);
            }
        }
        if !ctx.input(|i| i.pointer.any_down()) {
            self.flush_pending_edit();
            self.drag = None;
        }
        if self.modified
            && self.last_recovery.elapsed() > Duration::from_secs(30)
            && !self.state_sync_pending()
        {
            self.request_recovery_save();
            self.last_recovery = Instant::now();
        }
        if let Some(path) = &self.screenshot {
            if self.frame_count >= 8 && self.vst_scan.is_none() && !self.screenshot_requested {
                self.screenshot_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            let screenshot = ctx.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Screenshot { image, .. } = e {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(image) = screenshot {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                match image::save_buffer(
                    path,
                    &bytes,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                ) {
                    Ok(()) => println!("Screenshot: {}", path.display()),
                    Err(e) => eprintln!("Screenshot failed: {e}"),
                };
                self.request_quit(ctx);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(
            if self.playing()
                || self.loading.is_some()
                || self.export_job.is_some()
                || self.project_io_pending()
                || self.screenshot.is_some()
            {
                16
            } else {
                100
            },
        ));
    }
}

/// Cosmetic edits never rebuild the audio graph or restart plug-in processes.
pub(crate) fn audio_equivalent(a: &Project, b: &Project) -> bool {
    a.tempo == b.tempo
        && a.beats_per_bar == b.beats_per_bar
        && a.loop_enabled == b.loop_enabled
        && a.loop_start == b.loop_start
        && a.loop_end == b.loop_end
        && a.tracks.len() == b.tracks.len()
        && a.tracks.iter().zip(&b.tracks).all(|(a, b)| {
            a.id == b.id
                && a.kind == b.kind
                && a.gain == b.gain
                && a.pan == b.pan
                && a.mute == b.mute
                && a.solo == b.solo
                && a.instrument == b.instrument
                && a.instrument_state == b.instrument_state
                && a.instrument_parameters == b.instrument_parameters
                && a.latency_samples == b.latency_samples
                && a.automation == b.automation
                && a.inserts == b.inserts
                && a.sends == b.sends
                && a.output_bus == b.output_bus
                && a.parent_folder == b.parent_folder
                && a.clips.len() == b.clips.len()
                && a.clips.iter().zip(&b.clips).all(|(a, b)| {
                    a.start == b.start
                        && a.length == b.length
                        && a.offset == b.offset
                        && a.content == b.content
                })
        })
}

impl eframe::App for DawApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui_frame(ctx);
    }
}

#[cfg(test)]
#[path = "engine_lifecycle_tests.rs"]
mod engine_lifecycle_tests;

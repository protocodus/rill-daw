use crate::{settings::Settings, theme::*};
use daw_core::*;
use daw_engine::{AudioEngine, DeviceInfo};
use eframe::egui::{self, RichText};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Pencil,
    Duplicate,
    Cut,
    Glue,
}

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
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SetupPage {
    Audio,
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
    Notes {
        notes: Vec<Note>,
        resize: bool,
    },
    Marquee {
        origin: egui::Pos2,
    },
}

pub struct DawApp {
    pub project: Project,
    pub history: History,
    pub selected_track: Option<u64>,
    pub selected_clip: Option<u64>,
    pub selected_notes: HashSet<u64>,
    pub engine: Option<AudioEngine>,
    pub loading: Option<std::sync::mpsc::Receiver<(u64, anyhow::Result<AudioEngine>)>>,
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
    pub effects_open: bool,
    pub routing_open: bool,
    pub automation_open: bool,
    pub last_render_errors: u64,
    pub devices: Vec<DeviceInfo>,
    pub status: String,
    pub error: bool,
    pub project_path: Option<PathBuf>,
    pub modified: bool,
    pub edit_dirty: bool,
    pub before_edit: Option<Project>,
    pub zoom: f32,
    pub note_zoom: f32,
    pub snap: f64,
    pub root: u8,
    pub scale: Scale,
    pub tool: Tool,
    pub drag: Option<Drag>,
    pub waveforms: HashMap<String, AudioData>,
    pub drop_target: Option<(Option<u64>, f64)>,
    pub vst_paths: Vec<PathBuf>,
    pub vst_scan: Option<std::sync::mpsc::Receiver<anyhow::Result<Vec<PathBuf>>>>,
    pub plugin_filter: String,
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
            loading: None,
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
            effects_open: false,
            routing_open: false,
            automation_open: false,
            last_render_errors: 0,
            devices: Vec::new(),
            status: "Ready · Space to play · Drop audio anywhere in the arrangement".into(),
            error: false,
            project_path: None,
            modified: false,
            edit_dirty: false,
            before_edit: None,
            zoom: 48.0,
            note_zoom: 78.0,
            snap: 0.25,
            root: 0,
            scale: Scale::Minor,
            tool: Tool::Select,
            drag: None,
            waveforms: HashMap::new(),
            drop_target: None,
            vst_paths: Vec::new(),
            vst_scan: None,
            plugin_filter: String::new(),
            last_editor_clip: None,
            frame_count: 0,
            screenshot,
            last_recovery: Instant::now(),
        };
        app.cache_audio();
        app
    }

    pub fn close_popups(&mut self) {
        if self.view == WorkspaceView::Clip {
            self.selected_notes.clear();
        }
        self.library_open = false;
        self.setup_open = false;
        self.inspector_open = false;
        self.instrument_open = false;
        self.effects_open = false;
        self.routing_open = false;
        self.automation_open = false;
        self.view = WorkspaceView::Arrangement;
    }

    pub fn open_popup(&mut self, kind: PopupKind) {
        self.close_popups();
        match kind {
            PopupKind::Library => {
                self.library_open = true;
                if self.vst_paths.is_empty() && self.vst_scan.is_none() {
                    self.scan_plugins();
                }
            }
            PopupKind::Setup => {
                self.setup_draft = self.settings.clone();
                self.setup_open = true;
                self.refresh_devices();
            }
            PopupKind::Inspector => self.inspector_open = true,
            PopupKind::Instrument => {
                self.instrument_open = true;
                if self.vst_paths.is_empty() && self.vst_scan.is_none() {
                    self.scan_plugins();
                }
            }
            PopupKind::Effects => self.effects_open = true,
            PopupKind::Routing => self.routing_open = true,
            PopupKind::Automation => self.automation_open = true,
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
            self.desired_playing = true;
            let project = self.project.clone();
            let settings = self.settings.audio();
            let revision = self.revision;
            let (tx, rx) = std::sync::mpsc::channel();
            self.loading = Some(rx);
            std::thread::spawn(move || {
                let result = AudioEngine::start(&project, settings);
                let _ = tx.send((revision, result));
            });
            self.status = "Preparing audio and isolated plug-ins…".into();
            self.error = false;
        }
    }
    pub fn retire_engine(&mut self) {
        if let Some(engine) = self.engine.take() {
            engine.set_playing(false);
            std::thread::spawn(move || drop(engine));
        }
    }
    fn poll_engine(&mut self) {
        let ready = match self.loading.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(value)) => Some(value),
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                self.loading = None;
                self.desired_playing = false;
                self.message(
                    Err(anyhow::anyhow!(
                        "Audio preparation stopped unexpectedly; press Play to retry"
                    )),
                    "",
                );
                None
            }
            _ => None,
        };
        if let Some((revision, result)) = ready {
            self.loading = None;
            if revision != self.revision {
                if let Ok(engine) = result {
                    std::thread::spawn(move || drop(engine));
                }
                if self.desired_playing {
                    self.play();
                }
                return;
            }
            match result {
                Ok(engine) => {
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
                    self.message(Err(error), "");
                }
            }
        }
    }
    pub fn seek(&mut self, beat: f64) {
        self.cursor = beat.max(0.0);
        if let Some(e) = &self.engine {
            e.seek(self.cursor);
        }
    }
    pub fn rebuild(&mut self) {
        if self.loading.is_some() {
            return;
        }
        let playing = self.playing();
        if let Some(e) = &self.engine {
            self.cursor = e.position_beats();
        }
        self.retire_engine();
        if playing {
            self.play();
        }
    }
    pub fn cache_audio(&mut self) {
        for t in &self.project.tracks {
            for c in &t.clips {
                if let ClipContent::Audio { path, .. } = &c.content
                    && !self.waveforms.contains_key(path)
                    && let Ok(data) = AudioData::load_wav(path)
                {
                    self.waveforms.insert(path.clone(), data);
                }
            }
        }
    }
    pub fn import_audio(&mut self, path: &Path, track_id: Option<u64>, beat: f64) {
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
            self.waveforms.insert(path, source);
            self.project.track_mut(id).unwrap().clips.push(clip);
            self.changed();
            Ok(())
        })();
        self.message(
            result,
            "Audio imported · Drag clip to move, drag its right edge to trim",
        );
    }
    fn autosave_recovery(&mut self) {
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
                .set_file_name(format!("{}.daw.json", self.project.name))
                .add_filter("Rill session", &["json"])
                .save_file()
        });
        if let Some(path) = path {
            let result = self.project.save(&path);
            if result.is_ok() {
                self.modified = false;
                self.project_path = Some(path);
            }
            self.message(result, "Session saved");
        }
    }
    fn new_session(&mut self) {
        self.autosave_recovery();
        self.retire_engine();
        self.loading = None;
        self.desired_playing = false;
        self.project = Project::new();
        self.project_path = None;
        self.history.clear();
        self.before_edit = None;
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
            .add_filter("Rill session", &["json"])
            .pick_file()
        {
            match Project::load(&path) {
                Ok(project) => {
                    self.autosave_recovery();
                    self.retire_engine();
                    self.loading = None;
                    self.desired_playing = false;
                    self.history.clear();
                    self.before_edit = None;
                    self.project = project;
                    self.project_path = Some(path);
                    self.cursor = 0.0;
                    self.selected_track = self.project.tracks.first().map(|t| t.id);
                    self.selected_clip = None;
                    self.selected_notes.clear();
                    self.cache_audio();
                    self.revision = self.revision.wrapping_add(1);
                    self.modified = false;
                    self.edit_dirty = false;
                    self.close_popups();
                    self.status = "Session opened".into();
                }
                Err(e) => self.message(Err(e), ""),
            }
        }
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

    pub(crate) fn export_to(&mut self, path: PathBuf) {
        if self.export_job.is_some() {
            self.message(Err(anyhow::anyhow!("An export is already running")), "");
            return;
        }
        let project = self.project.clone();
        let rate = self.settings.sample_rate;
        let (tx, rx) = std::sync::mpsc::channel();
        self.export_job = Some(rx);
        self.status = "Rendering stereo mixdown in background…".into();
        self.error = false;
        std::thread::spawn(move || {
            let result = daw_engine::export_project_wav(&project, &path, rate).map(|_| path);
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
            self.close_popups();
        }
        if self.library_open
            || self.setup_open
            || self.inspector_open
            || self.instrument_open
            || self.effects_open
            || self.routing_open
            || self.automation_open
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
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) {
            self.undo_redo(ctx.input(|i| i.modifiers.shift));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
            self.delete_selection();
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::D)) {
            self.duplicate_selection();
        }
        for (key, tool) in [
            (egui::Key::Num1, Tool::Select),
            (egui::Key::Num2, Tool::Pencil),
            (egui::Key::Num3, Tool::Duplicate),
            (egui::Key::Num4, Tool::Cut),
            (egui::Key::Num5, Tool::Glue),
        ] {
            if self.view == WorkspaceView::Clip && ctx.input(|i| i.key_pressed(key)) {
                self.tool = tool;
            }
        }
    }
    fn transport(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("transport")
            .frame(daw_ui::panel().inner_margin(12))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("R I L L").size(18.0).strong().color(ACCENT));
                    ui.separator();
                    ui.add_space(12.0);
                    if daw_ui::button(ui, "|◀")
                        .on_hover_text("Return to start · Home")
                        .clicked()
                    {
                        self.seek(0.0);
                    }
                    let playing =
                        self.playing() || (self.loading.is_some() && self.desired_playing);
                    if daw_ui::primary_button(ui, if playing { "Ⅱ Pause" } else { "▶ Play" })
                        .on_hover_text("Space")
                        .clicked()
                    {
                        self.play();
                    }
                    if daw_ui::button(ui, "■")
                        .on_hover_text("Stop and reset instruments")
                        .clicked()
                    {
                        self.desired_playing = false;
                        self.loading = None;
                        self.retire_engine();
                        self.revision = self.revision.wrapping_add(1);
                        self.seek(0.0);
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
                        .color(ACCENT),
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
                        RichText::new(format!("{}/4", self.project.beats_per_bar)).color(MUTED),
                    );
                });
            });
    }

    fn menus(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("application-menu")
            .frame(daw_ui::panel().inner_margin(6))
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
                            self.autosave_recovery();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            ui.close();
                        }
                    });
                    daw_ui::menu_button(ui, "Edit", |ui| {
                        if ui
                            .add_enabled(self.history.can_undo(), daw_ui::button_widget("Undo"))
                            .clicked()
                        {
                            self.undo_redo(false);
                            ui.close();
                        }
                        if ui
                            .add_enabled(self.history.can_redo(), daw_ui::button_widget("Redo"))
                            .clicked()
                        {
                            self.undo_redo(true);
                            ui.close();
                        }
                        ui.separator();
                        let has_clip = self.selected().is_some();
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
                            .color(MUTED),
                        );
                    });
                });
            });
    }

    fn undo_redo(&mut self, redo: bool) {
        let changed = if redo {
            self.history.redo(&mut self.project)
        } else {
            self.history.undo(&mut self.project)
        };
        if changed {
            self.modified = true;
            self.revision = self.revision.wrapping_add(1);
            self.rebuild();
            self.cache_audio();
        }
    }

    fn track_menu(&mut self, ui: &mut egui::Ui) {
        daw_ui::menu_button(ui, "Track", |ui| {
            for (name, kind) in [
                ("Add MIDI track", TrackKind::Midi),
                ("Add audio track", TrackKind::Audio),
                ("Add bus", TrackKind::Bus),
                ("Add folder", TrackKind::Folder),
            ] {
                if daw_ui::button(ui, name).clicked() {
                    self.begin_edit();
                    self.selected_track = Some(match kind {
                        TrackKind::Midi => self.project.add_midi_track("New instrument"),
                        TrackKind::Audio => self.project.add_audio_track("New audio"),
                        TrackKind::Bus => self.project.add_bus("New bus"),
                        TrackKind::Folder => self.project.add_folder("New folder"),
                        TrackKind::Automation => {
                            unreachable!("Automation requires a parameter target")
                        }
                    });
                    self.selected_clip = None;
                    self.selected_notes.clear();
                    self.close_popups();
                    self.changed();
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

    fn assign_instrument(&mut self, name: &str, instrument: Instrument) {
        self.begin_edit();
        let track = self.selected_track.filter(|id| {
            self.project
                .track(*id)
                .is_some_and(|t| t.kind == TrackKind::Midi)
        });
        let track = track.unwrap_or_else(|| {
            self.selected_clip = None;
            self.project.add_midi_track(name)
        });
        self.selected_track = Some(track);
        let result = self.project.set_instrument(track, instrument);
        if let Err(error) = result {
            self.message(Err(error), "");
            return;
        }
        self.changed();
        self.library_open = false;
    }

    fn browser(&mut self, ctx: &egui::Context) {
        if !self.library_open {
            return;
        }
        let mut open = true;
        daw_ui::popup("Library")
            .open(&mut open)
            .default_width(400.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    daw_ui::text_field(ui, &mut self.plugin_filter, "Search instruments", 280.0);
                    if ui
                        .add_enabled(self.vst_scan.is_none(), daw_ui::button_widget("Scan"))
                        .clicked()
                    {
                        self.scan_plugins();
                    }
                    if self.vst_scan.is_some() {
                        ui.spinner();
                    }
                });
                ui.add_space(8.0);
                let filter = self.plugin_filter.to_lowercase();
                label(ui, "BUILT-IN");
                for (name, instrument) in [
                    ("Triangle", Instrument::Triangle),
                    ("Sine", Instrument::Sine),
                    ("Saw", Instrument::Saw),
                ] {
                    if name.to_lowercase().contains(&filter) && daw_ui::button(ui, name).clicked() {
                        self.assign_instrument(name, instrument);
                    }
                }
                ui.add_space(12.0);
                label(ui, "VST3");
                let mut selected = None;
                egui::ScrollArea::vertical()
                    .max_height(340.0)
                    .show(ui, |ui| {
                        for path in &self.vst_paths {
                            let name = path.file_stem().unwrap_or_default().to_string_lossy();
                            if name.to_lowercase().contains(&filter)
                                && daw_ui::button(ui, name.as_ref())
                                    .on_hover_text(path.display().to_string())
                                    .clicked()
                            {
                                selected = Some((name.into_owned(), path.clone()));
                            }
                        }
                        if self.vst_paths.is_empty() && self.vst_scan.is_none() {
                            ui.label(RichText::new("No plug-ins found").color(MUTED));
                        }
                    });
                if let Some((name, path)) = selected {
                    self.assign_instrument(
                        &name,
                        Instrument::Vst3 {
                            path: path.to_string_lossy().into(),
                        },
                    );
                }
            });
        self.library_open &= open;
    }
    pub(crate) fn scan_plugins(&mut self) {
        let mut dirs = daw_plugins::default_locations();
        dirs.extend(self.settings.vst_dirs.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        self.vst_scan = Some(rx);
        std::thread::spawn(move || {
            let result = daw_plugins::discover_in_paths(&dirs).map(|plugins| {
                let mut paths: Vec<_> = plugins.into_iter().map(|p| p.path).collect();
                paths.sort();
                paths.dedup();
                paths
            });
            let _ = tx.send(result);
        });
    }
    fn setup(&mut self, ctx: &egui::Context) {
        if !self.setup_open {
            return;
        }
        let mut open = true;
        daw_ui::popup("Setup").open(&mut open).default_width(520.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                daw_ui::selectable_value(ui, &mut self.setup_page, SetupPage::Audio, "Audio");
                daw_ui::selectable_value(ui, &mut self.setup_page, SetupPage::Storage, "Storage");
                daw_ui::selectable_value(ui, &mut self.setup_page, SetupPage::Plugins, "Plug-ins");
            });
            ui.separator();
            ui.add_space(8.0);
            match self.setup_page {
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
                    let rate = self.setup_draft.sample_rate.max(1) as f64;
                    ui.label(RichText::new(format!("{:.1} ms device buffer", self.setup_draft.buffer_frames as f64 * 1000.0 / rate)).small().color(MUTED))
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
                            match Project::load(self.setup_draft.scratch_dir.join("recovery.daw.json")) {
                                Ok(project) => {
                                    self.begin_edit(); self.retire_engine(); self.loading = None; self.desired_playing = false;
                                    self.project_path = None; self.project = project; self.selected_track = None;
                                    self.selected_clip = None; self.selected_notes.clear(); self.view = WorkspaceView::Arrangement; self.cursor = 0.0;
                                    self.cache_audio(); self.changed(); self.setup_open = false;
                                    self.message(Ok(()), "Recovery session opened");
                                }
                                Err(error) => self.message(Err(error), ""),
                            }
                        }
                    });
                }
                SetupPage::Plugins => {
                    label(ui, "VST3 SCAN DIRECTORIES");
                    ui.label(RichText::new("System folders included").small().color(MUTED)).on_hover_text(daw_plugins::default_locations().iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
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
                    if daw_ui::button(ui, "+ Folder…").clicked() && let Some(path) = rfd::FileDialog::new().pick_folder() && !self.setup_draft.vst_dirs.contains(&path) { self.setup_draft.vst_dirs.push(path); }
                }
            }
            ui.add_space(16.0);
            ui.separator();
            ui.horizontal(|ui| {
                if daw_ui::primary_button(ui, "Apply").clicked() {
                    let result = self.setup_draft.save();
                    if result.is_ok() {
                        self.settings = self.setup_draft.clone();
                        self.revision = self.revision.wrapping_add(1); self.rebuild(); self.scan_plugins(); self.setup_open = false;
                    }
                    self.message(result, "Setup saved");
                }
                if daw_ui::button(ui, "Cancel").clicked() { self.setup_open = false; }
            });
        });
        self.setup_open &= open;
    }
}

impl DawApp {
    pub fn ui_frame(&mut self, ctx: &egui::Context) {
        self.frame_count += 1;
        if ctx.input(|i| i.viewport().close_requested()) {
            self.autosave_recovery();
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
        }
        self.keyboard(ctx);
        self.poll_engine();
        self.poll_export();
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
                Ok(paths) => {
                    if !self.error {
                        self.status = format!("Found {} VST3 plug-ins", paths.len());
                    }
                    self.vst_paths = paths;
                }
                Err(e) => self.message(Err(e), ""),
            }
            self.vst_scan = None;
        }
        self.menus(ctx);
        self.transport(ctx);
        if self.error || self.loading.is_some() || self.export_job.is_some() {
            egui::TopBottomPanel::bottom("status")
                .frame(daw_ui::panel().inner_margin(8))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if !self.error {
                            ui.spinner();
                        }
                        ui.label(RichText::new(&self.status).size(12.0).color(if self.error {
                            ORANGE
                        } else {
                            MUTED
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
            || self.automation_open;
        egui::CentralPanel::default()
            .frame(daw_ui::inset().inner_margin(0))
            .show(ctx, |ui| {
                ui.add_enabled_ui(
                    self.view == WorkspaceView::Arrangement && !utility_open,
                    |ui| self.arrangement(ui),
                );
            });
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
                self.close_popups();
            }
        }
        self.browser(ctx);
        self.setup(ctx);
        self.inspector(ctx);
        self.instrument_editor(ctx);
        self.channel_rack(ctx);
        self.routing_popup(ctx);
        self.automation_editor(ctx);
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        for file in files {
            if let Some(path) = file.path {
                let (track, beat) = if self.view == WorkspaceView::Arrangement {
                    self.drop_target.unwrap_or((None, self.cursor))
                } else {
                    (None, self.cursor)
                };
                self.view = WorkspaceView::Arrangement;
                self.import_audio(&path, track, beat);
            }
        }
        if !ctx.input(|i| i.pointer.any_down()) {
            if self.edit_dirty {
                let audio_changed = self
                    .before_edit
                    .as_ref()
                    .is_none_or(|before| !audio_equivalent(before, &self.project));
                if let Some(before) = self.before_edit.take()
                    && before != self.project
                {
                    self.history.commit(&before);
                }
                self.edit_dirty = false;
                if audio_changed {
                    self.revision = self.revision.wrapping_add(1);
                    self.rebuild();
                }
            } else {
                self.before_edit = None;
            }
            self.drag = None;
        }
        if self.modified && self.last_recovery.elapsed() > Duration::from_secs(30) {
            if let Err(e) = std::fs::create_dir_all(&self.settings.scratch_dir)
                .map_err(anyhow::Error::from)
                .and_then(|_| {
                    self.project
                        .save(self.settings.scratch_dir.join("recovery.daw.json"))
                })
            {
                self.message(Err(e), "");
            }
            self.last_recovery = Instant::now();
        }
        if let Some(path) = &self.screenshot {
            if self.frame_count == 8 {
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
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(
            if self.playing()
                || self.loading.is_some()
                || self.export_job.is_some()
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
fn audio_equivalent(a: &Project, b: &Project) -> bool {
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

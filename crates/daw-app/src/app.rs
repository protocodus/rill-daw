use crate::{settings::Settings, theme::*};
use daw_core::*;
use daw_engine::{AudioEngine, DeviceInfo};
use eframe::egui;
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
enum ToolWorkspace {
    Arrangement,
    Midi,
}

struct WorkspaceTools {
    active: Option<ToolWorkspace>,
    arrangement: Tool,
    midi: Tool,
}

impl Default for WorkspaceTools {
    fn default() -> Self {
        Self {
            active: Some(ToolWorkspace::Arrangement),
            arrangement: Tool::Select,
            midi: Tool::Select,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InspectorPage {
    Clip,
    Track,
    Channel,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChannelDestination {
    Channel,
    Instrument,
    Effects,
    Routing,
    Mixer,
    PluginEditor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    Assistant,
    About,
    Library,
    Setup,
    Inspector,
    Instrument,
    Effects,
    Routing,
    Automation,
    History,
    Crossfade,
    AudioTakes,
    Recording,
    Join,
    Export,
    ExportMidi,
    Analysis,
    MidiExpression,
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
    inspector_page: InspectorPage,
    backdrop: daw_ui::ViewBackdrop,
    backdrop_title: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SetupPage {
    Ai,
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
        handle: u64,
        original: std::sync::Arc<Clip>,
        mode: ClipDragMode,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClipDragMode {
    Move,
    TrimStart,
    TrimEnd,
    Stretch,
}

#[derive(Clone, Copy)]
pub(crate) enum ClipCommand {
    Split(f64),
    Duplicate,
    Ghost,
    MakeIndependent,
    Delete,
}

pub struct DawApp {
    pub(crate) analysis: crate::analysis_ui::SignalAnalysis,
    pub(crate) midi_expression: Option<crate::midi_expression::MidiExpressionDraft>,
    pub(crate) midi_expression_open: bool,
    render_context: egui::Context,
    pub(crate) assistant: crate::assistant::Assistant,
    pub(crate) tool_joins: crate::join_tools::JoinTools,
    pub(crate) song_session: daw_core::SongSession,
    pub(crate) about_open: bool,
    pub project: Project,
    pub history: History,
    pub selected_track: Option<u64>,
    pub selected_clip: Option<u64>,
    pub(crate) arranger_selection: crate::arranger_selection::ArrangerSelection,
    pub(crate) selection_history: crate::selection_history::SelectionHistory,
    pub(crate) before_selection: Option<crate::selection_history::SelectionSnapshot>,
    pub selected_notes: HashSet<u64>,
    pub engine: Option<AudioEngine>,
    pub audio_device_enabled: bool,
    /// Enabled only by the isolated headless driver; never selects host hardware.
    pub(crate) virtual_audio: bool,
    pub(crate) automatic_plugin_previews: bool,
    pub(crate) audition_request: Option<(u64, u8, u8)>,
    pub loading: Option<std::sync::mpsc::Receiver<(u64, anyhow::Result<AudioEngine>)>>,
    loading_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    settings_rollback: Option<Settings>,
    retiring: Option<std::sync::mpsc::Receiver<()>>,
    pub(crate) project_io: crate::project_io::ProjectIo,
    pub(crate) audio_import: crate::audio_import::AudioImportState,
    pub(crate) freeze: crate::freeze_ui::FreezeState,
    pub(crate) document_guard: Option<crate::document_ui::DocumentGuard>,
    pub(crate) next_document_dialog: Option<crate::document_ui::DocumentDialogSelection>,
    pub export_job:
        Option<std::sync::mpsc::Receiver<anyhow::Result<crate::export_ui::ExportCompletion>>>,
    pub(crate) export_run: Option<crate::export_ui::ExportRun>,
    pub(crate) finishing: crate::finishing_actions::FinishingActions,
    pub(crate) exported_file: Option<PathBuf>,
    pub(crate) export_open: bool,
    pub(crate) midi_export_open: bool,
    pub revision: u64,
    pub desired_playing: bool,
    pub cursor: f64,
    pub settings: Settings,
    pub setup_draft: Settings,
    pub setup_page: SetupPage,
    pub view: WorkspaceView,
    pub(crate) mixer_reveal_channel: Option<u64>,
    pub library_open: bool,
    pub inspector_open: bool,
    pub(crate) inspector_page: InspectorPage,
    pub(crate) inspector_selection: (Option<u64>, Option<u64>),
    pub setup_open: bool,
    pub instrument_open: bool,
    pub effect_editor_target: Option<(u64, u64)>,
    navigation: Vec<NavigationSnapshot>,
    pub(crate) last_scene: daw_ui::ViewBackdrop,
    popup_backdrop: daw_ui::ViewBackdrop,
    backdrop_title: String,
    pub close_ready: bool,
    screenshot_requested: bool,
    reset_views: bool,
    pub instrument_ui: crate::plugin_ui::InstrumentUi,
    pub effects_open: bool,
    pub routing_open: bool,
    pub automation_open: bool,
    pub history_open: bool,
    pub crossfade_open: bool,
    pub join_open: bool,
    pub(crate) audio_takes_open: bool,
    pub(crate) recording_open: bool,
    pub(crate) recording: crate::recording_ui::RecordingUi,
    pub(crate) input_devices: Vec<DeviceInfo>,
    pub(crate) virtual_io: daw_engine::VirtualIoSettings,
    pub(crate) audio_takes_ui: crate::audio_takes_ui::AudioTakesUi,
    pub(crate) join_ui: crate::join_ui::JoinUi,
    pub last_render_errors: u64,
    render_diagnostic_engine: Option<u64>,
    output_dropout_notice: Option<u64>,
    midi_advisory_ack: Option<(u64, u64, u64)>,
    live_edit_error: Option<(u64, String)>,
    pub devices: Vec<DeviceInfo>,
    pub status: String,
    pub error: bool,
    pub(crate) last_error_report: Option<crate::error_ui::ErrorReport>,
    pub(crate) error_details: Option<crate::error_ui::ErrorDetails>,
    pub project_path: Option<PathBuf>,
    pub modified: bool,
    pub edit_dirty: bool,
    pub before_edit: Option<Project>,
    pub(crate) playback_preview_submitted: bool,
    pub(crate) playback_preview_matches_current: bool,
    pub(crate) playback_preview_dirty: bool,
    pub(crate) edit_label: Option<String>,
    pub(crate) edit_group: Option<String>,
    pub zoom: f32,
    pub note_zoom: f32,
    pub midi_editor: crate::midi_editor::MidiEditorState,
    pub snap: f64,
    pub root: u8,
    pub scale: Scale,
    pub tool: Tool,
    workspace_tools: WorkspaceTools,
    pub drag: Option<Drag>,
    pub waveforms: HashMap<String, AudioData>,
    pub(crate) waveform_warning: Option<String>,
    pub drop_target: Option<(Option<u64>, f64)>,
    pub vst_paths: Vec<PathBuf>,
    pub(crate) plugin_scan_roots: Option<Vec<PathBuf>>,
    pub vst_catalog: Vec<daw_plugins::catalog::CatalogEntry>,
    /// Scans, indexes and captures previews in the background; the catalog
    /// above is its latest snapshot.
    pub(crate) plugin_sync: daw_plugins::sync::CatalogSync,
    pub(crate) plugin_catalog_revision: u64,
    pub(crate) plugin_scan_reported: bool,
    pub(crate) plugin_library: crate::settings::PluginLibrary,
    pub plugin_filter: String,
    pub plugin_search_focus: bool,
    pub plugin_assign_target: Option<u64>,
    pub(crate) library_target: crate::library::LibraryTarget,
    pub instrument_autolaunch: Option<u64>,
    pub last_editor_clip: Option<u64>,
    pub frame_count: usize,
    pub screenshot: Option<PathBuf>,
    /// A `--view` the screenshot tool opens once the workspace has laid out.
    pub screenshot_view: Option<String>,
    pub last_recovery: Instant,
}

impl DawApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        project: Project,
        screenshot: Option<PathBuf>,
    ) -> Self {
        if let Some(gl) = &cc.gl {
            daw_ui::enable_backdrop_blur(&cc.egui_ctx, gl);
        }
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
            .and_then(|t| t.clips.iter().find(|clip| !clip.is_take_source()))
            .map(|c| c.id);
        let mut app = Self {
            analysis: Default::default(),
            midi_expression: None,
            midi_expression_open: false,
            render_context: ctx.clone(),
            assistant: Default::default(),
            tool_joins: Default::default(),
            song_session: Default::default(),
            about_open: false,
            project,
            history: History::with_limits(settings.history_limit, daw_core::HISTORY_BYTES),
            selected_track,
            selected_clip,
            arranger_selection: Default::default(),
            selection_history: Default::default(),
            before_selection: None,
            selected_notes: HashSet::new(),
            engine: None,
            audio_device_enabled: true,
            virtual_audio: false,
            automatic_plugin_previews: !cfg!(test),
            audition_request: None,
            loading: None,
            loading_cancel: None,
            settings_rollback: None,
            retiring: None,
            project_io: Default::default(),
            audio_import: Default::default(),
            freeze: Default::default(),
            document_guard: None,
            next_document_dialog: None,
            export_job: None,
            export_run: None,
            finishing: Default::default(),
            exported_file: None,
            export_open: false,
            midi_export_open: false,
            revision: 0,
            desired_playing: false,
            cursor: 0.0,
            setup_draft: settings.clone(),
            settings,
            setup_page: SetupPage::Audio,
            view: WorkspaceView::Arrangement,
            mixer_reveal_channel: None,
            library_open: false,
            inspector_open: false,
            inspector_page: InspectorPage::Clip,
            inspector_selection: (selected_track, selected_clip),
            setup_open: false,
            instrument_open: false,
            effect_editor_target: None,
            navigation: Vec::new(),
            last_scene: Default::default(),
            popup_backdrop: Default::default(),
            backdrop_title: String::new(),
            close_ready: false,
            screenshot_requested: false,
            reset_views: false,
            instrument_ui: Default::default(),
            effects_open: false,
            routing_open: false,
            automation_open: false,
            history_open: false,
            crossfade_open: false,
            join_open: false,
            audio_takes_open: false,
            recording_open: false,
            recording: Default::default(),
            input_devices: Vec::new(),
            virtual_io: Default::default(),
            audio_takes_ui: Default::default(),
            join_ui: Default::default(),
            last_render_errors: 0,
            render_diagnostic_engine: None,
            output_dropout_notice: None,
            midi_advisory_ack: None,
            live_edit_error: None,
            devices: Vec::new(),
            status: "Ready · Space to play · Drop audio anywhere in the arrangement".into(),
            error: false,
            last_error_report: None,
            error_details: None,
            project_path: None,
            modified: false,
            edit_dirty: false,
            before_edit: None,
            playback_preview_submitted: false,
            playback_preview_matches_current: false,
            playback_preview_dirty: false,
            edit_label: None,
            edit_group: None,
            zoom: 48.0,
            note_zoom: 78.0,
            midi_editor: Default::default(),
            snap: 0.25,
            root: 0,
            scale: Scale::Minor,
            tool: Tool::Select,
            workspace_tools: Default::default(),
            drag: None,
            waveforms: HashMap::new(),
            waveform_warning: None,
            drop_target: None,
            vst_paths: Vec::new(),
            plugin_scan_roots: None,
            vst_catalog: Vec::new(),
            plugin_sync: daw_plugins::sync::CatalogSync::new(
                daw_plugins::PluginCatalog::default(),
                daw_plugins::capture_editor_info,
                {
                    let ctx = ctx.clone();
                    move || ctx.request_repaint()
                },
            ),
            plugin_catalog_revision: 0,
            plugin_scan_reported: true,
            plugin_library: crate::settings::PluginLibrary::load(),
            plugin_filter: String::new(),
            plugin_search_focus: false,
            plugin_assign_target: None,
            library_target: crate::library::LibraryTarget::Browse,
            instrument_autolaunch: None,
            last_editor_clip: None,
            frame_count: 0,
            screenshot,
            screenshot_view: None,
            last_recovery: Instant::now(),
        };
        app.cache_audio();
        app
    }

    /// Arranger and piano-roll tools are independent editing choices. Audio
    /// already owns its tool in AudioView; utility navigation must not replace
    /// the caller's choice with the inert arranger behind its backdrop.
    pub(crate) fn sync_workspace_tool(&mut self) {
        let FocusedView::Workspace(view) = self.focused_view() else {
            return;
        };
        let next = match view {
            WorkspaceView::Arrangement => Some(ToolWorkspace::Arrangement),
            WorkspaceView::Clip
                if matches!(
                    self.selected().map(|clip| &clip.content),
                    Some(ClipContent::Midi { .. })
                ) =>
            {
                Some(ToolWorkspace::Midi)
            }
            WorkspaceView::Clip | WorkspaceView::Mixer => None,
        };
        if self.workspace_tools.active == next {
            return;
        }
        match self.workspace_tools.active {
            Some(ToolWorkspace::Arrangement) => self.workspace_tools.arrangement = self.tool,
            Some(ToolWorkspace::Midi) => self.workspace_tools.midi = self.tool,
            None => {}
        }
        self.workspace_tools.active = next;
        match next {
            Some(ToolWorkspace::Arrangement) => self.tool = self.workspace_tools.arrangement,
            Some(ToolWorkspace::Midi) => self.tool = self.workspace_tools.midi,
            None => {}
        }
    }

    fn focused_view(&self) -> FocusedView {
        for (open, kind) in [
            (self.assistant.open, PopupKind::Assistant),
            (self.about_open, PopupKind::About),
            (self.library_open, PopupKind::Library),
            (self.setup_open, PopupKind::Setup),
            (self.inspector_open, PopupKind::Inspector),
            (self.instrument_open, PopupKind::Instrument),
            (self.effects_open, PopupKind::Effects),
            (self.routing_open, PopupKind::Routing),
            (self.automation_open, PopupKind::Automation),
            (self.history_open, PopupKind::History),
            (self.crossfade_open, PopupKind::Crossfade),
            (self.join_open, PopupKind::Join),
            (self.audio_takes_open, PopupKind::AudioTakes),
            (self.recording_open, PopupKind::Recording),
            (self.export_open, PopupKind::Export),
            (self.midi_export_open, PopupKind::ExportMidi),
            (self.analysis.open, PopupKind::Analysis),
            (self.midi_expression_open, PopupKind::MidiExpression),
        ] {
            if open {
                return FocusedView::Popup(kind);
            }
        }
        FocusedView::Workspace(self.view)
    }

    fn hide_focused_view(&mut self) {
        self.midi_expression_open = false;
        self.midi_expression = None;
        self.stop_signal_analysis();
        self.assistant.clear_key_draft();
        self.assistant.open = false;
        self.midi_editor.cancel_generate();
        self.about_open = false;
        self.join_ui.cancel();
        self.library_open = false;
        self.setup_open = false;
        self.inspector_open = false;
        self.instrument_open = false;
        self.effects_open = false;
        self.routing_open = false;
        self.automation_open = false;
        self.history_open = false;
        self.crossfade_open = false;
        self.join_open = false;
        if self.audio_takes_open {
            self.audio_takes_ui.invalidate_import();
        }
        self.audio_takes_open = false;
        self.recording_open = false;
        self.export_open = false;
        self.midi_export_open = false;
        self.view = WorkspaceView::Arrangement;
    }

    /// Return to Arrangement deliberately, discarding all utility return paths.
    /// Document changes also use this to prevent restoring stale selections.
    pub fn close_popups(&mut self) {
        self.hide_focused_view();
        self.navigation.clear();
        self.selected_notes.clear();
        self.midi_editor.clear_hidden_selection();
        self.effect_editor_target = None;
        self.popup_backdrop = Default::default();
        self.backdrop_title.clear();
        self.sync_workspace_tool();
    }

    /// Return from nested utilities without leaving the composition workspace.
    pub(crate) fn close_utility_popups(&mut self) {
        while matches!(self.focused_view(), FocusedView::Popup(_)) {
            self.close_popup();
        }
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
            self.selected_clip = clip.and_then(|clip| {
                if clip.is_take_source() {
                    self.project
                        .audio_take_regions(previous.selected_track?, clip.take_group()?)
                        .first()
                        .map(|region| region.id)
                } else {
                    Some(clip.id)
                }
            });
            self.last_editor_clip = previous.last_editor_clip;
            self.effect_editor_target = previous.effect_editor_target;
            self.inspector_page = previous.inspector_page;
            self.inspector_selection = (self.selected_track, self.selected_clip);
            self.popup_backdrop = previous.backdrop;
            self.backdrop_title = previous.backdrop_title;
            match previous.view {
                FocusedView::Workspace(view) => self.view = view,
                FocusedView::Popup(kind) => self.show_popup(kind),
            }
            self.sync_workspace_tool();
            return;
        }
        self.selected_notes.clear();
        self.midi_editor.clear_hidden_selection();
        self.popup_backdrop = Default::default();
        self.backdrop_title.clear();
        self.sync_workspace_tool();
    }

    fn show_popup(&mut self, kind: PopupKind) {
        match kind {
            PopupKind::Assistant => self.assistant.open = true,
            PopupKind::About => self.about_open = true,
            PopupKind::Library => self.library_open = true,
            PopupKind::Setup => self.setup_open = true,
            PopupKind::Inspector => self.inspector_open = true,
            PopupKind::Instrument => self.instrument_open = true,
            PopupKind::Effects => {
                self.effects_open = true;
                if self.vst_paths.is_empty() && !self.plugin_sync.scanning() {
                    self.refresh_plugins();
                }
            }
            PopupKind::Routing => self.routing_open = true,
            PopupKind::Automation => self.automation_open = true,
            PopupKind::History => self.history_open = true,
            PopupKind::Crossfade => self.crossfade_open = true,
            PopupKind::Join => self.join_open = true,
            PopupKind::AudioTakes => self.audio_takes_open = true,
            PopupKind::Recording => {
                self.refresh_input_devices();
                self.refresh_midi_devices();
                self.recording_open = true;
            }
            PopupKind::Export => self.export_open = true,
            PopupKind::ExportMidi => self.midi_export_open = true,
            PopupKind::Analysis => self.analysis.open = true,
            PopupKind::MidiExpression => self.midi_expression_open = true,
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
                inspector_page: self.inspector_page,
                backdrop: self.popup_backdrop.clone(),
                backdrop_title: self.backdrop_title.clone(),
            });
        }
        if current != FocusedView::Popup(kind) {
            self.popup_backdrop = self.last_scene.clone();
            self.backdrop_title = self.focused_title();
        }
        self.replace_popup(kind);
        if kind == PopupKind::Inspector {
            self.inspector_page = if self.selected().is_some() {
                InspectorPage::Clip
            } else {
                InspectorPage::Track
            };
            self.inspector_selection = (self.selected_track, self.selected_clip);
        }
    }

    fn focused_title(&self) -> String {
        match self.focused_view() {
            FocusedView::Workspace(WorkspaceView::Arrangement) => "Arrangement".into(),
            FocusedView::Workspace(WorkspaceView::Mixer) => "Mixer".into(),
            FocusedView::Workspace(WorkspaceView::Clip) => self
                .selected()
                .map(|clip| format!("Clip Editor · {}", clip.name))
                .unwrap_or_else(|| "Clip Editor".into()),
            FocusedView::Popup(kind) => {
                let label = match kind {
                    PopupKind::Assistant => return "Assistant".into(),
                    PopupKind::About => return "About RILL".into(),
                    PopupKind::Analysis => return self.analysis_title(),
                    PopupKind::Inspector => "Properties",
                    PopupKind::Instrument => "Instrument",
                    PopupKind::Effects => "Effects",
                    PopupKind::Routing => "Routing",
                    PopupKind::Library => "Library",
                    PopupKind::Setup => "Setup",
                    PopupKind::Automation => "Automation",
                    PopupKind::History => "History",
                    PopupKind::Crossfade => "Crossfade",
                    PopupKind::Join => "Join clips",
                    PopupKind::AudioTakes => "Audio takes",
                    PopupKind::Recording => "Recording",
                    PopupKind::Export => "Export WAV",
                    PopupKind::ExportMidi => "Export MIDI 1.0 copy",
                    PopupKind::MidiExpression => "Note expression",
                };
                self.selected_track
                    .and_then(|id| self.project.track(id))
                    .map(|track| format!("{label} · {}", track.name))
                    .unwrap_or_else(|| label.into())
            }
        }
    }

    pub(crate) fn popup_context_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({
            "source": self.backdrop_title,
            "shapes": self.popup_backdrop.shape_count(),
            "depth": self.navigation.len(),
            "interactive": false,
        })
    }

    pub(crate) fn open_channel(&mut self, track: u64) {
        if self
            .project
            .track(track)
            .is_none_or(|track| matches!(track.kind, TrackKind::Folder | TrackKind::Automation))
        {
            return;
        }
        self.open_popup(PopupKind::Inspector);
        self.selected_track = Some(track);
        if self.selected().is_none() {
            self.selected_clip = None;
        }
        self.inspector_page = InspectorPage::Channel;
        self.inspector_selection = (self.selected_track, self.selected_clip);
    }

    /// Related views retain this channel and restore their caller on Back/Escape.
    /// The page links of the selected track's utility windows, in a fixed order.
    pub(crate) fn channel_links(
        &self,
        current: ChannelDestination,
    ) -> Vec<daw_ui::ViewLink<ChannelDestination>> {
        use ChannelDestination::*;
        let Some(track) = self.selected_track.and_then(|id| self.project.track(id)) else {
            return Vec::new();
        };
        // The Channel page lives under the Properties tabs, which already name
        // it; its row only links onward.
        let mut links = Vec::new();
        if current != Channel {
            links.push(daw_ui::ViewLink {
                value: Channel,
                label: "Channel",
                enabled: true,
            });
        }
        if track.kind == TrackKind::Midi {
            links.push(daw_ui::ViewLink {
                value: Instrument,
                label: "Instrument",
                enabled: true,
            });
        }
        links.extend([
            daw_ui::ViewLink {
                value: Effects,
                label: "Effects",
                enabled: true,
            },
            daw_ui::ViewLink {
                value: Routing,
                label: "Routing",
                enabled: true,
            },
            daw_ui::ViewLink {
                value: Mixer,
                label: "Mixer",
                enabled: true,
            },
        ]);
        links
    }

    pub(crate) fn channel_navigation(
        &mut self,
        ui: &mut egui::Ui,
        current: ChannelDestination,
    ) -> bool {
        use ChannelDestination::*;
        let Some(id) = self
            .selected_track
            .filter(|id| self.project.track(*id).is_some())
        else {
            return false;
        };
        let links = self.channel_links(current);
        let Some(destination) = daw_ui::view_links(ui, current, &links) else {
            return false;
        };
        if destination == current {
            return false;
        }
        match destination {
            Channel => self.open_channel(id),
            Instrument => self.open_popup(PopupKind::Instrument),
            Effects => self.open_popup(PopupKind::Effects),
            Routing => self.open_popup(PopupKind::Routing),
            Mixer => {
                self.navigation.push(NavigationSnapshot {
                    view: self.focused_view(),
                    selected_track: self.selected_track,
                    selected_clip: self.selected_clip,
                    selected_notes: self.selected_notes.clone(),
                    last_editor_clip: self.last_editor_clip,
                    effect_editor_target: self.effect_editor_target,
                    inspector_page: self.inspector_page,
                    backdrop: self.popup_backdrop.clone(),
                    backdrop_title: self.backdrop_title.clone(),
                });
                self.hide_focused_view();
                self.mixer_reveal_channel = Some(id);
                self.view = WorkspaceView::Mixer;
            }
            PluginEditor => return false,
        }
        true
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
                if self.vst_paths.is_empty() && !self.plugin_sync.scanning() {
                    self.refresh_plugins();
                }
            }
            PopupKind::Setup => {
                self.assistant.clear_key_draft();
                self.setup_draft = self.settings.clone();
                self.refresh_devices();
            }
            PopupKind::Instrument => {
                self.effect_editor_target = None;
                if self.vst_paths.is_empty() && !self.plugin_sync.scanning() {
                    self.refresh_plugins();
                }
            }
            PopupKind::Join => self.prepare_join(),
            PopupKind::Inspector
            | PopupKind::Assistant
            | PopupKind::Crossfade
            | PopupKind::AudioTakes
            | PopupKind::Recording
            | PopupKind::Effects
            | PopupKind::Routing
            | PopupKind::Automation
            | PopupKind::History
            | PopupKind::Export
            | PopupKind::ExportMidi
            | PopupKind::Analysis
            | PopupKind::MidiExpression
            | PopupKind::About => {}
        }
    }

    pub fn open_workspace(&mut self, view: WorkspaceView) {
        if self.view == WorkspaceView::Mixer
            && view == WorkspaceView::Arrangement
            && let Some(track) = self.selected_track
        {
            self.queue_arranger_channel_reveal(track);
        }
        if self.view != view {
            if view == WorkspaceView::Mixer {
                self.mixer_reveal_channel = self.selected_track;
            }
            if view != WorkspaceView::Mixer {
                self.finishing.clear_channels();
            }
            self.selected_notes.clear();
            self.midi_editor.clear_hidden_selection();
        }
        self.close_popups();
        self.view = view;
        self.sync_workspace_tool();
    }

    pub(crate) fn refresh_devices(&mut self) {
        self.refresh_input_devices();
        self.refresh_midi_devices();
        if !self.audio_device_enabled || self.virtual_audio {
            // Offline/headless work uses the portable render rates, independent
            // of whatever audio hardware happens to be attached to this host.
            self.devices = if self.virtual_audio {
                vec![DeviceInfo {
                    name: "Rill virtual output".into(),
                    is_default: true,
                    supported_sample_rates: vec![44100, 48000, 88200, 96000],
                    channels: 2,
                }]
            } else {
                Vec::new()
            };
            return;
        }
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
                self.last_error_report = None;
            }
            Err(e) => {
                self.status = format!("{e:#}");
                self.error = true;
                self.remember_error(&e);
            }
        }
    }
    pub fn begin_edit(&mut self) {
        if !self.edit_dirty {
            self.before_selection = Some(self.capture_selection());
        }
        if self.before_edit.is_none() {
            self.before_edit = Some(self.project.clone());
        }
    }
    pub fn changed(&mut self) {
        self.assistant.known_parameters.clear();
        if let Some((track, clip)) = self.selected_track.zip(self.selected_clip)
            && self.project.clip(track, clip).is_some()
            && let Err(error) = self.project.synchronize_linked_clip(track, clip)
        {
            if let Some(before) = self.before_edit.take() {
                self.project = before;
            }
            self.edit_dirty = false;
            self.playback_preview_dirty = false;
            self.playback_preview_matches_current = false;
            if std::mem::take(&mut self.playback_preview_submitted) {
                self.revision = self.revision.wrapping_add(1);
                self.update_playback();
            }
            self.edit_label = None;
            self.edit_group = None;
            self.modified = !self.history.is_saved();
            self.message(Err(error), "");
            return;
        }
        self.project.prune_automation();
        self.capture_manual_automation_overrides();
        self.modified = true;
        self.edit_dirty = true;
        self.playback_preview_dirty = true;
        self.playback_preview_matches_current = false;
    }
    pub fn selected(&self) -> Option<&Clip> {
        self.project
            .clip(self.selected_track?, self.selected_clip?)
            .filter(|clip| !clip.is_take_source())
    }
    pub fn selected_mut(&mut self) -> Option<&mut Clip> {
        self.project
            .clip_mut(self.selected_track?, self.selected_clip?)
            .filter(|clip| !clip.is_take_source())
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
        if self.recording_busy() {
            self.finish_recording(crate::recording_ui::AfterRecording::Pause);
            return;
        }
        if self.loading.is_some() {
            self.desired_playing = !self.desired_playing;
            return;
        }
        if let Some(e) = &self.engine {
            let state = e.snapshot();
            if state.output_failed {
                // Play is an explicit retry. Retire the failed stream rather
                // than trying to clear its terminal latch or resetting the song.
                self.prepare_engine(true);
            } else {
                self.desired_playing = !state.playing;
                e.set_playing(self.desired_playing);
            }
        } else {
            self.prepare_engine(true);
        }
    }

    pub(crate) fn prepare_engine(&mut self, playing: bool) {
        self.desired_playing = playing;
        self.retain_mixer_peak_holds(&self.render_context);
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
        let virtual_audio = self.virtual_audio;
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
            let result = if virtual_audio {
                AudioEngine::start_virtual_cancellable(&project, settings, cancel)
            } else {
                AudioEngine::start_cancellable(&project, settings, cancel)
            };
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
                && self.document_guard.is_none()
                && self.error_details.is_none()
                && !self.assistant.open
                && !self.library_open
                && !self.instrument_open
                && !self.effects_open
                && !self.setup_open
                && !self.routing_open
                && !self.inspector_open
                && !self.automation_open
                && !self.history_open
                && !self.export_open
                && !self.midi_export_open
                && !self.analysis.open
                && !self.about_open
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
        self.retain_mixer_peak_holds(&self.render_context);
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
                    self.recording_engine_failed(&error);
                    if let Some(mut previous) = self.settings_rollback.take() {
                        previous.last_project_directory =
                            self.settings.last_project_directory.clone();
                        previous.track_density = self.settings.track_density;
                        previous.export = self.settings.export.clone();
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
    pub(crate) fn stop(&mut self) {
        if self.recording_busy() {
            self.finish_recording(crate::recording_ui::AfterRecording::Stop);
            return;
        }
        self.desired_playing = false;
        self.audition_request = None;
        self.retire_engine();
        self.revision = self.revision.wrapping_add(1);
        self.seek(0.0);
    }

    pub fn seek(&mut self, beat: f64) {
        if self.recording_busy() {
            return;
        }
        self.cursor = beat.max(0.0);
        if let Some(e) = &self.engine {
            e.seek(self.cursor);
        }
    }
    pub fn rebuild(&mut self) {
        if self.recording_busy() {
            self.recording.rebuild_after = true;
            self.finish_recording(crate::recording_ui::AfterRecording::Pause);
            return;
        }
        let playing = self.playing() || self.desired_playing;
        if let Some(engine) = &self.engine {
            self.cursor = engine.position_beats();
        }
        if self.engine.is_some() || self.loading.is_some() {
            self.prepare_engine(playing);
        }
    }
    /// Musical edits use the existing stream and plugin instances when their
    /// topology is compatible. Device/structural changes retain full preparation.
    pub(crate) fn update_playback(&mut self) {
        let update = self
            .engine
            .as_ref()
            .map(|engine| engine.try_update_project(&self.project, self.revision));
        match update {
            Some(Ok(true)) => {}
            // A rejected edit must leave the last valid audio playing. The
            // project remains editable/savable and a subsequent edit can retry.
            Some(Err(error)) => self.message(Err(error), ""),
            Some(Ok(false)) | None => self.rebuild(),
        }
    }
    /// Preview compatible edits each gesture frame, retaining the original
    /// project for one Undo entry when the pointer is released. Structural
    /// changes wait for the commit rather than repeatedly rebuilding a graph.
    pub(crate) fn preview_playback_edit(&mut self) {
        if !std::mem::take(&mut self.playback_preview_dirty) || !self.edit_dirty {
            return;
        }
        if !self.playback_preview_submitted
            && self
                .before_edit
                .as_ref()
                .is_some_and(|previous| audio_equivalent(previous, &self.project))
        {
            return;
        }
        let revision = self.revision.wrapping_add(1);
        let result = self
            .engine
            .as_ref()
            .map(|engine| engine.try_update_project(&self.project, revision));
        match result {
            Some(Ok(true)) => {
                self.revision = revision;
                self.playback_preview_submitted = true;
                self.playback_preview_matches_current = true;
            }
            Some(Err(error)) => self.message(Err(error), ""),
            Some(Ok(false)) => {}
            // Engine startup may finish while the pointer remains still. Keep
            // this value pending so it reaches that engine without another move.
            None => self.playback_preview_dirty = true,
        }
    }
    fn poll_live_edits(&mut self) {
        let Some(engine) = &self.engine else { return };
        let snapshot = engine.snapshot();
        if let Some(error) = engine.take_edit_error() {
            self.live_edit_error = Some((snapshot.engine_id, error.clone()));
            self.message(Err(anyhow::anyhow!(error)), "");
        } else if self.live_edit_error.as_ref().is_some_and(|(engine_id, _)| {
            *engine_id != snapshot.engine_id
                || snapshot.applied_revision == snapshot.requested_revision
        }) {
            let (_, message) = self.live_edit_error.take().unwrap();
            // A later successful edit/Undo resolves this failure without
            // dismissing an unrelated save, export, or device error.
            if self.error && self.status == message {
                self.message(Ok(()), "Audio ready");
            }
        }
    }
    pub(crate) fn finish_settings_apply(&mut self, mut settings: Settings) {
        settings.last_project_directory = self.settings.last_project_directory.clone();
        // A native-editor barrier may have delayed this audio request while
        // the user changed a presentation or delivery preference.
        settings.track_density = self.settings.track_density;
        settings.export = self.settings.export.clone();
        self.assistant.known_parameters.clear();
        if let Err(error) = settings.save() {
            self.message(Err(error), "");
            return;
        }
        let restart = self.engine.is_some() || self.loading.is_some();
        let mut previous = std::mem::replace(&mut self.settings, settings);
        self.apply_history_limit();
        // Audio rollback must not undo an independent history preference: its
        // discarded snapshots cannot be restored by changing settings back.
        previous.history_limit = self.settings.history_limit;
        previous.track_density = self.settings.track_density;
        previous.export = self.settings.export.clone();
        if let Some(rollback) = &mut self.settings_rollback {
            rollback.history_limit = self.settings.history_limit;
            rollback.track_density = self.settings.track_density;
            rollback.export = self.settings.export.clone();
        }
        if restart {
            // Several Apply actions can arrive before preparation completes.
            // Keep the last verified settings, not an unverified intermediate.
            self.settings_rollback.get_or_insert(previous);
        }
        self.revision = self.revision.wrapping_add(1);
        self.rebuild();
        self.refresh_plugins();
        self.setup_open = false;
        if !restart {
            self.message(Ok(()), "Setup saved");
        }
    }

    pub(crate) fn apply_appearance_settings(&mut self, mut settings: Settings) {
        settings.last_project_directory = self.settings.last_project_directory.clone();
        match settings.save() {
            Ok(()) => {
                // A device preparation already in progress may still roll back
                // its audio settings. Preserve the independent appearance choice.
                if let Some(previous) = &mut self.settings_rollback {
                    previous.appearance = settings.appearance;
                    previous.ai = settings.ai.clone();
                    previous.history_limit = settings.history_limit;
                    previous.track_density = settings.track_density;
                    previous.export = settings.export.clone();
                }
                self.settings = settings;
                self.apply_history_limit();
                self.setup_open = false;
                self.message(Ok(()), "Setup saved");
            }
            Err(error) => self.message(Err(error), ""),
        }
    }

    /// Persist view/delivery choices without touching the song or audio graph.
    /// Pending audio settings and a later device rollback must retain them.
    pub(crate) fn save_independent_preferences(&mut self) -> anyhow::Result<()> {
        self.settings.save()?;
        self.setup_draft.track_density = self.settings.track_density;
        self.setup_draft.export = self.settings.export.clone();
        if let Some(previous) = &mut self.settings_rollback {
            previous.track_density = self.settings.track_density;
            previous.export = self.settings.export.clone();
        }
        Ok(())
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
        self.queue_audio_import(vec![path.to_path_buf()], track_id, beat);
    }
    pub(crate) fn autosave_recovery(&mut self) {
        self.queue_recovery_snapshot();
    }
    pub(crate) fn import_dialog(&mut self) {
        if !self.import_picker_ready() {
            return;
        }
        if let Some(path) = self.pick_document_path(crate::document_ui::DocumentDialog::Import) {
            if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("mid") || extension.eq_ignore_ascii_case("midi")
                })
            {
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
        .or_else(|| self.pick_document_path(crate::document_ui::DocumentDialog::Save));
        if let Some(path) = path {
            self.save_to(path);
        }
    }
    pub(crate) fn finish_new_session(&mut self) {
        self.cancel_audio_import();
        self.invalidate_assistant_session();
        self.autosave_recovery();
        self.reset_instrument_ui();
        self.retire_engine();
        self.loading = None;
        self.desired_playing = false;
        self.audition_request = None;
        self.settings_rollback = None;
        self.midi_editor = Default::default();
        self.reset_views = true;
        self.recording = Default::default();
        self.project = Project::new();
        let old_waveforms = std::mem::take(&mut self.waveforms);
        if !old_waveforms.is_empty() {
            std::thread::spawn(move || drop(old_waveforms));
        }
        self.exported_file = None;
        self.export_run = None;
        self.waveform_warning = None;
        self.project_path = None;
        self.history.clear();
        self.selection_history = Default::default();
        self.arranger_selection = Default::default();
        self.before_selection = None;
        self.finishing = Default::default();
        self.before_edit = None;
        self.playback_preview_submitted = false;
        self.playback_preview_matches_current = false;
        self.playback_preview_dirty = false;
        self.edit_label = None;
        self.edit_group = None;
        self.modified = false;
        self.edit_dirty = false;
        self.revision = self.revision.wrapping_add(1);
        self.cursor = 0.0;
        self.selected_track = None;
        self.selected_clip = None;
        self.selected_notes.clear();
        self.midi_editor.clear_hidden_selection();
        self.close_popups();
        self.status = "New session · Add a track or drop audio into the arrangement".into();
        self.error = false;
    }
    pub fn extend_selected_clip(&mut self) {
        let selected = self.selected_notes.clone();
        let initial_length = self
            .selected_track
            .zip(self.selected_clip)
            .and_then(|(track, id)| {
                let original = self.before_edit.as_ref()?.clip(track, id)?;
                let current = self.project.clip(track, id)?;
                (original.start == current.start
                    && original.offset == current.offset
                    && original.source_rate == current.source_rate)
                    .then_some(original.length)
            });
        if let Some(clip) = self.selected_mut()
            && let ClipContent::Midi { notes } = &clip.content
        {
            let end = notes
                .iter()
                .filter(|n| selected.contains(&n.id))
                .map(|n| clip.source_to_local(n.start + n.length))
                // Base each preview on the transaction's original window, so
                // returning from outside the clip also reverses its growth.
                .fold(initial_length.unwrap_or(clip.length), f64::max);
            clip.length = end;
        }
    }

    /// Accept one MIDI gesture update only after its source and instance bounds
    /// are valid. A rejected frame restores the preceding valid frame, including
    /// peers already updated by a core note operation, without losing the drag's
    /// original undo checkpoint. This scans only the edited source's notes.
    pub(crate) fn finish_midi_edit(
        &mut self,
        track: u64,
        before: Clip,
        selection: HashSet<u64>,
        result: anyhow::Result<()>,
        extend: bool,
        label: Option<&str>,
    ) {
        let clip_id = before.id;
        let result = result.and_then(|()| {
            if extend {
                self.extend_selected_clip();
            }
            let clip = self
                .project
                .clip(track, clip_id)
                .ok_or_else(|| anyhow::anyhow!("Clip not found"))?;
            let valid_range = |start: f64, length: f64| {
                start.is_finite()
                    && start >= 0.0
                    && length.is_finite()
                    && length > 0.0
                    && start + length <= MAX_BEAT
                    && start + length > start
            };
            anyhow::ensure!(
                valid_range(clip.start, clip.length)
                    && valid_range(clip.offset, clip.source_length()),
                "Note edit exceeds the supported clip range"
            );
            let ClipContent::Midi { notes } = &clip.content else {
                anyhow::bail!("Select a MIDI clip");
            };
            for note in notes {
                validate_note(note)?;
            }
            self.project.synchronize_linked_clip(track, clip_id)
        });
        match result {
            Ok(()) => {
                if let Some(label) = label {
                    self.edit_label = Some(label.into());
                }
                self.queue_midi_overlap_merge(track, clip_id);
                self.changed();
            }
            Err(error) => {
                if let Some(clip) = self.project.clip_mut(track, clip_id) {
                    *clip = before;
                }
                // Cut/Duplicate/Glue publish their source immediately. Restore
                // that shared source as well, retaining every peer's placement.
                let rollback = self.project.synchronize_linked_clip(track, clip_id);
                self.selected_notes = selection;
                self.message(rollback.and(Err(error)), "");
            }
        }
    }
    fn open(&mut self) {
        if let Some(path) = self.pick_document_path(crate::document_ui::DocumentDialog::Open) {
            self.request_document_transition(crate::document_ui::DocumentTransition::Open {
                path,
                recovery: false,
            });
        }
    }
    pub(crate) fn finish_open_project(
        &mut self,
        loaded: crate::project_io::LoadedProject,
        path: PathBuf,
        recovery: bool,
    ) {
        self.cancel_audio_import();
        self.invalidate_assistant_session();
        self.autosave_recovery();
        self.reset_instrument_ui();
        self.retire_engine();
        self.loading = None;
        self.desired_playing = false;
        self.audition_request = None;
        self.settings_rollback = None;
        self.midi_editor = Default::default();
        self.reset_views = true;
        self.recording = Default::default();
        self.history.clear();
        self.selection_history = Default::default();
        self.arranger_selection = Default::default();
        self.before_selection = None;
        self.finishing = Default::default();
        self.before_edit = None;
        self.playback_preview_submitted = false;
        self.playback_preview_matches_current = false;
        self.playback_preview_dirty = false;
        self.edit_label = None;
        self.edit_group = None;
        self.project = loaded.project;
        self.exported_file = None;
        self.export_run = None;
        self.waveform_warning = loaded.waveform_warning;
        let old_waveforms = std::mem::replace(&mut self.waveforms, loaded.waveforms);
        std::thread::spawn(move || drop(old_waveforms));
        self.project_path = if recovery { None } else { Some(path) };
        self.cursor = 0.0;
        self.selected_track = self.project.tracks.first().map(|t| t.id);
        self.selected_clip = None;
        self.selected_notes.clear();
        self.midi_editor.clear_hidden_selection();
        self.revision = self.revision.wrapping_add(1);
        self.modified = recovery;
        if recovery {
            self.history.mark_unsaved();
        }
        self.edit_dirty = false;
        self.close_popups();
        self.status = if recovery && loaded.previous_recovery {
            "Previous recovery opened · Latest checkpoint unavailable"
        } else if recovery {
            "Recovery session opened"
        } else {
            "Session opened"
        }
        .into();
        self.error = false;
    }
    fn poll_export(&mut self) {
        match self.export_job.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(result)) => {
                self.export_job = None;
                match result {
                    Ok(completion) => {
                        let path = completion.path;
                        self.export_run = None;
                        self.finishing.export_finished(&path, completion.report);
                        // A finished render must not erase an unrelated error.
                        if !self.error
                            && self.loading.is_none()
                            && !self.state_sync_pending()
                            && !self.project_io_pending()
                        {
                            self.status = format!("Exported {}", path.display());
                        }
                        self.exported_file = Some(path);
                    }
                    Err(e) => {
                        self.finishing.export_failed();
                        self.export_failed_notice(e);
                    }
                }
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                self.export_job = None;
                self.finishing.export_failed();
                self.export_failed_notice(anyhow::anyhow!("Export worker stopped unexpectedly"));
            }
            _ => {}
        }
    }

    fn status_notice(&mut self, ctx: &egui::Context) {
        use daw_ui::NoticeKind;
        let busy = self.loading.is_some()
            || self.export_job.is_some()
            || self.state_sync_pending()
            || self.project_io_pending()
            || self.audio_import_busy()
            || self.freeze_busy();
        let export_report = self.export_error_report().cloned();
        let show_export_failure = export_report
            .as_ref()
            .is_some_and(|report| !self.error || report.status == self.status);
        let progress = self.export_progress_message();
        let busy_message = progress.unwrap_or_else(|| {
            if self.project_io_pending() {
                "Saving or opening project…".into()
            } else if self.state_sync_pending() {
                "Finishing instrument changes…".into()
            } else if let Some(message) = self.audio_import_message() {
                message
            } else if let Some(message) = self.freeze_progress_message() {
                message
            } else if self.loading.is_some() {
                "Preparing audio…".into()
            } else {
                self.status.clone()
            }
        });
        let output_dropouts = self
            .engine
            .as_ref()
            .map(|engine| engine.snapshot())
            .filter(|snapshot| {
                self.output_dropout_notice == Some(snapshot.engine_id) && !snapshot.output_failed
            })
            .map(|snapshot| snapshot.output_dropouts);
        let mut showing_output_dropout = false;
        let midi_advisory = self
            .engine
            .as_ref()
            .map(|engine| {
                let midi = engine.midi_snapshot();
                (
                    engine.snapshot().engine_id,
                    midi.unsupported_events,
                    midi.unsupported_groups,
                )
            })
            .filter(|notice| {
                (notice.1 > 0 || notice.2 > 0) && self.midi_advisory_ack != Some(*notice)
            });
        let mut showing_midi_advisory = false;
        let clipped_samples = self
            .exported_file
            .as_deref()
            .map_or(0, |path| self.completed_export_clipped_samples(path));
        let completion_summary = self
            .exported_file
            .as_deref()
            .and_then(|path| self.completed_export_summary(path));
        let export_midi_advisories = self
            .exported_file
            .as_deref()
            .map_or((0, 0), |path| self.completed_export_midi_advisories(path));
        let showing_completion = self.exported_file.is_some() && !self.error && !busy;
        let (kind, message) = if show_export_failure {
            (
                NoticeKind::Error,
                export_report.as_ref().unwrap().summary.clone(),
            )
        } else if self.error {
            (
                NoticeKind::Error,
                self.last_error_report
                    .as_ref()
                    .filter(|report| report.status == self.status)
                    .map(|report| report.summary.clone())
                    .unwrap_or_else(|| self.status.clone()),
            )
        } else if busy {
            (NoticeKind::Busy, busy_message.clone())
        } else if let Some(path) = &self.exported_file {
            let name = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy();
            if clipped_samples > 0 {
                (
                    NoticeKind::Warning,
                    format!("Exported · {name} · {clipped_samples} clipped samples"),
                )
            } else if export_midi_advisories != (0, 0) {
                (
                    NoticeKind::Warning,
                    format!("Exported · {name} · Some MIDI controls are unsupported"),
                )
            } else {
                (NoticeKind::Success, format!("Exported · {name}"))
            }
        } else if let Some(count) = output_dropouts {
            showing_output_dropout = true;
            (
                NoticeKind::Warning,
                format!("Audio output dropout · {count} · Try a larger buffer in Audio setup"),
            )
        } else if let Some(warning) = &self.waveform_warning {
            (NoticeKind::Warning, warning.clone())
        } else if let Some((_, events, groups)) = midi_advisory {
            showing_midi_advisory = true;
            (
                NoticeKind::Warning,
                if groups > 0 {
                    "Extra MIDI groups require separate tracks · Performance data retained"
                        .to_owned()
                } else {
                    format!(
                        "Instrument does not support {events} MIDI control type(s) · Data retained"
                    )
                },
            )
        } else {
            return;
        };
        let secondary_completion = (kind == NoticeKind::Error)
            .then(|| self.exported_file.clone())
            .flatten();
        let mut dismiss = false;
        let mut dismiss_result = false;
        let mut retry_export = false;
        let mut retry_playback = false;
        let show_playback_retry = !show_export_failure && self.has_failed_output_notice();
        let playback_retry_enabled = self.can_retry_playback();
        let mut audio_setup = false;
        let mut edit_export = false;
        let mut cancel_import = false;
        let mut cancel_freeze = false;
        let has_audio_recovery = showing_output_dropout
            || kind == NoticeKind::Error
                && self
                    .last_error_report
                    .as_ref()
                    .is_some_and(|report| report.status == self.status && report.audio_setup);
        egui::TopBottomPanel::bottom("status")
            .frame(daw_ui::panel(ctx).inner_margin(egui::Margin::symmetric(
                daw_ui::tokens::SPACE_MD as i8,
                daw_ui::tokens::SPACE_SM as i8,
            )).corner_radius(0))
            .show(ctx, |ui| {
                if kind == NoticeKind::Error && busy {
                    daw_ui::notice(ui, NoticeKind::Busy, &busy_message, |_| {});
                }
                let response = daw_ui::notice(ui, kind, &message, |ui| {
                    if self.audio_import_busy() {
                        cancel_import = daw_ui::button(ui, "Cancel import").clicked();
                    }
                    if self.freeze_busy() {
                        cancel_freeze = daw_ui::button(ui, "Cancel Freeze").clicked();
                    }
                    if show_export_failure {
                        retry_export = ui.add_enabled(
                            !busy,
                            daw_ui::button_widget("Retry export"),
                        )
                        .on_hover_text("Retry the same mix, filename and export options, including the original notes and instruments.")
                        .clicked();
                    }
                    if has_audio_recovery && !show_export_failure {
                        audio_setup = daw_ui::button(ui, "Audio setup…")
                            .on_hover_text(
                                "Open output device and buffer size settings.",
                            )
                            .clicked();
                    }
                    if show_playback_retry {
                        retry_playback = ui
                            .add_enabled(playback_retry_enabled, daw_ui::button_widget("Retry playback"))
                            .on_hover_text("Reconnect the failed output and resume from the current position.")
                            .clicked();
                    }
                    if kind == NoticeKind::Error && daw_ui::button(ui, "Details").clicked() {
                        if show_export_failure {
                            self.show_error_report_details(export_report.as_ref().unwrap().clone());
                        } else {
                            self.show_status_error_details();
                        }
                    }
                    if showing_output_dropout || showing_midi_advisory || showing_completion || matches!(kind, NoticeKind::Error | NoticeKind::Success) {
                        dismiss = daw_ui::small_button(ui, "Dismiss").clicked();
                    }
                    if showing_completion
                        && daw_ui::button(ui, "Copy path")
                            .on_hover_text("Copy the full export path")
                            .clicked()
                        && let Some(path) = &self.exported_file
                    {
                        ctx.copy_text(path.to_string_lossy().into_owned());
                    }
                    if showing_completion && clipped_samples > 0 {
                        edit_export = daw_ui::button(ui, "Export options")
                            .on_hover_text("PCM has clipped. Lower the mix level before re-exporting, or choose 32-bit float to preserve headroom.")
                            .clicked();
                    }
                });
                if showing_completion
                    && let Some(path) = &self.exported_file
                {
                    response.response.on_hover_text(completion_summary.as_ref().map_or_else(
                        || path.display().to_string(),
                        |summary| format!("{summary}\n{}", path.display())));
                } else if showing_midi_advisory {
                    response.response.on_hover_text("The original MIDI data remains in the project. Use separate tracks for different UMP groups, or choose an instrument that advertises support for these controls.");
                }
                if let Some(path) = &secondary_completion {
                    let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
                    let result_kind = if clipped_samples > 0 || export_midi_advisories != (0, 0) { NoticeKind::Warning } else { NoticeKind::Success };
                    let result_message = if clipped_samples > 0 {
                        format!("Exported · {name} · {clipped_samples} clipped samples")
                    } else { format!("Exported · {name}") };
                    daw_ui::notice(ui, result_kind, &result_message, |ui| {
                        dismiss_result = daw_ui::small_button(ui, "Dismiss result").clicked();
                        if daw_ui::button(ui, "Copy path").clicked() {
                            ctx.copy_text(path.to_string_lossy().into_owned());
                        }
                    });
                }
            });
        if dismiss_result {
            self.exported_file = None;
        }
        if retry_export {
            self.retry_export();
        }
        if cancel_import {
            self.cancel_audio_import();
        }
        if cancel_freeze {
            self.cancel_freeze();
        }
        if edit_export {
            self.export();
        }
        if retry_playback {
            self.retry_failed_playback();
        }
        if audio_setup {
            self.setup_page = SetupPage::Audio;
            self.open_popup(PopupKind::Setup);
        }
        if dismiss {
            if showing_output_dropout {
                self.output_dropout_notice = None;
            } else if showing_midi_advisory {
                self.midi_advisory_ack = midi_advisory;
            } else if kind == NoticeKind::Error {
                self.error = false;
                if show_export_failure {
                    self.export_run = None;
                }
            } else {
                self.exported_file = None;
            }
        }
    }
    pub fn delete_selection(&mut self) {
        if self.arranger_delete_selection() {
            return;
        }
        if self.view == WorkspaceView::Clip
            && self.selected_notes.is_empty()
            && matches!(
                self.selected().map(|clip| &clip.content),
                Some(ClipContent::Midi { .. })
            )
        {
            // An empty piano selection never means the containing clip,
            // including when every note is hidden by the scale view.
            return;
        }
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
            self.midi_editor.clear_hidden_selection();
        } else if let (Some(t), Some(c)) = (self.selected_track, self.selected_clip) {
            self.apply_clip_command(t, c, ClipCommand::Delete);
        }
    }
    pub fn duplicate_selection(&mut self) {
        if self.arranger_duplicate_selection() {
            return;
        }
        if self.view == WorkspaceView::Clip
            && self.selected_notes.is_empty()
            && matches!(
                self.selected().map(|clip| &clip.content),
                Some(ClipContent::Midi { .. })
            )
        {
            // An empty piano selection never means the containing clip,
            // including when every note is hidden by the scale view.
            return;
        }
        self.midi_editor.clear_hidden_selection();
        self.begin_edit();
        if !self.selected_notes.is_empty() {
            let selected = self.selected_notes.clone();
            let original = self.selected().cloned();
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
                    n.refresh_identity();
                    n.start += end - start;
                    ids.insert(n.id);
                    notes.push(n);
                }
                self.selected_notes = ids;
                if let Some((track, original)) = self.selected_track.zip(original) {
                    self.finish_midi_edit(track, original, selected, Ok(()), true, None);
                }
            }
        } else if let (Some(t), Some(c)) = (self.selected_track, self.selected_clip) {
            self.apply_clip_command(t, c, ClipCommand::Duplicate);
        }
    }
    pub(crate) fn create_linked_clone(&mut self) {
        let Some((track, clip)) = self.selected_track.zip(self.selected_clip) else {
            return;
        };
        self.apply_clip_command(track, clip, ClipCommand::Ghost);
    }

    pub(crate) fn make_clip_independent(&mut self) {
        let Some((track, clip)) = self.selected_track.zip(self.selected_clip) else {
            return;
        };
        self.apply_clip_command(track, clip, ClipCommand::MakeIndependent);
    }

    /// Every clip entry point shares result selection and one history label.
    pub(crate) fn apply_clip_command(&mut self, track: u64, clip: u64, command: ClipCommand) {
        self.begin_edit();
        let (label, result) =
            if let Some(automation_clip) = self.project.automation_clip(track, clip).cloned() {
                match command {
                    ClipCommand::Split(_) => (
                        "Split clip",
                        Err(anyhow::anyhow!("Automation clips cannot be split")),
                    ),
                    ClipCommand::Duplicate => (
                        "Duplicate automation clip",
                        self.project
                            .copy_automation_clip(track, clip, automation_clip.end(), false)
                            .map(Some),
                    ),
                    ClipCommand::Ghost => (
                        "Create ghost automation clip",
                        self.project
                            .copy_automation_clip(track, clip, automation_clip.end(), true)
                            .map(Some),
                    ),
                    ClipCommand::MakeIndependent => (
                        "Make automation clip independent",
                        self.project
                            .make_automation_clip_independent(track, clip)
                            .map(|()| Some(clip)),
                    ),
                    ClipCommand::Delete => (
                        "Delete automation clip",
                        self.project
                            .remove_automation_clip(track, clip)
                            .map(|()| None),
                    ),
                }
            } else {
                match command {
                    ClipCommand::Split(beat) => (
                        "Split clip",
                        self.project.split_clip(track, clip, beat).map(Some),
                    ),
                    ClipCommand::Duplicate => (
                        "Duplicate clip",
                        self.project.duplicate_clip(track, clip).map(Some),
                    ),
                    ClipCommand::Ghost => (
                        "Create ghost copy",
                        self.project.clone_clip(track, clip).map(Some),
                    ),
                    ClipCommand::MakeIndependent => (
                        "Make clip independent",
                        self.project
                            .make_clip_independent(track, clip)
                            .map(|()| Some(clip)),
                    ),
                    ClipCommand::Delete => (
                        "Delete clip",
                        self.project.delete_clip(track, clip).map(|_| None),
                    ),
                }
            };
        match result {
            Ok(selected) => {
                self.selected_track = Some(track);
                self.selected_clip = selected;
                if matches!(command, ClipCommand::Split(_))
                    && let Some(right) = selected
                {
                    self.arranger_select_clips([(track, clip), (track, right)], None);
                }
                self.selected_notes.clear();
                self.midi_editor.clear_hidden_selection();
                self.edit_label = Some(label.into());
                self.changed();
            }
            Err(error) => self.message(Err(error), ""),
        }
    }
    fn keyboard(&mut self, ctx: &egui::Context) {
        self.sync_workspace_tool();
        if self.midi_expression_open {
            if !egui::Popup::is_any_open(ctx)
                && ctx
                    .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                self.close_popup();
            }
            return;
        }
        if self.document_guard.is_some()
            || self.error_details.is_some()
            || crate::plugin_details::is_open(ctx)
            || crate::audio_silence::is_open(ctx)
            || crate::audio_batch::is_open(ctx)
        {
            return;
        }
        if self.rename_active(ctx) {
            let document_shortcut = ctx.input(|input| input.events.iter().any(|event| {
                matches!(event, egui::Event::Key { key: egui::Key::S | egui::Key::O, pressed: true, modifiers, .. } if modifiers.command)
            }));
            if document_shortcut {
                self.finish_inline_rename(ctx, true);
            } else {
                return;
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if daw_ui::channel_gain_editing(ctx) {
                // The precise gain field cancels its draft before Escape is
                // allowed to navigate away from the containing workspace.
                return;
            }
            // Native menus and dropdowns process Escape during their own draw.
            // Leave the containing workspace intact until that inner popup closes.
            if !egui::Popup::is_any_open(ctx) {
                if self.midi_editor.cancel_generate() || self.midi_editor.cancel_note_transform() {
                    ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                    return;
                }
                if self.automation_open && crate::automation_editor::cancel_automation_creation(ctx)
                {
                    ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                    return;
                }
                if self.instrument_open && self.close_instrument_parameters() {
                    ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                    return;
                }
                self.close_popup();
            }
            return;
        }
        // Document shortcuts remain available from every focused workspace.
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::S,
            )
        }) {
            self.save(true);
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.save(false);
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.open();
            return;
        }
        let curve_focused = (self.crossfade_open
            && ctx
                .data(|data| data.get_temp::<[egui::Id; 3]>(egui::Id::new("crossfade_curve_focus")))
                .is_some_and(|ids| {
                    ctx.memory(|memory| ids.iter().any(|id| memory.has_focus(*id)))
                }))
            || self.automation_open
                && ctx
                    .data(|data| data.get_temp::<egui::Id>(egui::Id::new("automation_curve_focus")))
                    .is_some_and(|id| ctx.memory(|memory| memory.has_focus(id)));
        if !ctx.wants_keyboard_input()
            || curve_focused
            || daw_ui::parameter_owns_document_history(ctx)
        {
            // A chord may be pressed and released between repaints. Match the
            // modifiers captured by each key event, not the final held state.
            // Preserve arrival order when several undo/redo keys share a frame.
            let actions = ctx.input_mut(|input| {
                let mut actions = Vec::new();
                input.events.retain(|event| {
                    if let egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } = event
                        && modifiers.command
                        && matches!(key, egui::Key::Z | egui::Key::Y)
                    {
                        actions.push(modifiers.shift || *key == egui::Key::Y);
                        false
                    } else {
                        true
                    }
                });
                actions
            });
            if !actions.is_empty() {
                for redo in actions {
                    self.undo_redo(redo);
                }
                return;
            }
        }
        if self.library_open
            && !egui::Popup::is_any_open(ctx)
            && crate::library::library_owns_keys(ctx)
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
        {
            ctx.data_mut(|data| data.insert_temp(egui::Id::new("instrument_chooser_enter"), true));
        }
        if self.assistant.open
            || self.library_open
            || self.setup_open
            || self.inspector_open
            || self.instrument_open
            || self.effects_open
            || self.routing_open
            || self.automation_open
            || self.history_open
            || self.crossfade_open
            || self.join_open
            || self.audio_takes_open
            || self.recording_open
            || self.export_open
            || self.midi_export_open
            || self.analysis.open
            || self.midi_expression_open
            || self.about_open
            || self.midi_editor.generate_active()
        {
            return;
        }
        if ctx.wants_keyboard_input() {
            return;
        }
        if self.rename_shortcuts(ctx)
            || self.arranger_edit_shortcuts(ctx)
            || self.clip_navigation_shortcuts(ctx)
        {
            return;
        }
        if self.audio_edit_shortcuts(ctx) {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.play();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Home)) {
            self.seek(0.0);
        }
        if self.view == WorkspaceView::Clip
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A))
            && let Some(Clip {
                content: ClipContent::Midi { notes },
                ..
            }) = self.selected()
        {
            self.selected_notes = self
                .midi_editor
                .visible_note_ids(notes, self.root, self.scale);
            self.midi_editor.clear_hidden_selection();
        }
        if !self.midi_editor.note_transform_active()
            && ctx
                .input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            self.delete_selection();
        }
        if !self.midi_editor.note_transform_active() {
            let presses =
                ctx.input_mut(|i| i.count_and_consume_key(egui::Modifiers::COMMAND, egui::Key::D));
            for _ in 0..presses {
                self.duplicate_selection();
            }
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
                ctx.set_cursor_icon(self.tool.cursor_icon());
            }
        }
    }
    fn transport(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("transport")
            .frame(daw_ui::panel(ctx).inner_margin(egui::Margin::symmetric(
                daw_ui::tokens::SPACE_MD as i8,
                daw_ui::tokens::SPACE_SM as i8,
            )).corner_radius(0))
            .show(ctx, |ui| {
                // Groups by signal flow: view, playback, position, tempo, loop.
                // Each group reserves the width it measured last frame and wraps intact.
                let compact = ui.available_width() < 1400.0;
                let group_spacing = if compact {
                    daw_ui::tokens::SPACE_XS
                } else {
                    daw_ui::tokens::SPACE_SM
                };
                ui.spacing_mut().item_spacing = egui::vec2(
                    if compact { daw_ui::tokens::SPACE_SM } else { daw_ui::tokens::SPACE_MD },
                    daw_ui::tokens::SPACE_SM,
                );
                if compact {
                    ui.spacing_mut().button_padding.x = daw_ui::tokens::SPACE_SM;
                }
                let recording_busy = self.recording_busy();
                let count_in = self.engine.as_ref().and_then(|engine| {
                    let timing = engine.timing_snapshot();
                    timing.counting.then(|| format!("Count-in · {}", timing.remaining_beats.ceil().max(1.0) as u32))
                });
                let refining = self.engine.as_ref().is_some_and(|engine| engine.snapshot().refining_audio);
                ui.horizontal_wrapped(|ui| {
                    let row_height = daw_ui::tokens::CONTROL_HEIGHT + daw_ui::tokens::SPACE_XS * 2.0;
                    ui.set_min_height(row_height);
                    ui.set_row_height(row_height);
                    daw_ui::measured_toolbar_group(ui, "transport_views", group_spacing, |ui| {
                        let active = self.navigation.iter().rev().find_map(|snapshot| match snapshot.view {
                            FocusedView::Workspace(view) => Some(view),
                            _ => None,
                        }).filter(|_| matches!(self.focused_view(), FocusedView::Popup(_))).unwrap_or(self.view);
                        for (view, label) in [
                            (WorkspaceView::Arrangement, "Arranger"),
                            (WorkspaceView::Mixer, "Mixer"),
                        ] {
                            if daw_ui::tab_choice(ui, active == view, label).clicked() {
                                self.open_workspace(view);
                            }
                        }
                    });
                    daw_ui::measured_command_group(ui, "transport_playback", |ui| {
                        if refining {
                            daw_ui::text::note(ui, "Refining audio…");
                        }
                        if ui.add_enabled(!self.recording_busy(), daw_ui::quiet_button_widget("Start"))
                            .on_hover_text("Return to start · Home")
                            .clicked()
                        {
                            self.seek(0.0);
                        }
                        let playing = self.recording_busy() || self.playing() || (self.loading.is_some() && self.desired_playing);
                        if ui.add_sized(
                            egui::vec2(100.0, daw_ui::tokens::CONTROL_HEIGHT),
                            daw_ui::primary_button_widget(ui, if playing { "Pause" } else { "Play" }),
                        )
                            .on_hover_text("Space")
                            .clicked()
                        {
                            self.play();
                        }
                        if daw_ui::quiet_button(ui, "Stop")
                            .on_hover_text("Stop and reset instruments")
                            .clicked()
                        {
                            self.stop();
                        }
                        if daw_ui::record_button(ui, self.recording_busy(), !self.recording_finalizing()).clicked() {
                            if self.recording_busy() { self.finish_recording(crate::recording_ui::AfterRecording::Continue); }
                            else { self.begin_recording(); }
                        }
                        if self.recording_finalizing() { daw_ui::text::note(ui, "Finishing recording…"); }
                        if let Some(count_in) = &count_in { daw_ui::text::note(ui, count_in); }
                    });
                    daw_ui::measured_command_group(ui, "transport_position", |ui| {
                        daw_ui::transport_position(
                            ui,
                            self.cursor,
                            self.project.beats_per_bar,
                            self.cursor * 60.0 / self.project.tempo,
                        );
                    });
                    daw_ui::measured_command_group(ui, "transport_tempo", |ui| {
                        let mut tempo = self.project.tempo;
                        let tempo_edit = ui.add_enabled_ui(!recording_busy, |ui| {
                            daw_ui::number_stepper(ui, &mut tempo, 1.0, 20.0..=400.0, "tempo", |ui, value| {
                                ui.add(daw_ui::number(value)
                                    .speed(0.1)
                                    .range(20.0..=400.0)
                                    .suffix(" BPM"))
                            })
                        }).inner;
                        if tempo_edit.response.changed() {
                            if tempo_edit.stepped {
                                self.history.seal_group();
                            }
                            self.begin_edit();
                            self.project.tempo = tempo;
                            self.edit_label = Some("Tempo".into());
                            self.changed();
                        }
                        let mut beats_per_bar = self.project.beats_per_bar;
                        if ui.add_enabled_ui(!recording_busy, |ui| {
                            daw_ui::time_signature_choice(ui, &mut beats_per_bar)
                        }).inner.changed() {
                            self.begin_edit();
                            self.project.beats_per_bar = beats_per_bar;
                            self.edit_label = Some("Time signature".into());
                            self.changed();
                        }
                        let mut click = self.project.metronome_enabled;
                        if daw_ui::quiet_toggle(ui, &mut click, "Click").on_hover_text("Metronome · Pre-count and level are in Window → Recording").changed() {
                            self.begin_edit(); self.project.metronome_enabled = click; self.edit_label = Some("Metronome".into()); self.changed();
                        }
                    });
                    daw_ui::measured_toolbar_group(ui, "transport_loop", group_spacing, |ui| {
                        let mut looping = self.project.loop_enabled;
                        if ui.add_enabled_ui(!recording_busy, |ui| daw_ui::quiet_toggle(ui, &mut looping, "Loop")).inner
                            .on_hover_text("Repeat the range between Start beat and End beat")
                            .changed()
                        {
                            self.begin_edit();
                            self.project.loop_enabled = looping;
                            self.changed();
                        }
                        let range = daw_ui::quiet_picker_button(ui, "Range", |ui| {
                            if recording_busy { ui.disable(); }
                            let selection = self.loop_selection_range();
                            let clip_editor = self.view == WorkspaceView::Clip;
                            if ui.add_enabled(selection.is_some(), daw_ui::button_widget(
                                if clip_editor { "Loop playback over clip" } else { "Loop playback over selection" },
                            ))
                                .on_hover_text(if clip_editor {
                                    "Enable playback looping over the entire open clip. To create copies, select the clip in Arranger, set a later loop end, then choose Edit → Repeat to loop end."
                                } else {
                                    "Enable playback looping over the selected clips or time range, including rests. To create copies, set a later loop end, then choose Edit → Repeat to loop end."
                                })
                                .on_disabled_hover_text(if recording_busy {
                                    "Finish recording before changing the loop range"
                                } else {
                                    "Select clips or drag a time range with the Cursor tool"
                                })
                                .clicked()
                            {
                                if let Some((start, end)) = selection {
                                    self.loop_range(start, end);
                                }
                                ui.close();
                            }
                            let arrangement_end = self.project.end_beat();
                            if ui.add_enabled(arrangement_end > 0.0, daw_ui::button_widget("Loop playback over arrangement"))
                                .on_hover_text("Enable playback looping from the beginning through the last clip. To create copies, select a phrase, set a later loop end, then choose Edit → Repeat to loop end.")
                                .clicked()
                            {
                                let changed = !self.project.loop_enabled || self.project.loop_start != 0.0 || self.project.loop_end != arrangement_end;
                                self.loop_range(0.0, arrangement_end);
                                if changed { self.edit_label = Some("Loop arrangement".into()); }
                                ui.close();
                            }
                            ui.separator();
                            // Display musical beat numbers from one; the project and
                            // renderer retain their existing zero-based timeline.
                            let seconds_per_beat = 60.0 / self.project.tempo;
                            let duration = (self.project.loop_end - self.project.loop_start) * seconds_per_beat;
                            let maximum_duration = (daw_core::MAX_BEAT - self.project.loop_start) * seconds_per_beat;
                            if let Some(seconds) = daw_ui::duration::duration_presets(ui, duration, maximum_duration) {
                                let end = (self.project.loop_start + seconds / seconds_per_beat).min(daw_core::MAX_BEAT);
                                if end != self.project.loop_end {
                                    self.begin_edit();
                                    self.project.loop_end = end;
                                    self.edit_label = Some("Loop duration".into());
                                    self.changed();
                                }
                                ui.close();
                            }
                            let mut start = self.project.loop_start + 1.0;
                            let mut end = self.project.loop_end + 1.0;
                            let minimum_length = 0.001_f64.min(end - start);
                            let start_edit = daw_ui::number_stepper(
                                ui, &mut start, 0.25, 1.0..=(end - minimum_length), "loop start", |ui, value| {
                                    ui.add(daw_ui::number(value)
                                        .prefix("Start beat ")
                                        .speed(0.25)
                                        .range(1.0..=(end - minimum_length))
                                        .min_decimals(2)
                                        .max_decimals(3))
                                },
                            );
                            let start_changed = start_edit.response
                                .on_hover_text("First beat to repeat. Use −/+ for quarter-beat steps, drag for adjustment, or click to type.")
                                .changed();
                            let end_edit = daw_ui::number_stepper(
                                ui, &mut end, 0.25, (start + minimum_length)..=(daw_core::MAX_BEAT + 1.0), "loop end", |ui, value| {
                                    ui.add(daw_ui::number(value)
                                        .prefix("End beat ")
                                        .speed(0.25)
                                        .range((start + minimum_length)..=(daw_core::MAX_BEAT + 1.0))
                                        .min_decimals(2)
                                        .max_decimals(3))
                                },
                            );
                            let end_changed = end_edit.response
                                .on_hover_text("Exclusive end: playback jumps to Start before this beat. Use −/+ for quarter-beat steps, or drag for adjustment.")
                                .changed();
                            if (start_changed || end_changed)
                                && start.is_finite()
                                && end.is_finite()
                                && start >= 1.0
                                && end > start
                                && end <= daw_core::MAX_BEAT + 1.0
                            {
                                if start_edit.stepped || end_edit.stepped { self.history.seal_group(); }
                                self.begin_edit();
                                self.project.loop_start = start - 1.0;
                                self.project.loop_end = end - 1.0;
                                self.edit_label = Some("Loop range".into());
                                self.changed();
                            }
                            ui.separator();
                            daw_ui::form_grid(ui, "loop_time_summary", |ui| {
                                daw_ui::duration::timing_summary(
                                    ui, self.project.loop_start,
                                    self.project.loop_end - self.project.loop_start,
                                    self.project.tempo, self.project.beats_per_bar,
                                );
                            });
                        });
                        range.0.on_hover_text(format!("Loop beats {:.2}–{:.2} · Set start and end", self.project.loop_start + 1.0, self.project.loop_end + 1.0));
                    });
                });
            });
    }

    fn menus(&mut self, ctx: &egui::Context) {
        // The menu bar sits on the darker background fill, so no line is
        // needed between it and the transport beneath.
        egui::TopBottomPanel::top("application-menu")
            .show_separator_line(false)
            .frame(daw_ui::panel(ctx).fill(palette(ctx).bg).inner_margin(egui::Margin::symmetric(
                daw_ui::tokens::SPACE_MD as i8,
                daw_ui::tokens::SPACE_XS as i8,
            )).corner_radius(0))
            .show(ctx, |ui| {
                if self.document_guard.is_some() || self.midi_expression_open { ui.disable(); }
                daw_ui::menu_bar(ui, |ui| {
                    daw_ui::brand_lockup(ui, daw_ui::BrandSize::Compact);
                    // A gap, not a rule: the mark and the menus already differ.
                    ui.add_space(daw_ui::tokens::SPACE_MD);
                    daw_ui::command_menu_button(ui, "File", |ui| {
                        if daw_ui::button(ui, "New").clicked() {
                            self.request_document_transition(crate::document_ui::DocumentTransition::New);
                            ui.close();
                        }
                        if daw_ui::menu_item(ui, "Open…", Some("Mod+O")).clicked() {
                            self.open();
                            ui.close();
                        }
                        if daw_ui::button(ui, "Recover last session…")
                            .on_hover_text("Open the latest recovery snapshot from the recovery folder in Setup → Storage.")
                            .clicked() {
                            self.request_document_transition(crate::document_ui::DocumentTransition::Open { path: self.settings.scratch_dir.join("recovery.daw.json"), recovery: true });
                            ui.close();
                        }
                        ui.separator();
                        if daw_ui::menu_item(ui, "Save", Some("Mod+S")).clicked() {
                            self.save(false);
                            ui.close();
                        }
                        if daw_ui::menu_item(ui, "Save as…", Some("Mod+Shift+S")).clicked() {
                            self.save(true);
                            ui.close();
                        }
                        if daw_ui::button(ui, "Rename project…")
                            .on_hover_text("Change the project name. New projects use it for Save and export filenames; existing files keep their names.")
                            .clicked() {
                            self.start_project_rename(ui.ctx());
                            ui.close();
                        }
                        ui.separator();
                        if ui.add_enabled(!self.recording_busy(), daw_ui::button_widget("Import audio…")).clicked() {
                            self.import_audio_dialog();
                            ui.close();
                        }
                        if ui.add_enabled(!self.recording_busy(), daw_ui::button_widget("Import…")).clicked() {
                            self.import_dialog();
                            ui.close();
                        }
                        if daw_ui::button(ui, "Export WAV…").clicked() {
                            self.export();
                            ui.close();
                        }
                        self.reexport_menu(ui);
                        if ui.add_enabled(self.export_midi_ready(), daw_ui::button_widget("Export MIDI…"))
                            .on_disabled_hover_text("Wait for the current export or session update to finish.")
                            .clicked() {
                            self.export_midi_dialog();
                            ui.close();
                        }
                        self.recent_exports_menu(ui);
                        ui.separator();
                        if daw_ui::button(ui, "Quit").clicked() {
                            self.request_document_transition(crate::document_ui::DocumentTransition::Quit);
                            ui.close();
                        }
                    });
                    daw_ui::command_menu_button(ui, "Edit", |ui| {
                        let undo_ready = !self.state_sync_pending() && !self.recording_busy();
                        if daw_ui::menu_item_enabled(ui, self.history.can_undo() && undo_ready, "Undo", Some("Mod+Z"))
                            .on_hover_text(self.history.undo_labels().next().unwrap_or("").to_owned())
                            .clicked()
                        {
                            self.undo_redo(false);
                            ui.close();
                        }
                        if daw_ui::menu_item_enabled(ui, self.history.can_redo() && undo_ready, "Redo", Some("Mod+Shift+Z"))
                            .on_hover_text(self.history.redo_labels().next().unwrap_or("").to_owned())
                            .clicked()
                        {
                            self.undo_redo(true);
                            ui.close();
                        }
                        if daw_ui::button(ui, "History…").clicked() {
                            self.open_popup(PopupKind::History);
                            ui.close();
                        }
                        if daw_ui::menu_item_enabled(ui, self.automation_override_count() > 0,
                            "Cancel all automation overrides", None)
                            .on_hover_text("Clear all manual automation overrides. Enabled curves resume at the current playhead; bypass stays unchanged. Undo restores the overrides.")
                            .clicked() {
                            self.cancel_automation_overrides();
                            ui.close();
                        }
                        ui.separator();
                        let selected_automation_clip_linked = self
                            .selected_track
                            .zip(self.selected_clip)
                            .and_then(|(t, c)| self.project.automation_clip(t, c))
                            .map(|clip| clip.link_group.is_some());
                        let has_clip = (self.selected().is_some() || selected_automation_clip_linked.is_some())
                            && !self.midi_editor.note_transform_active()
                            && !self.midi_editor.generate_active();
                        let has_edit_selection = has_clip && !(self.view == WorkspaceView::Clip
                            && self.selected_notes.is_empty()
                            && matches!(self.selected().map(|clip| &clip.content), Some(ClipContent::Midi { .. })));
                        let clipboard = [
                            ("Cut", "Mod+X", crate::audio_editor::AudioClipboardAction::Cut),
                            ("Copy", "Mod+C", crate::audio_editor::AudioClipboardAction::Copy),
                            ("Paste", "Mod+V", crate::audio_editor::AudioClipboardAction::Paste),
                        ];
                        if self.view == WorkspaceView::Arrangement {
                            for (label, shortcut, action) in clipboard {
                                let enabled = if matches!(action, crate::audio_editor::AudioClipboardAction::Paste) {
                                    self.has_arranger_clipboard() || crate::audio_editor::has_audio_clipboard(ui.ctx())
                                } else { has_clip };
                                if daw_ui::menu_item_enabled(ui, enabled, label, Some(shortcut)).clicked() {
                                    if !self.arranger_clipboard_action(ui.ctx(), action) {
                                        self.audio_clipboard_action(ui.ctx(), action);
                                    }
                                    ui.close();
                                }
                            }
                        } else if self.view == WorkspaceView::Clip && self.selected_track.and_then(|id| self.project.track(id)).is_some_and(|track| track.kind == TrackKind::Audio) {
                            for (label, shortcut, action) in clipboard {
                                let enabled = if matches!(action, crate::audio_editor::AudioClipboardAction::Paste) {
                                    crate::audio_editor::has_audio_clipboard(ui.ctx())
                                } else { has_clip };
                                if daw_ui::menu_item_enabled(ui, enabled, label, Some(shortcut)).clicked() {
                                    self.audio_clipboard_action(ui.ctx(), action);
                                    ui.close();
                                }
                            }
                        }
                        if daw_ui::menu_item_enabled(ui, has_edit_selection, "Duplicate", Some("Mod+D"))
                            .on_hover_text("Duplicate the selection as an independent copy")
                            .clicked()
                        {
                            if self.view == WorkspaceView::Arrangement || !self.audio_clipboard_action(ui.ctx(), crate::audio_editor::AudioClipboardAction::Duplicate) {
                                self.duplicate_selection();
                            }
                            ui.close();
                        }
                        if daw_ui::menu_item_enabled(ui, has_edit_selection, "Delete", Some("Delete")).clicked() {
                            if self.view == WorkspaceView::Arrangement || !self.audio_clipboard_action(ui.ctx(), crate::audio_editor::AudioClipboardAction::Delete) {
                                self.delete_selection();
                            }
                            ui.close();
                        }
                        let midi_clip = matches!(self.selected().map(|clip| &clip.content), Some(ClipContent::Midi { .. }));
                        let audio_clip_view = matches!(self.selected().map(|clip| &clip.content), Some(ClipContent::Audio { .. }));
                        if self.view == WorkspaceView::Arrangement || (self.view == WorkspaceView::Clip && (midi_clip || audio_clip_view)) {
                            ui.separator();
                        }
                        if self.view == WorkspaceView::Arrangement {
                            if daw_ui::menu_item(ui, "Select all clips", Some("Mod+A")).clicked() {
                                self.arranger_select_all();
                                ui.close();
                            }
                            if ui.add_enabled(has_clip, daw_ui::button_widget("Clear clip selection")).clicked() {
                                self.arranger_select_clips([], None);
                                self.selected_clip = None;
                                ui.close();
                            }
                        } else if self.view == WorkspaceView::Clip && midi_clip {
                            if daw_ui::menu_item(ui, "Select all notes", Some("Mod+A")).clicked() {
                                if let Some(Clip { content: ClipContent::Midi { notes }, .. }) = self.selected() {
                                    self.selected_notes = self.midi_editor.visible_note_ids(notes, self.root, self.scale);
                                }
                                ui.close();
                            }
                            if ui.add_enabled(!self.selected_notes.is_empty(), daw_ui::button_widget("Clear note selection")).clicked() {
                                self.selected_notes.clear();
                                self.midi_editor.clear_hidden_selection();
                                ui.close();
                            }
                        } else if self.view == WorkspaceView::Clip && audio_clip_view {
                            if daw_ui::button(ui, "Select clip time").clicked() {
                                self.select_audio_clip_time(ui.ctx());
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    self.has_audio_time_selection(ui.ctx()),
                                    daw_ui::button_widget("Clear time selection"),
                                )
                                .clicked()
                            {
                                self.clear_audio_time_selection(ui.ctx());
                                ui.close();
                            }
                        }
                        ui.separator();
                        let rename_available = matches!(self.focused_view(), FocusedView::Workspace(_))
                            && (self.selected_clip.is_some() || self.selected_track.is_some());
                        if daw_ui::menu_item_enabled(ui, rename_available, "Rename…", Some("F2"))
                            .on_hover_text("Rename the selected clip or track").clicked() {
                            self.start_inline_rename(ui.ctx());
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .add_enabled(has_clip, daw_ui::button_widget("Split at cursor"))
                            .clicked()
                        {
                            if let (Some(track), Some(clip)) =
                                (self.selected_track, self.selected_clip)
                            {
                                self.apply_clip_command(track, clip, ClipCommand::Split(self.cursor));
                            }
                            ui.close();
                        }
                        if ui
                            .add_enabled(has_clip, daw_ui::button_widget("Join clips…"))
                            .clicked()
                        {
                            self.open_popup(PopupKind::Join);
                            ui.close();
                        }
                        if daw_ui::menu_item_enabled(ui, has_clip, "Stretch", Some("S"))
                            .on_hover_text("Activate the arranger stretch tool")
                            .clicked() {
                            self.open_workspace(WorkspaceView::Arrangement);
                            self.tool = Tool::Stretch;
                            ui.ctx().set_cursor_icon(self.tool.cursor_icon());
                            ui.close();
                        }
                        if ui.add_enabled(has_clip, daw_ui::button_widget("Ghost copy"))
                            .on_hover_text("Create a ghost copy sharing the name, notes, audio edits, fades and clip gain. Position and trim stay independent.")
                            .clicked() {
                            self.create_linked_clone();
                            ui.close();
                        }
                        if (self.selected().is_some_and(|clip| clip.link_group.is_some())
                            || selected_automation_clip_linked == Some(true))
                            && ui.add_enabled(has_clip, daw_ui::button_widget("Make independent"))
                                .on_hover_text("Disconnect this ghost while keeping its current name and edits")
                                .clicked() {
                            self.make_clip_independent();
                            ui.close();
                        }
                        self.repeat_to_loop_end_item(ui);
                        ui.separator();
                        let audio_clip = self
                            .selected()
                            .is_some_and(|clip| matches!(clip.content, ClipContent::Audio { .. }));
                        let audio_selected = !self.selected_audio_boundaries().is_empty();
                        daw_ui::menu_button(ui, "Audio", |ui| {
                            if ui
                                .add_enabled(audio_clip, daw_ui::button_widget("Crossfade…"))
                                .clicked()
                            {
                                self.open_popup(PopupKind::Crossfade);
                                ui.close();
                            }
                            if ui.add_enabled(audio_selected, daw_ui::button_widget("Smooth edit boundaries")).clicked() {
                                self.smooth_audio_boundaries(ui.ctx());
                                ui.close();
                            }
                            if ui.add_enabled(audio_selected, daw_ui::button_widget("Boundary fades…")).clicked() {
                                self.open_audio_batch_fades(ui.ctx());
                                ui.close();
                            }
                        });
                    });
                    self.track_menu(ui);
                    daw_ui::command_menu_button(ui, "Window", |ui| {
                        if daw_ui::button(ui, "Arranger").clicked() {
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
                            ("Recording", PopupKind::Recording, true),
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
                        if daw_ui::button(ui, "Assistant").clicked() {
                            self.open_global_assistant();
                            ui.close();
                        }
                        ui.separator();
                        if daw_ui::button(ui, "Setup").clicked() {
                            self.open_popup(PopupKind::Setup);
                            ui.close();
                        }
                    });
                    daw_ui::command_menu_button(ui, "Help", |ui| {
                        if daw_ui::button(ui, "About RILL").clicked() {
                            self.open_popup(PopupKind::About);
                            ui.close();
                        }
                    });
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width().max(0.0), daw_ui::tokens::CONTROL_HEIGHT),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if daw_ui::ai_button(ui, "AI").on_hover_text("Assistant · current song").clicked() {
                                self.open_global_assistant();
                            }
                            if self.assistant.busy() && daw_ui::small_button(ui, "Stop AI").clicked() {
                                self.assistant.stop();
                            }
                            if self.automation_override_count() > 0
                                && daw_ui::small_button(ui, "Cancel automation override")
                                    .on_hover_text("Clear all manual automation overrides. Enabled curves resume at the current playhead; bypass stays unchanged. Undo restores the overrides.")
                                    .clicked() {
                                self.cancel_automation_overrides();
                            }
                            ui.add_space(daw_ui::tokens::SPACE_LG);
                            // The project identity is centered in the space the
                            // menus and AI leave; it truncates before it moves them.
                            let seconds = self.project.end_beat() * 60.0 / self.project.tempo;
                            let duration = daw_ui::format_clock_time(seconds, 1);
                            let mut job = egui::text::LayoutJob::default();
                            let name_format = egui::TextFormat {
                                font_id: daw_ui::text::TextRole::Label.font(),
                                color: palette(ui.ctx()).text,
                                ..Default::default()
                            };
                            job.append(&self.document_title(), 0.0, name_format);
                            job.append(
                                &format!(" · {duration}"),
                                0.0,
                                egui::TextFormat {
                                    font_id: daw_ui::text::TextRole::Note.font(),
                                    color: palette(ui.ctx()).muted,
                                    valign: egui::Align::Center,
                                    ..Default::default()
                                },
                            );
                            let title = ui
                                .allocate_ui_with_layout(
                                    egui::vec2(ui.available_width().max(0.0), daw_ui::tokens::CONTROL_HEIGHT),
                                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                                    |ui| ui.add(egui::Label::new(job).truncate()),
                                )
                                .inner;
                            ctx.data_mut(|data| data.insert_temp(egui::Id::new("project_title_rect"), title.rect));
                            title.on_hover_text(format!("Project: {}\nArrangement duration: {duration}\nRename from File → Rename project…{}", self.project.name,
                                self.project_path.as_ref().map(|path| format!("\n{}", path.display())).unwrap_or_default()));
                        },
                    );
                });
            });
    }

    fn about_popup(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let mut open = true;
        let mut close = false;
        daw_ui::popup("About RILL", ctx)
            .id(egui::Id::new("about_rill"))
            .default_width(daw_ui::tokens::POPUP_WIDTH_SM)
            .resizable(false)
            .vscroll(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let mut footer = daw_ui::popup_footer(ui, "about_actions");
                let window_ui = ui;
                let ui = footer.body();
                ui.set_width(daw_ui::tokens::POPUP_WIDTH_SM);
                ui.vertical_centered(|ui| {
                    daw_ui::brand_lockup(ui, daw_ui::BrandSize::Hero);
                    ui.add_space(daw_ui::tokens::SPACE_LG);
                    daw_ui::text::body(ui, "Audio workstation");
                    daw_ui::text::note(ui, format!("Version {}", env!("CARGO_PKG_VERSION")));
                });
                close = footer
                    .show(window_ui, |ui| {
                        daw_ui::dialog_actions(
                            ui,
                            |ui| daw_ui::dialog_primary(ui, "Close").clicked(),
                            |_| {},
                        )
                        .inner
                        .0
                    })
                    .inner;
            });
        self.about_open = open && !close;
    }

    pub(crate) fn undo_redo(&mut self, redo: bool) {
        self.undo_redo_steps(redo, 1);
    }

    pub(crate) fn undo_redo_steps(&mut self, redo: bool, steps: usize) {
        if self.recording_busy() {
            return;
        }
        self.midi_editor.cancel_generate();
        self.midi_editor.cancel_note_transform();
        if self.state_sync_pending() {
            return;
        }
        self.flush_pending_edit();
        let playback_before = self.project.clone();
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
            self.render_context.stop_dragging();
            self.restore_history_selection();
            self.assistant.known_parameters.clear();
            self.song_session.bump();
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
            if self.selected_clip.is_some()
                && self.selected().is_none()
                && self
                    .selected_track
                    .zip(self.selected_clip)
                    .is_none_or(|(t, c)| self.project.automation_clip(t, c).is_none())
            {
                self.selected_clip = self
                    .selected_track
                    .and_then(|id| self.project.track(id))
                    .and_then(|track| {
                        track
                            .clips
                            .iter()
                            .filter(|clip| !clip.is_take_source())
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
            if !audio_equivalent(&playback_before, &self.project) {
                self.revision = self.revision.wrapping_add(1);
                self.update_playback();
            }
            self.cache_audio();
        }
    }

    fn track_menu(&mut self, ui: &mut egui::Ui) {
        daw_ui::command_menu_button(ui, "Track", |ui| {
            if daw_ui::button(ui, "Add instrument track…").clicked() {
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
                    let (edit_kind, track_name) = match kind {
                        TrackKind::Midi => (EditableTrackKind::Midi, "New instrument".to_owned()),
                        TrackKind::Audio => (EditableTrackKind::Audio, "New audio".to_owned()),
                        TrackKind::Bus => (EditableTrackKind::Bus, self.available_bus_name()),
                        TrackKind::Folder => (EditableTrackKind::Folder, "New folder".to_owned()),
                        TrackKind::Automation => {
                            unreachable!("Automation requires a parameter target")
                        }
                    };
                    let mut edits = vec![SongEdit::AddTrack {
                        kind: edit_kind,
                        name: track_name,
                        key: Some("new_track".into()),
                    }];
                    if kind == TrackKind::Midi {
                        edits.push(SongEdit::AddMidiClip {
                            track: EntityRef::Key("new_track".into()),
                            name: "New pattern".into(),
                            start: self.snapped(self.cursor).max(0.0),
                            length: 4.0,
                            notes: Vec::new(),
                            key: Some("new_clip".into()),
                        });
                    }
                    let batch = EditBatch {
                        label: "Add track".into(),
                        edits,
                    };
                    match daw_tools::SongToolHost::apply_edits(self, batch) {
                        Ok(result) => {
                            self.selected_track = result
                                .created
                                .iter()
                                .find(|entity| entity.key.as_deref() == Some("new_track"))
                                .map(|entity| entity.id);
                            self.selected_clip = result
                                .created
                                .iter()
                                .find(|entity| entity.key.as_deref() == Some("new_clip"))
                                .map(|entity| entity.id);
                            self.selected_notes.clear();
                            self.midi_editor.clear_hidden_selection();
                            self.close_popups();
                            self.sync_arranger_selection();
                            let state = self.history.state_id();
                            let selection = self.capture_selection();
                            self.selection_history.record(
                                (state, selection.clone()),
                                (state, selection),
                                self.history.retained_state_ids(),
                            );
                            if kind == TrackKind::Midi {
                                self.open_new_instrument_chooser();
                            }
                        }
                        Err(error) => self.message(Err(anyhow::anyhow!(error)), ""),
                    }
                    ui.close();
                }
            }
            if daw_ui::button(ui, "Add automation track…").clicked() {
                self.start_automation_creation(ui.ctx());
                self.open_popup(PopupKind::Automation);
                ui.close();
            }
            ui.separator();
            let track = self.selected_track.and_then(|id| self.project.track(id));
            let track_id = track.map(|track| track.id);
            let take_source = track
                .and_then(|track| track.clips.iter().find(|clip| clip.is_take_source()))
                .cloned();
            let channel = track.is_some_and(|track| {
                matches!(
                    track.kind,
                    TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                )
            });
            if daw_ui::menu_item_enabled(ui, track_id.is_some(), "Rename…", Some("F2")).clicked()
                && let Some(id) = track_id
            {
                self.start_track_rename(ui.ctx(), id);
                ui.close();
            }
            if ui
                .add_enabled(track_id.is_some(), daw_ui::button_widget("Properties"))
                .clicked()
            {
                self.selected_clip = None;
                self.open_popup(PopupKind::Inspector);
                ui.close();
            }
            if ui
                .add_enabled(channel, daw_ui::button_widget("Channel"))
                .clicked()
                && let Some(id) = track_id
            {
                self.open_channel(id);
                ui.close();
            }
            if let Some(clip) = take_source
                && daw_ui::button(ui, "Audio takes…").clicked()
            {
                self.open_audio_takes(&clip, None);
                ui.close();
            }
            if ui
                .add_enabled(track_id.is_some(), daw_ui::button_widget("Delete track"))
                .clicked()
                && let Some(id) = track_id
            {
                let batch = EditBatch {
                    label: "Delete track".into(),
                    edits: vec![SongEdit::DeleteTrack {
                        track: EntityRef::Id(id),
                    }],
                };
                match daw_tools::SongToolHost::apply_edits(self, batch) {
                    Ok(_) => {
                        self.selected_track = None;
                        self.selected_clip = None;
                        self.selected_notes.clear();
                        self.sync_arranger_selection();
                    }
                    Err(error) => self.message(Err(anyhow::anyhow!(error)), ""),
                }
                ui.close();
            }
            if !self.finishing_selected_channels().is_empty() {
                ui.separator();
                self.route_selected_channels_menu(ui);
            }
        });
    }

    fn browser(&mut self, ctx: &egui::Context) {
        self.plugin_library(ctx);
    }
    /// Rescan plug-in folders and update the persisted index in the
    /// background, then capture missing previews when automatic capture is on.
    /// The library shows the previous index until the scan finishes.
    pub(crate) fn refresh_plugins(&mut self) {
        let mut dirs = self
            .plugin_scan_roots
            .clone()
            .unwrap_or_else(daw_plugins::default_locations);
        dirs.extend(self.settings.vst_dirs.clone());
        self.plugin_scan_reported = false;
        self.plugin_sync.sync(dirs, self.automatic_plugin_previews);
    }
    fn setup(&mut self, ctx: &egui::Context) {
        if !self.setup_open {
            return;
        }
        let mut open = true;
        daw_ui::popup("Setup", ctx).open(&mut open).default_width(daw_ui::tokens::POPUP_WIDTH_MD).vscroll(false).show(ctx, |ui| {
            daw_ui::popup_tabs(ui, &mut self.setup_page, &[
                (SetupPage::Audio, "Audio"),
                (SetupPage::Appearance, "Appearance"),
                (SetupPage::Storage, "Storage"),
                (SetupPage::Plugins, "Plug-ins"),
                (SetupPage::Ai, "AI"),
            ], |_| {});
            let mut footer = daw_ui::popup_footer(ui, "setup_actions");
            let window_ui = ui;
            let ui = footer.body();
            daw_ui::popup_scroll_body(ui, "setup_body", 0.0, |ui| { match self.setup_page {
                SetupPage::Ai => self.ai_setup(ui),
                SetupPage::Appearance => {
                    daw_ui::form_grid(ui, "appearance_settings", |ui| {
                        let theme_label = daw_ui::form_label(ui, "Theme");
                        daw_ui::select("appearance", self.setup_draft.appearance.name()).width(daw_ui::tokens::FORM_FIELD_WIDTH).show_ui(ui, |ui| {
                            for appearance in [Appearance::System, Appearance::Light, Appearance::Dark] {
                                daw_ui::selectable_value(ui, &mut self.setup_draft.appearance, appearance, appearance.name());
                            }
                        }).response.labelled_by(theme_label.id).on_hover_text("System follows the operating system appearance.");
                        ui.end_row();
                    });
                }
                SetupPage::Audio => {
                    daw_ui::form_grid(ui, "audio_settings", |ui| {
                        let previous_device = self.setup_draft.device_name.clone();
                        let output_label = daw_ui::form_label(ui, "Output");
                        ui.horizontal(|ui| {
                        daw_ui::select("output", self.setup_draft.device_name.as_deref().unwrap_or("System default")).width(daw_ui::tokens::FORM_FIELD_WIDTH).truncate().show_ui(ui, |ui| {
                            daw_ui::selectable_value(ui, &mut self.setup_draft.device_name, None, "System default");
                            for device in &self.devices { daw_ui::selectable_value(ui, &mut self.setup_draft.device_name, Some(device.name.clone()), &device.name); }
                        }).response.labelled_by(output_label.id).on_hover_text(self.setup_draft.device_name.as_deref().unwrap_or("System default"));
                        if daw_ui::quiet_button(ui, "Refresh devices").clicked() { self.refresh_devices(); }
                        });
                        ui.end_row();
                        let input_label = daw_ui::form_label(ui, "Input device");
                        let picker = daw_ui::select("input_device", self.setup_draft.input_device_name.as_deref().unwrap_or("System default input")).width(daw_ui::tokens::FORM_FIELD_WIDTH).truncate().show_ui(ui, |ui| {
                            daw_ui::selectable_value(ui, &mut self.setup_draft.input_device_name, None, "System default input");
                            for device in &self.input_devices { daw_ui::selectable_value(ui, &mut self.setup_draft.input_device_name, Some(device.name.clone()), &device.name); }
                        }).response.labelled_by(input_label.id);
                        ctx.data_mut(|data| data.insert_temp(egui::Id::new("input_device_picker_rect"), picker.rect));
                        ui.end_row();
                        let midi_label = daw_ui::form_label(ui, "MIDI input");
                        let midi_name = self.setup_draft.midi_input_device.as_ref().map(|id| self.recording.midi_ports.iter().find(|port| &port.id == id).map(|port| port.name.as_str()).unwrap_or("Unavailable MIDI input")).unwrap_or("No MIDI input");
                        daw_ui::select("midi_device", midi_name).width(daw_ui::tokens::FORM_FIELD_WIDTH).truncate().show_ui(ui, |ui| {
                            daw_ui::selectable_value(ui, &mut self.setup_draft.midi_input_device, None, "No MIDI input");
                            for port in &self.recording.midi_ports { daw_ui::selectable_value(ui, &mut self.setup_draft.midi_input_device, Some(port.id.clone()), &port.name); }
                        }).response.labelled_by(midi_label.id);
                        ui.end_row();
                        let rates = self.devices.iter().find(|d| self.setup_draft.device_name.as_ref().map_or(d.is_default, |name| name == &d.name)).map(|d| d.supported_sample_rates.clone()).unwrap_or_else(|| vec![44_100, 48_000, 88_200, 96_000]);
                        if self.setup_draft.device_name != previous_device && !rates.is_empty() && !rates.contains(&self.setup_draft.sample_rate) {
                            self.setup_draft.sample_rate = if rates.contains(&48_000) { 48_000 } else { rates[0] };
                        }
                        let sample_rate_label = daw_ui::form_label(ui, "Sample rate");
                        daw_ui::select("sample_rate", format!("{} Hz", self.setup_draft.sample_rate)).width(daw_ui::tokens::FORM_FIELD_WIDTH).show_ui(ui, |ui| {
                            for rate in rates { daw_ui::selectable_value(ui, &mut self.setup_draft.sample_rate, rate, format!("{rate} Hz")); }
                        }).response.labelled_by(sample_rate_label.id);
                        ui.end_row();
                        let buffer_label = daw_ui::form_label(ui, "Buffer");
                        ui.horizontal(|ui| {
                        daw_ui::select("buffer", format!("{} samples", self.setup_draft.buffer_frames)).width(daw_ui::tokens::FORM_FIELD_WIDTH).show_ui(ui, |ui| {
                            for size in [64, 128, 256, 512, 1024, 2048] { daw_ui::selectable_value(ui, &mut self.setup_draft.buffer_frames, size, format!("{size} samples")); }
                        }).response.labelled_by(buffer_label.id);
                        if daw_ui::quiet_button(ui, "Recommended").on_hover_text("Balanced starting point for Apple Silicon MacBooks and other modern computers: 256 samples at 44.1/48 kHz, 512 at 88.2/96 kHz. Use a larger buffer if a demanding mix drops out.").clicked() {
                            self.setup_draft.buffer_frames = daw_engine::DeviceSettings::recommended_buffer_frames(self.setup_draft.sample_rate);
                        }
                        });
                        ui.end_row();
                        let conversion_label = daw_ui::form_label(ui, "Resampling");
                        daw_ui::select("sample_conversion", match self.setup_draft.resample_quality { daw_engine::ResampleQuality::High => "High quality", daw_engine::ResampleQuality::Fast => "Fast" }).width(daw_ui::tokens::FORM_FIELD_WIDTH).show_ui(ui, |ui| {
                            daw_ui::selectable_value(ui, &mut self.setup_draft.resample_quality, daw_engine::ResampleQuality::High, "High quality");
                            daw_ui::selectable_value(ui, &mut self.setup_draft.resample_quality, daw_engine::ResampleQuality::Fast, "Fast");
                        }).response.labelled_by(conversion_label.id).on_hover_text("High quality filters sample-rate conversion; Fast prepares mixed-rate audio sooner. Audio uses 32-bit float with 64-bit summing.");
                        ui.end_row();
                    });
                    let rate = self.setup_draft.sample_rate.max(1) as f64;
                    daw_ui::text::note(ui, format!("{:.1} ms device buffer", self.setup_draft.buffer_frames as f64 * 1000.0 / rate))
                        .on_hover_text(format!("The render queue adds up to {:.1} ms, with a 512-frame minimum. Device and plug-in latency may add more. Smaller buffers reduce device latency and increase callback overhead; larger buffers give demanding mixes more scheduling headroom.", self.setup_draft.audio().render_queue_frames() as f64 * 1000.0 / rate));
                }
                SetupPage::Storage => {
                    daw_ui::text::section_heading(ui, "Undo history");
                    daw_ui::form_grid(ui, "history_settings", |ui| {
                        let label = daw_ui::form_label(ui, "Undo actions");
                        ui.add_sized(
                            [daw_ui::tokens::FORM_FIELD_WIDTH, daw_ui::tokens::CONTROL_HEIGHT],
                            daw_ui::number(&mut self.setup_draft.history_limit)
                                .range(1..=daw_core::MAX_HISTORY_STEPS)
                                .speed(1.0),
                        ).labelled_by(label.id).on_hover_text("Shared by manual edits, workflow tools and AI. Drag to change, or double-click to enter an exact count. Apply trims older history immediately; Cancel leaves it unchanged.");
                        ui.end_row();
                    });
                    daw_ui::text::note(ui, "128 MiB memory cap may retain fewer actions");
                    ui.add_space(daw_ui::tokens::SPACE_LG);
                    daw_ui::text::section_heading(ui, "Recovery folder");
                    daw_ui::text::truncated(ui, daw_ui::text::TextRole::Body, self.setup_draft.scratch_dir.display().to_string())
                        .on_hover_text("Committed edits are checkpointed here in the background. Plug-in state is refreshed every 30 seconds; the previous checkpoint is retained too.");
                    ui.add_space(daw_ui::tokens::SPACE_MD);
                    if daw_ui::button(ui, "Choose…").clicked() && let Some(path) = daw_ui::with_native_dialog_menu(|| rfd::FileDialog::new().pick_folder()) { self.setup_draft.scratch_dir = path; }
                    daw_ui::text::note(ui, "Recover from File → Recover last session…");
                }
                SetupPage::Plugins => {
                    daw_ui::text::section_heading(ui, "VST3 folders");
                    daw_ui::text::note(ui, "System folders included").on_hover_text(daw_plugins::default_locations().iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
                    let mut remove = None;
                    // Keep the wrapped folder field/actions and Apply/Cancel
                    // outside the scrolling list even in a small viewport.
                    let controls_height = 2.0 * daw_ui::tokens::CONTROL_HEIGHT
                        + 3.0 * ui.spacing().item_spacing.y;
                    let list_height = (ui.available_height() - controls_height)
                        .clamp(daw_ui::tokens::COMPACT_HEIGHT, 233.0);
                    egui::ScrollArea::vertical().max_height(list_height)
                        .min_scrolled_height(daw_ui::tokens::COMPACT_HEIGHT).show(ui, |ui| {
                        for (index, path) in self.setup_draft.vst_dirs.iter().enumerate() {
                            ui.push_id(index, |ui| { daw_ui::path_row(ui, path, |ui| {
                                if daw_ui::small_button(ui, "Remove").clicked() { remove = Some(index); }
                            }); });
                        }
                    });
                    if let Some(index) = remove { self.setup_draft.vst_dirs.remove(index); }
                    let path_key = egui::Id::new("plugin_scan_folder_input");
                    let mut folder = ctx.data_mut(|data| data.get_temp::<String>(path_key)).unwrap_or_default();
                    ui.horizontal_wrapped(|ui| {
                        daw_ui::text_field(ui, &mut folder, "Folder path", daw_ui::tokens::FORM_FIELD_WIDTH);
                        daw_ui::toolbar_group(ui, |ui| {
                            if daw_ui::button(ui, "Add folder").clicked() {
                                let path = PathBuf::from(folder.trim());
                                if path.is_dir() {
                                    if !self.setup_draft.vst_dirs.contains(&path) { self.setup_draft.vst_dirs.push(path); }
                                    folder.clear();
                                } else { self.message(Err(anyhow::anyhow!("Choose an existing plug-in folder")), ""); }
                            }
                            if daw_ui::button(ui, "Browse…").clicked() && let Some(path) = daw_ui::with_native_dialog_menu(|| rfd::FileDialog::new().pick_folder()) && !self.setup_draft.vst_dirs.contains(&path) { self.setup_draft.vst_dirs.push(path); }
                        });
                    });
                    ctx.data_mut(|data| data.insert_temp(path_key, folder));
                }
            }
            });
            footer.show(window_ui, |ui| daw_ui::dialog_actions(ui, |ui| {
                if daw_ui::dialog_primary(ui, "Apply").clicked() && self.prepare_ai_settings() {self.request_settings_apply();}
                if daw_ui::dialog_button(ui, "Cancel").clicked() { self.setup_open = false; }
            }, |_| {}));
        });
        self.setup_open &= open;
    }
}

impl DawApp {
    pub fn ui_frame(&mut self, ctx: &egui::Context) {
        crate::theme::set_appearance(ctx, self.settings.appearance);
        self.frame_count += 1;
        // Opening the view only after the first frames lets a popup's blurred
        // backdrop show the laid-out scene instead of an empty first pass.
        if self.frame_count == 3
            && let Some(view) = self.screenshot_view.take()
        {
            self.open_screenshot_view(&view);
        }
        if std::mem::take(&mut self.reset_views) {
            crate::plugin_details::close(ctx);
            self.midi_editor.clear_generation_memory();
            ctx.data_mut(|data| {
                data.remove::<daw_ui::TimelineViewport>(egui::Id::new(
                    "arrangement_timeline_viewport",
                ));
                for id in ["timeline_diagnostics", "midi_editor_diagnostics"] {
                    data.remove::<serde_json::Value>(egui::Id::new(id));
                }
            });
            crate::audio_editor::reset_audio_editor(ctx);
            crate::audio_batch::reset(ctx);
            crate::rename::reset(ctx);
            for scope in ["arrangement", "audio", "midi", "automation"] {
                daw_ui::reset_timeline_view_history(ctx, scope);
            }
            crate::spectrogram::reset(ctx);
            crate::automation_editor::reset_automation_editor(ctx);
        }
        crate::spectrogram::begin_frame(ctx);
        if !self.close_ready && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.error_details = None;
            crate::plugin_details::close(ctx);
            self.finish_inline_rename(ctx, true);
            self.request_document_transition(crate::document_ui::DocumentTransition::Quit);
        }
        if self.document_guard.is_none()
            && self.error_details.is_none()
            && !crate::plugin_details::is_open(ctx)
            && ctx.input(|i| {
                i.pointer.any_pressed()
                    || i.events.iter().any(|e| {
                        matches!(
                            e,
                            egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Key { .. }
                        )
                    })
            })
        {
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
        self.sync_signal_analysis();
        self.poll_live_edits();
        self.poll_instrument_editor(ctx);
        self.poll_recording(ctx);
        self.poll_project_io(ctx);
        self.poll_audio_import(ctx);
        self.poll_freeze(ctx);
        self.poll_export();
        self.poll_join(ctx);
        self.update_note_audition(ctx);
        let mut render_error = None;
        if let Some(e) = &self.engine {
            self.cursor = e.position_beats();
            let s = e.snapshot();
            if s.output_failed || self.output_dropout_notice != Some(s.engine_id) {
                self.output_dropout_notice = None;
            }
            if e.take_output_dropout_notice().is_some() {
                self.output_dropout_notice = Some(s.engine_id);
            }
            if s.render_errors > self.last_render_errors {
                self.last_render_errors = s.render_errors;
                // The worker publishes details and the atomic count separately.
                // A later count observation must not erase its useful diagnostics.
                if self.render_diagnostic_engine != Some(s.engine_id) || !self.error {
                    render_error = Some(format!(
                        "Plug-in/render error · {} failures · Stop and restart playback to retry",
                        s.render_errors
                    ));
                }
            }
            if let Some(details) = e.take_render_error() {
                self.render_diagnostic_engine = Some(s.engine_id);
                render_error = Some(details);
            }
        } else {
            self.output_dropout_notice = None;
        }
        if let Some(error) = render_error {
            self.message(Err(anyhow::anyhow!(error)), "");
        }
        self.poll_audio_take_import(ctx);
        self.poll_plugin_catalog(ctx);
        self.poll_document_guard(ctx);
        self.poll_assistant(ctx);
        if crate::plugin_details::is_open(ctx) {
            crate::plugin_details::show_window(ctx);
            self.capture_screenshot(ctx);
            return;
        }
        if self.error_details.is_some() {
            self.error_details_frame(ctx);
            self.capture_screenshot(ctx);
            return;
        }
        self.menus(ctx);
        if self.document_guard.is_some() {
            self.document_guard_frame(ctx);
            self.error_details_frame(ctx);
            self.capture_screenshot(ctx);
            return;
        }
        self.transport(ctx);
        self.status_notice(ctx);
        if self.view == WorkspaceView::Clip && self.selected().is_none() {
            self.close_popups();
        }
        // Also cover workspace changes made by menu/toolbar actions this frame,
        // and clip-kind changes that keep WorkspaceView::Clip unchanged.
        self.sync_workspace_tool();
        let utility_open = self.assistant.open
            || self.library_open
            || self.setup_open
            || self.inspector_open
            || self.instrument_open
            || self.effects_open
            || self.routing_open
            || self.automation_open
            || self.history_open
            || self.crossfade_open
            || self.join_open
            || self.audio_takes_open
            || self.recording_open
            || self.export_open
            || self.midi_export_open
            || self.analysis.open
            || self.midi_expression_open
            || self.about_open;
        let focused_before_draw = self.focused_view();
        let workspace = self.view;
        let mut scene_bounds = ctx.content_rect();
        egui::CentralPanel::default()
            .frame(daw_ui::inset(ctx).inner_margin(0).corner_radius(0))
            .show(ctx, |ui| {
                scene_bounds = ui.max_rect();
                if utility_open {
                    if self.popup_backdrop.shape_count() == 0 {
                        // Startup previews may open a utility before the first
                        // workspace frame. Paint its caller once, then retain
                        // only inert pixels just as for ordinary navigation.
                        let layer = ui.layer_id();
                        let start = ctx.graphics(|graphics| {
                            graphics.get(layer).map_or(0, |list| list.next_idx().0)
                        });
                        ui.add_enabled_ui(false, |ui| self.arrangement(ui));
                        self.popup_backdrop = daw_ui::ViewBackdrop::capture(ctx, scene_bounds);
                        if self.backdrop_title.is_empty() {
                            self.backdrop_title = "Arrangement".into();
                        }
                        ctx.graphics_mut(|graphics| {
                            let list = graphics.entry(layer);
                            for index in start..list.next_idx().0 {
                                list.reset_shape(egui::layers::ShapeIdx(index));
                            }
                        });
                    }
                    self.popup_backdrop.paint(ui);
                    self.drop_target = None;
                    return;
                }
                if workspace == WorkspaceView::Arrangement {
                    self.arrangement(ui);
                    return;
                }
                self.drop_target = None;
                let title = if workspace == WorkspaceView::Clip {
                    self.selected()
                        .map(|clip| format!("Clip Editor · {}", clip.name))
                        .unwrap_or_else(|| "Clip Editor".into())
                } else {
                    "Mixer".into()
                };
                let clip_context = (workspace == WorkspaceView::Clip)
                    .then(|| {
                        let clip = self.selected()?;
                        let track = self.selected_track.and_then(|id| self.project.track(id))?;
                        let position = |beat: f64| {
                            daw_ui::duration::compact_musical_position(
                                beat,
                                self.project.beats_per_bar,
                            )
                        };
                        Some(format!(
                            "Song {}–{} · {}",
                            position(clip.start),
                            position(clip.start + clip.length),
                            track.name,
                        ))
                    })
                    .flatten();
                let header = daw_ui::clip_workspace_header(ui, &title, clip_context.as_deref(), |ui, slot| {
                    if slot == daw_ui::HeaderSlot::Navigation {
                        // Previous/Next belong to the clip identity, right after Back.
                        if workspace == WorkspaceView::Clip {
                            self.clip_navigation_header(ui);
                        }
                        return;
                    }
                    // The first trailing command occupies the same slot for
                    // audio and MIDI, regardless of their other header tools.
                    if workspace == WorkspaceView::Clip
                        && ui
                            .add(
                                daw_ui::IconButton::with_icon(daw_ui::Icon::Settings, "Properties")
                                    .quiet(),
                            )
                            .on_hover_text("Clip position, duration, name and other properties")
                            .clicked()
                    {
                        self.open_popup(PopupKind::Inspector);
                    }
                    let ai_scope = if workspace == WorkspaceView::Clip {
                        self.selected_clip_ai_scope()
                    } else {
                        self.selected_track
                            .filter(|id| {
                                self.project.track(*id).is_some_and(|track| {
                                    !matches!(track.kind, TrackKind::Folder | TrackKind::Automation)
                                })
                            })
                            .map(|track_id| daw_tools::ToolScope::Channel { track_id })
                    };
                    if let Some(scope) = ai_scope {
                        let label = scope.label(&self.project);
                        let response = if workspace == WorkspaceView::Clip {
                            // Quiet in the editor header; the pink text keeps its AI role.
                            let compact = ui.ctx().content_rect().width() < 1000.0;
                            ui.add(
                                daw_ui::ai_button_widget(if compact { "AI" } else { "AI for this context" })
                                    .quiet(),
                            )
                            .on_hover_text(format!("Open AI scoped to {label}"))
                        } else {
                            daw_ui::contextual_ai_button(ui, &label)
                        };
                        if response.clicked() {
                            self.open_context_assistant(scope);
                            return;
                        }
                    }
                    if workspace == WorkspaceView::Mixer {
                        self.mixer_track_navigation(ui);
                        self.mixer_order_menu(ui);
                        self.mixer_add_send_menu(ui);
                        if daw_ui::button(ui, "Add bus").clicked() {
                            self.mixer_add_bus(ui.ctx());
                        }
                    }
                    if workspace == WorkspaceView::Clip
                        && let Some(track) = self
                            .selected_track
                            .and_then(|id| self.project.track(id))
                            .filter(|track| track.kind == TrackKind::Midi)
                    {
                        let unassigned = track.instrument == Instrument::None;
                        let response = if unassigned {
                            daw_ui::primary_button(ui, "Choose instrument…")
                                .on_hover_text("These MIDI notes are silent until an instrument is assigned")
                        } else {
                            ui.add(
                                daw_ui::IconButton::with_icon(daw_ui::Icon::Instrument, "Instrument")
                                    .quiet(),
                            )
                            .on_hover_text(
                                "Open this track's instrument; return to the same notes and editing position",
                            )
                        };
                        if response.clicked() {
                            if unassigned {
                                self.open_instrument_chooser(true);
                            } else {
                                self.open_popup(PopupKind::Instrument);
                            }
                        }
                    }
                    if workspace == WorkspaceView::Clip
                        && self
                            .selected()
                            .is_some_and(|clip| clip.link_group.is_some())
                    {
                        daw_ui::clip_label(ui, "Ghost", true);
                    }
                });
                ctx.data_mut(|data| {
                    data.insert_temp(egui::Id::new("workspace_title_rect"), header.title.rect)
                });
                if let Some(context) = header.context
                    && let Some(clip) = self.selected()
                {
                    let ruler_context = if matches!(&clip.content, ClipContent::Midi { .. }) {
                        "The editor ruler starts at local bar 1 for this clip."
                    } else {
                        "The editor ruler shows song bars and beats."
                    };
                    context.on_hover_text(format!(
                        "{}\nSong position: {} to {} (bar:beat)\n{}–{}\n{ruler_context}",
                        clip_context.as_deref().unwrap_or_default(),
                        daw_ui::duration::compact_musical_position(clip.start, self.project.beats_per_bar),
                        daw_ui::duration::compact_musical_position(clip.start + clip.length, self.project.beats_per_bar),
                        daw_ui::duration::format_seconds(clip.start * 60.0 / self.project.tempo),
                        daw_ui::duration::format_seconds((clip.start + clip.length) * 60.0 / self.project.tempo),
                    ));
                }
                if header.back.clicked() {
                    self.close_popup();
                } else if self.view == workspace {
                    if workspace == WorkspaceView::Clip {
                        self.editor(ui);
                    } else {
                        self.mixer(ui);
                    }
                }
            });
        self.browser(ctx);
        self.setup(ctx);
        self.inspector(ctx);
        self.instrument_editor(ctx);
        self.channel_rack(ctx);
        self.routing_popup(ctx);
        self.automation_editor(ctx);
        self.history_popup(ctx);
        self.crossfade_popup(ctx);
        self.join_popup(ctx);
        self.audio_takes_popup(ctx);
        self.recording_popup(ctx);
        self.export_popup(ctx);
        self.about_popup(ctx);
        self.assistant_popup(ctx);
        self.signal_analysis_popup(ctx);
        self.midi_expression_popup(ctx);
        self.audio_silence_dialog(ctx);
        self.audio_batch_fades_dialog(ctx);
        self.rename_overlay(ctx);
        self.last_scene = if utility_open {
            daw_ui::ViewBackdrop::capture_overlay(ctx, scene_bounds, &self.popup_backdrop)
        } else {
            daw_ui::ViewBackdrop::capture(ctx, scene_bounds)
        };
        daw_ui::paint_popup_backdrop(ctx, utility_open);
        self.update_note_audition(ctx);
        // Utility windows own their close button and Apply/Cancel flags. Restore
        // their caller only on dismissal, never on a transition to another tool.
        if matches!(focused_before_draw, FocusedView::Popup(_))
            && self.focused_view() == FocusedView::Workspace(WorkspaceView::Arrangement)
        {
            self.close_popup();
        }
        let files = if self.error_details.is_none()
            && !crate::audio_silence::is_open(ctx)
            && !crate::audio_batch::is_open(ctx)
            && !self.rename_active(ctx)
        {
            ctx.input(|i| i.raw.dropped_files.clone())
        } else {
            Vec::new()
        };
        let paths: Vec<_> = files.into_iter().filter_map(|file| file.path).collect();
        if !paths.is_empty() {
            let (track, beat) = if self.view == WorkspaceView::Arrangement {
                self.drop_target.unwrap_or((None, self.cursor))
            } else {
                (None, self.cursor)
            };
            self.close_popups();
            self.queue_audio_import(paths, track, beat);
        }
        if !ctx.input(|i| i.pointer.any_down()) {
            self.flush_pending_edit();
            self.drag = None;
        } else {
            self.preview_playback_edit();
        }
        if self.modified
            && self.last_recovery.elapsed() > Duration::from_secs(30)
            && !self.state_sync_pending()
        {
            self.request_recovery_save();
            self.last_recovery = Instant::now();
        }
        self.error_details_frame(ctx);
        crate::spectrogram::end_frame(ctx);
        self.capture_screenshot(ctx);
        ctx.request_repaint_after(Duration::from_millis(
            if self.playing()
                || self.loading.is_some()
                || self.export_job.is_some()
                || self.project_io_pending()
                || self.screenshot.is_some()
                || self.assistant.busy()
            {
                16
            } else {
                100
            },
        ));
    }
    /// Open the workspace or popup named by the `--view` command-line option.
    pub fn open_screenshot_view(&mut self, view: &str) {
        match view {
            "assistant" => self.open_global_assistant(),
            "setup" => self.open_popup(PopupKind::Setup),
            "mixer" => self.view = WorkspaceView::Mixer,
            "analysis" => {
                self.open_workspace(WorkspaceView::Mixer);
                let target = self.selected_track.map_or(
                    daw_engine::SignalMonitorTarget::Master,
                    daw_engine::SignalMonitorTarget::Channel,
                );
                self.open_signal_analysis(target);
            }
            "channel" => {
                if let Some(track) = self.selected_track {
                    self.open_channel(track);
                }
            }
            "clip" => self.view = WorkspaceView::Clip,
            "expression" => {
                let target = self.project.tracks.iter().find_map(|track| {
                    track
                        .clips
                        .iter()
                        .filter(|clip| !clip.is_take_source())
                        .find_map(|clip| {
                            let ClipContent::Midi { notes } = &clip.content else {
                                return None;
                            };
                            notes.first().map(|note| (track.id, clip.id, note.id))
                        })
                });
                if let Some((track, clip, note)) = target {
                    self.selected_track = Some(track);
                    self.selected_clip = Some(clip);
                    self.selected_notes = HashSet::from([note]);
                    self.view = WorkspaceView::Clip;
                    self.open_midi_expression(track, clip, note);
                }
            }
            "library" => self.open_popup(PopupKind::Library),
            "automation" => self.open_popup(PopupKind::Automation),
            "crossfade" => self.open_popup(PopupKind::Crossfade),
            "join" => self.open_popup(PopupKind::Join),
            "routing" => self.open_popup(PopupKind::Routing),
            "properties" => self.open_popup(PopupKind::Inspector),
            "unsaved-changes" if self.screenshot.is_some() => {
                self.modified = true;
                self.history.mark_unsaved();
                self.request_document_transition(crate::document_ui::DocumentTransition::New);
            }
            "instrument" => self.open_popup(PopupKind::Instrument),
            "rack" | "effects" => self.open_popup(PopupKind::Effects),
            _ => {}
        }
    }
    fn capture_screenshot(&mut self, ctx: &egui::Context) {
        if let Some(path) = &self.screenshot {
            if self.frame_count >= 8
                && !self.plugin_sync.scanning()
                && !self.screenshot_requested
                && (!crate::spectrogram::has_pending_analysis(ctx) || self.frame_count >= 180)
            {
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
        if self.screenshot.is_some() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}

/// Cosmetic edits never rebuild the audio graph or restart plug-in processes.
pub(crate) fn audio_equivalent(a: &Project, b: &Project) -> bool {
    // Track order belongs to the workspace. Compare stable channel identities
    // so rearranging headers cannot restart a running graph or its plug-ins.
    let tracks: std::collections::HashMap<_, _> =
        b.channels().map(|track| (track.id, track)).collect();
    a.tempo == b.tempo
        && a.beats_per_bar == b.beats_per_bar
        && a.loop_enabled == b.loop_enabled
        && a.loop_start == b.loop_start
        && a.loop_end == b.loop_end
        && a.tracks.len() == b.tracks.len()
        && tracks.len() == b.tracks.len() + 1
        && a.channels().all(|a| {
            tracks.get(&a.id).is_some_and(|b| {
                a.kind == b.kind
                    && a.input_gain == b.input_gain
                    && a.gain == b.gain
                    && a.pan == b.pan
                    && a.mute == b.mute
                    && a.solo == b.solo
                    && a.instrument == b.instrument
                    && a.instrument_state == b.instrument_state
                    && a.instrument_parameters == b.instrument_parameters
                    && a.instrument_inputs == b.instrument_inputs
                    && a.instrument_output == b.instrument_output
                    && a.latency_samples == b.latency_samples
                    && a.automation == b.automation
                    && a.inserts.len() == b.inserts.len()
                    && a.inserts.iter().zip(&b.inserts).all(|(a, b)| {
                        a.id == b.id
                            && a.enabled == b.enabled
                            && a.effect == b.effect
                            && a.state == b.state
                            && a.parameters == b.parameters
                            && a.inputs == b.inputs
                    })
                    && a.sends == b.sends
                    && a.output_bus == b.output_bus
                    && a.parent_folder == b.parent_folder
                    && a.clips.len() == b.clips.len()
                    && a.clips.iter().zip(&b.clips).all(|(a, b)| {
                        a.start == b.start
                            && a.muted == b.muted
                            && a.length == b.length
                            && a.offset == b.offset
                            && a.source_rate == b.source_rate
                            && a.audio_take == b.audio_take
                            && a.midi_events == b.midi_events
                            && match (&a.content, &b.content) {
                                (
                                    ClipContent::Midi { notes: a },
                                    ClipContent::Midi { notes: b },
                                ) => {
                                    a.len() == b.len()
                                        && a.iter().zip(b).all(|(a, b)| {
                                            a.pitch == b.pitch
                                                && a.start == b.start
                                                && a.length == b.length
                                                && a.velocity == b.velocity
                                                && a.midi2 == b.midi2
                                        })
                                }
                                (a, b) => a == b,
                            }
                    })
            })
        })
}

impl eframe::App for DawApp {
    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let Some(gl) = gl {
            daw_ui::destroy_backdrop_blur(&self.render_context, gl);
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui_frame(ctx);
    }
}

#[cfg(test)]
#[path = "engine_lifecycle_tests.rs"]
mod engine_lifecycle_tests;

#[cfg(test)]
mod export_feedback_tests {
    use super::*;

    #[test]
    fn completed_export_preserves_an_unrelated_error_and_pending_feedback() {
        let ctx = egui::Context::default();
        let mut app = DawApp::with_context(&ctx, Project::new(), None);
        let (tx, rx) = std::sync::mpsc::channel();
        app.export_job = Some(rx);
        app.error = true;
        app.status = "Audio device disconnected".into();
        let path = PathBuf::from("Finished.wav");
        tx.send(Ok(crate::export_ui::ExportCompletion {
            path: path.clone(),
            report: daw_engine::ExportReport {
                sample_peak: 0.0,
                clipped_samples: 0,
                frames: 0,
                sample_rate: 48_000,
                encoding: daw_engine::WavEncoding::Float32,
                dither: false,
                unsupported_midi_events: 0,
                unsupported_midi_groups: 0,
            },
        }))
        .unwrap();
        app.poll_export();
        assert!(app.export_job.is_none());
        assert!(app.error);
        assert_eq!(app.status, "Audio device disconnected");
        assert_eq!(app.exported_file, Some(path));
    }
}

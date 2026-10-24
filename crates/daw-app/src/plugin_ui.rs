use crate::{
    app::{DawApp, PopupKind, SetupPage},
    theme::*,
};
use daw_core::{
    AutomationParameter, AutomationTarget, Clip, Instrument, PluginParameterValue, TrackKind,
};
use daw_plugins::catalog::{CatalogEntry, PluginCatalog, ThumbnailStatus};
use daw_plugins::{EditorEvent, EditorSession, ParameterInfo};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

pub(crate) type EditorStateSignature = (u64, Instrument, Vec<u8>, Vec<PluginParameterValue>);

type CaptureJob = Arc<Mutex<Option<Result<CatalogEntry, String>>>>;

#[derive(Clone, Default)]
struct LibraryUi {
    activate_when_ready: bool,
    selected: Option<String>,
    preview: Option<(PathBuf, egui::TextureHandle)>,
    capture: Option<CaptureJob>,
    attempted: HashSet<String>,
    thumbnails: HashMap<String, egui::TextureHandle>,
}

enum PendingAction {
    Save(PathBuf),
    Export(PathBuf),
    Quit,
    New,
    Open {
        project: Box<daw_core::Project>,
        path: PathBuf,
        recovery: bool,
    },
}

impl PendingAction {
    fn closes_editor(&self) -> bool {
        matches!(self, Self::Quit | Self::New | Self::Open { .. })
    }
}

struct PendingState {
    action: PendingAction,
    deadline: Instant,
    request_id: Option<u64>,
    saved: bool,
}

impl PendingState {
    fn observe_snapshot(&mut self, request_id: Option<u64>) {
        self.saved |= self.request_id.is_some() && request_id == self.request_id;
    }

    fn completed(&self, closed_request: Option<Option<u64>>, failed: bool) -> bool {
        !failed
            && self.saved
            && (!self.action.closes_editor()
                || (self.request_id.is_some() && closed_request == Some(self.request_id)))
    }
}

#[derive(Default)]
pub(crate) struct InstrumentUi {
    owner: Option<(u64, Instrument)>,
    session: Option<EditorSession>,
    opening: Option<mpsc::Receiver<anyhow::Result<EditorSession>>>,
    metadata_job: Option<mpsc::Receiver<anyhow::Result<Vec<ParameterInfo>>>>,
    parameters: Vec<ParameterInfo>,
    selected: Option<u32>,
    last_touched: Option<u32>,
    filter: String,
    closing: bool,
    last_saved_request: Option<u64>,
    error: Option<String>,
    pending: Option<PendingState>,
}

impl DawApp {
    pub(crate) fn reset_instrument_ui(&mut self) {
        if let Some(session) = self.instrument_ui.session.take() {
            std::thread::spawn(move || drop(session));
        }
        self.instrument_ui = InstrumentUi::default();
        self.instrument_autolaunch = None;
        self.plugin_assign_target = None;
        self.plugin_search_focus = false;
    }

    pub(crate) fn editor_state_signature(&self) -> Option<EditorStateSignature> {
        let (id, _) = self.instrument_ui.owner.as_ref()?;
        let track = self.project.track(*id)?;
        Some((
            *id,
            track.instrument.clone(),
            track.instrument_state.clone(),
            track.instrument_parameters.clone(),
        ))
    }

    pub(crate) fn reconcile_editor_history(&mut self, previous: Option<EditorStateSignature>) {
        if previous.is_some() && previous != self.editor_state_signature() {
            // A native snapshot from the abandoned history state must never
            // overwrite the restored project, including when the class is unchanged.
            if let Some(session) = self.instrument_ui.session.take() {
                std::thread::spawn(move || drop(session));
            }
            let pending = self.instrument_ui.pending.is_some();
            self.instrument_ui = InstrumentUi::default();
            if pending {
                self.message(
                    Err(anyhow::anyhow!(
                        "Instrument state save cancelled by Undo/Redo"
                    )),
                    "",
                );
            }
        }
    }

    pub(crate) fn instrument_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({
            "open": self.instrument_ui.session.is_some(),
            "opening": self.instrument_ui.opening.is_some(),
            "closing": self.instrument_ui.closing,
            "parameter_count": self.instrument_ui.parameters.len(),
            "selected": self.instrument_ui.selected,
            "last_touched": self.instrument_ui.last_touched,
            "error": self.instrument_ui.error,
        })
    }

    pub(crate) fn state_sync_pending(&self) -> bool {
        self.instrument_ui.pending.is_some()
    }

    fn defer_for_editor(&mut self, action: PendingAction) -> Option<PendingAction> {
        if self.instrument_ui.pending.is_some() {
            self.message(
                Err(anyhow::anyhow!(
                    "Wait for the instrument state to finish saving"
                )),
                "",
            );
            return None;
        }
        if self.instrument_ui.session.is_none() && self.instrument_ui.opening.is_none() {
            return Some(action);
        }
        let closing = action.closes_editor();
        let result = self
            .instrument_ui
            .session
            .as_ref()
            .map(|session| {
                if closing {
                    session.close()
                } else {
                    session.save_state()
                }
            })
            .transpose();
        let request_id = match result {
            Ok(request_id) => request_id,
            Err(error) => {
                self.message(Err(error), "");
                return None;
            }
        };
        self.instrument_ui.closing |= closing;
        self.instrument_ui.pending = Some(PendingState {
            action,
            deadline: Instant::now() + Duration::from_secs(30),
            request_id,
            saved: request_id.is_some() && request_id == self.instrument_ui.last_saved_request,
        });
        self.status = "Saving instrument state…".into();
        self.error = false;
        None
    }

    pub(crate) fn new_session(&mut self) {
        if let Some(PendingAction::New) = self.defer_for_editor(PendingAction::New) {
            self.finish_new_session();
        }
    }

    pub(crate) fn request_open_project(&mut self, path: PathBuf, recovery: bool) {
        let project = match daw_core::Project::load(&path) {
            Ok(project) => Box::new(project),
            Err(error) => {
                self.message(Err(error), "");
                return;
            }
        };
        if let Some(PendingAction::Open {
            project,
            path,
            recovery,
        }) = self.defer_for_editor(PendingAction::Open {
            project,
            path,
            recovery,
        }) {
            self.finish_open_project(*project, path, recovery);
        }
    }

    pub(crate) fn save_to(&mut self, path: PathBuf) {
        if let Some(PendingAction::Save(path)) = self.defer_for_editor(PendingAction::Save(path)) {
            self.finish_project_save(path);
        }
    }

    pub(crate) fn export_to(&mut self, path: PathBuf) {
        if self.export_job.is_some() {
            self.message(Err(anyhow::anyhow!("An export is already running")), "");
            return;
        }
        if let Some(PendingAction::Export(path)) =
            self.defer_for_editor(PendingAction::Export(path))
        {
            self.start_export(path);
        }
    }

    pub(crate) fn request_quit(&mut self, ctx: &egui::Context) {
        if matches!(
            self.instrument_ui
                .pending
                .as_ref()
                .map(|pending| &pending.action),
            Some(PendingAction::Quit)
        ) {
            return;
        }
        if let Some(PendingAction::Quit) = self.defer_for_editor(PendingAction::Quit) {
            self.autosave_recovery();
            self.close_ready = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Capture one verified instrument at a time. Discovery remains metadata-only;
    /// initialization and screenshot work always happen in isolated helpers.
    pub(crate) fn poll_plugin_previews(&mut self, ctx: &egui::Context) {
        let key = egui::Id::new("plugin_library_state");
        let mut state = ctx
            .data_mut(|data| data.get_temp::<LibraryUi>(key))
            .unwrap_or_default();
        if !self.library_open {
            state.activate_when_ready = false;
        }
        if let Some(job) = &state.capture {
            let result = job.lock().ok().and_then(|mut job| job.take());
            if let Some(result) = result {
                state.capture = None;
                match result {
                    Ok(entry) => {
                        if let Some(current) = self
                            .vst_catalog
                            .iter_mut()
                            .find(|item| item.fingerprint == entry.fingerprint)
                        {
                            *current = entry;
                        }
                        state.preview = None;
                    }
                    Err(error) => self.message(Err(anyhow::anyhow!(error)), ""),
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        if state.capture.is_none()
            && !self.state_sync_pending()
            && let Some(entry) = self
                .vst_catalog
                .iter()
                .filter(|entry| {
                    (entry.plugin.is_instrument() == Some(true)
                        || state.selected.as_ref() == Some(&entry.fingerprint))
                        && matches!(entry.status, ThumbnailStatus::Missing)
                        && !state.attempted.contains(&entry.fingerprint)
                })
                .min_by_key(|entry| state.selected.as_ref() != Some(&entry.fingerprint))
        {
            state.attempted.insert(entry.fingerprint.clone());
            let job: CaptureJob = Arc::new(Mutex::new(None));
            state.capture = Some(job.clone());
            let plugin = entry.plugin.clone();
            std::thread::spawn(move || {
                let result = PluginCatalog::default()
                    .capture(&plugin, |plugin, output| {
                        daw_plugins::capture_editor_info(plugin, output)
                    })
                    .map_err(|error| format!("{error:#}"));
                if let Ok(mut job) = job.lock() {
                    *job = Some(result);
                }
            });
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        ctx.data_mut(|data| data.insert_temp(key, state));
    }

    pub(crate) fn open_instrument_chooser(&mut self, assign: bool) {
        let target = self.selected_track.filter(|id| {
            self.project
                .track(*id)
                .is_some_and(|track| track.kind == TrackKind::Midi)
        });
        self.open_popup(PopupKind::Library);
        self.plugin_filter.clear();
        self.plugin_assign_target = if assign { target } else { None };
    }

    pub(crate) fn plugin_library(&mut self, ctx: &egui::Context) {
        if !self.library_open {
            return;
        }
        let key = egui::Id::new("plugin_library_state");
        let mut state = ctx
            .data_mut(|data| data.get_temp::<LibraryUi>(key))
            .unwrap_or_default();
        let mut open = true;
        let mut action = None;
        let mut folders = false;
        let assign = self.plugin_assign_target.is_some();
        let pressed_enter = ctx
            .data_mut(|data| data.remove_temp::<bool>(egui::Id::new("instrument_chooser_enter")))
            .unwrap_or(false)
            || ctx.input(|i| i.key_pressed(egui::Key::Enter));
        state.activate_when_ready |= pressed_enter;
        let activate = state.activate_when_ready && self.vst_scan.is_none();
        if activate {
            state.activate_when_ready = false;
        }
        daw_ui::popup("Instrument library")
            .id(egui::Id::new("library"))
            .open(&mut open)
            .default_width(780.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let search = daw_ui::text_field(
                        ui,
                        &mut self.plugin_filter,
                        "Search instruments",
                        390.0,
                    );
                    if self.plugin_search_focus {
                        if search.has_focus() {
                            self.plugin_search_focus = false;
                        } else {
                            search.request_focus();
                        }
                    }
                    if search.changed() {
                        state.selected = None;
                    }
                    if ui
                        .add_enabled(
                            self.vst_scan.is_none(),
                            daw_ui::button_widget(if self.vst_scan.is_some() {
                                "Scanning…"
                            } else {
                                "Scan"
                            }),
                        )
                        .clicked()
                    {
                        self.scan_plugins();
                    }
                    folders = daw_ui::button(ui, "Folders").clicked();
                });
                ui.add_space(8.0);
                let filter = self.plugin_filter.to_lowercase();
                let matches: Vec<_> = self
                    .vst_catalog
                    .iter()
                    .filter(|entry| {
                        entry.plugin.is_instrument() != Some(false)
                            && format!("{} {}", entry.plugin.name, entry.plugin.vendor)
                                .to_lowercase()
                                .contains(&filter)
                    })
                    .collect();
                if !matches
                    .iter()
                    .any(|entry| state.selected.as_ref() == Some(&entry.fingerprint))
                {
                    state.selected = matches.first().map(|entry| entry.fingerprint.clone());
                }
                let step = ctx.input(|i| {
                    if i.key_pressed(egui::Key::ArrowDown) {
                        1
                    } else if i.key_pressed(egui::Key::ArrowUp) {
                        -1
                    } else {
                        0
                    }
                });
                if step != 0 && !matches.is_empty() {
                    let index = matches
                        .iter()
                        .position(|entry| state.selected.as_ref() == Some(&entry.fingerprint))
                        .unwrap_or(0);
                    let index =
                        (index as isize + step).clamp(0, matches.len() as isize - 1) as usize;
                    state.selected = Some(matches[index].fingerprint.clone());
                }
                if activate
                    && let Some(entry) = matches
                        .iter()
                        .find(|entry| state.selected.as_ref() == Some(&entry.fingerprint))
                {
                    action = Some((entry.plugin.clone(), assign));
                }
                ui.columns(2, |columns| {
                    egui::ScrollArea::vertical()
                        .id_salt("instrument_catalog")
                        .max_height(380.0)
                        .show(&mut columns[0], |ui| {
                            let mut count = 0;
                            for entry in self
                                .vst_catalog
                                .iter()
                                .filter(|entry| entry.plugin.is_instrument() != Some(false))
                            {
                                let plugin = &entry.plugin;
                                if !format!("{} {}", plugin.name, plugin.vendor)
                                    .to_lowercase()
                                    .contains(&filter)
                                {
                                    continue;
                                }
                                count += 1;
                                let detail = if plugin.is_instrument().is_none() {
                                    "Unverified VST3".to_owned()
                                } else if plugin.vendor.is_empty() {
                                    "VST3 instrument".to_owned()
                                } else {
                                    plugin.vendor.clone()
                                };
                                let row_visible =
                                    ui.clip_rect().intersects(egui::Rect::from_min_size(
                                        ui.cursor().min,
                                        egui::vec2(ui.available_width(), 58.0),
                                    ));
                                if row_visible
                                    && !state.thumbnails.contains_key(&entry.fingerprint)
                                    && let Some(path) = &entry.thumbnail
                                    && let Ok(image) = image::open(path)
                                {
                                    let image = image.thumbnail(96, 64).to_rgba8();
                                    let pixels = egui::ColorImage::from_rgba_unmultiplied(
                                        [image.width() as usize, image.height() as usize],
                                        image.as_raw(),
                                    );
                                    state.thumbnails.insert(
                                        entry.fingerprint.clone(),
                                        ctx.load_texture(
                                            format!("thumb-{}", entry.fingerprint),
                                            pixels,
                                            egui::TextureOptions::LINEAR,
                                        ),
                                    );
                                }
                                let response = daw_ui::catalog_item(
                                    ui,
                                    state.selected.as_ref() == Some(&entry.fingerprint),
                                    &plugin.name,
                                    &detail,
                                    state.thumbnails.get(&entry.fingerprint),
                                );
                                if response.clicked() {
                                    state.selected = Some(entry.fingerprint.clone());
                                    state.preview = None;
                                }
                                if response.double_clicked() {
                                    action = Some((plugin.clone(), assign));
                                }
                            }
                            if count == 0 && self.vst_scan.is_none() {
                                ui.label("No instruments found");
                            }
                        });
                    let ui = &mut columns[1];
                    if let Some(entry) = state.selected.as_ref().and_then(|path| {
                        self.vst_catalog
                            .iter()
                            .find(|entry| &entry.fingerprint == path)
                    }) {
                        ui.heading(&entry.plugin.name);
                        if !entry.plugin.vendor.is_empty() {
                            ui.label(&entry.plugin.vendor);
                        }
                        if let Some(path) = &entry.thumbnail {
                            if state
                                .preview
                                .as_ref()
                                .is_none_or(|(loaded, _)| loaded != path)
                                && let Ok(image) = image::open(path).map(|image| image.to_rgba8())
                            {
                                let size = [image.width() as usize, image.height() as usize];
                                let pixels =
                                    egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                                state.preview = Some((
                                    path.clone(),
                                    ctx.load_texture(
                                        entry.fingerprint.clone(),
                                        pixels,
                                        egui::TextureOptions::LINEAR,
                                    ),
                                ));
                            }
                            if let Some((_, texture)) = &state.preview {
                                daw_ui::image_preview(
                                    ui,
                                    texture,
                                    egui::vec2(ui.available_width(), 240.0),
                                );
                            }
                        }
                        if state.capture.is_some() {
                            ui.label("Capturing editor…");
                        } else if (matches!(entry.status, ThumbnailStatus::Missing)
                            && !state.attempted.contains(&entry.fingerprint))
                            || daw_ui::button(
                                ui,
                                if entry.thumbnail.is_some() {
                                    "Refresh preview"
                                } else {
                                    "Capture preview"
                                },
                            )
                            .clicked()
                        {
                            state.attempted.insert(entry.fingerprint.clone());
                            let job: CaptureJob = Arc::new(Mutex::new(None));
                            state.capture = Some(job.clone());
                            let plugin = entry.plugin.clone();
                            std::thread::spawn(move || {
                                let result = PluginCatalog::default()
                                    .capture(&plugin, |plugin, output| {
                                        daw_plugins::capture_editor_info(plugin, output)
                                    })
                                    .map_err(|error| format!("{error:#}"));
                                if let Ok(mut job) = job.lock() {
                                    *job = Some(result);
                                }
                            });
                        }
                        if let ThumbnailStatus::Failed { message } = &entry.status {
                            ui.label(
                                egui::RichText::new("Preview unavailable")
                                    .small()
                                    .color(MUTED),
                            )
                            .on_hover_text(message);
                        }
                        ui.add_space(10.0);
                        if daw_ui::primary_button(
                            ui,
                            if assign {
                                "Assign instrument"
                            } else {
                                "Add instrument track"
                            },
                        )
                        .clicked()
                        {
                            action = Some((entry.plugin.clone(), assign));
                        }
                        let can_assign = self
                            .selected_track
                            .and_then(|id| self.project.track(id))
                            .is_some_and(|track| track.kind == TrackKind::Midi);
                        if !assign
                            && ui
                                .add_enabled(
                                    can_assign,
                                    daw_ui::button_widget("Assign to selected track"),
                                )
                                .clicked()
                        {
                            action = Some((entry.plugin.clone(), true));
                        }
                    } else {
                        ui.label(egui::RichText::new("Select an instrument").color(MUTED));
                    }
                });
            });
        self.library_open &= open;
        if folders {
            self.open_popup(PopupKind::Setup);
            self.setup_page = SetupPage::Plugins;
        }
        if let Some((plugin, replace)) = action {
            self.begin_edit();
            let track = if replace {
                self.plugin_assign_target.or(self.selected_track).unwrap()
            } else {
                let id = self.project.add_midi_track(&plugin.name);
                let clip = Clip::midi(
                    "New pattern",
                    self.snapped(self.cursor).max(0.0),
                    4.0,
                    Vec::new(),
                );
                self.selected_clip = Some(clip.id);
                self.project.track_mut(id).unwrap().clips.push(clip);
                id
            };
            let result = self.project.set_instrument(
                track,
                Instrument::Vst3 {
                    path: plugin.path.to_string_lossy().into_owned(),
                    class_id: plugin.class_id.clone(),
                },
            );
            if result.is_ok() {
                self.selected_track = Some(track);
                if replace {
                    self.selected_clip = self
                        .project
                        .track(track)
                        .and_then(|track| track.clips.first())
                        .map(|clip| clip.id);
                }
                self.selected_notes.clear();
                self.changed();
                self.open_popup(PopupKind::Instrument);
                self.instrument_autolaunch = Some(track);
            }
            self.message(
                result,
                if replace {
                    "Instrument assigned"
                } else {
                    "Instrument track added"
                },
            );
        }
        ctx.data_mut(|data| data.insert_temp(key, state));
    }

    pub(crate) fn plugin_instrument_editor(&mut self, ctx: &egui::Context) {
        if !self.instrument_open {
            return;
        }
        let Some(track) = self
            .selected_track
            .and_then(|id| self.project.track(id))
            .filter(|track| track.kind == TrackKind::Midi)
        else {
            self.instrument_open = false;
            return;
        };
        let track_id = track.id;
        let name = track.name.clone();
        let instrument = track.instrument.clone();
        let owner = (track_id, instrument.clone());
        if self.instrument_ui.owner.as_ref() != Some(&owner) {
            if self.instrument_ui.session.is_some() || self.instrument_ui.opening.is_some() {
                if !self.instrument_ui.closing
                    && let Some(session) = &self.instrument_ui.session
                {
                    let _ = session.close();
                }
                self.instrument_ui.closing = true;
                ctx.request_repaint_after(Duration::from_millis(25));
                return;
            }
            self.instrument_ui = InstrumentUi {
                owner: Some(owner),
                ..Default::default()
            };
        }
        let mut open = true;
        let mut choose = false;
        let mut launch = false;
        let mut fetch = false;
        let mut update = None;
        let mut automate = None;
        daw_ui::popup(format!("Instrument · {name}"))
            .id(egui::Id::new("instrument_editor"))
            .open(&mut open)
            .default_width(600.0)
            .show(ctx, |ui| match &instrument {
                Instrument::None => {
                    ui.label("No instrument assigned");
                    choose = daw_ui::primary_button(ui, "Choose instrument").clicked();
                }
                Instrument::Vst3 { path, .. } => {
                    ui.label(
                        PathBuf::from(path)
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                    );
                    ui.horizontal(|ui| {
                        launch = ui
                            .add_enabled(
                                self.instrument_ui.session.is_none()
                                    && self.instrument_ui.opening.is_none(),
                                daw_ui::button_widget(if self.instrument_ui.opening.is_some() {
                                    "Opening editor…"
                                } else {
                                    "Open editor"
                                }),
                            )
                            .clicked();
                        choose = daw_ui::button(ui, "Change instrument").clicked();
                        if let Some(session) = &self.instrument_ui.session {
                            if daw_ui::button(ui, "Save state").clicked()
                                && let Err(error) = session.save_state()
                            {
                                self.instrument_ui.error = Some(error.to_string());
                            }
                            if daw_ui::button(ui, "Close editor").clicked() {
                                let _ = session.close();
                                self.instrument_ui.closing = true;
                            }
                        }
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        daw_ui::text_field(
                            ui,
                            &mut self.instrument_ui.filter,
                            "Search parameters",
                            300.0,
                        );
                        fetch = ui
                            .add_enabled(
                                self.instrument_ui.metadata_job.is_none()
                                    && self.instrument_ui.session.is_none()
                                    && self.instrument_ui.opening.is_none(),
                                daw_ui::button_widget(
                                    if self.instrument_ui.metadata_job.is_some() {
                                        "Loading…"
                                    } else {
                                        "Read parameters"
                                    },
                                ),
                            )
                            .clicked();
                    });
                    let filter = self.instrument_ui.filter.to_lowercase();
                    egui::ScrollArea::vertical()
                        .id_salt("instrument_parameters")
                        .max_height(260.0)
                        .show(ui, |ui| {
                            for parameter in
                                self.instrument_ui.parameters.iter().filter(|p| {
                                    !p.read_only && p.name.to_lowercase().contains(&filter)
                                })
                            {
                                daw_ui::selectable_value(
                                    ui,
                                    &mut self.instrument_ui.selected,
                                    Some(parameter.id),
                                    &parameter.name,
                                );
                            }
                        });
                    if let Some(parameter) = self
                        .instrument_ui
                        .selected
                        .and_then(|id| self.instrument_ui.parameters.iter().find(|p| p.id == id))
                    {
                        let mut value = parameter.value;
                        let response =
                            ui.add(daw_ui::slider(&mut value, 0.0..=1.0).text(&parameter.name));
                        ctx.data_mut(|data| {
                            data.insert_temp(
                                egui::Id::new("instrument_parameter_rect"),
                                response.rect,
                            )
                        });
                        if response.changed() {
                            update = Some((parameter.id, value));
                        }
                        if daw_ui::button(ui, "Automate parameter").clicked() {
                            automate =
                                Some((parameter.id, parameter.value, parameter.name.clone()));
                        }
                    }
                    if let Some(parameter) = self
                        .instrument_ui
                        .last_touched
                        .and_then(|id| self.instrument_ui.parameters.iter().find(|p| p.id == id))
                        && daw_ui::button(ui, format!("Automate last touched · {}", parameter.name))
                            .clicked()
                    {
                        automate = Some((parameter.id, parameter.value, parameter.name.clone()));
                    }
                    if let Some(error) = &self.instrument_ui.error {
                        ui.colored_label(ORANGE, error);
                    }
                }
            });
        self.instrument_open &= open;
        if self.instrument_autolaunch == Some(track_id)
            && (self.instrument_ui.session.is_some() || self.instrument_ui.opening.is_some())
        {
            self.instrument_autolaunch = None;
        }
        launch |= self.instrument_autolaunch == Some(track_id)
            && self.instrument_ui.session.is_none()
            && self.instrument_ui.opening.is_none();
        if launch {
            self.instrument_autolaunch = None;
        }
        if launch || fetch {
            let track = self.project.track(track_id).unwrap();
            if let Instrument::Vst3 { path, class_id } = &track.instrument {
                let path = PathBuf::from(path);
                let class_id = class_id.clone();
                let state = track.instrument_state.clone();
                let parameters: Vec<_> = track
                    .instrument_parameters
                    .iter()
                    .map(|p| (p.id, p.value))
                    .collect();
                self.instrument_ui.error = None;
                if launch {
                    // Ready provides current values. An older metadata-only probe
                    // must never replace them after the editor starts receiving edits.
                    self.instrument_ui.metadata_job = None;
                    let (tx, rx) = mpsc::channel();
                    self.instrument_ui.opening = Some(rx);
                    std::thread::spawn(move || {
                        let _ = tx.send(EditorSession::open_class(
                            &path,
                            class_id.as_deref(),
                            &state,
                            &parameters,
                        ));
                    });
                } else {
                    let (tx, rx) = mpsc::channel();
                    self.instrument_ui.metadata_job = Some(rx);
                    std::thread::spawn(move || {
                        let result = (|| -> anyhow::Result<_> {
                            let mut plugin = daw_plugins::VstInstrument::load_class(
                                &path,
                                class_id.as_deref(),
                                48_000.0,
                                256,
                            )?;
                            if !state.is_empty() {
                                plugin.load_state(&state)?;
                            }
                            for (id, value) in parameters {
                                plugin.set_parameter(id, value)?;
                            }
                            plugin.parameters()
                        })();
                        let _ = tx.send(result);
                    });
                }
            }
        }
        if let Some((id, value)) = update {
            if let Some(session) = &self.instrument_ui.session
                && let Err(error) = session.set_parameter(id, value)
            {
                self.instrument_ui.error = Some(error.to_string());
            }
            self.apply_instrument_parameters(&[(id, value)]);
        }
        if let Some((id, value, label)) = automate {
            self.begin_edit();
            match self.project.add_automation_track_with_value(
                format!("{name} · {label}"),
                AutomationTarget {
                    track: track_id,
                    parameter: AutomationParameter::VstParameter { effect: None, id },
                },
                value as f32,
            ) {
                Ok(id) => {
                    self.selected_track = Some(id);
                    self.selected_clip = None;
                    self.changed();
                    self.open_popup(PopupKind::Automation);
                }
                Err(error) => self.message(Err(error), ""),
            }
        }
        if choose {
            self.open_instrument_chooser(true);
        }
    }

    pub(crate) fn poll_instrument_editor(&mut self, ctx: &egui::Context) {
        let mut initialization_failed = false;
        let mut editor_failed = false;
        let opening = self
            .instrument_ui
            .opening
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(result) => {
                    initialization_failed = result.is_err();
                    Some(result)
                }
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!(
                    "Editor startup worker stopped unexpectedly"
                ))),
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = opening {
            self.instrument_ui.opening = None;
            match result {
                Ok(session) => {
                    if let Some(pending) = &mut self.instrument_ui.pending {
                        let result = if pending.action.closes_editor() {
                            session.close()
                        } else {
                            session.save_state()
                        };
                        match result {
                            Ok(request_id) => pending.request_id = Some(request_id),
                            Err(error) => {
                                self.instrument_ui.error = Some(error.to_string());
                                editor_failed = true;
                            }
                        }
                    } else if self.instrument_ui.closing
                        && let Err(error) = session.close()
                    {
                        self.instrument_ui.error = Some(error.to_string());
                        editor_failed = true;
                    }
                    self.instrument_ui.session = Some(session);
                }
                Err(error) => {
                    self.instrument_ui.error = Some(format!("{error:#}"));
                    self.instrument_ui.closing = false;
                }
            }
        }
        // A reported load failure occurred before a usable editor session existed,
        // so no native edits need flushing. Disconnection/timeouts remain failures:
        // their outcome is unknown and must retain the ordinary recovery safeguard.
        if initialization_failed
            && self.instrument_ui.session.is_none()
            && matches!(
                self.instrument_ui
                    .pending
                    .as_ref()
                    .map(|pending| &pending.action),
                Some(PendingAction::Quit)
            )
        {
            self.instrument_ui.pending = None;
            self.autosave_recovery();
            self.close_ready = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let metadata =
            self.instrument_ui
                .metadata_job
                .as_ref()
                .and_then(|rx| match rx.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!(
                        "Parameter worker stopped unexpectedly"
                    ))),
                    Err(mpsc::TryRecvError::Empty) => None,
                });
        if let Some(result) = metadata {
            self.instrument_ui.metadata_job = None;
            match result {
                Ok(parameters) => {
                    self.instrument_ui.selected =
                        parameters.iter().find(|p| !p.read_only).map(|p| p.id);
                    self.instrument_ui.parameters = parameters;
                }
                Err(error) => self.instrument_ui.error = Some(format!("{error:#}")),
            }
        }
        let valid = self
            .instrument_ui
            .owner
            .as_ref()
            .is_some_and(|(id, instrument)| {
                self.project
                    .track(*id)
                    .is_some_and(|track| &track.instrument == instrument)
            });
        if !valid
            && !self.instrument_ui.closing
            && let Some(session) = &self.instrument_ui.session
        {
            let _ = session.close();
            self.instrument_ui.closing = true;
        }
        let mut updates = Vec::new();
        let mut saved = None;
        let mut closed = editor_failed;
        let mut closed_request = None;
        if let Some(session) = &mut self.instrument_ui.session {
            for _ in 0..256 {
                let Some(event) = session.try_recv() else {
                    break;
                };
                match event {
                    EditorEvent::Ready { parameters } => {
                        self.instrument_ui.selected =
                            parameters.iter().find(|p| !p.read_only).map(|p| p.id);
                        self.instrument_ui.parameters = parameters;
                    }
                    EditorEvent::ParameterChanged { id, value } => updates.push((id, value)),
                    EditorEvent::StateSaved { request_id, state } => {
                        if valid {
                            self.instrument_ui.last_saved_request = request_id;
                        }
                        if valid && let Some(pending) = &mut self.instrument_ui.pending {
                            pending.observe_snapshot(request_id);
                        }
                        // The opaque snapshot contains every earlier gesture; keep only
                        // changes received after it as normalized overrides.
                        updates.clear();
                        saved = Some(state);
                    }
                    EditorEvent::Closed { request_id } => {
                        closed_request = Some(request_id);
                        closed = true;
                        break;
                    }
                    EditorEvent::Error { message } => {
                        self.instrument_ui.error = Some(message);
                        editor_failed = true;
                        closed = true;
                        break;
                    }
                    EditorEvent::Captured { .. } | EditorEvent::Heartbeat => {}
                }
            }
        }
        if valid {
            if let Some(state) = saved {
                let id = self.instrument_ui.owner.as_ref().unwrap().0;
                if self.project.track(id).is_some_and(|track| {
                    track.instrument_state != state || !track.instrument_parameters.is_empty()
                }) {
                    self.begin_edit();
                    let track = self.project.track_mut(id).unwrap();
                    track.instrument_state = state;
                    track.instrument_parameters.clear();
                    self.changed();
                }
            }
            self.apply_instrument_parameters(&updates);
        }
        if closed {
            if let Some(session) = self.instrument_ui.session.take() {
                std::thread::spawn(move || drop(session));
            }
            self.instrument_ui.closing = false;
        }
        if let Some(pending) = &mut self.instrument_ui.pending {
            let done = pending.completed(closed_request, editor_failed);
            let failed = editor_failed
                || !valid
                || Instant::now() >= pending.deadline
                || (self.instrument_ui.session.is_none()
                    && self.instrument_ui.opening.is_none()
                    && !done);
            if done || failed {
                let pending = self.instrument_ui.pending.take().unwrap();
                if failed {
                    if let Some(session) = self.instrument_ui.session.take() {
                        std::thread::spawn(move || drop(session));
                    }
                    self.instrument_ui.opening = None;
                    self.instrument_ui.closing = false;
                    self.autosave_recovery();
                    self.message(Err(anyhow::anyhow!("Instrument state could not be saved. The operation was cancelled; the last known session was saved to recovery. Retry after reopening the instrument.")), "");
                } else {
                    match pending.action {
                        PendingAction::Save(path) => self.finish_project_save(path),
                        PendingAction::Export(path) => self.start_export(path),
                        PendingAction::New => self.finish_new_session(),
                        PendingAction::Open {
                            project,
                            path,
                            recovery,
                        } => self.finish_open_project(*project, path, recovery),
                        PendingAction::Quit => {
                            self.autosave_recovery();
                            self.close_ready = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                }
            }
        }
        if self.instrument_ui.session.is_some()
            || self.instrument_ui.opening.is_some()
            || self.instrument_ui.metadata_job.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(25));
        }
    }

    fn apply_instrument_parameters(&mut self, updates: &[(u32, f64)]) {
        let Some((track_id, instrument)) = self.instrument_ui.owner.clone() else {
            return;
        };
        if updates.is_empty()
            || !self
                .project
                .track(track_id)
                .is_some_and(|track| track.instrument == instrument)
        {
            return;
        }
        let before = self.project.clone();
        for &(id, value) in updates {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                continue;
            }
            let track = self.project.track_mut(track_id).unwrap();
            if let Some(parameter) = track.instrument_parameters.iter_mut().find(|p| p.id == id) {
                parameter.value = value;
            } else {
                track
                    .instrument_parameters
                    .push(PluginParameterValue { id, value });
            }
            if let Some(parameter) = self
                .instrument_ui
                .parameters
                .iter_mut()
                .find(|p| p.id == id)
            {
                parameter.value = value;
            }
            self.instrument_ui.last_touched = Some(id);
            if let Some(engine) = &self.engine
                && let Err(error) = engine.set_instrument_parameter(track_id, id, value)
            {
                self.instrument_ui.error = Some(error.to_string());
            }
        }
        if self.project != before {
            self.history.commit(&before);
            self.modified = true;
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::Project;

    fn pending_app(ctx: &egui::Context, directory: &std::path::Path) -> DawApp {
        let mut project = Project::new();
        let id = project.add_midi_track("Instrument");
        let instrument = Instrument::Vst3 {
            path: "/missing/Test.vst3".into(),
            class_id: None,
        };
        project.set_instrument(id, instrument.clone()).unwrap();
        let mut app = DawApp::with_context(ctx, project, None);
        app.settings.scratch_dir = directory.join("recovery");
        app.modified = true;
        app.instrument_ui.owner = Some((id, instrument));
        app
    }

    #[test]
    fn pending_document_action_requires_its_snapshot_and_successful_close() {
        let mut save = PendingState {
            action: PendingAction::Save(PathBuf::from("session.json")),
            deadline: Instant::now() + Duration::from_secs(30),
            request_id: Some(42),
            saved: false,
        };
        save.observe_snapshot(Some(41)); // earlier manual Save state
        save.observe_snapshot(None); // unsolicited native-window snapshot
        assert!(!save.completed(None, false));
        save.observe_snapshot(Some(42));
        assert!(save.completed(None, false));
        assert!(!save.completed(None, true)); // a following helper error is not success
        save.action = PendingAction::Quit;
        assert!(!save.completed(Some(Some(41)), false));
        assert!(!save.completed(Some(None), false));
        assert!(save.completed(Some(Some(42)), false));
        assert!(!save.completed(Some(Some(42)), true));
    }

    #[test]
    fn switching_documents_discards_editor_metadata_and_pending_autolaunch() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        for load_existing in [false, true] {
            let mut app = pending_app(&ctx, directory.path());
            let (sender, receiver) = mpsc::channel();
            sender
                .send(Ok(vec![ParameterInfo {
                    id: 91,
                    name: "Old document cutoff".into(),
                    value: 0.7,
                    default: 0.5,
                    unit: String::new(),
                    steps: 0,
                    read_only: false,
                }]))
                .unwrap();
            app.instrument_ui.metadata_job = Some(receiver);
            app.instrument_ui.selected = Some(91);
            app.instrument_ui.last_touched = Some(91);
            app.instrument_ui.filter = "old document".into();
            app.instrument_ui.error = Some("old error".into());
            app.instrument_autolaunch = app.selected_track;
            app.plugin_assign_target = app.selected_track;
            if load_existing {
                // An opened document can reuse the exact same track and class IDs.
                app.finish_open_project(
                    app.project.clone(),
                    directory.path().join("other.json"),
                    false,
                );
            } else {
                app.finish_new_session();
            }
            app.poll_instrument_editor(&ctx);
            assert!(app.instrument_ui.owner.is_none());
            assert!(app.instrument_ui.metadata_job.is_none());
            assert!(app.instrument_ui.selected.is_none());
            assert!(app.instrument_ui.parameters.is_empty());
            assert!(app.instrument_ui.last_touched.is_none());
            assert!(app.instrument_ui.filter.is_empty());
            assert!(app.instrument_ui.error.is_none());
            assert!(app.instrument_autolaunch.is_none());
            assert!(app.plugin_assign_target.is_none());
        }
    }

    #[test]
    fn failed_editor_save_keeps_document_unsaved_and_writes_last_known_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = pending_app(&ctx, directory.path());
        let output = directory.path().join("session.daw.json");
        app.instrument_ui.pending = Some(PendingState {
            action: PendingAction::Save(output.clone()),
            deadline: Instant::now() - Duration::from_millis(1),
            request_id: Some(1),
            saved: false,
        });
        app.poll_instrument_editor(&ctx);
        assert!(!app.state_sync_pending());
        assert!(app.error && app.modified);
        assert!(!output.exists());
        assert_eq!(
            Project::load(app.settings.scratch_dir.join("recovery.daw.json")).unwrap(),
            app.project
        );
        assert!(!app.close_ready);
    }

    #[test]
    fn reported_editor_initialization_failure_allows_pending_quit_with_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = pending_app(&ctx, directory.path());
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Err(anyhow::anyhow!("Plug-in executable could not load")))
            .unwrap();
        app.instrument_ui.opening = Some(receiver);
        app.request_quit(&ctx);
        assert!(app.state_sync_pending());
        app.poll_instrument_editor(&ctx);
        assert!(!app.state_sync_pending());
        assert!(app.close_ready);
        assert!(!app.error);
        assert_eq!(
            app.instrument_ui.error.as_deref(),
            Some("Plug-in executable could not load")
        );
        assert_eq!(
            Project::load(app.settings.scratch_dir.join("recovery.daw.json")).unwrap(),
            app.project
        );
    }

    #[test]
    fn disconnected_editor_startup_cancels_quit_instead_of_hanging_or_losing_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = pending_app(&ctx, directory.path());
        let (sender, receiver) = mpsc::channel();
        drop(sender);
        app.instrument_ui.opening = Some(receiver);
        app.request_quit(&ctx);
        assert!(app.state_sync_pending());
        app.poll_instrument_editor(&ctx);
        assert!(!app.state_sync_pending());
        assert!(app.error);
        assert!(!app.close_ready);
        assert!(app.settings.scratch_dir.join("recovery.daw.json").exists());
    }
}

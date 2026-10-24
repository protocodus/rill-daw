use crate::{
    app::{DawApp, PopupKind},
    project_io::{DocumentRequest, LoadedProject},
    theme::*,
};
use daw_core::{
    AutomationParameter, AutomationTarget, Instrument, PluginParameterValue, TrackKind,
};
use daw_plugins::{EditorEvent, EditorSession, ParameterInfo};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EditorTarget {
    track: u64,
    effect: Option<u64>,
}

impl From<u64> for EditorTarget {
    fn from(track: u64) -> Self {
        Self {
            track,
            effect: None,
        }
    }
}

pub(crate) type EditorStateSignature =
    (EditorTarget, Instrument, Vec<u8>, Vec<PluginParameterValue>);

enum PendingAction {
    Save(PathBuf),
    Export(PathBuf, bool),
    Quit,
    New,
    Recovery,
    DocumentSnapshot {
        close: bool,
    },
    ApplySettings(crate::settings::Settings),
    Open {
        loaded: Box<LoadedProject>,
        path: PathBuf,
        recovery: bool,
    },
}

impl PendingAction {
    fn closes_editor(&self) -> bool {
        matches!(
            self,
            Self::Quit
                | Self::New
                | Self::Open { .. }
                | Self::ApplySettings(_)
                | Self::DocumentSnapshot { close: true }
        )
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
    owner: Option<(EditorTarget, Instrument)>,
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
    fn editor_plugin(&self, target: EditorTarget) -> Option<Instrument> {
        let track = self.project.track(target.track)?;
        if let Some(id) = target.effect {
            let slot = track.inserts.iter().find(|slot| slot.id == id)?;
            let daw_core::Effect::Vst3 { path, class_id } = &slot.effect;
            Some(Instrument::Vst3 {
                path: path.clone(),
                class_id: class_id.clone(),
            })
        } else {
            Some(track.instrument.clone())
        }
    }

    fn editor_state(&self, target: EditorTarget) -> Option<(&[u8], &[PluginParameterValue])> {
        let track = self.project.track(target.track)?;
        if let Some(id) = target.effect {
            let slot = track.inserts.iter().find(|slot| slot.id == id)?;
            Some((&slot.state, &slot.parameters))
        } else {
            Some((&track.instrument_state, &track.instrument_parameters))
        }
    }

    fn editor_state_mut(
        &mut self,
        target: EditorTarget,
    ) -> Option<(&mut Vec<u8>, &mut Vec<PluginParameterValue>)> {
        let track = self.project.track_mut(target.track)?;
        if let Some(id) = target.effect {
            let slot = track.inserts.iter_mut().find(|slot| slot.id == id)?;
            Some((&mut slot.state, &mut slot.parameters))
        } else {
            Some((
                &mut track.instrument_state,
                &mut track.instrument_parameters,
            ))
        }
    }

    pub(crate) fn open_effect_editor(&mut self, track: u64, effect: u64) {
        self.open_popup(PopupKind::Instrument);
        self.selected_track = Some(track);
        self.effect_editor_target = Some((track, effect));
        self.instrument_autolaunch = Some(track);
    }

    pub(crate) fn request_settings_apply(&mut self) {
        self.apply_settings_snapshot(self.setup_draft.clone());
    }

    pub(crate) fn apply_settings_snapshot(&mut self, settings: crate::settings::Settings) {
        // Appearance is independent of the audio graph and native editor state.
        // Applying it must not interrupt a playing session or close its editor.
        let mut same_audio_and_storage = self.settings.clone();
        same_audio_and_storage.appearance = settings.appearance;
        if same_audio_and_storage == settings {
            self.apply_appearance_settings(settings);
            return;
        }
        if let Err(error) = settings.validate() {
            self.message(Err(error), "");
            return;
        }
        if self.defer_document_request(DocumentRequest::Settings(settings.clone())) {
            return;
        }
        if let Some(PendingAction::ApplySettings(settings)) =
            self.defer_for_editor(PendingAction::ApplySettings(settings))
        {
            self.finish_settings_apply(settings);
        }
    }

    pub(crate) fn request_recovery_save(&mut self) {
        if self.modified
            && !self.project_io_pending()
            && !self.state_sync_pending()
            && let Some(PendingAction::Recovery) = self.defer_for_editor(PendingAction::Recovery)
        {
            self.autosave_recovery();
        }
    }

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
        let (target, _) = self.instrument_ui.owner.as_ref()?;
        let (state, parameters) = self.editor_state(*target)?;
        Some((
            *target,
            self.editor_plugin(*target)?,
            state.to_vec(),
            parameters.to_vec(),
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
                    Err(anyhow::anyhow!("Plugin state save cancelled by Undo/Redo")),
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
            "track": self.instrument_ui.owner.as_ref().map(|(target, _)| target.track),
            "effect": self.instrument_ui.owner.as_ref().and_then(|(target, _)| target.effect),
            "pid": self.instrument_ui.session.as_ref().map(EditorSession::process_id),
            "selected_value": self.instrument_ui.selected.and_then(|id| self.instrument_ui.parameters.iter().find(|parameter| parameter.id == id)).map(|parameter| parameter.value),
        })
    }

    pub(crate) fn state_sync_pending(&self) -> bool {
        self.instrument_ui.pending.is_some()
    }

    fn defer_for_editor(&mut self, action: PendingAction) -> Option<PendingAction> {
        if self.instrument_ui.pending.is_some() {
            self.message(
                Err(anyhow::anyhow!(
                    "Wait for the plugin state to finish saving"
                )),
                "",
            );
            return None;
        }
        if self
            .instrument_ui
            .owner
            .as_ref()
            .is_some_and(|(target, plugin)| self.editor_plugin(*target).as_ref() != Some(plugin))
        {
            // Assignment/deletion already discarded this owner's state. Its
            // delayed startup or close must not hold the current document's
            // Save/Quit barrier or write a snapshot into the replacement class.
            // Retire the receiver too: it may already contain a native session
            // whose destructor must run away from the UI thread.
            let obsolete = std::mem::take(&mut self.instrument_ui);
            std::thread::spawn(move || drop(obsolete));
            if matches!(action, PendingAction::Quit) {
                self.instrument_autolaunch = None;
            }
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
        self.status = "Saving plugin state…".into();
        self.error = false;
        None
    }

    pub(crate) fn request_document_snapshot(&mut self, close: bool) {
        if let Some(PendingAction::DocumentSnapshot { .. }) =
            self.defer_for_editor(PendingAction::DocumentSnapshot { close })
        {
            self.document_snapshot_ready();
        } else if !self.state_sync_pending() {
            self.document_snapshot_failed(self.status.clone());
        }
    }

    pub(crate) fn new_session(&mut self) {
        if self.defer_document_request(DocumentRequest::New) {
            return;
        }
        if let Some(PendingAction::New) = self.defer_for_editor(PendingAction::New) {
            self.finish_new_session();
        }
    }

    pub(crate) fn request_open_project(&mut self, path: PathBuf, recovery: bool) {
        if self.defer_document_request(DocumentRequest::Open(path.clone(), recovery)) {
            return;
        }
        self.start_project_load(path, recovery);
    }

    pub(crate) fn accept_loaded_project(
        &mut self,
        loaded: LoadedProject,
        path: PathBuf,
        recovery: bool,
    ) {
        if let Some(PendingAction::Open {
            loaded,
            path,
            recovery,
        }) = self.defer_for_editor(PendingAction::Open {
            loaded: Box::new(loaded),
            path,
            recovery,
        }) {
            self.finish_open_project(*loaded, path, recovery);
        }
    }

    pub(crate) fn save_to(&mut self, path: PathBuf) {
        if self.defer_document_request(DocumentRequest::Save(path.clone())) {
            return;
        }
        if let Some(PendingAction::Save(path)) = self.defer_for_editor(PendingAction::Save(path)) {
            self.finish_project_save(path);
        }
    }

    pub(crate) fn export_to(&mut self, path: PathBuf) {
        self.export_to_with_tails(path, true);
    }

    pub(crate) fn export_to_with_tails(&mut self, path: PathBuf, include_effect_tails: bool) {
        if self.defer_document_request(DocumentRequest::Export(path.clone(), include_effect_tails))
        {
            return;
        }
        if self.export_job.is_some() {
            self.message(Err(anyhow::anyhow!("An export is already running")), "");
            return;
        }
        if let Some(PendingAction::Export(path, include_effect_tails)) =
            self.defer_for_editor(PendingAction::Export(path, include_effect_tails))
        {
            self.start_export(path, include_effect_tails);
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
        if self.defer_document_request(DocumentRequest::Quit) {
            return;
        }
        self.error = false;
        if let Some(PendingAction::Quit) = self.defer_for_editor(PendingAction::Quit) {
            self.autosave_recovery();
            self.retire_engine();
            self.close_ready = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    pub(crate) fn plugin_instrument_editor(&mut self, ctx: &egui::Context) {
        if !self.instrument_open {
            return;
        }
        let target = if let Some((track, effect)) = self.effect_editor_target {
            EditorTarget {
                track,
                effect: Some(effect),
            }
        } else if let Some(track) = self.selected_track.filter(|id| {
            self.project
                .track(*id)
                .is_some_and(|track| track.kind == TrackKind::Midi)
        }) {
            EditorTarget::from(track)
        } else {
            self.instrument_open = false;
            return;
        };
        let Some(instrument) = self.editor_plugin(target) else {
            self.instrument_open = false;
            return;
        };
        let track_id = target.track;
        let track = self.project.track(track_id).unwrap();
        let name = track.name.clone();
        let context_name = target
            .effect
            .and_then(|effect| track.inserts.iter().find(|slot| slot.id == effect))
            .map(|slot| format!("{name} · {}", slot.name))
            .unwrap_or_else(|| name.clone());
        let title = if target.effect.is_some() {
            "Effect"
        } else {
            "Instrument"
        };
        let owner = (target, instrument.clone());
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
        daw_ui::popup(title, ctx)
            .id(egui::Id::new("instrument_editor"))
            .open(&mut open)
            .default_width(600.0)
            // The parameter list owns scrolling. An outer window scroller
            // would hide actions when asynchronous metadata grows the body.
            .vscroll(false)
            .show(ctx, |ui| {
                daw_ui::text::truncated(ui, daw_ui::text::TextRole::Subtitle, &context_name);
                match &instrument {
                    Instrument::None => {
                        ui.label("No instrument assigned");
                        choose = daw_ui::primary_button(ui, "Choose instrument").clicked();
                    }
                    Instrument::Vst3 { path, .. } => {
                        daw_ui::text::bounded_block(
                            ui,
                            daw_ui::text::TextRole::Label,
                            PathBuf::from(path)
                                .file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string(),
                            2,
                        );
                        ui.horizontal_wrapped(|ui| {
                            launch = ui
                                .add_enabled(
                                    self.instrument_ui.session.is_none()
                                        && self.instrument_ui.opening.is_none(),
                                    daw_ui::button_widget(
                                        if self.instrument_ui.opening.is_some() {
                                            "Opening editor…"
                                        } else {
                                            "Open editor"
                                        },
                                    ),
                                )
                                .clicked();
                            if target.effect.is_none() {
                                choose = daw_ui::button(ui, "Change instrument").clicked();
                            }
                            if let Some(session) = &self.instrument_ui.session
                                && daw_ui::button(ui, "Close editor").clicked()
                            {
                                let _ = session.close();
                                self.instrument_ui.closing = true;
                            }
                        });
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            let search_width = (ui.available_width() - 190.0).clamp(80.0, 300.0);
                            daw_ui::text_field(
                                ui,
                                &mut self.instrument_ui.filter,
                                "Search parameters",
                                search_width,
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
                        if let Some(parameter) = self.instrument_ui.selected.and_then(|id| {
                            self.instrument_ui.parameters.iter().find(|p| p.id == id)
                        }) {
                            let mut value = parameter.value;
                            let response = ui.add_enabled(
                                self.parameter_editing_enabled(),
                                daw_ui::slider(&mut value, 0.0..=1.0).text(&parameter.name),
                            );
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
                        if let Some(parameter) = self.instrument_ui.last_touched.and_then(|id| {
                            self.instrument_ui.parameters.iter().find(|p| p.id == id)
                        }) && daw_ui::button(
                            ui,
                            format!("Automate last touched · {}", parameter.name),
                        )
                        .clicked()
                        {
                            automate =
                                Some((parameter.id, parameter.value, parameter.name.clone()));
                        }
                        if let Some(error) = &self.instrument_ui.error {
                            ui.colored_label(palette(ui.ctx()).orange, error);
                        }
                        // Keep the current value and automation commands visible;
                        // only the potentially thousands of parameter names scroll.
                        let filter = self.instrument_ui.filter.to_lowercase();
                        egui::ScrollArea::vertical()
                            .id_salt("instrument_parameters")
                            .min_scrolled_height(daw_ui::tokens::CONTROL_HEIGHT)
                            .max_height(
                                ui.available_height()
                                    .clamp(daw_ui::tokens::CONTROL_HEIGHT, 260.0),
                            )
                            .show(ui, |ui| {
                                for parameter in self.instrument_ui.parameters.iter().filter(|p| {
                                    !p.read_only && p.name.to_lowercase().contains(&filter)
                                }) {
                                    daw_ui::selectable_value(
                                        ui,
                                        &mut self.instrument_ui.selected,
                                        Some(parameter.id),
                                        &parameter.name,
                                    );
                                }
                            });
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
        if (launch || fetch)
            && let Instrument::Vst3 { path, class_id } = &instrument
        {
            let path = PathBuf::from(path);
            let class_id = class_id.clone();
            let (state, parameters) = self.editor_state(target).unwrap();
            let state = state.to_vec();
            let parameters: Vec<_> = parameters.iter().map(|p| (p.id, p.value)).collect();
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
                        if target.effect.is_some() {
                            let mut plugin = daw_plugins::VstEffect::load_class(
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
                        } else {
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
                        }
                    })();
                    let _ = tx.send(result);
                });
            }
        }
        if let Some((id, value)) = update
            && self.parameter_editing_enabled()
        {
            if let Some(session) = &self.instrument_ui.session
                && let Err(error) = session.set_parameter(id, value)
            {
                self.instrument_ui.error = Some(error.to_string());
            } else {
                self.apply_instrument_parameters(&[(id, value)]);
            }
        }
        if let Some((id, value, label)) = automate {
            self.begin_edit();
            match self.project.add_automation_track_with_value(
                format!("{name} · {label}"),
                AutomationTarget {
                    track: track_id,
                    parameter: AutomationParameter::VstParameter {
                        effect: target.effect,
                        id,
                    },
                },
                value as f32,
            ) {
                Ok(id) => {
                    self.open_popup(PopupKind::Automation);
                    self.selected_track = Some(id);
                    self.selected_clip = None;
                    self.changed();
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
            self.retire_engine();
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
            .is_some_and(|(target, instrument)| {
                self.editor_plugin(*target).as_ref() == Some(instrument)
            });
        if !valid
            && !self.instrument_ui.closing
            && let Some(session) = &self.instrument_ui.session
        {
            let _ = session.close();
            self.instrument_ui.closing = true;
        }
        let mut updates = Vec::new();
        let mut closed = editor_failed;
        let mut closed_request = None;
        let mut events = Vec::new();
        if let Some(session) = &mut self.instrument_ui.session {
            for _ in 0..256 {
                let Some(event) = session.try_recv() else {
                    break;
                };
                events.push(event);
            }
        }
        for event in events {
            match event {
                EditorEvent::Ready { parameters } => {
                    if !parameters
                        .iter()
                        .any(|p| Some(p.id) == self.instrument_ui.selected && !p.read_only)
                    {
                        self.instrument_ui.selected =
                            parameters.iter().find(|p| !p.read_only).map(|p| p.id);
                    }
                    self.instrument_ui.parameters = parameters;
                }
                EditorEvent::ParameterChanged { id, value } => {
                    if value.is_finite() && (0.0..=1.0).contains(&value) {
                        // A following snapshot may subsume the value override,
                        // but must not discard the identity of the last gesture.
                        self.instrument_ui.last_touched = Some(id);
                        if let Some(parameter) = self
                            .instrument_ui
                            .parameters
                            .iter_mut()
                            .find(|p| p.id == id)
                        {
                            parameter.value = value;
                        }
                        updates.push((id, value));
                    }
                }
                EditorEvent::StateSaved { request_id, state } => {
                    if state.len() > daw_core::MAX_PLUGIN_STATE_BYTES {
                        self.instrument_ui.error =
                            Some("Plugin state exceeds the supported size".into());
                        editor_failed = true;
                        closed = true;
                        break;
                    }
                    if valid {
                        self.instrument_ui.last_saved_request = request_id;
                    }
                    if valid && let Some(pending) = &mut self.instrument_ui.pending {
                        pending.observe_snapshot(request_id);
                    }
                    // Commit preceding gestures first so consolidation cannot erase
                    // their Undo boundary when events arrive in one UI frame.
                    if valid {
                        self.apply_instrument_parameters(&updates);
                        self.apply_editor_snapshot(state);
                    }
                    updates.clear();
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
        if valid {
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
                    self.message(Err(anyhow::anyhow!("Plugin state could not be saved. The operation was cancelled; the last known session was saved to recovery. Retry after reopening the plugin.")), "");
                    if matches!(pending.action, PendingAction::DocumentSnapshot { .. }) {
                        self.document_snapshot_failed(self.status.clone());
                    }
                } else {
                    match pending.action {
                        PendingAction::Save(path) => self.finish_project_save(path),
                        PendingAction::Export(path, include_effect_tails) => {
                            self.start_export(path, include_effect_tails)
                        }
                        PendingAction::New => self.finish_new_session(),
                        PendingAction::Recovery => self.autosave_recovery(),
                        PendingAction::DocumentSnapshot { .. } => self.document_snapshot_ready(),
                        PendingAction::ApplySettings(settings) => {
                            self.finish_settings_apply(settings)
                        }
                        PendingAction::Open {
                            loaded,
                            path,
                            recovery,
                        } => self.finish_open_project(*loaded, path, recovery),
                        PendingAction::Quit => {
                            self.autosave_recovery();
                            self.retire_engine();
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

    fn parameter_editing_enabled(&self) -> bool {
        self.instrument_ui.opening.is_none()
            && !self.instrument_ui.closing
            && !self.state_sync_pending()
    }

    fn apply_editor_snapshot(&mut self, state: Vec<u8>) {
        let Some((target, _)) = self.instrument_ui.owner.as_ref() else {
            return;
        };
        let target = *target;
        let Some((previous, overrides)) = self.editor_state(target) else {
            return;
        };
        if previous == state && overrides.is_empty() {
            return;
        }
        let opaque_edit = !previous.is_empty() && overrides.is_empty();
        // A native event can arrive while an application drag is still pending.
        // Seal that earlier edit before inserting a later native-history entry.
        self.flush_pending_edit();
        // Parameter gestures already own an Undo entry. Encoding those values in
        // an opaque blob is persistence bookkeeping, as is the initial capture.
        // Preset changes without parameter callbacks retain a distinct Undo step.
        if opaque_edit {
            self.history
                .commit_named(&self.project, "Change plug-in preset");
        }
        let (saved, overrides) = self.editor_state_mut(target).unwrap();
        *saved = state;
        overrides.clear();
        // A first capture or representation consolidation has no extra Undo
        // step, but it still changes the document that must be persisted.
        if self.history.is_saved() {
            self.history.mark_unsaved();
        }
        self.changed();
    }

    fn apply_instrument_parameters(&mut self, updates: &[(u32, f64)]) {
        let Some((target, instrument)) = self.instrument_ui.owner.clone() else {
            return;
        };
        if updates.is_empty() || self.editor_plugin(target).as_ref() != Some(&instrument) {
            return;
        }
        self.flush_pending_edit();
        let before = self.project.clone();
        for &(id, value) in updates {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                continue;
            }
            let (_, parameters) = self.editor_state_mut(target).unwrap();
            if let Some(parameter) = parameters.iter_mut().find(|p| p.id == id) {
                parameter.value = value;
            } else {
                parameters.push(PluginParameterValue { id, value });
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
                && let Err(error) =
                    engine.set_plugin_parameter(target.track, target.effect, id, value)
            {
                self.instrument_ui.error = Some(error.to_string());
            }
        }
        if self.project != before {
            let label = if target.effect.is_some() {
                "Effect parameter"
            } else {
                "Instrument parameter"
            };
            let mut ids: Vec<_> = updates.iter().map(|(id, _)| *id).collect();
            ids.sort_unstable();
            ids.dedup();
            self.history.commit_grouped(
                &before,
                label,
                format!("plugin:{}:{:?}:{ids:?}", target.track, target.effect),
            );
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
            path: directory
                .join("missing/Test.vst3")
                .to_string_lossy()
                .into_owned(),
            class_id: None,
        };
        project.set_instrument(id, instrument.clone()).unwrap();
        let mut app = DawApp::with_context(ctx, project, None);
        app.settings.scratch_dir = directory.join("recovery");
        app.modified = true;
        app.instrument_ui.owner = Some((id.into(), instrument));
        app
    }

    #[test]
    fn late_parameter_metadata_keeps_automation_actions_visible_and_clickable() {
        for (size, appearance, last_touched) in [
            (
                egui::vec2(1100.0, 720.0),
                daw_ui::theme::Appearance::Dark,
                false,
            ),
            (
                egui::vec2(1100.0, 720.0),
                daw_ui::theme::Appearance::Light,
                true,
            ),
            (
                egui::vec2(1600.0, 1200.0),
                daw_ui::theme::Appearance::Dark,
                true,
            ),
            (
                egui::vec2(1600.0, 1200.0),
                daw_ui::theme::Appearance::Light,
                false,
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let ctx = egui::Context::default();
            let mut app = pending_app(&ctx, directory.path());
            daw_ui::theme::set_appearance(&ctx, appearance);
            app.instrument_open = true;
            let draw = |app: &mut DawApp, events| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |ctx| app.plugin_instrument_editor(ctx),
                )
            };
            // Real editors open before their asynchronous metadata arrives.
            for _ in 0..3 {
                draw(&mut app, vec![]);
            }
            app.instrument_ui.parameters = (0..2855)
                .map(|id| ParameterInfo {
                    id,
                    name: format!("Parameter {id}"),
                    value: 0.5,
                    default: 0.5,
                    unit: String::new(),
                    steps: 0,
                    read_only: false,
                })
                .collect();
            app.instrument_ui.selected = Some(100);
            app.instrument_ui.last_touched = Some(101);
            let mut output = draw(&mut app, vec![]);
            for _ in 0..3 {
                output = draw(&mut app, vec![]);
            }
            let label = if last_touched {
                "Automate last touched · Parameter 101"
            } else {
                "Automate parameter"
            };
            let rect = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        let rect = text.galley.rect.translate(text.pos.to_vec2());
                        shape.clip_rect.contains_rect(rect).then_some(rect)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| {
                    panic!(
                        "{label} must remain fully visible after2855 parameters arrive at {size:?}"
                    )
                });
            assert!(
                egui::Rect::from_min_size(egui::pos2(0.0, 104.0), size - egui::vec2(0.0, 104.0))
                    .contains_rect(rect)
            );
            for pressed in [true, false] {
                draw(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(rect.center()),
                        egui::Event::PointerButton {
                            pos: rect.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert_eq!(app.project.tracks.len(), 2);
            assert_eq!(
                app.project.tracks[1]
                    .automation
                    .as_ref()
                    .unwrap()
                    .target
                    .parameter,
                AutomationParameter::VstParameter {
                    effect: None,
                    id: if last_touched { 101 } else { 100 }
                }
            );
            assert!(app.automation_open);
        }
    }

    #[test]
    fn automatic_snapshots_preserve_parameter_undo_without_extra_steps() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = pending_app(&ctx, directory.path());
        let target = app.instrument_ui.owner.as_ref().unwrap().0;
        app.apply_editor_snapshot(vec![1]);
        assert!(
            !app.history.can_undo(),
            "Opening an unchanged editor is not an edit"
        );
        app.edit_dirty = false;
        app.apply_instrument_parameters(&[(7, 0.8)]);
        app.apply_editor_snapshot(vec![2]);
        assert!(app.history.undo(&mut app.project));
        let (state, parameters) = app.editor_state(target).unwrap();
        assert_eq!(state, &[1]);
        assert!(parameters.is_empty());
        assert!(
            !app.history.can_undo(),
            "Encoding a parameter edit must not add an Undo"
        );
        assert!(app.history.redo(&mut app.project));
        assert_eq!(app.editor_state(target).unwrap().0, &[2]);
        app.apply_editor_snapshot(vec![3]);
        assert!(app.history.undo(&mut app.project));
        assert_eq!(
            app.editor_state(target).unwrap().0,
            &[2],
            "Opaque preset edits remain undoable"
        );
    }

    #[test]
    fn parameter_controls_wait_for_start_close_and_state_barriers() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = pending_app(&ctx, directory.path());
        assert!(app.parameter_editing_enabled());
        let (_sender, receiver) = mpsc::channel();
        app.instrument_ui.opening = Some(receiver);
        assert!(!app.parameter_editing_enabled());
        app.instrument_ui.opening = None;
        app.instrument_ui.closing = true;
        assert!(!app.parameter_editing_enabled());
        app.instrument_ui.closing = false;
        app.instrument_ui.pending = Some(PendingState {
            action: PendingAction::Recovery,
            deadline: Instant::now() + Duration::from_secs(30),
            request_id: Some(1),
            saved: false,
        });
        assert!(!app.parameter_editing_enabled());
        app.instrument_ui.pending = None;
        assert!(app.parameter_editing_enabled());
    }

    #[test]
    fn native_parameter_change_follows_an_unflushed_ui_edit_in_history() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = pending_app(&ctx, directory.path());
        let target = app.instrument_ui.owner.as_ref().unwrap().0;
        app.apply_editor_snapshot(vec![1]);
        app.flush_pending_edit();
        let original = app.project.clone();
        app.begin_edit();
        app.project.tracks[0].name = "Edited channel".into();
        app.changed();
        let renamed = app.project.clone();
        app.apply_instrument_parameters(&[(7, 0.8)]);
        app.flush_pending_edit();
        assert!(app.history.undo(&mut app.project));
        assert_eq!(
            app.project, renamed,
            "Undo native gesture must retain the preceding UI edit"
        );
        assert!(app.editor_state(target).unwrap().1.is_empty());
        assert!(app.history.undo(&mut app.project));
        assert_eq!(app.project, original);
        assert!(app.history.redo(&mut app.project));
        assert_eq!(app.project, renamed);
        assert!(app.history.redo(&mut app.project));
        assert_eq!(app.editor_state(target).unwrap().1[0].value, 0.8);
    }

    #[test]
    fn opaque_preset_follows_an_unflushed_ui_edit_without_duplicate_steps() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = pending_app(&ctx, directory.path());
        app.apply_editor_snapshot(vec![1]);
        app.flush_pending_edit();
        let original = app.project.clone();
        app.begin_edit();
        app.project.tracks[0].gain = 0.6;
        app.changed();
        let fader = app.project.clone();
        app.apply_editor_snapshot(vec![2]);
        app.flush_pending_edit();
        assert_eq!(app.history.undo_labels().count(), 2);
        assert!(app.history.undo(&mut app.project));
        assert_eq!(
            app.project, fader,
            "Undo preset must retain the preceding fader move"
        );
        assert!(app.history.undo(&mut app.project));
        assert_eq!(app.project, original);
    }

    #[test]
    fn first_native_capture_stays_dirty_without_an_artificial_undo_step() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = pending_app(&ctx, directory.path());
        app.modified = false;
        assert!(app.history.is_saved());
        app.apply_editor_snapshot(vec![1]);
        app.flush_pending_edit();
        assert!(app.modified);
        assert!(!app.history.is_saved());
        assert!(!app.history.can_undo());
        app.history.mark_saved_at(app.history.state_id());
        app.modified = false;
        app.apply_editor_snapshot(vec![1]);
        app.flush_pending_edit();
        assert!(
            !app.modified,
            "An identical snapshot must not dirty a saved document"
        );
        assert!(app.history.is_saved());
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
                    app.project.clone().into(),
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
    fn quitting_after_reassignment_does_not_wait_for_the_obsolete_editor_startup() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = pending_app(&ctx, directory.path());
        let (sender, receiver) = mpsc::channel();
        app.instrument_ui.opening = Some(receiver);
        let track = app.instrument_ui.owner.as_ref().unwrap().0.track;
        let Instrument::Vst3 { path, .. } = app.project.track(track).unwrap().instrument.clone()
        else {
            unreachable!()
        };
        app.project
            .set_instrument(
                track,
                Instrument::Vst3 {
                    path,
                    class_id: Some("22222222222222222222222222222222".into()),
                },
            )
            .unwrap();
        app.instrument_autolaunch = Some(track);
        app.request_quit(&ctx);
        assert!(
            app.close_ready,
            "The replaced class cannot save into the new class"
        );
        assert!(!app.state_sync_pending());
        assert!(app.instrument_ui.owner.is_none());
        assert!(app.instrument_ui.opening.is_none());
        assert!(app.instrument_autolaunch.is_none());
        // A late startup outcome belongs to the abandoned owner and cannot
        // cancel Quit or overwrite the current assignment's recovery snapshot.
        let _ = sender.send(Err(anyhow::anyhow!("Obsolete class failed to load")));
        app.poll_instrument_editor(&ctx);
        assert!(!app.error);
        assert!(app.close_ready);
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

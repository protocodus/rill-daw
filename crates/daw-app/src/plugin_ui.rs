use crate::{
    app::{ChannelDestination, DawApp, PopupKind},
    project_io::{DocumentRequest, LoadedProject},
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
    ApplySettings(Box<crate::settings::Settings>),
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

// These explicit startup errors are emitted before Ready: the helper could not
// create a native view, so it never accepted an editor gesture or owned new
// state. Match complete diagnostic lines, never a later running-session error.
fn native_view_unavailable(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.to_string().lines().any(|line| {
            matches!(
                line,
                "This plug-in has no native editor" | "Failed to create editor view"
            )
        })
    })
}

#[derive(Default)]
pub(crate) struct InstrumentUi {
    owner: Option<(EditorTarget, Instrument)>,
    session: Option<EditorSession>,
    opening: Option<mpsc::Receiver<anyhow::Result<EditorSession>>>,
    metadata_job: Option<mpsc::Receiver<anyhow::Result<Vec<ParameterInfo>>>>,
    parameters: Vec<ParameterInfo>,
    browser: daw_ui::ParameterBrowserState,
    parameter_control: Option<daw_ui::KnobGeometry>,
    parameter_control_clip: Option<egui::Rect>,
    last_touched: Option<u32>,
    browse_target: Option<EditorTarget>,
    parameters_open: bool,
    closing: bool,
    last_saved_request: Option<u64>,
    error: Option<String>,
    pending: Option<PendingState>,
}

#[derive(Default)]
struct ParameterActions {
    refresh: bool,
    update: Option<(u32, f64)>,
    automate: Option<(u32, f64)>,
    resume: Option<u64>,
}

impl DawApp {
    fn plugin_error_details_button(&mut self, ui: &mut egui::Ui, target: EditorTarget, name: &str) {
        if daw_ui::button(ui, "Details").clicked() {
            let Some(error) = self.instrument_ui.error.clone() else {
                return;
            };
            let channel = self
                .project
                .track(target.track)
                .map(|track| track.name.as_str())
                .unwrap_or("Unknown channel");
            let plugin = match self.editor_plugin(target) {
                Some(Instrument::Vst3 { path, class_id }) => format!(
                    "\nPlugin: {path}\nClass ID: {}",
                    class_id.as_deref().unwrap_or("Default")
                ),
                _ => String::new(),
            };
            self.show_error_details(
                format!("Error · {name}"),
                error.lines().next().unwrap_or(&error).to_owned(),
                format!(
                    "{error}\n\nDevice: {name}\nChannel: {channel} (#{}){plugin}",
                    target.track
                ),
            );
        }
    }

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

    pub(crate) fn open_effect_automation(&mut self, track: u64, effect: u64) {
        self.open_popup(PopupKind::Instrument);
        self.selected_track = Some(track);
        self.effect_editor_target = Some((track, effect));
        self.instrument_autolaunch = None;
        self.instrument_ui.browse_target = Some(EditorTarget {
            track,
            effect: Some(effect),
        });
    }

    pub(crate) fn request_settings_apply(&mut self) {
        self.apply_settings_snapshot(self.setup_draft.clone());
    }

    pub(crate) fn apply_settings_snapshot(&mut self, settings: crate::settings::Settings) {
        // Appearance is independent of the audio graph and native editor state.
        // Applying it must not interrupt a playing session or close its editor.
        let mut same_audio_and_storage = self.settings.clone();
        same_audio_and_storage.appearance = settings.appearance;
        same_audio_and_storage.ai = settings.ai.clone();
        if same_audio_and_storage == settings {
            self.apply_appearance_settings(settings);
            return;
        }
        if let Err(error) = settings.validate() {
            self.message(Err(error), "");
            return;
        }
        if self.defer_document_request(DocumentRequest::Settings(Box::new(settings.clone()))) {
            return;
        }
        if let Some(PendingAction::ApplySettings(settings)) =
            self.defer_for_editor(PendingAction::ApplySettings(Box::new(settings)))
        {
            self.finish_settings_apply(*settings);
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
        let rect = |rect: egui::Rect| {
            serde_json::json!({
                "min": [rect.min.x, rect.min.y], "max": [rect.max.x, rect.max.y]
            })
        };
        let selected = self.instrument_ui.browser.selected.and_then(|id| {
            self.instrument_ui
                .parameters
                .iter()
                .find(|parameter| parameter.id == id)
        });
        let automation = selected.and_then(|parameter| {
            let target = self.instrument_ui.owner.as_ref()?.0;
            self.automation_control_state(AutomationTarget {
                track: target.track,
                parameter: AutomationParameter::VstParameter {
                    effect: target.effect,
                    id: parameter.id,
                },
            })
        });
        serde_json::json!({
            "open": self.instrument_ui.session.is_some(),
            "opening": self.instrument_ui.opening.is_some(),
            "closing": self.instrument_ui.closing,
            "parameter_count": self.instrument_ui.parameters.len(),
            "parameters_open": self.instrument_ui.parameters_open,
            "selected": self.instrument_ui.browser.selected,
            "selected_steps": selected.map(|parameter| parameter.steps),
            "parameter_control": self.instrument_ui.parameter_control.as_ref().map(|geometry| serde_json::json!({
                "dial":rect(geometry.dial),"number":rect(geometry.number),
                "clip":self.instrument_ui.parameter_control_clip.map(rect),
                "center":[geometry.center.x,geometry.center.y],
                "minimum":[geometry.minimum.x,geometry.minimum.y],
                "maximum":[geometry.maximum.x,geometry.maximum.y],
                "index":[geometry.index.x,geometry.index.y],
                "range":geometry.range,"value":geometry.value,"divisions":geometry.divisions,
            })),
            "parameter_query": self.instrument_ui.browser.query,
            "last_touched": self.instrument_ui.last_touched,
            "error": self.instrument_ui.error,
            "track": self.instrument_ui.owner.as_ref().map(|(target, _)| target.track),
            "effect": self.instrument_ui.owner.as_ref().and_then(|(target, _)| target.effect),
            "pid": self.instrument_ui.session.as_ref().map(EditorSession::process_id),
            "selected_value": selected.map(|parameter| automation
                .filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                .map_or(parameter.value, |(_, _, value)| f64::from(value))),
            "automation_state": automation.map(|(_, state, _)| state.label()),
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

    /// Escape leaves the parameter chooser before dismissing its plugin context.
    pub(crate) fn close_instrument_parameters(&mut self) -> bool {
        if self.instrument_open && self.instrument_ui.parameters_open {
            self.instrument_ui.parameters_open = false;
            true
        } else {
            false
        }
    }

    pub(crate) fn last_touched_plugin_parameter(
        &self,
        track: u64,
        effect: Option<u64>,
    ) -> Option<u32> {
        let target = EditorTarget { track, effect };
        self.instrument_ui
            .owner
            .as_ref()
            .filter(|(owner, plugin)| {
                *owner == target && self.editor_plugin(target).as_ref() == Some(plugin)
            })
            .and(self.instrument_ui.last_touched)
    }

    fn plugin_parameter_browser(
        &mut self,
        ui: &mut egui::Ui,
        target: EditorTarget,
        device_name: &str,
    ) -> ParameterActions {
        let mut actions = ParameterActions::default();
        let loading =
            self.instrument_ui.metadata_job.is_some() || self.instrument_ui.opening.is_some();
        let can_refresh = !loading && self.instrument_ui.session.is_none();
        daw_ui::action_row(
            ui,
            |ui| {
                daw_ui::action_menu(ui, "More", |ui| {
                    if ui
                        .add_enabled(can_refresh, daw_ui::button_widget("Read parameters"))
                        .clicked()
                    {
                        actions.refresh = true;
                        ui.close();
                    }
                });
                if daw_ui::button(ui, "Back").clicked() {
                    self.instrument_ui.parameters_open = false;
                }
            },
            |ui| {
                daw_ui::text::truncated(
                    ui,
                    daw_ui::text::TextRole::Subtitle,
                    format!("Automate {device_name}"),
                );
            },
        );
        if let Some(error) = &self.instrument_ui.error {
            daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note, error, 2)
                .on_hover_text(error);
            self.plugin_error_details_button(ui, target, device_name);
            if ui
                .add_enabled(can_refresh, daw_ui::button_widget("Retry"))
                .clicked()
            {
                actions.refresh = true;
            }
        } else if loading {
            ui.horizontal(|ui| {
                ui.spinner();
                daw_ui::text::note(ui, "Loading parameters…");
            });
        }
        let editing_enabled = self.parameter_editing_enabled();
        // The parameter catalogue may contain thousands of entries. Walk the
        // automation lanes once and index only this device's lanes, rather than
        // searching every project track for each visible or hidden parameter.
        let automation_states: std::collections::HashMap<_, _> = self
            .project
            .tracks
            .iter()
            .filter_map(|track| {
                let lane = track.automation.as_ref()?;
                match lane.target.parameter {
                    AutomationParameter::VstParameter { effect, id }
                        if lane.target.track == target.track && effect == target.effect =>
                    {
                        Some((id, self.automation_lane_control_state(track.id, lane)))
                    }
                    _ => None,
                }
            })
            .collect();
        let items: Vec<_> = self
            .instrument_ui
            .parameters
            .iter()
            .filter(|p| !p.read_only)
            .map(|p| daw_ui::ParameterItem {
                id: p.id,
                name: p.name.as_str(),
                unit: p.unit.as_str(),
                value: automation_states
                    .get(&p.id)
                    .filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                    .map_or(p.value, |(_, _, value)| f64::from(*value)),
                automated: automation_states.contains_key(&p.id),
                last_touched: self.instrument_ui.last_touched == Some(p.id),
            })
            .collect();
        let metadata = &self.instrument_ui.parameters;
        let mut control_geometry = None;
        let mut control_clip = None;
        let browser = daw_ui::parameter_browser(
            ui,
            "plugin_parameter_browser",
            &mut self.instrument_ui.browser,
            &items,
            ui.available_height().max(0.0),
            |ui, selected| {
                let Some(parameter) = selected else {
                    daw_ui::text::subtitle(ui, "Choose a parameter");
                    daw_ui::text::note(ui, "Search by name or unit.");
                    return;
                };
                let steps = metadata
                    .iter()
                    .find(|item| item.id == parameter.id)
                    .map_or(0, |item| item.steps.max(0));
                daw_ui::action_row(
                    ui,
                    |ui| {
                        if daw_ui::primary_button(
                            ui,
                            if parameter.automated {
                                "Edit automation"
                            } else {
                                "Automate parameter"
                            },
                        )
                        .clicked()
                        {
                            actions.automate = Some((parameter.id, parameter.value));
                        }
                    },
                    |ui| {
                        daw_ui::text::bounded_block(
                            ui,
                            daw_ui::text::TextRole::Title,
                            parameter.name,
                            3,
                        );
                    },
                );
                // This host metadata exposes normalized values, not the VST
                // controller's physical-unit conversion. Keep percentages
                // explicit instead of presenting, for example, 0.5 as Hz.
                let mut percent = parameter.value * 100.0;
                let automated_value = automation_states
                    .get(&parameter.id)
                    .is_some_and(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed);
                // Keep the exact numeric field visible before secondary metadata.
                // The details pane scrolls; use its visible bounds, not the
                // unbounded content height, to size only the dial preview.
                let label_height = egui::WidgetText::from(
                    daw_ui::text::TextRole::Label.rich(ui.ctx(), "Normalized value"),
                )
                .into_galley(
                    ui,
                    Some(egui::TextWrapMode::Wrap),
                    ui.available_width().max(112.0),
                    egui::TextStyle::Body,
                )
                .size()
                .y;
                let control_chrome = label_height
                    + daw_ui::tokens::FONT_SMALL
                    + daw_ui::tokens::SPACE_XS
                    + daw_ui::tokens::CONTROL_HEIGHT
                    + 3.0 * ui.spacing().item_spacing.y;
                let metadata_height = if !parameter.unit.is_empty()
                    || steps > 0
                    || automation_states.contains_key(&parameter.id)
                {
                    daw_ui::tokens::CONTROL_HEIGHT + ui.spacing().item_spacing.y
                } else {
                    0.0
                };
                let preceding_height = ui.cursor().min.y - ui.min_rect().top();
                let diameter =
                    (ui.clip_rect().height() - preceding_height - control_chrome - metadata_height)
                        .clamp(48.0, 96.0);
                let mut knob = daw_ui::knob(&mut percent, 0.0..=100.0)
                    .text("Normalized value")
                    .diameter(diameter)
                    .suffix(" %")
                    .min_decimals(2)
                    .max_decimals(6)
                    .display_round_trip(move |percent| {
                        // Match the accepted normalized value, including the
                        // f32 storage of an active automation override.
                        let mut value = (percent / 100.0).clamp(0.0, 1.0);
                        if steps > 0 {
                            value = (value * f64::from(steps)).round() / f64::from(steps);
                        }
                        if automated_value {
                            value = f64::from(value as f32);
                        }
                        value * 100.0
                    })
                    .show_automation_label(false)
                    .automation(
                        automation_states
                            .get(&parameter.id)
                            .map(|(_, state, _)| *state),
                    );
                if steps > 0 {
                    knob = knob.step_by(100.0 / f64::from(steps));
                }
                let response = ui.add_enabled(editing_enabled, knob);
                let mut entered = None;
                egui::Popup::context_menu(&response)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        ui.add_enabled_ui(editing_enabled, |ui| {
                            entered = daw_ui::value_entry_named(
                                ui,
                                (target.track, target.effect, parameter.id),
                                percent,
                                0.0..=100.0,
                                "%",
                                &format!(
                                    "{} · {device_name} · {}",
                                    self.project
                                        .track(target.track)
                                        .map(|track| track.name.as_str())
                                        .unwrap_or("Channel"),
                                    parameter.name
                                ),
                            );
                        });
                    });
                daw_ui::hover_text(response.clone(), "Right-click for exact value entry");
                control_geometry = daw_ui::knob_geometry(ui.ctx(), response.id);
                control_clip = Some(ui.clip_rect());
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new("instrument_parameter_rect"),
                        control_geometry
                            .as_ref()
                            .map_or(response.rect, |geometry| geometry.dial),
                    )
                });
                if response.changed() || entered.is_some() {
                    let value = (entered.unwrap_or(percent) / 100.0).clamp(0.0, 1.0);
                    let value = if steps > 0 {
                        (value * f64::from(steps)).round() / f64::from(steps)
                    } else {
                        value
                    };
                    actions.update = Some((parameter.id, value));
                }
                ui.horizontal_wrapped(|ui| {
                    if !parameter.unit.is_empty() {
                        daw_ui::text::note(ui, format!("Plug-in unit · {}", parameter.unit))
                            .on_hover_text("This control shows normalized percentages. Use the plug-in editor for physical units.");
                    }
                    if steps > 0 {
                        daw_ui::text::note(ui, format!("{} discrete values", i64::from(steps) + 1));
                    }
                    if let Some(&(lane, state, _)) = automation_states.get(&parameter.id) {
                        daw_ui::automation_badge(ui, state);
                        match state {
                            daw_ui::AutomationState::Read => {
                                if ui.add_enabled(editing_enabled,
                                    daw_ui::button_widget("Override automation"))
                                    .on_hover_text("Hold this parameter at its current value until automation resumes")
                                    .clicked()
                                {
                                    actions.update = Some((parameter.id, parameter.value));
                                }
                            }
                            daw_ui::AutomationState::Overridden => {
                                if ui.add_enabled(editing_enabled,
                                    daw_ui::button_widget("Resume automation"))
                                    .on_hover_text("Return to the existing curve at the playback position")
                                    .clicked()
                                {
                                    actions.resume = Some(lane);
                                }
                            }
                            daw_ui::AutomationState::Bypassed => {}
                        }
                    }
                });
                if parameter.last_touched {
                    daw_ui::text::note(ui, "Last touched");
                }
                daw_ui::text::note(ui, "Enter or double-click to automate");
            },
        );
        self.instrument_ui.parameter_control = control_geometry;
        self.instrument_ui.parameter_control_clip = control_clip;
        if let Some(id) = browser.activated
            && let Some(parameter) = items.iter().find(|p| p.id == id)
        {
            actions.automate = Some((id, parameter.value));
        }
        if actions.automate.is_some() {
            // Returning from the nested lane editor should expose its Edit
            // automation action, even if the detail pane had been scrolled.
            self.instrument_ui.browser.reset_details = true;
        }
        actions
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
        let effect_name = target
            .effect
            .and_then(|effect| track.inserts.iter().find(|slot| slot.id == effect))
            .map(|slot| slot.name.clone());
        let title = effect_name
            .as_ref()
            .map(|effect| format!("{effect} of {name}"))
            .unwrap_or_else(|| format!("Instrument of {name}"));
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
                browse_target: self.instrument_ui.browse_target.take(),
                ..Default::default()
            };
        }
        let mut open = true;
        let mut choose = false;
        let mut launch = false;
        let mut fetch = false;
        let mut update = None;
        let mut automate = None;
        let mut resume = None;
        if self.instrument_ui.browse_target.take() == Some(target) {
            self.instrument_ui.parameters_open = true;
            self.instrument_ui.browser.focus_search = true;
            fetch = self.instrument_ui.parameters.is_empty()
                && self.instrument_ui.metadata_job.is_none()
                && self.instrument_ui.session.is_none()
                && self.instrument_ui.opening.is_none();
        }
        let parameter_page = self.instrument_ui.parameters_open;
        daw_ui::popup(&title, ctx)
            .id(egui::Id::new(("instrument_editor", parameter_page)))
            .open(&mut open)
            .default_width(if parameter_page { 920.0 } else { 600.0 })
            .default_height(if parameter_page { 520.0 } else { 400.0 })
            .vscroll(false)
            .show(ctx, |ui| {
                let scope = match target.effect {
                    Some(effect_id) => daw_tools::ToolScope::Effect {
                        track_id,
                        effect_id,
                    },
                    None => daw_tools::ToolScope::Instrument { track_id },
                };
                let scope_label = scope.label(&self.project);
                let (open_ai, navigating) = daw_ui::action_row(
                    ui,
                    |ui| {
                        !matches!(instrument, Instrument::None)
                            && daw_ui::contextual_ai_button(ui, &scope_label).clicked()
                    },
                    |ui| {
                        self.channel_navigation(
                            ui,
                            if target.effect.is_some() {
                                ChannelDestination::PluginEditor
                            } else {
                                ChannelDestination::Instrument
                            },
                        )
                    },
                )
                .inner;
                if navigating {
                    return;
                }
                if open_ai {
                    self.open_context_assistant(scope);
                    return;
                }
                ui.add_space((daw_ui::tokens::SPACE_LG - ui.spacing().item_spacing.y).max(0.0));
                let mut footer =
                    (!parameter_page).then(|| daw_ui::popup_footer(ui, "device_actions"));
                let mut body = |ui: &mut egui::Ui| match &instrument {
                    Instrument::None => {
                        daw_ui::text::subtitle(ui, "No instrument assigned");
                        daw_ui::text::note(
                            ui,
                            "This MIDI track is silent until you choose a VST3 instrument.",
                        );
                    }
                    Instrument::Vst3 { path, class_id } => {
                        let name_draft_id =
                            ui.id()
                                .with(("device_name", track_id, target.effect, path, class_id));
                        let stored_name = effect_name.clone().unwrap_or_else(|| {
                            self.project
                                .track(track_id)
                                .map(|track| track.instrument_name.clone())
                                .unwrap_or_default()
                        });
                        let mut device_name = effect_name
                            .clone()
                            .or_else(|| {
                                self.project
                                    .track(track_id)
                                    .map(|track| track.instrument_name.clone())
                                    .filter(|name| !name.is_empty())
                            })
                            .or_else(|| {
                                self.vst_catalog
                                    .iter()
                                    .find(|entry| {
                                        entry.plugin.path == std::path::Path::new(path)
                                            && entry.plugin.class_id == *class_id
                                    })
                                    .map(|entry| entry.plugin.name.clone())
                            })
                            .unwrap_or_else(|| {
                                PathBuf::from(path)
                                    .file_stem()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned()
                            });
                        if let Some((text, saved)) = ui
                            .ctx()
                            .data(|data| data.get_temp::<(String, String)>(name_draft_id))
                            && saved == stored_name
                        {
                            device_name = text;
                        }
                        if !self.instrument_ui.parameters_open {
                            let label = daw_ui::text::label(ui, "Device name");
                            let (keyboard, field) = daw_ui::action_row(
                                ui,
                                |ui| {
                                    daw_ui::small_button(ui, "Keyboard…")
                                        .on_hover_text("Name this device with the mouse")
                                        .clicked()
                                },
                                |ui| {
                                    daw_ui::text_field(
                                        ui,
                                        &mut device_name,
                                        "Device name",
                                        ui.available_width(),
                                    )
                                    .labelled_by(label.id)
                                },
                            )
                            .inner;
                            let reveal_keyboard =
                                keyboard && !daw_ui::name_keyboard_open(ui.ctx(), field.id);
                            if keyboard {
                                daw_ui::set_name_keyboard_open(
                                    ui.ctx(),
                                    field.id,
                                    !daw_ui::name_keyboard_open(ui.ctx(), field.id),
                                );
                            }
                            let keyboard_top = ui.next_widget_position();
                            let keyboard_changed =
                                daw_ui::name_keyboard(ui, field.id, &mut device_name);
                            if reveal_keyboard {
                                // Reveal the keyboard inside the scroll body;
                                // opening it must not resize the dialog or footer.
                                ui.scroll_to_rect(
                                    egui::Rect::from_min_size(
                                        keyboard_top,
                                        egui::vec2(
                                            ui.available_width(),
                                            daw_ui::name_keyboard_height(ui.ctx()),
                                        ),
                                    ),
                                    Some(egui::Align::BOTTOM),
                                );
                            }
                            if field.changed() || keyboard_changed {
                                self.begin_edit();
                                let track = self.project.track_mut(track_id).unwrap();
                                if let Some(effect) = target.effect {
                                    track
                                        .inserts
                                        .iter_mut()
                                        .find(|slot| slot.id == effect)
                                        .unwrap()
                                        .name = device_name.clone();
                                } else {
                                    track.instrument_name = device_name.clone();
                                }
                                self.edit_label = Some("Rename device".into());
                                self.changed();
                            }
                            let saved_name = if field.changed() || keyboard_changed {
                                device_name.clone()
                            } else {
                                stored_name.clone()
                            };
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(name_draft_id, (device_name.clone(), saved_name))
                            });
                            daw_ui::text::note(
                                ui,
                                crate::library::plugin_model(&self.vst_catalog, path, class_id),
                            );
                        } else {
                            let actions = self.plugin_parameter_browser(ui, target, &device_name);
                            fetch |= actions.refresh;
                            update = actions.update;
                            automate = actions.automate;
                            resume = actions.resume;
                        }
                        if !self.instrument_ui.parameters_open
                            && let Some(error) = &self.instrument_ui.error
                        {
                            daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note, error, 2)
                                .on_hover_text(error);
                        }
                    }
                };
                if let Some(footer) = &mut footer {
                    daw_ui::scroll_body(footer.body(), "device_overview", body);
                } else {
                    body(ui);
                }
                if let Some(footer) = footer {
                    footer.show(ui, |ui| match &instrument {
                        Instrument::None => {
                            choose = daw_ui::primary_button(ui, "Choose instrument").clicked();
                        }
                        Instrument::Vst3 { .. } => {
                            let device_name = title.clone();
                            if let Some(parameter) =
                                self.instrument_ui.last_touched.and_then(|id| {
                                    self.instrument_ui
                                        .parameters
                                        .iter()
                                        .find(|p| p.id == id && !p.read_only)
                                })
                                && daw_ui::button(
                                    ui,
                                    format!("Automate last touched · {}", parameter.name),
                                )
                                .clicked()
                            {
                                automate = Some((parameter.id, parameter.value));
                            }

                            if self.instrument_ui.error.is_some() {
                                self.plugin_error_details_button(ui, target, &device_name);
                            }
                            ui.horizontal_wrapped(|ui| {
                                launch = ui
                                    .add_enabled_ui(
                                        self.instrument_ui.session.is_none()
                                            && self.instrument_ui.opening.is_none(),
                                        |ui| {
                                            daw_ui::primary_button(
                                                ui,
                                                if self.instrument_ui.opening.is_some() {
                                                    "Opening editor…"
                                                } else {
                                                    "Open editor"
                                                },
                                            )
                                        },
                                    )
                                    .inner
                                    .clicked();
                                if let Some(session) = &self.instrument_ui.session
                                    && daw_ui::button(ui, "Close editor").clicked()
                                {
                                    let _ = session.close();
                                    self.instrument_ui.closing = true;
                                }
                                if daw_ui::button(ui, "Automate…").clicked() {
                                    self.instrument_ui.parameters_open = true;
                                    self.instrument_ui.browser.focus_search = true;
                                    fetch = self.instrument_ui.parameters.is_empty()
                                        && self.instrument_ui.metadata_job.is_none()
                                        && self.instrument_ui.session.is_none()
                                        && self.instrument_ui.opening.is_none();
                                }
                                if target.effect.is_none() {
                                    choose = daw_ui::button(ui, "Change instrument").clicked();
                                }
                            });
                        }
                    });
                }
            });
        self.instrument_open &= open;
        if !self.instrument_open {
            return;
        }
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
        if let Some((id, value)) = automate {
            self.automate_plugin_parameter(target, id, value);
        }
        if let Some(lane) = resume {
            self.resume_automation(lane);
        }
        if choose {
            self.open_instrument_chooser(true);
        }
    }

    /// Both the parameter browser and native context menu resolve against the
    /// editor owner. Changing the arranger selection cannot redirect a request.
    fn automate_plugin_parameter(
        &mut self,
        target: EditorTarget,
        id: u32,
        value: f64,
    ) -> Option<u64> {
        if !self.parameter_editing_enabled() || !value.is_finite() || !(0.0..=1.0).contains(&value)
        {
            return None;
        }
        let (owner, plugin) = self.instrument_ui.owner.as_ref()?;
        if *owner != target
            || !matches!(plugin, Instrument::Vst3 { .. })
            || self.editor_plugin(target).as_ref() != Some(plugin)
        {
            return None;
        }
        let parameter = self
            .instrument_ui
            .parameters
            .iter()
            .find(|parameter| parameter.id == id && !parameter.read_only)?;
        let source = self.project.track(target.track)?;
        let name = if let Some(effect) = target.effect {
            let effect = source.inserts.iter().find(|slot| slot.id == effect)?;
            format!("{} · {} · {}", source.name, effect.name, parameter.name)
        } else {
            format!("{} · {}", source.name, parameter.name)
        };
        let automation_target = AutomationTarget {
            track: target.track,
            parameter: AutomationParameter::VstParameter {
                effect: target.effect,
                id,
            },
        };
        let existing = self.project.tracks.iter().find_map(|track| {
            track
                .automation
                .as_ref()
                .filter(|lane| lane.target == automation_target)
                .map(|_| track.id)
        });
        let result = if let Some(id) = existing {
            Ok(id)
        } else {
            // Native parameter gestures and a new lane are separate Undo steps.
            self.flush_pending_edit();
            self.begin_edit();
            self.project
                .add_automation_track_with_value(name, automation_target, value as f32)
        };
        match result {
            Ok(lane) => {
                self.instrument_ui.browser.selected = Some(id);
                self.open_popup(PopupKind::Automation);
                self.selected_track = Some(lane);
                self.selected_clip = None;
                if existing.is_none() {
                    self.changed();
                }
                Some(lane)
            }
            Err(error) => {
                self.message(Err(error), "");
                None
            }
        }
    }

    pub(crate) fn poll_instrument_editor(&mut self, ctx: &egui::Context) {
        let mut initialization_failed = false;
        let mut view_unavailable = false;
        let mut editor_failed = false;
        let opening = self
            .instrument_ui
            .opening
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(result) => {
                    initialization_failed = result.is_err();
                    view_unavailable = result.as_ref().err().is_some_and(native_view_unavailable);
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
                    self.instrument_ui.error = Some(if view_unavailable {
                        format!(
                            "No native editor is available. Save and export remain available.\n\n{error:#}"
                        )
                    } else {
                        format!("{error:#}")
                    });
                    self.instrument_ui.closing = false;
                }
            }
        }
        // An explicitly unsupported view has no new native state to flush.
        // Continue the action that waited for startup using the project state.
        // Other startup failures retain their existing Quit-only exception;
        // running-session errors and unknown outcomes keep the recovery barrier.
        if initialization_failed
            && self.instrument_ui.session.is_none()
            && (view_unavailable
                || matches!(
                    self.instrument_ui
                        .pending
                        .as_ref()
                        .map(|pending| &pending.action),
                    Some(PendingAction::Quit)
                ))
            && let Some(pending) = self.instrument_ui.pending.take()
        {
            self.finish_editor_action(pending.action, ctx);
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
                    if !parameters
                        .iter()
                        .any(|p| !p.read_only && Some(p.id) == self.instrument_ui.browser.selected)
                    {
                        self.instrument_ui.browser.selected =
                            parameters.iter().find(|p| !p.read_only).map(|p| p.id);
                    }
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
                        .any(|p| Some(p.id) == self.instrument_ui.browser.selected && !p.read_only)
                    {
                        self.instrument_ui.browser.selected =
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
                EditorEvent::AutomateParameter { id, value } => {
                    if valid {
                        // Preserve wire order: the gesture before this action
                        // belongs to its own history entry and current state.
                        self.apply_instrument_parameters(&updates);
                        updates.clear();
                        if let Some((target, _)) = self.instrument_ui.owner.as_ref()
                            && self.automate_plugin_parameter(*target, id, value).is_some()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        }
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
                // Host setters were already recorded when the browser gesture
                // happened. A late acknowledgement must not undo a later Resume
                // or replace metadata from a newer gesture.
                EditorEvent::ParameterApplied { .. }
                | EditorEvent::Captured { .. }
                | EditorEvent::Heartbeat => {}
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
                    self.finish_editor_action(pending.action, ctx);
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

    fn finish_editor_action(&mut self, action: PendingAction, ctx: &egui::Context) {
        match action {
            PendingAction::Save(path) => self.finish_project_save(path),
            PendingAction::Export(path, include_effect_tails) => {
                self.start_export(path, include_effect_tails)
            }
            PendingAction::New => self.finish_new_session(),
            PendingAction::Recovery => self.autosave_recovery(),
            PendingAction::DocumentSnapshot { .. } => self.document_snapshot_ready(),
            PendingAction::ApplySettings(settings) => self.finish_settings_apply(*settings),
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

    fn parameter_editing_enabled(&self) -> bool {
        self.instrument_ui.opening.is_none()
            && !self.instrument_ui.closing
            && !self.state_sync_pending()
            && !self.project_io_pending()
            && self.document_guard.is_none()
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
        self.song_session.bump();
        self.assistant.known_parameters.clear();
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
        let mut automation_changed = false;
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
            let automation_target = AutomationTarget {
                track: target.track,
                parameter: AutomationParameter::VstParameter {
                    effect: target.effect,
                    id,
                },
            };
            automation_changed |=
                self.override_automated_parameter(automation_target, value as f32);
            if let Some(engine) = &self.engine {
                let overridden = self
                    .automation_control_state(automation_target)
                    .is_some_and(|(_, state, _)| state == daw_ui::AutomationState::Overridden);
                let result = if overridden {
                    engine.set_plugin_parameter_override(target.track, target.effect, id, value)
                } else {
                    engine.set_plugin_parameter(target.track, target.effect, id, value)
                };
                if let Err(error) = result {
                    self.instrument_ui.error = Some(error.to_string());
                }
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
            self.song_session.bump();
            self.modified = true;
            self.revision = self.revision.wrapping_add(1);
            if automation_changed {
                // Native editor gestures use grouped history rather than
                // changed(); publish their lane takeover through the same live
                // project path so automation cannot reclaim the value.
                self.update_playback();
            }
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

    fn automation_parameter(id: u32, read_only: bool) -> ParameterInfo {
        ParameterInfo {
            id,
            name: "Cutoff Ω".into(),
            value: 0.4,
            default: 0.5,
            unit: "Hz".into(),
            steps: 0,
            read_only,
        }
    }

    #[test]
    fn native_automation_uses_owner_zero_id_and_reuses_without_history_or_value_changes() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = pending_app(&ctx, directory.path());
        let target = app.instrument_ui.owner.as_ref().unwrap().0;
        app.instrument_ui.parameters = vec![automation_parameter(0, false)];
        app.selected_track = Some(app.project.add_midi_track("Other selection"));
        app.apply_instrument_parameters(&[(0, 0.73)]);
        let lane = app.automate_plugin_parameter(target, 0, 0.73).unwrap();
        app.flush_pending_edit();
        let automation = app
            .project
            .track(lane)
            .unwrap()
            .automation
            .as_ref()
            .unwrap();
        assert_eq!(automation.target.track, target.track);
        assert_eq!(
            automation.target.parameter,
            AutomationParameter::VstParameter {
                effect: None,
                id: 0
            }
        );
        assert!((automation.points[0].value - 0.73).abs() < 1e-6);
        assert_eq!(
            app.project.track(lane).unwrap().name,
            "Instrument · Cutoff Ω"
        );
        let original = app.project.clone();
        assert_eq!(app.automate_plugin_parameter(target, 0, 0.2), Some(lane));
        assert_eq!(
            app.project, original,
            "opening a lane must preserve its curve"
        );
        app.flush_pending_edit();
        app.undo_redo(false);
        assert!(
            app.project.track(lane).is_none(),
            "reopening adds no Undo step"
        );
        assert_eq!(app.editor_state(target).unwrap().1[0].value, 0.73);
        app.undo_redo(true);
        assert_eq!(app.project, original);
    }

    #[test]
    fn native_effect_automation_uses_slot_identity_and_channel_context() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = pending_app(&ctx, directory.path());
        let track = app.project.tracks[0].id;
        let mut slots = vec![];
        for name in ["Echo A", "Echo B"] {
            let slot = daw_core::EffectSlot::new(
                name,
                daw_core::Effect::Vst3 {
                    path: "effect.vst3".into(),
                    class_id: None,
                },
            );
            slots.push(slot.id);
            app.project.track_mut(track).unwrap().inserts.push(slot);
        }
        app.instrument_ui.parameters = vec![automation_parameter(0, false)];
        for (index, effect) in slots.into_iter().enumerate() {
            let target = EditorTarget {
                track,
                effect: Some(effect),
            };
            app.instrument_ui.owner = Some((target, app.editor_plugin(target).unwrap()));
            let lane = app.automate_plugin_parameter(target, 0, 0.3).unwrap();
            assert_eq!(
                app.project.track(lane).unwrap().name,
                format!(
                    "Instrument · Echo {} · Cutoff Ω",
                    if index == 0 { "A" } else { "B" }
                )
            );
            assert_eq!(
                app.project
                    .track(lane)
                    .unwrap()
                    .automation
                    .as_ref()
                    .unwrap()
                    .target
                    .parameter,
                AutomationParameter::VstParameter {
                    effect: Some(effect),
                    id: 0
                }
            );
        }
        assert_eq!(
            app.project
                .tracks
                .iter()
                .filter(|track| track.automation.is_some())
                .count(),
            2
        );
    }

    #[test]
    fn native_automation_rejects_invalid_metadata_stale_owners_and_document_transitions() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = pending_app(&ctx, directory.path());
        let target = app.instrument_ui.owner.as_ref().unwrap().0;
        app.instrument_ui.parameters = vec![
            automation_parameter(0, false),
            automation_parameter(43, true),
        ];
        let original = app.project.clone();
        for (id, value) in [
            (43, 0.4),
            (999, 0.4),
            (0, -0.1),
            (0, 1.1),
            (0, f64::NAN),
            (0, f64::INFINITY),
        ] {
            assert_eq!(app.automate_plugin_parameter(target, id, value), None);
        }
        app.instrument_ui.closing = true;
        assert_eq!(app.automate_plugin_parameter(target, 0, 0.4), None);
        app.instrument_ui.closing = false;
        app.instrument_ui.pending = Some(PendingState {
            action: PendingAction::Save(directory.path().join("waiting.rill")),
            deadline: Instant::now() + Duration::from_secs(30),
            request_id: Some(1),
            saved: false,
        });
        assert_eq!(app.automate_plugin_parameter(target, 0, 0.4), None);
        app.instrument_ui.pending = None;
        app.request_document_transition(crate::document_ui::DocumentTransition::New);
        assert_eq!(app.automate_plugin_parameter(target, 0, 0.4), None);
        app.document_guard = None;
        assert_eq!(app.project, original);
        let Instrument::Vst3 { class_id, .. } =
            &mut app.project.track_mut(target.track).unwrap().instrument
        else {
            unreachable!()
        };
        *class_id = Some("replacement".into());
        assert_eq!(app.automate_plugin_parameter(target, 0, 0.4), None);
        app.project.tracks.clear();
        assert_eq!(app.automate_plugin_parameter(target, 0, 0.4), None);
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
            app.instrument_ui.browser.selected = Some(100);
            app.instrument_ui.last_touched = Some(101);
            // Metadata must not turn the compact native-editor overview into a
            // generic parameter editor. Its chooser is an explicit user action.
            let mut overview = draw(&mut app, vec![]);
            for _ in 0..3 {
                overview = draw(&mut app, vec![]);
            }
            assert!(!overview.shapes.iter().any(|shape| matches!(
                &shape.shape,
                egui::Shape::Text(text) if text.galley.text() == "Search parameters"
            )));
            if !last_touched {
                let action = overview
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == "Automate…" => {
                            Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                        }
                        _ => None,
                    })
                    .expect("overview offers automation on demand");
                for pressed in [true, false] {
                    draw(
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(action),
                            egui::Event::PointerButton {
                                pos: action,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                }
                assert!(app.instrument_ui.parameters_open);
            }
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
            let saved = app.project.clone();
            let lane = app.selected_track;
            app.close_popup();
            assert!(app.instrument_open);
            let mut output = draw(&mut app, vec![]);
            for _ in 0..3 {
                output = draw(&mut app, vec![]);
            }
            let existing_action = if last_touched {
                "Automate last touched · Parameter 101"
            } else {
                "Edit automation"
            };
            let point = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == existing_action => {
                        let rect = text.galley.rect.translate(text.pos.to_vec2());
                        shape.clip_rect.contains_rect(rect).then_some(rect.center())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| {
                    let candidates: Vec<_> = output.shapes.iter().filter_map(|shape| {
                        match &shape.shape {
                            egui::Shape::Text(text) if text.galley.text() == existing_action => {
                                Some((text.galley.rect.translate(text.pos.to_vec2()), shape.clip_rect))
                            }
                            _ => None,
                        }
                    }).collect();
                    panic!("Existing automation must remain visible at {size:?}, last touched={last_touched}: {candidates:?}")
                });
            for pressed in [true, false] {
                draw(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert_eq!(
                app.project, saved,
                "Reopening automation must not create a duplicate lane"
            );
            assert_eq!(
                app.selected_track, lane,
                "Existing action at {point:?}, viewport {size:?}, last touched={last_touched}"
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
            app.instrument_ui.browser.selected = Some(91);
            app.instrument_ui.last_touched = Some(91);
            app.instrument_ui.browser.query = "old document".into();
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
            assert!(app.instrument_ui.browser.selected.is_none());
            assert!(app.instrument_ui.parameters.is_empty());
            assert!(app.instrument_ui.last_touched.is_none());
            assert!(app.instrument_ui.browser.query.is_empty());
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

//! On-demand, isolated bus inspection and persistent channel routing.
use crate::app::DawApp;
use daw_core::{Effect, Instrument, PluginInputRoute, TrackKind};
use daw_plugins::{PluginInfo, PluginInspection};
use eframe::egui;
use std::sync::{Arc, Mutex, OnceLock, mpsc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IoTarget {
    pub track: u64,
    pub effect: Option<u64>,
}

#[derive(Clone)]
struct Configuration {
    info: PluginInfo,
    state: Vec<u8>,
    parameters: Vec<(u32, f64)>,
}

#[derive(Clone)]
enum InspectionStatus {
    Loading,
    Ready(Arc<PluginInspection>),
    Failed(String),
}

#[derive(Clone)]
struct Inspection {
    session: u64,
    target: IoTarget,
    configuration: Arc<Configuration>,
    status: Arc<Mutex<InspectionStatus>>,
    checked: Arc<Mutex<(daw_core::SongSession, bool)>>,
}

struct Request {
    inspection: Inspection,
    ctx: egui::Context,
}

fn worker() -> anyhow::Result<&'static mpsc::SyncSender<Request>> {
    static WORKER: OnceLock<Result<mpsc::SyncSender<Request>, String>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<Request>(1);
            std::thread::Builder::new()
                .name("plugin-io-inspection".into())
                .spawn(move || {
                    for request in receiver {
                        let configuration = &request.inspection.configuration;
                        let result = std::panic::catch_unwind(|| {
                            daw_plugins::inspect_plugin_with_configuration(
                                &configuration.info,
                                &configuration.state,
                                &configuration.parameters,
                            )
                        })
                        .unwrap_or_else(|_| {
                            Err(anyhow::anyhow!(
                                "The plug-in I/O inspector stopped unexpectedly. Retry inspection."
                            ))
                        });
                        *request.inspection.status.lock().unwrap() = match result {
                            Ok(details) => InspectionStatus::Ready(Arc::new(details)),
                            Err(error) => InspectionStatus::Failed(format!("{error:#}")),
                        };
                        request.ctx.request_repaint();
                    }
                })
                .map_err(|error| error.to_string())?;
            Ok(sender)
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("Could not start plug-in I/O inspection: {error}"))
}

fn inspection_id() -> egui::Id {
    egui::Id::new("plugin_io_inspection")
}

impl DawApp {
    fn io_configuration(&self, target: IoTarget) -> Option<Configuration> {
        let channel = self.project.track(target.track)?;
        let (path, class_id, state, parameters) = if let Some(id) = target.effect {
            let effect = channel.inserts.iter().find(|effect| effect.id == id)?;
            let Effect::Vst3 { path, class_id } = &effect.effect;
            (path, class_id, &effect.state, &effect.parameters)
        } else {
            let Instrument::Vst3 { path, class_id } = &channel.instrument else {
                return None;
            };
            (
                path,
                class_id,
                &channel.instrument_state,
                &channel.instrument_parameters,
            )
        };
        let info = self
            .vst_catalog
            .iter()
            .find(|entry| {
                entry.plugin.path == std::path::Path::new(path)
                    && entry.plugin.class_id == *class_id
            })
            .map(|entry| entry.plugin.clone())
            .unwrap_or_else(|| PluginInfo {
                name: std::path::Path::new(path)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path: path.into(),
                class_id: class_id.clone(),
                ..Default::default()
            });
        Some(Configuration {
            info,
            state: state.clone(),
            parameters: parameters
                .iter()
                .map(|parameter| (parameter.id, parameter.value))
                .collect(),
        })
    }

    fn io_configuration_matches(&self, target: IoTarget, configuration: &Configuration) -> bool {
        let Some(channel) = self.project.track(target.track) else {
            return false;
        };
        let (path, class_id, state, parameters) = if let Some(id) = target.effect {
            let Some(effect) = channel.inserts.iter().find(|effect| effect.id == id) else {
                return false;
            };
            let Effect::Vst3 { path, class_id } = &effect.effect;
            (path, class_id, &effect.state, &effect.parameters)
        } else {
            let Instrument::Vst3 { path, class_id } = &channel.instrument else {
                return false;
            };
            (
                path,
                class_id,
                &channel.instrument_state,
                &channel.instrument_parameters,
            )
        };
        std::path::Path::new(path) == configuration.info.path
            && *class_id == configuration.info.class_id
            && *state == configuration.state
            && parameters.len() == configuration.parameters.len()
            && parameters
                .iter()
                .zip(&configuration.parameters)
                .all(|(current, saved)| current.id == saved.0 && current.value == saved.1)
    }

    fn current_io_inspection(&self, inspection: &Inspection) -> bool {
        if inspection.session != self.song_session.id {
            return false;
        }
        let mut checked = inspection.checked.lock().unwrap();
        if checked.0 != self.song_session || self.edit_dirty {
            *checked = (
                self.song_session,
                self.io_configuration_matches(inspection.target, &inspection.configuration),
            );
        }
        checked.1
    }

    pub(crate) fn begin_plugin_io_inspection(&mut self, ctx: &egui::Context, target: IoTarget) {
        let Some(configuration) = self.io_configuration(target) else {
            return;
        };
        let inspection = Inspection {
            session: self.song_session.id,
            target,
            configuration: Arc::new(configuration),
            status: Arc::new(Mutex::new(InspectionStatus::Loading)),
            checked: Arc::new(Mutex::new((self.song_session, true))),
        };
        let result = worker().and_then(|worker| {
            worker
                .try_send(Request {
                    inspection: inspection.clone(),
                    ctx: ctx.clone(),
                })
                .map_err(|error| anyhow::anyhow!("Could not queue plug-in I/O inspection: {error}"))
        });
        if let Err(error) = result {
            *inspection.status.lock().unwrap() = InspectionStatus::Failed(format!("{error:#}"));
        }
        // One retained result per application context, regardless of how many
        // plug-ins or sessions are browsed. The worker itself has a bounded queue.
        ctx.data_mut(|data| data.insert_temp(inspection_id(), inspection));
        ctx.request_repaint();
    }

    pub(crate) fn plugin_io_controls(&mut self, ui: &mut egui::Ui, track: u64) {
        let Some(channel) = self.project.track(track) else {
            return;
        };
        let mut choices = Vec::new();
        if matches!(channel.instrument, Instrument::Vst3 { .. }) {
            choices.push((None, "Instrument".to_owned()));
        }
        choices.extend(channel.inserts.iter().enumerate().map(|(index, effect)| {
            (
                Some(effect.id),
                format!("{:02} · {}", index + 1, effect.name),
            )
        }));
        if choices.is_empty() {
            return;
        }
        ui.add_space(daw_ui::tokens::SPACE_LG);
        daw_ui::text::section_heading(ui, "Plug-in inputs and outputs");
        let choice_id = egui::Id::new(("plugin_io_choice", self.song_session.id, track));
        let mut effect = ui
            .ctx()
            .data(|data| data.get_temp::<Option<u64>>(choice_id))
            .unwrap_or(choices[0].0);
        if !choices.iter().any(|choice| choice.0 == effect) {
            effect = choices[0].0;
        }
        daw_ui::select(
            "plugin_io_processor",
            choices
                .iter()
                .find(|choice| choice.0 == effect)
                .unwrap()
                .1
                .as_str(),
        )
        .show_ui(ui, |ui| {
            for (id, name) in &choices {
                daw_ui::selectable_value(ui, &mut effect, *id, name);
            }
        });
        ui.ctx()
            .data_mut(|data| data.insert_temp(choice_id, effect));
        let target = IoTarget { track, effect };
        let inspection = ui
            .ctx()
            .data(|data| data.get_temp::<Inspection>(inspection_id()))
            .filter(|inspection| {
                inspection.target == target && self.current_io_inspection(inspection)
            });
        let status = inspection
            .as_ref()
            .map(|inspection| inspection.status.lock().unwrap().clone());
        let loading = matches!(status, Some(InspectionStatus::Loading));
        if ui.add_enabled(!loading && !self.state_sync_pending(), daw_ui::button_widget(if inspection.is_some() { "Refresh I/O" } else { "Inspect I/O" }))
            .on_hover_text("Save the native editor state, then inspect audio buses in a separate, time-limited process")
            .clicked()
        {
            self.request_plugin_io_inspection(ui.ctx(), target);
        }
        let details = match status {
            Some(InspectionStatus::Ready(details)) => details,
            Some(InspectionStatus::Loading) => {
                ui.spinner();
                daw_ui::text::note(ui, "Inspecting plug-in buses…");
                return;
            }
            Some(InspectionStatus::Failed(message)) => {
                daw_ui::text::note(ui, message);
                return;
            }
            None => {
                daw_ui::text::note(
                    ui,
                    "Inspect this processor to connect extra inputs or expose instrument outputs.",
                );
                return;
            }
        };
        if effect.is_none()
            && self
                .project
                .track(track)
                .is_some_and(|channel| channel.kind == TrackKind::Midi)
        {
            self.instrument_output_controls(ui, track, &details);
        }
        daw_ui::text::section_heading(ui, "Audio inputs");
        let mut routes = self
            .project
            .track(track)
            .map(|channel| {
                effect
                    .and_then(|id| {
                        channel
                            .inserts
                            .iter()
                            .find(|effect| effect.id == id)
                            .map(|effect| effect.inputs.clone())
                    })
                    .unwrap_or_else(|| channel.instrument_inputs.clone())
            })
            .unwrap_or_default();
        let sources: Vec<_> = self
            .project
            .eligible_plugin_input_sources(track)
            .unwrap_or_default()
            .into_iter()
            .map(|source| (source.id, source.name.clone()))
            .collect();
        let mut changed = false;
        let mut remove = None;
        let main_input = details.inputs.iter().position(|bus| bus.main).unwrap_or(0);
        for (bus_index, bus) in details.inputs.iter().enumerate() {
            for first_channel in (0..bus.channels).step_by(2) {
                if effect.is_some() && bus_index == main_input && first_channel == 0 {
                    continue;
                }
                let name = pair_name(&bus.name, bus_index, first_channel, bus.channels, "Input");
                ui.push_id(("extra_input", bus_index, first_channel), |ui| {
                    daw_ui::text::label(ui, &name);
                    for (index, route) in routes.iter_mut().enumerate().filter(|(_, route)| {
                        route.bus as usize == bus_index
                            && route.first_channel as usize == first_channel
                    }) {
                        ui.push_id((index, route.source_track), |ui| {
                            daw_ui::parameter_row(ui, |ui| {
                                let mut source = route.source_track;
                                daw_ui::select(
                                    "input_source",
                                    sources
                                        .iter()
                                        .find(|source| source.0 == route.source_track)
                                        .map(|source| source.1.as_str())
                                        .unwrap_or("Unavailable source"),
                                )
                                .show_ui(ui, |ui| {
                                    for (id, name) in &sources {
                                        daw_ui::selectable_value(ui, &mut source, *id, name);
                                    }
                                });
                                if source != route.source_track {
                                    route.source_track = source;
                                    changed = true;
                                }
                                changed |= daw_ui::gain_value_button(
                                    ui,
                                    "input_level",
                                    &mut route.gain,
                                    16.0,
                                    "Gain",
                                )
                                .changed();
                                changed |= daw_ui::checkbox(ui, &mut route.pre_fader, "Pre volume")
                                    .changed();
                                if daw_ui::small_button(ui, "Remove").clicked() {
                                    remove = Some(index);
                                }
                            });
                        });
                    }
                    daw_ui::menu_button(ui, "Add source…", |ui| {
                        for (source, source_name) in &sources {
                            let exists = routes.iter().any(|route| {
                                route.source_track == *source
                                    && route.bus as usize == bus_index
                                    && route.first_channel as usize == first_channel
                            });
                            if ui
                                .add_enabled(!exists, daw_ui::button_widget(source_name))
                                .clicked()
                            {
                                routes.push(PluginInputRoute {
                                    source_track: *source,
                                    bus: bus_index as u32,
                                    first_channel: first_channel as u32,
                                    gain: 1.0,
                                    pre_fader: false,
                                });
                                changed = true;
                                ui.close();
                            }
                        }
                        if sources.is_empty() {
                            daw_ui::text::note(
                                ui,
                                "No sources can reach this input without feedback.",
                            );
                        }
                    });
                });
            }
        }
        if let Some(index) = remove {
            routes.remove(index);
            changed = true;
        }
        if changed {
            self.begin_edit();
            match self.project.set_plugin_inputs(track, effect, routes) {
                Ok(()) => {
                    self.edit_label = Some("Route plug-in inputs".into());
                    self.changed();
                }
                Err(error) => self.message(Err(error), ""),
            }
        }
    }

    fn instrument_output_controls(
        &mut self,
        ui: &mut egui::Ui,
        track: u64,
        details: &PluginInspection,
    ) {
        daw_ui::text::section_heading(ui, "Instrument output channels");
        let mut available = Vec::new();
        let main_output = details.outputs.iter().position(|bus| bus.main).unwrap_or(0);
        for (bus_index, bus) in details.outputs.iter().enumerate() {
            for first_channel in (0..bus.channels).step_by(2) {
                if bus_index == main_output && first_channel == 0 {
                    continue;
                }
                if !self.project.tracks.iter().any(|channel| {
                    channel.instrument_output.as_ref().is_some_and(|output| {
                        output.track == track
                            && output.bus as usize == bus_index
                            && output.first_channel as usize == first_channel
                    })
                }) {
                    available.push((
                        pair_name(&bus.name, bus_index, first_channel, bus.channels, "Output"),
                        bus_index as u32,
                        first_channel as u32,
                    ));
                }
            }
        }
        if ui.add_enabled(!available.is_empty(), daw_ui::button_widget("Enable all output channels"))
            .on_hover_text("Create grouped mixer channels for every remaining native output pair; the instrument runs once")
            .clicked()
        { self.add_instrument_output_channels(track, available.clone()); }
        daw_ui::menu_button(ui, "Enable output…", |ui| {
            for (name, bus, first_channel) in &available {
                if daw_ui::button(ui, name).clicked() {
                    self.add_instrument_output_channels(
                        track,
                        vec![(name.clone(), *bus, *first_channel)],
                    );
                    ui.close();
                }
            }
            if available.is_empty() {
                daw_ui::text::note(ui, "All available outputs are enabled.");
            }
        });
        let outputs: Vec<_> = self
            .project
            .tracks
            .iter()
            .filter(|channel| {
                channel
                    .instrument_output
                    .as_ref()
                    .is_some_and(|output| output.track == track)
            })
            .map(|channel| (channel.id, channel.name.clone()))
            .collect();
        for (id, name) in outputs {
            daw_ui::parameter_row(ui, |ui| {
                daw_ui::text::label(ui, name);
                if daw_ui::small_button(ui, "Show in Mixer").clicked() {
                    self.show_channel_in_mixer(ui.ctx(), id);
                }
                if daw_ui::small_button(ui, "Remove output").clicked() {
                    self.flush_pending_edit();
                    self.begin_edit();
                    match self.project.delete_track(id) {
                        Ok(_) => {
                            self.edit_label = Some("Remove instrument output".into());
                            self.changed();
                            self.flush_pending_edit();
                        }
                        Err(error) => self.message(Err(error), ""),
                    }
                }
            });
        }
    }

    fn add_instrument_output_channels(&mut self, track: u64, outputs: Vec<(String, u32, u32)>) {
        self.flush_pending_edit();
        self.begin_edit();
        match self.project.add_instrument_outputs(track, outputs) {
            Ok(_) => {
                self.edit_label = Some("Enable instrument outputs".into());
                self.changed();
                self.flush_pending_edit();
            }
            Err(error) => self.message(Err(error), ""),
        }
    }

    pub(crate) fn plugin_io_diagnostics(&self, ctx: &egui::Context) -> serde_json::Value {
        let Some(inspection) = ctx.data(|data| data.get_temp::<Inspection>(inspection_id())) else {
            return serde_json::Value::Null;
        };
        let (status, details, error) = match &*inspection.status.lock().unwrap() {
            InspectionStatus::Loading => ("loading", serde_json::Value::Null, None),
            InspectionStatus::Ready(details) => (
                "ready",
                serde_json::to_value(details.as_ref()).unwrap_or_default(),
                None,
            ),
            InspectionStatus::Failed(error) => {
                ("failed", serde_json::Value::Null, Some(error.clone()))
            }
        };
        serde_json::json!({"track":inspection.target.track,"effect":inspection.target.effect,
            "session":inspection.session,"current":self.current_io_inspection(&inspection),
            "status":status,"details":details,"error":error})
    }
}

fn pair_name(name: &str, bus: usize, first: usize, channels: usize, fallback: &str) -> String {
    let name = if name.is_empty() {
        format!("{fallback} {}", bus + 1)
    } else {
        name.to_owned()
    };
    if channels <= 2 {
        name
    } else if first + 1 < channels {
        format!("{name} · {}/{}", first + 1, first + 2)
    } else {
        format!("{name} · {} (mono)", first + 1)
    }
}

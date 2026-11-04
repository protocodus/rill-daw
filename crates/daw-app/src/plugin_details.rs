//! On-demand catalogue inspection, independent of any loaded project instance.
use daw_plugins::{CatalogEntry, PluginInspection};
use eframe::egui;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, mpsc},
};

#[derive(Clone)]
enum Inspection {
    Loading,
    Ready(Arc<PluginInspection>),
    Failed(String),
}

#[derive(Default)]
struct CacheEntries {
    entries: HashMap<String, Inspection>,
    completed: VecDeque<String>,
}

impl CacheEntries {
    fn insert(&mut self, key: String, inspection: Inspection) {
        self.completed.retain(|completed| completed != &key);
        if !matches!(inspection, Inspection::Loading) {
            self.completed.push_back(key.clone());
        }
        self.entries.insert(key, inspection);
        while self.completed.len() > 32 {
            if let Some(oldest) = self.completed.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

struct Request {
    entry: CatalogEntry,
    ctx: egui::Context,
}

#[derive(Clone)]
struct Cache {
    entries: Arc<Mutex<CacheEntries>>,
    requests: mpsc::SyncSender<Request>,
}

impl Default for Cache {
    fn default() -> Self {
        // A single worker and one queued request keep rapid browsing from
        // launching an unbounded number of third-party plug-in processes.
        let (requests, receiver) = mpsc::sync_channel::<Request>(1);
        let entries = Arc::new(Mutex::new(CacheEntries::default()));
        let completed = entries.clone();
        std::thread::spawn(move || {
            for request in receiver {
                let result = match daw_plugins::inspect_plugin(&request.entry.plugin) {
                    Ok(details) => Inspection::Ready(Arc::new(details)),
                    Err(error) => Inspection::Failed(format!("{error:#}")),
                };
                completed
                    .lock()
                    .unwrap()
                    .insert(request.entry.fingerprint, result);
                request.ctx.request_repaint();
            }
        });
        Self { entries, requests }
    }
}

#[derive(Clone)]
struct Dialog {
    entry: CatalogEntry,
    query: String,
    technical: bool,
    report: String,
    backdrop: Option<daw_ui::ViewBackdrop>,
}

fn cache(ctx: &egui::Context) -> Cache {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Cache>(egui::Id::new("plugin_inspection_cache"))
            .clone()
    })
}

fn status(ctx: &egui::Context, entry: &CatalogEntry) -> Option<Inspection> {
    let cache = cache(ctx);
    let mut cache = cache.entries.lock().unwrap();
    let status = cache.entries.get(&entry.fingerprint).cloned();
    // Keep a displayed result resident while other queued inspections finish.
    if matches!(status, Some(Inspection::Ready(_) | Inspection::Failed(_))) {
        cache.completed.retain(|key| key != &entry.fingerprint);
        cache.completed.push_back(entry.fingerprint.clone());
    }
    status
}

fn start(ctx: &egui::Context, entry: &CatalogEntry) {
    let cache = cache(ctx);
    {
        let mut cache = cache.entries.lock().unwrap();
        if matches!(
            cache.entries.get(&entry.fingerprint),
            Some(Inspection::Loading)
        ) {
            return;
        }
        cache.insert(entry.fingerprint.clone(), Inspection::Loading);
    }
    if let Err(error) = cache.requests.try_send(Request {
        entry: entry.clone(),
        ctx: ctx.clone(),
    }) {
        let message = match error {
            mpsc::TrySendError::Full(_) => {
                "Another device is being inspected. Try again when it finishes."
            }
            mpsc::TrySendError::Disconnected(_) => {
                "The device inspector stopped. Reopen Rill to retry."
            }
        };
        cache.entries.lock().unwrap().insert(
            entry.fingerprint.clone(),
            Inspection::Failed(message.into()),
        );
    }
}

fn summary(details: &PluginInspection) -> daw_ui::DeviceIoSummary {
    daw_ui::DeviceIoSummary {
        input_channels: details.inputs.iter().map(|bus| bus.channels).sum(),
        input_buses: details.inputs.len(),
        output_channels: details.outputs.iter().map(|bus| bus.channels).sum(),
        output_buses: details.outputs.len(),
        midi_input: details.midi_input,
        midi_output: details.midi_output,
        parameters: details.parameters.len(),
    }
}

pub(crate) fn show_summary(ui: &mut egui::Ui, entry: &CatalogEntry) {
    let status = status(ui.ctx(), entry);
    daw_ui::device_io_summary(
        ui,
        match &status {
            Some(Inspection::Ready(details)) => Some(summary(details)),
            _ => None,
        },
    );
    let label = match status {
        Some(Inspection::Ready(_)) => "Browse parameters",
        Some(Inspection::Loading) => "Reading details…",
        Some(Inspection::Failed(_)) => "Retry details",
        None => "Read details",
    };
    if ui
        .add_enabled(
            !matches!(status, Some(Inspection::Loading)),
            daw_ui::button_widget(label),
        )
        .clicked()
    {
        if matches!(status, Some(Inspection::Failed(_))) {
            start(ui.ctx(), entry);
        }
        open(ui.ctx(), entry);
    }
}

pub(crate) fn open(ctx: &egui::Context, entry: &CatalogEntry) {
    if status(ctx, entry).is_none() {
        start(ctx, entry);
    }
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new("plugin_details_dialog"),
            Dialog {
                entry: entry.clone(),
                query: String::new(),
                technical: false,
                report: crate::library::device_diagnostics(entry).1,
                backdrop: None,
            },
        )
    });
}

pub(crate) fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|data| {
        data.get_temp::<Dialog>(egui::Id::new("plugin_details_dialog"))
            .is_some()
    })
}

pub(crate) fn close(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<Dialog>(egui::Id::new("plugin_details_dialog"));
    });
}

/// Serialize the potentially large catalogue only when an inspector requests
/// it, rather than allocating one JSON object per parameter on every UI frame.
pub(crate) fn diagnostics(ctx: &egui::Context) -> Option<serde_json::Value> {
    let dialog =
        ctx.data(|data| data.get_temp::<Dialog>(egui::Id::new("plugin_details_dialog")))?;
    let mut diagnostic = match status(ctx, &dialog.entry)? {
        Inspection::Ready(details) => {
            serde_json::json!({ "status":"ready", "metadata":details.as_ref() })
        }
        Inspection::Loading => serde_json::json!({ "status":"loading" }),
        Inspection::Failed(error) => serde_json::json!({ "status":"failed", "error":error }),
    };
    diagnostic["name"] = serde_json::json!(dialog.entry.plugin.name);
    diagnostic["query"] = serde_json::json!(dialog.query);
    Some(diagnostic)
}

fn inspection_report(base: &str, status: &Inspection) -> String {
    let mut report = base.to_owned();
    match status {
        Inspection::Ready(details) => {
            let ports = summary(details);
            report.push_str(&format!("\n\nInputs: {} audio channels, {} buses, MIDI {}\nOutputs: {} audio channels, {} buses, MIDI {}\nParameters: {}\n", ports.input_channels, ports.input_buses, ports.midi_input, ports.output_channels, ports.output_buses, ports.midi_output, ports.parameters));
            for parameter in &details.parameters {
                report.push_str(&format!(
                    "\n#{} {} [{}]{}{}",
                    parameter.id,
                    parameter.name,
                    parameter.unit,
                    if parameter.read_only {
                        " · Read-only"
                    } else {
                        ""
                    },
                    if parameter.can_automate {
                        " · Automatable"
                    } else {
                        ""
                    }
                ));
            }
        }
        Inspection::Failed(error) => report.push_str(&format!("\n\nInspection failed: {error}")),
        Inspection::Loading => report.push_str("\n\nInspection in progress"),
    }
    report
}

/// Call instead of background app editing while open, and once in the opening
/// frame to retain the completed caller as the modal backdrop.
pub(crate) fn show_window(ctx: &egui::Context) {
    let key = egui::Id::new("plugin_details_dialog");
    let Some(mut dialog) = ctx.data_mut(|data| {
        let dialog = data.get_temp::<Dialog>(key);
        data.remove::<Dialog>(key);
        dialog
    }) else {
        return;
    };
    if let Some(backdrop) = &dialog.backdrop {
        egui::CentralPanel::default()
            .frame(daw_ui::inset(ctx).inner_margin(0).corner_radius(0))
            .show(ctx, |ui| backdrop.paint(ui));
    } else {
        dialog.backdrop = Some(daw_ui::ViewBackdrop::capture_dialog_context(
            ctx,
            ctx.content_rect(),
        ));
    }
    let status = status(ctx, &dialog.entry).unwrap_or(Inspection::Loading);
    let mut retry = false;
    let modal = daw_ui::modal(
        ctx,
        egui::Id::new("plugin_details_modal"),
        "Device details",
        |ui| {
            daw_ui::text::bounded_block(
                ui,
                daw_ui::text::TextRole::Subtitle,
                &dialog.entry.plugin.name,
                1,
            );
            daw_ui::text::bounded_block(
                ui,
                daw_ui::text::TextRole::Note,
                format!(
                    "{} · {}",
                    dialog.entry.plugin.vendor,
                    dialog.entry.plugin.category.replace('|', " · ")
                ),
                1,
            );
            ui.add_space(daw_ui::tokens::SPACE_SM);
            if dialog.technical {
                let report = inspection_report(&dialog.report, &status);
                let height = (ctx.content_rect().height()
                    - 4.0 * daw_ui::tokens::POPUP_MARGIN
                    - (ui.cursor().top() - ui.min_rect().top())
                    - daw_ui::tokens::CONTROL_HEIGHT
                    - 2.0 * daw_ui::tokens::SPACE_MD)
                    .clamp(0.0, 377.0);
                daw_ui::scroll_area(ui, "plugin_technical_details", [false, true])
                    .max_height(height)
                    .min_scrolled_height(0.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        daw_ui::text::selectable_block(ui, daw_ui::text::TextRole::Body, &report);
                    });
            } else {
                match &status {
                    Inspection::Loading => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            daw_ui::text::note(ui, "Reading inputs, outputs and parameters…");
                        });
                    }
                    Inspection::Failed(message) => {
                        daw_ui::text::subtitle(ui, "Details unavailable");
                        daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note, message, 5)
                            .on_hover_text(message);
                        retry = daw_ui::button(ui, "Retry details").clicked();
                    }
                    Inspection::Ready(details) => {
                        daw_ui::device_io_summary(ui, Some(summary(details)));
                        daw_ui::text::note(ui, format!("{} input {} · {} output {}", details.inputs.len(), if details.inputs.len() == 1 { "bus" } else { "buses" }, details.outputs.len(), if details.outputs.len() == 1 { "bus" } else { "buses" }))
                    .on_hover_text("The plug-in's default bus layout; the project host may route a subset of these channels.");
                        ui.add_space(daw_ui::tokens::SPACE_SM);
                        let parameters: Vec<_> = details
                            .parameters
                            .iter()
                            .map(|parameter| daw_ui::DeviceParameterMetadata {
                                id: parameter.id,
                                name: &parameter.name,
                                unit: &parameter.unit,
                                steps: parameter.steps,
                                read_only: parameter.read_only,
                                can_automate: parameter.can_automate,
                            })
                            .collect();
                        // Reserve search, result count and Close before assigning the
                        // list its viewport, including on compact laptop screens.
                        let height = (ctx.content_rect().height()
                            - 4.0 * daw_ui::tokens::POPUP_MARGIN
                            - (ui.cursor().top() - ui.min_rect().top())
                            - 3.0 * daw_ui::tokens::CONTROL_HEIGHT
                            - 5.0 * daw_ui::tokens::SPACE_MD)
                            .clamp(0.0, 233.0);
                        daw_ui::device_parameter_catalog(
                            ui,
                            &mut dialog.query,
                            &parameters,
                            height,
                        );
                    }
                }
            }
            ui.add_space(daw_ui::tokens::SPACE_MD);
            daw_ui::action_row(
                ui,
                |ui| {
                    let close = daw_ui::button(ui, "Close");
                    if daw_ui::button(ui, "Copy details").clicked() {
                        ctx.copy_text(inspection_report(&dialog.report, &status));
                    }
                    if daw_ui::view_button(
                        ui,
                        if dialog.technical {
                            "Parameters"
                        } else {
                            "Technical details"
                        },
                    )
                    .clicked()
                    {
                        dialog.technical = !dialog.technical;
                    }
                    close
                },
                |_| {},
            )
            .inner
            .0
            .clicked()
        },
    );
    if retry {
        start(ctx, &dialog.entry);
    }
    let closed = modal.should_close() || modal.inner;
    ctx.data_mut(|data| {
        if !closed {
            data.insert_temp(key, dialog);
        }
    });
}

//! Device discovery, cached previews and focused insertion workflows.
use crate::app::{DawApp, PopupKind, SetupPage};
use crate::library_preview::PreviewCache;
use daw_core::{Clip, Effect, EffectSlot, Instrument, TrackKind};
use daw_plugins::catalog::{CatalogEntry, PluginCatalog, ThumbnailStatus};
use eframe::egui;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) enum LibraryTarget {
    #[default]
    Browse,
    Instrument {
        track: Option<u64>,
    },
    Insert(u64),
    Send(u64),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum DeviceKind {
    #[default]
    Instrument,
    Effect,
}

type CaptureJob = Arc<Mutex<Option<Result<CatalogEntry, String>>>>;

#[derive(Clone, Default)]
struct LibraryUi {
    activate_when_ready: bool,
    kind: DeviceKind,
    target: LibraryTarget,
    developer: String,
    selected: Option<String>,
    capture: Option<CaptureJob>,
    queued_capture: Option<daw_plugins::PluginInfo>,
    attempted: HashSet<String>,
    previews: PreviewCache,
    grid_filter: Option<(DeviceKind, String, String)>,
    grid_columns: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeviceAction {
    Instrument,
    Assign,
    Insert,
    Send,
}

impl DeviceKind {
    fn accepts(self, entry: &CatalogEntry) -> bool {
        match self {
            Self::Instrument => entry.plugin.is_instrument() != Some(false),
            Self::Effect => entry.plugin.is_effect() != Some(false),
        }
    }
}

fn start_capture(plugin: daw_plugins::PluginInfo) -> CaptureJob {
    let job: CaptureJob = Arc::new(Mutex::new(None));
    let worker = job.clone();
    std::thread::spawn(move || {
        let result = PluginCatalog::default()
            .capture(&plugin, daw_plugins::capture_editor_info)
            .map_err(|error| format!("{error:#}"));
        if let Ok(mut worker) = worker.lock() {
            *worker = Some(result);
        }
    });
    job
}

impl DawApp {
    /// Capture one verified device at a time. Discovery remains metadata-only;
    /// initialization and screenshot work always happen in isolated helpers.
    pub(crate) fn poll_plugin_previews(&mut self, ctx: &egui::Context) {
        let key = egui::Id::new("plugin_library_state");
        let mut state = ctx
            .data_mut(|data| data.remove_temp::<LibraryUi>(key))
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
                        state.previews.invalidate(&entry.fingerprint);
                        if let Some(current) = self
                            .vst_catalog
                            .iter_mut()
                            .find(|item| item.fingerprint == entry.fingerprint)
                        {
                            *current = entry;
                        }
                    }
                    Err(error) => self.message(Err(anyhow::anyhow!(error)), ""),
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        if state.capture.is_none()
            && !self.state_sync_pending()
            && let Some(plugin) = state.queued_capture.take()
        {
            state.capture = Some(start_capture(plugin));
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.automatic_plugin_previews
            && state.capture.is_none()
            && !self.state_sync_pending()
            && let Some(entry) = self
                .vst_catalog
                .iter()
                .filter(|entry| {
                    (entry.plugin.is_instrument() == Some(true)
                        || entry.plugin.is_effect() == Some(true)
                        || state.selected.as_ref() == Some(&entry.fingerprint))
                        && matches!(entry.status, ThumbnailStatus::Missing)
                        && !state.attempted.contains(&entry.fingerprint)
                })
                .min_by_key(|entry| state.selected.as_ref() != Some(&entry.fingerprint))
        {
            state.attempted.insert(entry.fingerprint.clone());
            state.capture = Some(start_capture(entry.plugin.clone()));
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
        self.library_target = LibraryTarget::Instrument {
            track: self.plugin_assign_target,
        };
    }

    pub(crate) fn open_effect_library(&mut self, track: u64, send: bool) {
        self.open_popup(PopupKind::Library);
        self.plugin_filter.clear();
        self.library_target = if send {
            LibraryTarget::Send(track)
        } else {
            LibraryTarget::Insert(track)
        };
    }

    pub(crate) fn plugin_library(&mut self, ctx: &egui::Context) {
        if !self.library_open {
            return;
        }
        let key = egui::Id::new("plugin_library_state");
        let mut state = ctx
            .data_mut(|data| data.remove_temp::<LibraryUi>(key))
            .unwrap_or_default();
        if state.target != self.library_target {
            state.target = self.library_target;
            state.kind = if matches!(
                state.target,
                LibraryTarget::Insert(_) | LibraryTarget::Send(_)
            ) {
                DeviceKind::Effect
            } else {
                DeviceKind::Instrument
            };
            state.selected = None;
            state.developer.clear();
            state.grid_filter = None;
        }
        // The chooser owns its assignment destination. A native preview or
        // later selection change must never turn Assign into Add, or send it
        // to a different MIDI track.
        let assignment_target = match self.library_target {
            LibraryTarget::Instrument { track } => track,
            _ => self.plugin_assign_target,
        };
        let assign = assignment_target.is_some();
        let assignment_target = assignment_target.filter(|id| {
            self.project
                .track(*id)
                .is_some_and(|track| track.kind == TrackKind::Midi)
        });
        let source = match self.library_target {
            LibraryTarget::Insert(track) | LibraryTarget::Send(track) => Some(track),
            LibraryTarget::Instrument { track: Some(track) } => Some(track),
            _ => self.selected_track,
        }
        .filter(|id| {
            self.project.track(*id).is_some_and(|track| {
                matches!(
                    track.kind,
                    TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                )
            })
        });
        let mut action = None;
        let mut folders = false;
        let mut retry = None;
        let mut open = true;
        let pressed_enter = ctx
            .data_mut(|data| data.remove_temp::<bool>(egui::Id::new("instrument_chooser_enter")))
            .unwrap_or(false)
            || (!egui::Popup::is_any_open(ctx)
                && ctx.input(|input| input.key_pressed(egui::Key::Enter)));
        state.activate_when_ready |= pressed_enter;
        let activate = state.activate_when_ready && self.vst_scan.is_none();
        if activate {
            state.activate_when_ready = false;
        }
        daw_ui::popup("Device library", ctx)
            .id(egui::Id::new("library"))
            .open(&mut open)
            .default_width(940.0)
            .default_height(650.0)
            .vscroll(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let mut kind = state.kind;
                    daw_ui::selectable_value(ui, &mut kind, DeviceKind::Instrument, "Instruments");
                    daw_ui::selectable_value(ui, &mut kind, DeviceKind::Effect, "Effects");
                    if kind != state.kind {
                        state.kind = kind;
                        state.selected = None;
                        state.developer.clear();
                        self.plugin_filter.clear();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        daw_ui::action_menu(ui, "Manage", |ui| {
                            if daw_ui::button(ui, "Folders").clicked() {
                                folders = true;
                                ui.close();
                            }
                        });
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
                    });
                });
                let search = ui.horizontal_wrapped(|ui| {
                    let search_width = (ui.available_width()
                        - daw_ui::tokens::SELECT_WIDTH
                        - daw_ui::tokens::SPACE_SM
                        - 2.0 * daw_ui::tokens::BUTTON_PADDING_X)
                        .clamp(80.0, 460.0);
                    let search = daw_ui::text_field(
                        ui,
                        &mut self.plugin_filter,
                        if state.kind == DeviceKind::Instrument {
                            "Search instruments"
                        } else {
                            "Search effects"
                        },
                        search_width,
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
                    daw_ui::select(
                        "library_developer",
                        if state.developer.is_empty() {
                            "All developers"
                        } else {
                            &state.developer
                        },
                    )
                    .show_ui(ui, |ui| {
                        // The inventory can contain thousands of devices. Build
                        // developer choices only while their menu is open.
                        let mut developers: Vec<_> = self
                            .vst_catalog
                            .iter()
                            .filter(|entry| state.kind.accepts(entry))
                            .map(|entry| entry.plugin.vendor.as_str())
                            .filter(|vendor| !vendor.is_empty())
                            .collect();
                        developers.sort_unstable();
                        developers.dedup();
                        daw_ui::selectable_value(
                            ui,
                            &mut state.developer,
                            String::new(),
                            "All developers",
                        );
                        for developer in developers {
                            daw_ui::selectable_value(
                                ui,
                                &mut state.developer,
                                developer.to_owned(),
                                developer,
                            );
                        }
                    });
                    search
                }).inner;
                ui.add_space(8.0);
                let filter = self.plugin_filter.to_lowercase();
                // A filtered-out device needs a different next step from an
                // inventory with no instruments/effects of the requested type.
                let has_kind = self.vst_catalog.iter().any(|entry| state.kind.accepts(entry));
                let matches: Vec<_> = self
                    .vst_catalog
                    .iter()
                    .filter(|entry| {
                        state.kind.accepts(entry)
                            && (state.developer.is_empty()
                                || entry.plugin.vendor == state.developer)
                            && (filter.is_empty() || format!(
                                "{} {} {}",
                                entry.plugin.name, entry.plugin.vendor, entry.plugin.category
                            )
                            .to_lowercase()
                            .contains(&filter))
                    })
                    .collect();
                let signature = (state.kind, self.plugin_filter.clone(), state.developer.clone());
                let filter_changed = state.grid_filter.as_ref() != Some(&signature);
                state.grid_filter = Some(signature);
                if !matches.iter().any(|entry| state.selected.as_ref() == Some(&entry.fingerprint)) {
                    state.selected = matches.first().map(|entry| entry.fingerprint.clone());
                }
                let primary = match state.kind {
                    DeviceKind::Instrument if assign => DeviceAction::Assign,
                    DeviceKind::Instrument => DeviceAction::Instrument,
                    DeviceKind::Effect if matches!(self.library_target, LibraryTarget::Send(_)) => DeviceAction::Send,
                    DeviceKind::Effect => DeviceAction::Insert,
                };
                let enabled = match state.kind {
                    DeviceKind::Instrument => !assign || assignment_target.is_some(),
                    DeviceKind::Effect => source.is_some(),
                };
                let layout = daw_ui::device_grid_layout(ui.available_width());
                let reflowed = state.grid_columns != layout.columns;
                state.grid_columns = layout.columns;
                let popup_open = egui::Popup::is_any_open(ctx);
                let search_focused = ui.memory(|memory| memory.has_focus(search.id));
                let step = ctx.input_mut(|input| {
                    if popup_open { return 0; }
                    if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                        layout.columns as isize
                    } else if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                        -(layout.columns as isize)
                    } else if !search_focused && input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight) {
                        1
                    } else if !search_focused && input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft) {
                        -1
                    } else { 0 }
                });
                let mut scroll_to = if filter_changed || reflowed {
                    matches.iter().position(|entry| state.selected.as_ref() == Some(&entry.fingerprint))
                } else {
                    None
                };
                if step != 0 && !matches.is_empty() {
                    let index = matches.iter().position(|entry| state.selected.as_ref() == Some(&entry.fingerprint)).unwrap_or(0);
                    let next = if (step == -(layout.columns as isize) && index < layout.columns)
                        || (step == layout.columns as isize && index / layout.columns == (matches.len() - 1) / layout.columns)
                    {
                        index
                    } else {
                        (index as isize + step).clamp(0, matches.len() as isize - 1) as usize
                    };
                    state.selected = Some(matches[next].fingerprint.clone());
                    scroll_to = Some(next);
                }
                if activate && enabled
                    && let Some(entry) = matches.iter().find(|entry| state.selected.as_ref() == Some(&entry.fingerprint))
                {
                    action = Some((entry.plugin.clone(), primary));
                }
                // A fixed footer keeps the insertion target stable as previews
                // arrive, filters change, and long device names wrap on cards.
                // Includes separator spacing, the selected name and action row.
                // A reserve smaller than their total feeds window growth back
                // into this available-height calculation on every frame.
                let grid_height = (ui.available_height() - 84.0).max(0.0);
                let grid = daw_ui::device_grid(ui, "device_catalog", matches.len(), grid_height, scroll_to, |ui, index, width| {
                    let entry = matches[index];
                    let texture = entry.thumbnail.as_ref().and_then(|path| state.previews.request(ctx, &entry.fingerprint, path));
                    let producer = if entry.plugin.vendor.is_empty() { "Unknown developer" } else { &entry.plugin.vendor };
                    let card = daw_ui::device_card(ui, &daw_ui::DeviceCard {
                        id: egui::Id::new(("library_device", &entry.fingerprint)),
                        name: &entry.plugin.name,
                        producer,
                        preview: texture.as_ref(),
                        selected: state.selected.as_ref() == Some(&entry.fingerprint),
                        icon: if state.kind == DeviceKind::Instrument { daw_ui::Icon::Instrument } else { daw_ui::Icon::Effect },
                    }, width);
                    if step != 0 && state.selected.as_ref() == Some(&entry.fingerprint) {
                        card.request_focus();
                    }
                    if card.response.clicked() {
                        state.selected = Some(entry.fingerprint.clone());
                    }
                    if card.activated && enabled {
                        action = Some((entry.plugin.clone(), primary));
                    }
                    let rect = card.response.rect;
                    let preview = card.preview_rect;
                    serde_json::json!({
                        "fingerprint": entry.fingerprint, "name": entry.plugin.name, "producer": producer,
                        "rect": {"min": [rect.min.x, rect.min.y], "max": [rect.max.x, rect.max.y]},
                        "preview_rect": {"min": [preview.min.x, preview.min.y], "max": [preview.max.x, preview.max.y]},
                        "preview": texture.is_some(), "texture_id": texture.as_ref().map(|texture| format!("{:?}", texture.id())),
                    })
                });
                if matches.is_empty() && self.vst_scan.is_none() {
                    let mut empty = ui.new_child(egui::UiBuilder::new().max_rect(grid.rect));
                    empty.vertical_centered(|ui| {
                        ui.add_space(daw_ui::tokens::SPACE_XL);
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width().min(540.0), 0.0),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                if has_kind {
                                    daw_ui::text::subtitle(ui, if state.kind == DeviceKind::Instrument {
                                        "No matching instruments"
                                    } else {
                                        "No matching effects"
                                    });
                                    daw_ui::text::note(ui, "Try another search or clear the filters.");
                                } else {
                                    daw_ui::text::subtitle(ui, if state.kind == DeviceKind::Instrument {
                                        "No VST3 instruments found"
                                    } else {
                                        "No VST3 effects found"
                                    });
                                    if state.kind == DeviceKind::Instrument {
                                        daw_ui::text::note(ui, "MIDI notes stay silent until a VST3 instrument is assigned.");
                                    }
                                    daw_ui::text::note(ui, if state.kind == DeviceKind::Instrument {
                                        "Install a VST3 instrument, then Scan. Use Plug-in folders if it is installed elsewhere."
                                    } else {
                                        "Install a VST3 effect, then Scan. Use Plug-in folders if it is installed elsewhere."
                                    });
                                    if daw_ui::button(ui, "Plug-in folders").clicked() {
                                        folders = true;
                                    }
                                }
                                if (!self.plugin_filter.is_empty() || !state.developer.is_empty())
                                    && daw_ui::button(ui, "Clear filters").clicked()
                                {
                                    self.plugin_filter.clear();
                                    state.developer.clear();
                                    state.selected = None;
                                    state.grid_filter = None;
                                    self.plugin_search_focus = true;
                                }
                            },
                        );
                    });
                }
                let mut cards: Vec<_> = grid.cards.into_iter().map(|(_, card)| card).collect();
                for card in &mut cards {
                    card["selected"] = serde_json::json!(card["fingerprint"].as_str() == state.selected.as_deref());
                }
                ui.separator();
                let footer_start = ui.cursor().min;
                let selected = state.selected.as_ref().and_then(|selected| matches.iter().find(|entry| &entry.fingerprint == selected)).copied();
                if let Some(entry) = selected {
                    daw_ui::text::truncated(ui, daw_ui::text::TextRole::Label, &entry.plugin.name);
                } else {
                    ui.add_space(daw_ui::tokens::FONT_BODY + daw_ui::tokens::SPACE_SM);
                }
                let primary_label = match primary {
                    DeviceAction::Instrument => "Add instrument track",
                    DeviceAction::Assign => "Assign instrument",
                    DeviceAction::Insert => "Add to effect chain",
                    DeviceAction::Send => "Create send track",
                };
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(enabled && selected.is_some(), |ui| {
                        if daw_ui::primary_button(ui, primary_label).clicked()
                            && let Some(entry) = selected
                        { action = Some((entry.plugin.clone(), primary)); }
                    });
                    ui.add_enabled_ui(selected.is_some(), |ui| {
                        daw_ui::action_menu(ui, "More", |ui| {
                            let Some(entry) = selected else { return; };
                            ui.set_max_width(360.0);
                            if state.kind == DeviceKind::Instrument {
                                let can_assign = self.selected_track.and_then(|id| self.project.track(id)).is_some_and(|track| track.kind == TrackKind::Midi);
                                if !assign && can_assign && daw_ui::button(ui, "Assign to selected track").clicked() {
                                    action = Some((entry.plugin.clone(), DeviceAction::Assign));
                                    ui.close();
                                }
                            } else if source.is_some() {
                                let (label, alternate) = if primary == DeviceAction::Send { ("Add to effect chain", DeviceAction::Insert) } else { ("Create send track", DeviceAction::Send) };
                                if daw_ui::button(ui, label).clicked() {
                                    action = Some((entry.plugin.clone(), alternate));
                                    ui.close();
                                }
                            }
                            if daw_ui::button(ui, if entry.thumbnail.is_some() { "Refresh preview" } else { "Capture preview" }).clicked() {
                                retry = Some(entry.plugin.clone());
                                ui.close();
                            }
                            ui.separator();
                            daw_ui::text::note(ui, if entry.plugin.category.is_empty() { "Unverified VST3".to_owned() } else { format!("VST3 · {}", entry.plugin.category.replace('|', " · ")) });
                            if let ThumbnailStatus::Failed { message } = &entry.status {
                                daw_ui::text::note(ui, "Preview unavailable").on_hover_text(message);
                            }
                            daw_ui::text::note(ui, entry.plugin.path.display().to_string()).on_hover_text(entry.plugin.class_id.as_deref().unwrap_or("Unverified class"));
                        });
                    });
                    if state.kind == DeviceKind::Instrument {
                        let context = if assign {
                            assignment_target.and_then(|id| self.project.track(id))
                                .map(|track| format!("Assign to {}", track.name))
                                .unwrap_or_default()
                        } else {
                            "Create a new MIDI track".to_owned()
                        };
                        daw_ui::text::truncated(ui, daw_ui::text::TextRole::Note, context);
                    } else {
                        let context = source.and_then(|id| self.project.track(id)).map(|track| {
                            if primary == DeviceAction::Send { format!("Send from {}", track.name) } else { track.name.clone() }
                        }).unwrap_or_else(|| "Select a channel to add effects".to_owned());
                        daw_ui::text::truncated(ui, daw_ui::text::TextRole::Note, context);
                    }
                });
                let footer = egui::Rect::from_min_max(footer_start, ui.min_rect().max);
                ctx.data_mut(|data| data.insert_temp(egui::Id::new("library_diagnostics"), serde_json::json!({
                    "catalog": {"min": [grid.rect.min.x, grid.rect.min.y], "max": [grid.rect.max.x, grid.rect.max.y]},
                    "footer": {"min": [footer.min.x, footer.min.y], "max": [footer.max.x, footer.max.y]},
                    "scroll_offset": grid.scroll_offset,
                    "selected": state.selected, "columns": grid.layout.columns,
                    "card_size": [grid.layout.card_width, grid.layout.row_height - daw_ui::tokens::SPACE_MD],
                    "total": matches.len(), "cards": cards,
                })));
            });
        self.library_open &= open;
        if folders {
            self.open_popup(PopupKind::Setup);
            self.setup_page = SetupPage::Plugins;
        }
        if let Some(plugin) = retry {
            state.queued_capture = Some(plugin);
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if let Some((plugin, action)) = action {
            self.add_library_device(plugin, action, source);
        }
        ctx.data_mut(|data| data.insert_temp(key, state));
    }

    fn add_library_device(
        &mut self,
        plugin: daw_plugins::PluginInfo,
        action: DeviceAction,
        source: Option<u64>,
    ) {
        if matches!(action, DeviceAction::Insert | DeviceAction::Send) {
            let Some(source) = source else {
                return;
            };
            self.begin_edit();
            let track = if action == DeviceAction::Send {
                self.project.add_bus(format!("{} return", plugin.name))
            } else {
                source
            };
            let slot = EffectSlot::new(
                &plugin.name,
                Effect::Vst3 {
                    path: plugin.path.to_string_lossy().into_owned(),
                    class_id: plugin.class_id,
                },
            );
            self.project.track_mut(track).unwrap().inserts.push(slot);
            if action == DeviceAction::Send
                && let Err(error) = self.project.add_send(source, track, 0.25, false)
            {
                self.project.tracks.retain(|item| item.id != track);
                self.message(Err(error), "");
                return;
            }
            self.selected_track = Some(track);
            self.selected_clip = None;
            self.changed();
            self.replace_popup(PopupKind::Effects);
            return;
        }
        let replace = action == DeviceAction::Assign;
        let target = source.filter(|id| {
            self.project
                .track(*id)
                .is_some_and(|track| track.kind == TrackKind::Midi)
        });
        if replace && target.is_none() {
            return;
        }
        self.begin_edit();
        let track = if replace {
            target.unwrap()
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
                class_id: plugin.class_id,
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
            if replace {
                // Assigning a sound completes the chooser. Restore the caller
                // so note entry can continue without dismissing an overview or
                // an automatically launched native editor first.
                self.instrument_autolaunch = None;
                self.close_popup();
            } else {
                self.replace_popup(PopupKind::Instrument);
                self.instrument_autolaunch = Some(track);
            }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::Project;
    #[test]
    fn preview_completion_preserves_the_assign_action_and_track_count() {
        for failed in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let ctx = egui::Context::default();
            let mut project = Project::new();
            let target = project.add_midi_track("Existing MIDI track");
            let mut app = DawApp::with_context(&ctx, project, None);
            app.library_open = true;
            app.selected_track = Some(target);
            let entry = CatalogEntry {
                plugin: daw_plugins::PluginInfo {
                    name: "Selected instrument".into(),
                    path: directory.path().join("Fixture.vst3"),
                    vendor: "Test vendor".into(),
                    category: "Instrument".into(),
                    class_id: Some("11111111111111111111111111111111".into()),
                },
                fingerprint: "fixture".into(),
                thumbnail: None,
                thumbnail_size: None,
                status: ThumbnailStatus::Missing,
            };
            app.vst_catalog = vec![entry.clone()];
            let capture: CaptureJob = Arc::new(Mutex::new(None));
            ctx.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("plugin_library_state"),
                    LibraryUi {
                        selected: Some(entry.fingerprint.clone()),
                        attempted: HashSet::from([entry.fingerprint.clone()]),
                        capture: Some(capture.clone()),
                        ..Default::default()
                    },
                )
            });
            let mut time = 0.0;
            let mut frame = |app: &mut DawApp, events| {
                time += 1.0 / 60.0;
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1600.0, 1000.0),
                        )),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        app.poll_plugin_previews(ctx);
                        app.plugin_library(ctx);
                    },
                )
            };
            fn action_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape
                            && text.galley.text() == label
                        {
                            Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center())
                        } else {
                            None
                        }
                    })
                    .expect("visible library action")
            }
            frame(&mut app, vec![]);
            frame(&mut app, vec![]);
            app.plugin_assign_target = Some(target);
            let loading = frame(&mut app, vec![]);
            let assign = action_position(&loading, "Assign instrument");
            let mut completed = entry.clone();
            if failed {
                completed.status = ThumbnailStatus::Failed {
                    message: "Preview failed".into(),
                };
            } else {
                let png = directory.path().join("preview.png");
                image::RgbaImage::from_pixel(16, 16, image::Rgba([120, 80, 40, 255]))
                    .save(&png)
                    .unwrap();
                completed.thumbnail = Some(png);
                completed.thumbnail_size = Some([16, 16]);
                completed.status = ThumbnailStatus::Ready;
            }
            *capture.lock().unwrap() = Some(Ok(completed));
            let ready = frame(&mut app, vec![]);
            assert!(action_position(&ready, "Assign instrument").distance(assign) <= 1.0);
            for pressed in [true, false] {
                frame(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(assign),
                        egui::Event::PointerButton {
                            pos: assign,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert_eq!(
                app.project.tracks.len(),
                1,
                "Assign must never create a second track"
            );
            assert_eq!(app.selected_track, Some(target));
            assert_eq!(
                app.project.track(target).unwrap().instrument,
                Instrument::Vst3 {
                    path: entry.plugin.path.to_string_lossy().into_owned(),
                    class_id: entry.plugin.class_id,
                }
            );
        }
    }
}

//! Device discovery, cached previews and focused insertion workflows.
use crate::app::{DawApp, PopupKind, SetupPage};
use daw_core::{Clip, Effect, EffectSlot, Instrument, TrackKind};
use daw_plugins::catalog::{CatalogEntry, PluginCatalog, ThumbnailStatus};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) enum LibraryTarget {
    #[default]
    Browse,
    Instrument,
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
    preview: Option<(PathBuf, egui::TextureHandle)>,
    capture: Option<CaptureJob>,
    queued_capture: Option<daw_plugins::PluginInfo>,
    attempted: HashSet<String>,
    thumbnails: HashMap<String, egui::TextureHandle>,
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
            && let Some(plugin) = state.queued_capture.take()
        {
            state.capture = Some(start_capture(plugin));
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if state.capture.is_none()
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
        self.library_target = LibraryTarget::Instrument;
        self.plugin_assign_target = if assign { target } else { None };
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
            .data_mut(|data| data.get_temp::<LibraryUi>(key))
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
        }
        let assign = self.plugin_assign_target.is_some();
        let source = match self.library_target {
            LibraryTarget::Insert(track) | LibraryTarget::Send(track) => Some(track),
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
        let default_action = match state.kind {
            DeviceKind::Instrument if assign => DeviceAction::Assign,
            DeviceKind::Instrument => DeviceAction::Instrument,
            DeviceKind::Effect if matches!(self.library_target, LibraryTarget::Send(_)) => {
                DeviceAction::Send
            }
            DeviceKind::Effect => DeviceAction::Insert,
        };
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
            .min_height(550.0)
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
                ui.horizontal(|ui| {
                    let search = daw_ui::text_field(
                        ui,
                        &mut self.plugin_filter,
                        if state.kind == DeviceKind::Instrument {
                            "Search instruments"
                        } else {
                            "Search effects"
                        },
                        460.0,
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
                    let mut developers: Vec<_> = self
                        .vst_catalog
                        .iter()
                        .filter(|entry| state.kind.accepts(entry))
                        .map(|entry| entry.plugin.vendor.clone())
                        .filter(|vendor| !vendor.is_empty())
                        .collect();
                    developers.sort_unstable();
                    developers.dedup();
                    daw_ui::select(
                        "library_developer",
                        if state.developer.is_empty() {
                            "All developers"
                        } else {
                            &state.developer
                        },
                    )
                    .show_ui(ui, |ui| {
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
                                developer.clone(),
                                developer,
                            );
                        }
                    });
                });
                ui.add_space(8.0);
                let filter = self.plugin_filter.to_lowercase();
                let matches: Vec<_> = self
                    .vst_catalog
                    .iter()
                    .filter(|entry| {
                        state.kind.accepts(entry)
                            && (state.developer.is_empty()
                                || entry.plugin.vendor == state.developer)
                            && format!(
                                "{} {} {}",
                                entry.plugin.name, entry.plugin.vendor, entry.plugin.category
                            )
                            .to_lowercase()
                            .contains(&filter)
                    })
                    .collect();
                if !matches
                    .iter()
                    .any(|entry| state.selected.as_ref() == Some(&entry.fingerprint))
                {
                    state.selected = matches.first().map(|entry| entry.fingerprint.clone());
                    state.preview = None;
                }
                let popup_open = egui::Popup::is_any_open(ctx);
                let step = ctx.input(|input| {
                    if popup_open {
                        0
                    } else if input.key_pressed(egui::Key::ArrowDown) {
                        1
                    } else if input.key_pressed(egui::Key::ArrowUp) {
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
                    state.selected = Some(
                        matches
                            [(index as isize + step).clamp(0, matches.len() as isize - 1) as usize]
                            .fingerprint
                            .clone(),
                    );
                }
                if activate
                    && let Some(entry) = matches
                        .iter()
                        .find(|entry| state.selected.as_ref() == Some(&entry.fingerprint))
                {
                    action = Some((entry.plugin.clone(), default_action));
                }
                ui.columns(2, |columns| {
                    egui::ScrollArea::vertical()
                        .id_salt("device_catalog")
                        .min_scrolled_height(420.0)
                        .max_height(420.0)
                        .show(&mut columns[0], |ui| {
                            for entry in &matches {
                                let plugin = &entry.plugin;
                                let detail = if !plugin.vendor.is_empty() {
                                    plugin.vendor.clone()
                                } else if plugin.is_instrument().is_some() {
                                    plugin.category.replace('|', " · ")
                                } else {
                                    String::new()
                                };
                                let visible = ui.clip_rect().intersects(egui::Rect::from_min_size(
                                    ui.cursor().min,
                                    egui::vec2(ui.available_width(), 58.0),
                                ));
                                if visible
                                    && !state.thumbnails.contains_key(&entry.fingerprint)
                                    && let Some(path) = &entry.thumbnail
                                    && let Ok(image) = image::open(path)
                                {
                                    let image = image.thumbnail(144, 96).to_rgba8();
                                    let pixels = egui::ColorImage::from_rgba_unmultiplied(
                                        [image.width() as usize, image.height() as usize],
                                        image.as_raw(),
                                    );
                                    // Bound GPU memory even when scrolling a very large library.
                                    if state.thumbnails.len() >= 128 {
                                        state.thumbnails.clear();
                                    }
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
                                }
                                if response.double_clicked() {
                                    action = Some((plugin.clone(), default_action));
                                }
                            }
                            if matches.is_empty() && self.vst_scan.is_none() {
                                ui.label("No matching devices");
                            }
                        });
                    let ui = &mut columns[1];
                    if let Some(entry) = state.selected.as_ref().and_then(|selected| {
                        matches.iter().find(|entry| &entry.fingerprint == selected)
                    }) {
                        daw_ui::text::title(ui, &entry.plugin.name);
                        if !entry.plugin.vendor.is_empty() {
                            daw_ui::text::note(ui, &entry.plugin.vendor);
                        }
                        if entry.plugin.is_instrument().is_none() {
                            daw_ui::text::note(ui, "Unverified VST3");
                        }
                        ui.add_space(10.0);
                        // Keep the main action before asynchronous preview content.
                        // Context determines the primary action; alternatives stay on demand.
                        let primary = match state.kind {
                            DeviceKind::Instrument if assign => DeviceAction::Assign,
                            DeviceKind::Instrument => DeviceAction::Instrument,
                            DeviceKind::Effect
                                if matches!(self.library_target, LibraryTarget::Send(_)) =>
                            {
                                DeviceAction::Send
                            }
                            DeviceKind::Effect => DeviceAction::Insert,
                        };
                        let primary_label = match primary {
                            DeviceAction::Instrument => "Add instrument track",
                            DeviceAction::Assign => "Assign instrument",
                            DeviceAction::Insert => "Add to effect chain",
                            DeviceAction::Send => "Create send track",
                        };
                        let enabled = state.kind == DeviceKind::Instrument || source.is_some();
                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(enabled, |ui| {
                                if daw_ui::primary_button(ui, primary_label).clicked() {
                                    action = Some((entry.plugin.clone(), primary));
                                }
                            });
                            daw_ui::action_menu(ui, "More", |ui| {
                                ui.set_max_width(360.0);
                                if state.kind == DeviceKind::Instrument {
                                    let can_assign = self
                                        .selected_track
                                        .and_then(|id| self.project.track(id))
                                        .is_some_and(|track| track.kind == TrackKind::Midi);
                                    if !assign
                                        && can_assign
                                        && daw_ui::button(ui, "Assign to selected track").clicked()
                                    {
                                        action = Some((entry.plugin.clone(), DeviceAction::Assign));
                                        ui.close();
                                    }
                                } else if source.is_some() {
                                    let (label, alternate) = if primary == DeviceAction::Send {
                                        ("Add to effect chain", DeviceAction::Insert)
                                    } else {
                                        ("Create send track", DeviceAction::Send)
                                    };
                                    if daw_ui::button(ui, label).clicked() {
                                        action = Some((entry.plugin.clone(), alternate));
                                        ui.close();
                                    }
                                }
                                if daw_ui::button(
                                    ui,
                                    if entry.thumbnail.is_some() {
                                        "Refresh preview"
                                    } else {
                                        "Capture preview"
                                    },
                                )
                                .clicked()
                                {
                                    retry = Some(entry.plugin.clone());
                                    ui.close();
                                }
                                ui.separator();
                                daw_ui::text::note(
                                    ui,
                                    if entry.plugin.category.is_empty() {
                                        "VST3".to_owned()
                                    } else {
                                        format!(
                                            "VST3 · {}",
                                            entry.plugin.category.replace('|', " · ")
                                        )
                                    },
                                );
                                daw_ui::text::note(ui, entry.plugin.path.display().to_string())
                                    .on_hover_text(
                                        entry
                                            .plugin
                                            .class_id
                                            .as_deref()
                                            .unwrap_or("Unverified class"),
                                    );
                            });
                        });
                        if state.kind == DeviceKind::Effect {
                            if let Some(track) = source.and_then(|id| self.project.track(id)) {
                                daw_ui::text::note(
                                    ui,
                                    if primary == DeviceAction::Send {
                                        format!("Send from {}", track.name)
                                    } else {
                                        track.name.clone()
                                    },
                                );
                            } else {
                                daw_ui::text::note(ui, "Select a channel to add effects");
                            }
                        }
                        ui.add_space(10.0);
                        if let Some(path) = &entry.thumbnail {
                            if state
                                .preview
                                .as_ref()
                                .is_some_and(|(loaded, _)| loaded != path)
                            {
                                state.preview = None;
                            }
                            if state.preview.is_none()
                                && let Ok(image) = image::open(path)
                            {
                                let image = image.thumbnail(1000, 600).to_rgba8();
                                let pixels = egui::ColorImage::from_rgba_unmultiplied(
                                    [image.width() as usize, image.height() as usize],
                                    image.as_raw(),
                                );
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
                                    egui::vec2(ui.available_width(), 250.0),
                                );
                            }
                        }
                        if let ThumbnailStatus::Failed { message } = &entry.status {
                            daw_ui::text::note(ui, "Preview unavailable").on_hover_text(message);
                        }
                    }
                });
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
        let target = self
            .plugin_assign_target
            .or(self.selected_track)
            .filter(|id| {
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
            self.replace_popup(PopupKind::Instrument);
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

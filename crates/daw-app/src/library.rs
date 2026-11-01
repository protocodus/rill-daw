//! Device discovery, cached previews and focused insertion workflows. Scanning,
//! indexing and preview capture run in `daw_plugins::sync`; this view reads its
//! snapshots and the user's favorites and categories.
use crate::app::{DawApp, PopupKind, SetupPage};
use crate::library_preview::PreviewCache;
use crate::settings::PluginLibrary;
use anyhow::Context as _;
use daw_core::{Clip, Effect, EffectSlot, Instrument, TrackKind};
use daw_plugins::{
    PluginInfo,
    catalog::{CatalogEntry, ThumbnailStatus},
    sync::SyncPhase,
};
use daw_ui::tokens::{SPACE_MD, SPACE_SM};
use eframe::egui;
use std::collections::{BTreeMap, BTreeSet, HashSet};

const SIDEBAR_WIDTH: f32 = 184.0;
const DETAILS_WIDTH: f32 = 300.0;
/// Two card columns and the grid's scrollbar gutter.
const CATALOG_MIN_WIDTH: f32 = 2.0 * daw_ui::DEVICE_CARD_MIN_WIDTH + SPACE_MD + 16.0;
const PANE_GAP: f32 = daw_ui::tokens::SPACE_LG;
const PREVIEW_IMAGE_HELP: &str = "An optional image of the plug-in editor. Image capture does not verify whether the plug-in can load or play; assignment remains available.";

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) enum LibraryTarget {
    #[default]
    Browse,
    Instrument {
        track: Option<u64>,
        /// Only the chooser opened as part of creating a track may replace its
        /// provisional name. Existing names never imply naming permission.
        name_new_track: bool,
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

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
enum Collection {
    #[default]
    All,
    Favorites,
    WithPreview,
    WithoutPreview,
}

impl Collection {
    const ALL: [Self; 4] = [
        Self::All,
        Self::Favorites,
        Self::WithPreview,
        Self::WithoutPreview,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All plug-ins",
            Self::Favorites => "Favorites",
            Self::WithPreview => "With preview",
            Self::WithoutPreview => "Without preview",
        }
    }

    fn accepts(self, item: &Item<'_>) -> bool {
        match self {
            Self::All => true,
            Self::Favorites => item.favorite,
            Self::WithPreview => item.entry.status == ThumbnailStatus::Ready,
            Self::WithoutPreview => item.entry.status != ThumbnailStatus::Ready,
        }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
enum Sort {
    #[default]
    Name,
    Developer,
    Recent,
}

impl Sort {
    const ALL: [Self; 3] = [Self::Name, Self::Developer, Self::Recent];

    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Developer => "Developer",
            Self::Recent => "Recently added",
        }
    }
}

/// Everything that changes which devices are listed, or where.
type GridFilter = (DeviceKind, String, Collection, String, String, Sort, bool);

#[derive(Clone, Default)]
struct LibraryUi {
    activate_when_ready: bool,
    kind: DeviceKind,
    target: LibraryTarget,
    collection: Collection,
    developer: String,
    category: String,
    sort: Sort,
    list: bool,
    selected: Option<String>,
    search_id: Option<egui::Id>,
    category_draft: Option<CategoryDraft>,
    attempted: HashSet<String>,
    previews: PreviewCache,
    grid_filter: Option<GridFilter>,
    grid_columns: usize,
}

/// The selected device's categories field. `saved` is what it was filled
/// from, so changes made elsewhere replace an unedited draft.
#[derive(Clone)]
struct CategoryDraft {
    fingerprint: String,
    saved: String,
    text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeviceAction {
    Instrument,
    Assign,
    Insert,
    Send,
}

/// A catalog entry with the user's organization resolved for this frame.
struct Item<'a> {
    entry: &'a CatalogEntry,
    favorite: bool,
    categories: Vec<String>,
    custom_categories: bool,
}

pub(crate) enum LibraryEdit {
    Favorite(PluginInfo, bool),
    /// Comma-separated categories; empty restores the plug-in's own.
    Categories(PluginInfo, String),
}

impl DeviceKind {
    fn accepts(self, entry: &CatalogEntry) -> bool {
        match self {
            Self::Instrument => entry.plugin.is_instrument() != Some(false),
            Self::Effect => entry.plugin.is_effect() != Some(false),
        }
    }
}

fn device_id(fingerprint: &str) -> egui::Id {
    egui::Id::new(("library_device", fingerprint))
}

const KEY_TARGETS: &str = "library_key_targets";

/// Enter and arrows choose devices only from the search field, the selected
/// device, or with nothing focused; other fields and buttons keep their keys.
pub(crate) fn library_owns_keys(ctx: &egui::Context) -> bool {
    let targets = ctx
        .data(|data| data.get_temp::<[Option<egui::Id>; 2]>(egui::Id::new(KEY_TARGETS)))
        .unwrap_or_default();
    ctx.memory(|memory| memory.focused())
        .is_none_or(|focused| targets.contains(&Some(focused)))
}

/// VST3 subcategories without the instrument/effect markers, e.g. `Fx|Delay`
/// becomes `Delay`.
fn automatic_categories(plugin: &PluginInfo) -> Vec<String> {
    plugin
        .category
        .split('|')
        .filter(|part| {
            !part.is_empty()
                && !part.eq_ignore_ascii_case("Fx")
                && !part.eq_ignore_ascii_case("Instrument")
        })
        .map(str::to_owned)
        .collect()
}

fn parse_categories(text: &str) -> Vec<String> {
    let mut categories: Vec<String> = Vec::new();
    for category in text
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if !categories
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(category))
        {
            categories.push(category.to_owned());
        }
    }
    categories
}

fn kind_label(plugin: &PluginInfo) -> &'static str {
    match (plugin.is_instrument(), plugin.is_effect()) {
        (Some(true), _) => "Instrument",
        (_, Some(true)) => "Effect",
        (None, _) => "Unverified VST3",
        _ => "Other",
    }
}

fn item<'a>(library: &PluginLibrary, entry: &'a CatalogEntry) -> Item<'a> {
    let key = PluginLibrary::key(&entry.plugin);
    let custom = library.categories.get(&key);
    Item {
        entry,
        favorite: library.favorites.contains(&key),
        categories: custom
            .cloned()
            .unwrap_or_else(|| automatic_categories(&entry.plugin)),
        custom_categories: custom.is_some(),
    }
}

impl DawApp {
    pub(crate) fn plugin_favorite(&self, plugin: &PluginInfo) -> bool {
        self.plugin_library
            .favorites
            .contains(&PluginLibrary::key(plugin))
    }

    pub(crate) fn plugin_categories(&self, plugin: &PluginInfo) -> Vec<String> {
        self.plugin_library
            .categories
            .get(&PluginLibrary::key(plugin))
            .cloned()
            .unwrap_or_else(|| automatic_categories(plugin))
    }

    /// Shared by the library view and the `organize_plugin` tool. A change
    /// that cannot be saved is undone, so the session never shows it alone.
    pub(crate) fn edit_plugin_library(&mut self, edit: LibraryEdit) -> anyhow::Result<()> {
        let previous = self.plugin_library.clone();
        match edit {
            LibraryEdit::Favorite(plugin, favorite) => {
                let key = PluginLibrary::key(&plugin);
                if favorite {
                    self.plugin_library.favorites.insert(key);
                } else {
                    self.plugin_library.favorites.remove(&key);
                }
            }
            LibraryEdit::Categories(plugin, text) => {
                let key = PluginLibrary::key(&plugin);
                let categories = parse_categories(&text);
                // Empty or unchanged input returns to the plug-in's own
                // categories, which then follow its updates.
                if categories.is_empty() || categories == automatic_categories(&plugin) {
                    self.plugin_library.categories.remove(&key);
                } else {
                    self.plugin_library.categories.insert(key, categories);
                }
            }
        }
        let saved = self
            .plugin_library
            .save()
            .context("Saving the plug-in library");
        if saved.is_err() {
            self.plugin_library = previous;
        }
        saved
    }

    /// Adopt the latest catalog snapshot and report sync problems. Discovery
    /// stays metadata-only; editors open only in isolated capture helpers.
    pub(crate) fn poll_plugin_catalog(&mut self, ctx: &egui::Context) {
        if let Some((revision, entries)) =
            self.plugin_sync.entries_since(self.plugin_catalog_revision)
        {
            self.plugin_catalog_revision = revision;
            self.vst_paths = entries
                .iter()
                .map(|entry| entry.plugin.path.clone())
                .collect();
            self.vst_paths.sort();
            self.vst_paths.dedup();
            self.vst_catalog = entries;
        }
        for error in self.plugin_sync.take_errors() {
            self.message(Err(anyhow::anyhow!(error)), "");
        }
        if !self.plugin_scan_reported && !self.plugin_sync.scanning() {
            self.plugin_scan_reported = true;
            if !self.error {
                self.status = format!("Found {} VST3 plug-ins", self.vst_catalog.len());
            }
        }
        let key = egui::Id::new("plugin_library_state");
        let mut state = ctx
            .data_mut(|data| data.remove_temp::<LibraryUi>(key))
            .unwrap_or_default();
        for fingerprint in self.plugin_sync.take_captured() {
            state.previews.invalidate(&fingerprint);
        }
        if !self.library_open {
            state.activate_when_ready = false;
        }
        // Verified devices are captured by the sync itself. An unverified
        // bundle is only opened once the user selects it.
        if self.automatic_plugin_previews
            && self.library_open
            && let Some(entry) = self.vst_catalog.iter().find(|entry| {
                state.selected.as_ref() == Some(&entry.fingerprint)
                    && entry.plugin.is_instrument().is_none()
                    && entry.status == ThumbnailStatus::Missing
            })
            && state.attempted.insert(entry.fingerprint.clone())
        {
            self.plugin_sync.capture(entry.plugin.clone());
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
            name_new_track: false,
        };
    }

    pub(crate) fn open_new_instrument_chooser(&mut self) {
        self.open_instrument_chooser(true);
        if let LibraryTarget::Instrument { name_new_track, .. } = &mut self.library_target {
            *name_new_track = true;
        }
    }

    fn available_instrument_name(&self, name: &str, target: Option<u64>) -> String {
        let name = name.trim();
        let name = if name.is_empty() { "Instrument" } else { name };
        for index in 1.. {
            let candidate = if index == 1 {
                name.to_owned()
            } else {
                format!("{name} {index}")
            };
            if !self
                .project
                .tracks
                .iter()
                .any(|track| Some(track.id) != target && track.name == candidate)
            {
                return candidate;
            }
        }
        unreachable!()
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
            state.category.clear();
            state.grid_filter = None;
        }
        // The chooser owns its assignment destination. A native preview or
        // later selection change must never turn Assign into Add, or send it
        // to a different MIDI track.
        let assignment_target = match self.library_target {
            LibraryTarget::Instrument { track, .. } => track,
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
            LibraryTarget::Instrument {
                track: Some(track), ..
            } => Some(track),
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
        let mut refresh = false;
        let mut edit = None;
        let mut copy = None;
        let mut open = true;
        let search_focused = state
            .search_id
            .is_some_and(|id| ctx.memory(|memory| memory.has_focus(id)));
        let device_keys = library_owns_keys(ctx);
        let pressed_enter = ctx
            .data_mut(|data| data.remove_temp::<bool>(egui::Id::new("instrument_chooser_enter")))
            .unwrap_or(false)
            || (!egui::Popup::is_any_open(ctx)
                && device_keys
                && ctx.input(|input| input.key_pressed(egui::Key::Enter)));
        state.activate_when_ready |= pressed_enter;
        let scanning = self.plugin_sync.scanning();
        let status = self.plugin_sync.status();
        let activate = state.activate_when_ready && !scanning;
        if activate {
            state.activate_when_ready = false;
        }
        daw_ui::popup("Device library", ctx)
            .id(egui::Id::new("library"))
            .open(&mut open)
            .default_width(1240.0)
            .default_height(760.0)
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
                        state.category.clear();
                        self.plugin_filter.clear();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        daw_ui::action_menu(ui, "Manage", |ui| {
                            if daw_ui::button(ui, "Folders").clicked() {
                                folders = true;
                                ui.close();
                            }
                        });
                        refresh = ui
                            .add_enabled(
                                !scanning,
                                daw_ui::button_widget(if scanning { "Scanning…" } else { "Refresh" }),
                            )
                            .on_hover_text("Rescan plug-in folders and capture missing previews")
                            .clicked();
                        let text = match status.phase {
                            SyncPhase::Capturing => format!(
                                "Capturing previews · {} left",
                                status.queued + usize::from(status.current.is_some())
                            ),
                            _ => format!("{} plug-ins", self.vst_catalog.len()),
                        };
                        daw_ui::text::truncated(ui, daw_ui::text::TextRole::Note, text);
                    });
                });
                let kind_items: Vec<_> = self
                    .vst_catalog
                    .iter()
                    .filter(|entry| state.kind.accepts(entry))
                    .map(|entry| item(&self.plugin_library, entry))
                    .collect();
                let categories: BTreeSet<_> =
                    kind_items.iter().flat_map(|item| &item.categories).collect();
                // Trailing choices are laid out right to left, so the search
                // field takes the remaining width without wrapping the row.
                let search = daw_ui::action_row(
                    ui,
                    |ui| {
                        daw_ui::view_menu(ui, "View", |ui| {
                            if daw_ui::selectable_value(ui, &mut state.list, false, "Grid").clicked()
                                || daw_ui::selectable_value(ui, &mut state.list, true, "List").clicked()
                            {
                                ui.close();
                            }
                        });
                        daw_ui::select("library_sort", format!("Sort: {}", state.sort.label()))
                            .show_ui(ui, |ui| {
                                for sort in Sort::ALL {
                                    daw_ui::selectable_value(ui, &mut state.sort, sort, sort.label());
                                }
                            });
                        daw_ui::select(
                            "library_category",
                            if state.category.is_empty() { "All categories" } else { &state.category },
                        )
                        .show_ui(ui, |ui| {
                            daw_ui::selectable_value(ui, &mut state.category, String::new(), "All categories");
                            for category in categories {
                                daw_ui::selectable_value(ui, &mut state.category, category.clone(), category);
                            }
                        });
                    },
                    |ui| {
                        let search = daw_ui::text_field(
                            ui,
                            &mut self.plugin_filter,
                            if state.kind == DeviceKind::Instrument {
                                "Search instruments"
                            } else {
                                "Search effects"
                            },
                            ui.available_width().min(460.0),
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
                        search
                    },
                )
                .inner
                .1;
                state.search_id = Some(search.id);
                let chips = [
                    (state.collection != Collection::All).then(|| state.collection.label().to_owned()),
                    (!state.developer.is_empty()).then(|| state.developer.clone()),
                    (!state.category.is_empty()).then(|| state.category.clone()),
                ];
                if chips.iter().any(Option::is_some) {
                    ui.horizontal_wrapped(|ui| {
                        for (filter, chip) in chips.iter().enumerate() {
                            if let Some(chip) = chip
                                && daw_ui::filter_chip(ui, chip).clicked()
                            {
                                match filter {
                                    0 => state.collection = Collection::All,
                                    1 => state.developer.clear(),
                                    _ => state.category.clear(),
                                }
                            }
                        }
                        if daw_ui::small_button(ui, "Clear filters").clicked() {
                            state.collection = Collection::All;
                            state.developer.clear();
                            state.category.clear();
                            self.plugin_filter.clear();
                        }
                    });
                }
                ui.add_space(SPACE_SM);
                let filter = self.plugin_filter.to_lowercase();
                // A filtered-out device needs a different next step from an
                // inventory with no instruments/effects of the requested type.
                let has_kind = !kind_items.is_empty();
                let mut matches: Vec<&Item> = kind_items
                    .iter()
                    .filter(|item| {
                        state.collection.accepts(item)
                            && (state.developer.is_empty()
                                || item.entry.plugin.vendor == state.developer)
                            && (state.category.is_empty()
                                || item.categories.contains(&state.category))
                            && (filter.is_empty() || format!(
                                "{} {} {} {}",
                                item.entry.plugin.name,
                                item.entry.plugin.vendor,
                                item.entry.plugin.category,
                                item.categories.join(" ")
                            )
                            .to_lowercase()
                            .contains(&filter))
                    })
                    .collect();
                let name = |item: &Item| item.entry.plugin.name.to_lowercase();
                match state.sort {
                    Sort::Name => matches.sort_by_cached_key(|item| name(item)),
                    Sort::Developer => matches
                        .sort_by_cached_key(|item| (item.entry.plugin.vendor.to_lowercase(), name(item))),
                    Sort::Recent => matches.sort_by_cached_key(|item| {
                        (std::cmp::Reverse(item.entry.added_unix_ms), name(item))
                    }),
                }
                let signature = (
                    state.kind,
                    self.plugin_filter.clone(),
                    state.collection,
                    state.developer.clone(),
                    state.category.clone(),
                    state.sort,
                    state.list,
                );
                let filter_changed = state.grid_filter.as_ref() != Some(&signature);
                state.grid_filter = Some(signature);
                if !matches.iter().any(|item| state.selected.as_ref() == Some(&item.entry.fingerprint)) {
                    state.selected = matches.first().map(|item| item.entry.fingerprint.clone());
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
                let width = ui.available_width();
                let sidebar = if width >= SIDEBAR_WIDTH + PANE_GAP + CATALOG_MIN_WIDTH { SIDEBAR_WIDTH } else { 0.0 };
                let details = if width >= sidebar + DETAILS_WIDTH + 2.0 * PANE_GAP + CATALOG_MIN_WIDTH { DETAILS_WIDTH } else { 0.0 };
                let catalog_width = width
                    - if sidebar > 0.0 { sidebar + PANE_GAP } else { 0.0 }
                    - if details > 0.0 { details + PANE_GAP } else { 0.0 };
                let layout = if state.list {
                    daw_ui::device_list_layout(catalog_width)
                } else {
                    daw_ui::device_grid_layout(catalog_width)
                };
                let reflowed = state.grid_columns != layout.columns;
                state.grid_columns = layout.columns;
                let popup_open = egui::Popup::is_any_open(ctx);
                let step = ctx.input_mut(|input| {
                    if popup_open || !device_keys { return 0; }
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
                    matches.iter().position(|item| state.selected.as_ref() == Some(&item.entry.fingerprint))
                } else {
                    None
                };
                if step != 0 && !matches.is_empty() {
                    let index = matches.iter().position(|item| state.selected.as_ref() == Some(&item.entry.fingerprint)).unwrap_or(0);
                    let next = if (step == -(layout.columns as isize) && index < layout.columns)
                        || (step == layout.columns as isize && index / layout.columns == (matches.len() - 1) / layout.columns)
                    {
                        index
                    } else {
                        (index as isize + step).clamp(0, matches.len() as isize - 1) as usize
                    };
                    state.selected = Some(matches[next].entry.fingerprint.clone());
                    scroll_to = Some(next);
                }
                if activate && enabled
                    && let Some(item) = matches.iter().find(|item| state.selected.as_ref() == Some(&item.entry.fingerprint))
                {
                    action = Some((item.entry.plugin.clone(), primary));
                }
                let icon = if state.kind == DeviceKind::Instrument { daw_ui::Icon::Instrument } else { daw_ui::Icon::Effect };
                // A fixed footer keeps the insertion target stable as previews
                // arrive, filters change, and long device names wrap on cards.
                // Includes separator spacing, the selected name and action row.
                // A reserve smaller than their total feeds window growth back
                // into this available-height calculation on every frame.
                let body_height = (ui.available_height() - 84.0).max(0.0);
                let mut grid = None;
                let mut sidebar_rect = None;
                let mut details_rect = None;
                let developers = kind_items.iter().filter(|item| !item.entry.plugin.vendor.is_empty()).fold(
                    BTreeMap::<&str, usize>::new(),
                    |mut developers, item| {
                        *developers.entry(&item.entry.plugin.vendor).or_default() += 1;
                        developers
                    },
                );
                daw_ui::bounded_panes(ui, "library_panes", body_height, [sidebar, details], |pane, ui| match pane {
                    0 => {
                        sidebar_rect = Some(ui.max_rect());
                        egui::ScrollArea::vertical().id_salt("library_sidebar").auto_shrink([false, false]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            daw_ui::theme::label(ui, "LIBRARY");
                            for collection in Collection::ALL {
                                let count = kind_items.iter().filter(|item| collection.accepts(item)).count();
                                let response = daw_ui::count_item(ui, state.collection == collection, collection.label(), count);
                                let response = if matches!(collection, Collection::WithPreview | Collection::WithoutPreview) {
                                    response.on_hover_text(PREVIEW_IMAGE_HELP)
                                } else {
                                    response
                                };
                                if response.clicked() {
                                    state.collection = collection;
                                }
                            }
                            ui.add_space(SPACE_MD);
                            daw_ui::theme::label(ui, "DEVELOPERS");
                            if daw_ui::count_item(ui, state.developer.is_empty(), "All developers", kind_items.len()).clicked() {
                                state.developer.clear();
                            }
                            for (developer, count) in &developers {
                                if daw_ui::count_item(ui, state.developer == *developer, developer, *count).clicked() {
                                    state.developer = (*developer).to_owned();
                                }
                            }
                        });
                    }
                    1 => {
                        let mut render = |ui: &mut egui::Ui, index: usize, width: f32| {
                            let item = matches[index];
                            let entry = item.entry;
                            let texture = entry.thumbnail.as_ref().and_then(|path| state.previews.request(ctx, &entry.fingerprint, path));
                            let selected = state.selected.as_ref() == Some(&entry.fingerprint);
                            let device = daw_ui::DeviceCard {
                                id: device_id(&entry.fingerprint),
                                name: &entry.plugin.name,
                                producer: &entry.plugin.vendor,
                                preview: texture.as_ref(),
                                selected,
                                favorite: item.favorite,
                                icon,
                            };
                            let card = if state.list {
                                let detail = if item.categories.is_empty() { kind_label(&entry.plugin).to_owned() } else { item.categories.join(" · ") };
                                daw_ui::device_row(ui, &device, &detail, width)
                            } else {
                                daw_ui::device_card(ui, &device, width)
                            };
                            if step != 0 && selected {
                                card.request_focus();
                            }
                            if card.response.clicked() {
                                state.selected = Some(entry.fingerprint.clone());
                            }
                            card.response.context_menu(|ui| {
                                let label = if item.favorite { "Remove from favorites" } else { "Add to favorites" };
                                if daw_ui::button(ui, label).clicked() {
                                    edit = Some(LibraryEdit::Favorite(entry.plugin.clone(), !item.favorite));
                                    ui.close();
                                }
                            });
                            if card.activated && enabled {
                                action = Some((entry.plugin.clone(), primary));
                            }
                            let rect = card.response.rect;
                            let preview = card.preview_rect;
                            serde_json::json!({
                                "fingerprint": entry.fingerprint, "name": entry.plugin.name,
                                "producer": if entry.plugin.vendor.is_empty() { "Unknown developer" } else { &entry.plugin.vendor },
                                "rect": {"min": [rect.min.x, rect.min.y], "max": [rect.max.x, rect.max.y]},
                                "preview_rect": {"min": [preview.min.x, preview.min.y], "max": [preview.max.x, preview.max.y]},
                                "preview": texture.is_some(), "texture_id": texture.as_ref().map(|texture| format!("{:?}", texture.id())),
                                "favorite": item.favorite,
                            })
                        };
                        let height = ui.available_height();
                        let result = if state.list {
                            daw_ui::device_list(ui, "device_catalog", matches.len(), height, scroll_to, &mut render)
                        } else {
                            daw_ui::device_grid(ui, "device_catalog", matches.len(), height, scroll_to, &mut render)
                        };
                        if matches.is_empty() && !scanning {
                            let mut empty = ui.new_child(egui::UiBuilder::new().max_rect(result.rect));
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
                                                "Install a VST3 instrument, then Refresh. Use Plug-in folders if it is installed elsewhere."
                                            } else {
                                                "Install a VST3 effect, then Refresh. Use Plug-in folders if it is installed elsewhere."
                                            });
                                            if daw_ui::button(ui, "Plug-in folders").clicked() {
                                                folders = true;
                                            }
                                        }
                                        if (!self.plugin_filter.is_empty() || chips.iter().any(Option::is_some))
                                            && daw_ui::button(ui, "Clear filters").clicked()
                                        {
                                            self.plugin_filter.clear();
                                            state.collection = Collection::All;
                                            state.developer.clear();
                                            state.category.clear();
                                            state.selected = None;
                                            state.grid_filter = None;
                                            self.plugin_search_focus = true;
                                        }
                                    },
                                );
                            });
                        }
                        grid = Some(result);
                    }
                    _ => {
                        details_rect = Some(ui.max_rect());
                        let Some(item) = matches.iter().find(|item| state.selected.as_ref() == Some(&item.entry.fingerprint)) else {
                            return;
                        };
                        let entry = item.entry;
                        let saved = item.categories.join(", ");
                        if state.category_draft.as_ref().is_none_or(|draft| {
                            draft.fingerprint != entry.fingerprint
                                || (draft.text == draft.saved && draft.saved != saved)
                        }) {
                            state.category_draft = Some(CategoryDraft {
                                fingerprint: entry.fingerprint.clone(),
                                text: saved.clone(),
                                saved,
                            });
                        }
                        egui::ScrollArea::vertical().id_salt("library_details").auto_shrink([false, false]).show(ui, |ui| {
                            let width = ui.available_width();
                            let texture = entry.thumbnail.as_ref().and_then(|path| state.previews.request(ctx, &entry.fingerprint, path));
                            daw_ui::device_preview(ui, texture.as_ref(), icon, egui::vec2(width, width * 0.6));
                            daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Title, &entry.plugin.name, 2);
                            daw_ui::text::truncated(ui, daw_ui::text::TextRole::Note, if entry.plugin.vendor.is_empty() { "Unknown developer" } else { &entry.plugin.vendor });
                            let mut favorite = item.favorite;
                            if daw_ui::favorite_toggle(ui, &mut favorite).changed() {
                                edit = Some(LibraryEdit::Favorite(entry.plugin.clone(), favorite));
                            }
                            ui.add_space(SPACE_SM);
                            daw_ui::theme::label(ui, "INSTALL PATH");
                            daw_ui::path_row(ui, &entry.plugin.path, |ui| {
                                if daw_ui::button(ui, "Copy path").clicked() {
                                    copy = Some(entry.plugin.path.display().to_string());
                                }
                            });
                            ui.add_space(SPACE_SM);
                            daw_ui::theme::label(ui, "CATEGORIES");
                            if let Some(CategoryDraft { text: draft, .. }) = &mut state.category_draft {
                                let (save, field) = daw_ui::action_row(
                                    ui,
                                    |ui| daw_ui::button(ui, "Save").clicked(),
                                    |ui| {
                                        daw_ui::text_field(ui, draft, "Compressor, Vintage", ui.available_width())
                                            .on_hover_text("Separate categories with commas")
                                    },
                                )
                                .inner;
                                let submitted = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                                if save || submitted {
                                    edit = Some(LibraryEdit::Categories(entry.plugin.clone(), draft.clone()));
                                }
                            }
                            if matches!(edit, Some(LibraryEdit::Categories(..))) {
                                // Show the saved, normalized categories next frame.
                                state.category_draft = None;
                            }
                            daw_ui::text::note(ui, if item.custom_categories {
                                "Yours · save empty to reset"
                            } else {
                                "From the plug-in"
                            });
                            ui.add_space(SPACE_SM);
                            egui::Grid::new("library_device_details").num_columns(2).min_col_width(88.0).min_row_height(0.0).spacing(egui::vec2(SPACE_MD, daw_ui::tokens::SPACE_XS)).show(ui, |ui| {
                                let mut row = |label: &str, value: String| {
                                    daw_ui::text::note(ui, label);
                                    daw_ui::text::body(ui, value);
                                    ui.end_row();
                                };
                                row("Format", "VST3".into());
                                row("Kind", kind_label(&entry.plugin).into());
                                row("Version", if entry.plugin.version.is_empty() { "Unknown".into() } else { entry.plugin.version.clone() });
                                row("Architecture", if entry.plugin.architectures.is_empty() { "Unknown".into() } else { entry.plugin.architectures.join(", ") });
                                row("Preview image", match &entry.status {
                                    ThumbnailStatus::Ready => "Captured".into(),
                                    ThumbnailStatus::Missing => "Not captured".into(),
                                    ThumbnailStatus::Failed { .. } => "Unavailable".into(),
                                });
                            });
                            if let ThumbnailStatus::Failed { message } = &entry.status {
                                daw_ui::text::note(ui, "Optional image; assignment remains available.")
                                    .on_hover_text(PREVIEW_IMAGE_HELP);
                                daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note, message, 3)
                                    .on_hover_text(format!("Preview image capture: {message}"));
                            }
                        });
                    }
                });
                let grid = grid.expect("the catalog pane is always shown");
                let mut cards: Vec<_> = grid.cards.into_iter().map(|(_, card)| card).collect();
                for card in &mut cards {
                    card["selected"] = serde_json::json!(card["fingerprint"].as_str() == state.selected.as_deref());
                }
                ui.separator();
                let footer_start = ui.cursor().min;
                let selected = state.selected.as_ref().and_then(|selected| matches.iter().find(|item| &item.entry.fingerprint == selected)).copied();
                if let Some(item) = selected {
                    daw_ui::text::truncated(ui, daw_ui::text::TextRole::Label, &item.entry.plugin.name);
                } else {
                    ui.add_space(daw_ui::tokens::FONT_BODY + SPACE_SM);
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
                            && let Some(item) = selected
                        { action = Some((item.entry.plugin.clone(), primary)); }
                    });
                    ui.add_enabled_ui(selected.is_some(), |ui| {
                        daw_ui::action_menu(ui, "More", |ui| {
                            let Some(item) = selected else { return; };
                            let entry = item.entry;
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
                            if daw_ui::button(ui, if item.favorite { "Remove from favorites" } else { "Add to favorites" }).clicked() {
                                edit = Some(LibraryEdit::Favorite(entry.plugin.clone(), !item.favorite));
                                ui.close();
                            }
                            if daw_ui::button(ui, if entry.thumbnail.is_some() { "Refresh preview" } else { "Capture preview" })
                                .on_hover_text("Capture an optional image of the plug-in editor. This does not assign or replace the plug-in.")
                                .clicked() {
                                retry = Some(entry.plugin.clone());
                                ui.close();
                            }
                            if let ThumbnailStatus::Failed { message } = &entry.status {
                                daw_ui::text::note(ui, "Image unavailable")
                                    .on_hover_text(format!("{PREVIEW_IMAGE_HELP}\n\nPreview image capture: {message}"));
                            }
                            if details == 0.0 {
                                // Without room for the detail pane, keep its facts reachable.
                                ui.separator();
                                daw_ui::text::note(ui, if entry.plugin.category.is_empty() { "Unverified VST3".to_owned() } else { format!("VST3 · {}", entry.plugin.category.replace('|', " · ")) });
                                daw_ui::text::note(ui, entry.plugin.path.display().to_string()).on_hover_text(entry.plugin.class_id.as_deref().unwrap_or("Unverified class"));
                            }
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
                let rect = |rect: egui::Rect| serde_json::json!({"min": [rect.min.x, rect.min.y], "max": [rect.max.x, rect.max.y]});
                ctx.data_mut(|data| data.insert_temp(egui::Id::new("library_diagnostics"), serde_json::json!({
                    "catalog": rect(grid.rect), "footer": rect(footer),
                    "sidebar": sidebar_rect.map(rect), "details": details_rect.map(rect),
                    "scroll_offset": grid.scroll_offset,
                    "selected": state.selected, "columns": grid.layout.columns,
                    "card_size": [grid.layout.card_width, grid.layout.row_height - if state.list { daw_ui::tokens::SPACE_XS } else { daw_ui::tokens::SPACE_MD }],
                    "view": if state.list { "list" } else { "grid" },
                    "collection": state.collection.label(), "developer": state.developer,
                    "category": state.category, "sort": state.sort.label(),
                    "total": matches.len(), "cards": cards,
                    "order": matches.iter().map(|item| &item.entry.plugin.name).collect::<Vec<_>>(),
                    "sync": {
                        "scanning": scanning, "phase": format!("{:?}", status.phase),
                        "captured": status.captured, "queued": status.queued,
                        "current": status.current, "synced_unix_ms": status.synced_unix_ms,
                    },
                })));
            });
        self.library_open &= open;
        if folders {
            self.open_popup(PopupKind::Setup);
            self.setup_page = SetupPage::Plugins;
        }
        if refresh {
            self.refresh_plugins();
        }
        if let Some(plugin) = retry {
            self.plugin_sync.capture(plugin);
        }
        if let Some(edit) = edit
            && let Err(error) = self.edit_plugin_library(edit)
        {
            self.message(Err(error), "");
        }
        if let Some(path) = copy {
            ctx.copy_text(path);
        }
        if let Some((plugin, action)) = action {
            self.add_library_device(plugin, action, source);
        }
        let targets = [state.search_id, state.selected.as_deref().map(device_id)];
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new(KEY_TARGETS), targets);
            data.insert_temp(key, state);
        });
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
        let name_new_track = replace
            && matches!(
                self.library_target,
                LibraryTarget::Instrument {
                    track,
                    name_new_track: true,
                } if track == target
            )
            && target
                .and_then(|id| self.project.track(id))
                .is_some_and(|track| {
                    track.name == "New instrument" && track.instrument == Instrument::None
                });
        let instrument_name = self
            .available_instrument_name(&plugin.name, if name_new_track { target } else { None });
        self.begin_edit();
        let track = if replace {
            target.unwrap()
        } else {
            let id = self.project.add_midi_track(&instrument_name);
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
            if name_new_track {
                self.project.track_mut(track).unwrap().name = instrument_name;
            }
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

    fn naming_app(name: &str) -> (DawApp, u64) {
        let ctx = egui::Context::default();
        let mut project = Project::new();
        let track = project.add_midi_track(name);
        let mut app = DawApp::with_context(&ctx, project, None);
        app.audio_device_enabled = false;
        app.selected_track = Some(track);
        (app, track)
    }

    fn naming_plugin() -> PluginInfo {
        PluginInfo {
            name: "Test synth".into(),
            path: "/catalog/Test.vst3".into(),
            category: "Instrument|Synth".into(),
            class_id: Some("11111111111111111111111111111111".into()),
            ..PluginInfo::default()
        }
    }

    #[test]
    fn initial_assignment_names_the_track_in_the_same_undo_step() {
        let (mut app, track) = naming_app("New instrument");
        let before = app.project.clone();
        app.open_new_instrument_chooser();
        app.add_library_device(naming_plugin(), DeviceAction::Assign, Some(track));
        app.flush_pending_edit();
        assert_eq!(app.project.track(track).unwrap().name, "Test synth");
        assert_eq!(app.history.undo_labels().count(), 1);
        let assigned = app.project.clone();
        app.undo_redo(false);
        assert_eq!(app.project, before);
        app.undo_redo(true);
        assert_eq!(app.project, assigned);
    }

    #[test]
    fn existing_names_and_edits_during_the_initial_chooser_are_preserved() {
        for name in ["New instrument", "My lead", "Test synth"] {
            let (mut app, track) = naming_app(name);
            app.open_instrument_chooser(true);
            app.add_library_device(naming_plugin(), DeviceAction::Assign, Some(track));
            assert_eq!(app.project.track(track).unwrap().name, name);
        }
        let (mut app, track) = naming_app("New instrument");
        app.open_new_instrument_chooser();
        app.project.track_mut(track).unwrap().name = "My lead".into();
        app.add_library_device(naming_plugin(), DeviceAction::Assign, Some(track));
        assert_eq!(app.project.track(track).unwrap().name, "My lead");
    }

    #[test]
    fn instrument_names_skip_used_numbers_and_only_exclude_the_renamed_track() {
        let (mut app, track) = naming_app("Test synth");
        app.project.add_audio_track("Test synth 2");
        assert_eq!(
            app.available_instrument_name("Test synth", None),
            "Test synth 3"
        );
        assert_eq!(
            app.available_instrument_name("Test synth", Some(track)),
            "Test synth"
        );
        // Explicit Add instrument must count the selected track as occupied.
        app.add_library_device(naming_plugin(), DeviceAction::Instrument, Some(track));
        assert_eq!(app.project.tracks.last().unwrap().name, "Test synth 3");
    }

    #[test]
    fn categories_come_from_vst3_metadata_until_the_user_names_their_own() {
        let plugin = PluginInfo {
            category: "Fx|Delay|Reverb".into(),
            ..PluginInfo::default()
        };
        assert_eq!(automatic_categories(&plugin), ["Delay", "Reverb"]);
        assert_eq!(
            parse_categories(" Vintage, , vintage ,Tape ,"),
            ["Vintage", "Tape"]
        );
        let instrument = PluginInfo {
            category: "Instrument".into(),
            ..PluginInfo::default()
        };
        assert!(automatic_categories(&instrument).is_empty());
    }

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
                    ..Default::default()
                },
                fingerprint: "fixture".into(),
                thumbnail: None,
                thumbnail_size: None,
                status: ThumbnailStatus::Missing,
                added_unix_ms: 0,
            };
            app.vst_catalog = vec![entry.clone()];
            ctx.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("plugin_library_state"),
                    LibraryUi {
                        selected: Some(entry.fingerprint.clone()),
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
                    |ctx| app.plugin_library(ctx),
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
            // A background capture publishes a new catalog snapshot.
            app.vst_catalog = vec![completed];
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

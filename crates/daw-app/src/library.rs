//! Device discovery, cached previews and focused insertion workflows. Scanning,
//! indexing and preview capture run in `daw_plugins::sync`; this view reads its
//! snapshots and the user's favorites and categories.
use crate::app::{DawApp, PopupKind, SetupPage};
use crate::library_preview::PreviewCache;
use crate::settings::PluginLibrary;
use anyhow::Context as _;
use daw_core::{Clip, Effect, EffectSlot, Instrument, TrackKind, TrackPlacement};
use daw_plugins::{
    PluginInfo,
    catalog::{CatalogEntry, ThumbnailStatus},
    sync::SyncPhase,
};
use daw_ui::tokens::{SPACE_MD, SPACE_SM};
use eframe::egui;
use std::collections::{BTreeSet, HashSet};

const SIDEBAR_WIDTH: f32 = 196.0;
const DETAILS_WIDTH: f32 = 233.0;
/// Two card columns and the grid's scrollbar gutter.
const CATALOG_MIN_WIDTH: f32 = 440.0;
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
enum LibrarySection {
    #[default]
    Instrument,
    Effect,
    Unknown,
    Sample,
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
enum Collection {
    #[default]
    All,
    Builtin,
    RecentlyUsed,
    Favorites,
}

impl Collection {
    const ALL: [Self; 4] = [
        Self::All,
        Self::Favorites,
        Self::RecentlyUsed,
        Self::Builtin,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All devices",
            Self::Builtin => "Built-in (R1)",
            Self::RecentlyUsed => "Recently used",
            Self::Favorites => "Favorites",
        }
    }

    fn accepts(self, item: &Item<'_>) -> bool {
        match self {
            Self::All => true,
            Self::Builtin => crate::builtin_devices::is_builtin(&item.entry.plugin),
            Self::RecentlyUsed => item.recent_position.is_some(),
            Self::Favorites => item.favorite,
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
type GridFilter = (
    LibrarySection,
    String,
    Collection,
    String,
    String,
    Sort,
    bool,
);

#[derive(Clone)]
struct LibraryUi {
    generation: u64,
    organize: bool,
    kind: LibrarySection,
    target: LibraryTarget,
    collection: Collection,
    developer: String,
    developer_query: String,
    category: String,
    category_query: String,
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

impl Default for LibraryUi {
    fn default() -> Self {
        Self {
            generation: 0,
            organize: false,
            kind: LibrarySection::default(),
            target: LibraryTarget::default(),
            collection: Collection::default(),
            developer: String::new(),
            developer_query: String::new(),
            category: String::new(),
            category_query: String::new(),
            sort: Sort::default(),
            list: true,
            selected: None,
            search_id: None,
            category_draft: None,
            attempted: HashSet::new(),
            previews: PreviewCache::default(),
            grid_filter: None,
            grid_columns: 0,
        }
    }
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

fn action_hint(action: DeviceAction) -> &'static str {
    match action {
        DeviceAction::Instrument => "Create a MIDI track with the selected instrument",
        DeviceAction::Assign => "Replace the destination MIDI track’s instrument with this device",
        DeviceAction::Insert => "Add this effect to the destination channel’s effect chain",
        DeviceAction::Send => {
            "Create an effect return track and route a send from the destination channel to it"
        }
    }
}

/// A catalog entry with the user's organization resolved for this frame.
struct Item<'a> {
    entry: &'a CatalogEntry,
    favorite: bool,
    recent_position: Option<usize>,
    categories: Vec<String>,
    custom_categories: bool,
}

pub(crate) enum LibraryEdit {
    Favorite(PluginInfo, bool),
    /// Comma-separated categories; empty restores the plug-in's own.
    Categories(PluginInfo, String),
}

impl LibrarySection {
    fn accepts(self, entry: &CatalogEntry) -> bool {
        match self {
            Self::Instrument => entry.plugin.is_instrument() == Some(true),
            Self::Effect => entry.plugin.is_effect() == Some(true),
            Self::Sample => false,
            Self::Unknown => {
                entry.plugin.is_instrument() != Some(true) && entry.plugin.is_effect() != Some(true)
            }
        }
    }
}

/// Built-in devices ship their image; hosted plug-ins use their capture.
fn preview(
    ctx: &egui::Context,
    previews: &PreviewCache,
    entry: &CatalogEntry,
) -> Option<egui::TextureHandle> {
    crate::builtin_devices::preview(ctx, &entry.plugin).or_else(|| {
        entry
            .thumbnail
            .as_ref()
            .and_then(|path| previews.request(ctx, &entry.fingerprint, path))
    })
}

fn device_id(fingerprint: &str) -> egui::Id {
    egui::Id::new(("library_device", fingerprint))
}

const KEY_TARGETS: &str = "library_key_targets";

/// Enter and arrows choose devices only from the search field, the selected
/// device, or with nothing focused; other fields and buttons keep their keys.
pub(crate) fn samples_visible(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<LibraryUi>(egui::Id::new("plugin_library_state")))
        .is_some_and(|state| state.kind == LibrarySection::Sample)
}

pub(crate) fn library_owns_keys(ctx: &egui::Context) -> bool {
    if samples_visible(ctx) {
        return false;
    }
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
        (None, _) => match plugin.format {
            daw_plugins::PluginFormat::Vst3 => "Unverified VST3",
            daw_plugins::PluginFormat::Clap => "Unverified CLAP",
            daw_plugins::PluginFormat::Lv2 => "Unverified LV2",
            daw_plugins::PluginFormat::Au => "Unverified AU",
        },
        _ => "Other",
    }
}

fn item<'a>(library: &PluginLibrary, entry: &'a CatalogEntry) -> Item<'a> {
    let key = PluginLibrary::key(&entry.plugin);
    let custom = library.categories.get(&key);
    Item {
        entry,
        favorite: library.favorites.contains(&key),
        recent_position: library
            .recently_used
            .iter()
            .position(|recent| recent == &key),
        categories: custom
            .cloned()
            .unwrap_or_else(|| automatic_categories(&entry.plugin)),
        custom_categories: custom.is_some(),
    }
}

fn reset_filters(state: &mut LibraryUi, query: &mut String) {
    state.collection = Collection::All;
    state.developer.clear();
    state.developer_query.clear();
    state.category.clear();
    state.category_query.clear();
    query.clear();
    state.grid_filter = None;
}

#[derive(Clone)]
struct LibraryDetails {
    entry: CatalogEntry,
    state: LibraryUi,
    error: Option<String>,
    backdrop: Option<daw_ui::ViewBackdrop>,
}

pub(crate) fn details_open(ctx: &egui::Context) -> bool {
    ctx.data(|data| {
        data.get_temp::<LibraryDetails>(egui::Id::new("library_details_popup"))
            .is_some()
    })
}

pub(crate) fn close_details(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<LibraryDetails>(egui::Id::new("library_details_popup"));
    });
}

#[derive(Default)]
struct InspectorResponse {
    edit: Option<LibraryEdit>,
    technical: bool,
}

fn inspector(
    ui: &mut egui::Ui,
    state: &mut LibraryUi,
    item: &Item<'_>,
    show_name: bool,
) -> InspectorResponse {
    let entry = item.entry;
    let texture = preview(ui.ctx(), &state.previews, entry);
    daw_ui::device_browser_identity(
        ui,
        &daw_ui::DeviceBrowserIdentity {
            name: if show_name { &entry.plugin.name } else { "" },
            developer: &entry.plugin.vendor,
            kind_format: &format!(
                "{} · {}",
                kind_label(&entry.plugin),
                crate::builtin_devices::format_label(&entry.plugin)
            ),
            categories: &item.categories.join(" · "),
            preview: texture.as_ref(),
            icon: if entry.plugin.is_instrument() == Some(true) {
                daw_ui::Icon::Instrument
            } else {
                daw_ui::Icon::Effect
            },
        },
    );
    let mut result = InspectorResponse::default();
    let mut favorite = item.favorite;
    if daw_ui::favorite_toggle(ui, &mut favorite).changed() {
        result.edit = Some(LibraryEdit::Favorite(entry.plugin.clone(), favorite));
    }
    let organize_clicked = daw_ui::view_button(ui, "Organize").clicked();
    if organize_clicked {
        state.organize = !state.organize;
    }
    if state.organize {
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
        let categories_heading = daw_ui::text::section_heading(ui, "Categories");
        if organize_clicked {
            categories_heading.scroll_to_me(Some(egui::Align::Min));
        }
        if let Some(draft) = &mut state.category_draft {
            let field = daw_ui::text_field(
                ui,
                &mut draft.text,
                "Compressor, Vintage",
                ui.available_width(),
            )
            .on_hover_text("Separate categories with commas");
            let submitted =
                field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            ui.horizontal_wrapped(|ui| {
                if daw_ui::button(ui, "Save").clicked() || submitted {
                    result.edit = Some(LibraryEdit::Categories(
                        entry.plugin.clone(),
                        draft.text.clone(),
                    ));
                }
                if daw_ui::button(ui, "Reset categories").clicked() {
                    result.edit =
                        Some(LibraryEdit::Categories(entry.plugin.clone(), String::new()));
                }
            });
        }
        daw_ui::text::note(
            ui,
            if item.custom_categories {
                "Your categories"
            } else {
                "From the device"
            },
        );
    }
    if daw_ui::view_button(ui, "Device details")
        .on_hover_text("Open the selected device’s information and controls")
        .clicked()
    {
        result.technical = true;
    }
    result
}

impl DawApp {
    pub(crate) fn record_plugin_use(&mut self, plugin: &PluginInfo) {
        let previous = self.plugin_library.clone();
        self.plugin_library.record_use(plugin);
        // In-process unit apps have no isolated settings directory. Persistence
        // is exercised by the real-app subprocess suite with its own config.
        #[cfg(test)]
        if std::env::var_os("RILL_CONFIG_DIR").is_none() {
            return;
        }
        if let Err(error) = self
            .plugin_library
            .save()
            .context("Saving recently used plug-ins")
        {
            self.plugin_library = previous;
            self.message(Err(error), "");
        }
    }

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
                self.status = format!(
                    "Found {}",
                    daw_ui::text::count(
                        self.vst_catalog.len() + crate::builtin_devices::catalog().len(),
                        "device",
                        "devices"
                    )
                );
            }
        }
        let key = egui::Id::new("plugin_library_state");
        let mut state = ctx
            .data_mut(|data| data.remove_temp::<LibraryUi>(key))
            .unwrap_or_default();
        for fingerprint in self.plugin_sync.take_captured() {
            state.previews.invalidate(&fingerprint);
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
        self.new_track_after = None;
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
        let previous_section = state.kind;
        if state.target != self.library_target || state.generation != self.library_generation {
            state.target = self.library_target;
            state.generation = self.library_generation;
            state.grid_filter = None;
            state.category_draft = None;
            state.organize = false;
            if !matches!(state.target, LibraryTarget::Browse) {
                state.selected = None;
                state.kind = if matches!(
                    state.target,
                    LibraryTarget::Insert(_) | LibraryTarget::Send(_)
                ) {
                    LibrarySection::Effect
                } else {
                    LibrarySection::Instrument
                };
                self.plugin_filter.clear();
                state.developer.clear();
                state.developer_query.clear();
                state.category.clear();
                state.category_query.clear();
                state.collection = Collection::All;
            }
        }
        if std::mem::take(&mut self.sample_browser.requested) {
            state.kind = LibrarySection::Sample;
        }
        if previous_section == LibrarySection::Sample && state.kind != LibrarySection::Sample {
            self.reset_sample_browser_context();
            self.stop_sample_preview();
        }
        let pinned = match self.library_target {
            LibraryTarget::Instrument { track, .. } => track,
            LibraryTarget::Insert(track) | LibraryTarget::Send(track) => Some(track),
            LibraryTarget::Browse => self.library_source,
        };
        let assign = matches!(
            self.library_target,
            LibraryTarget::Instrument { track: Some(_), .. }
        );
        let assignment_target = pinned.filter(|id| {
            self.project
                .track(*id)
                .is_some_and(|track| track.kind == TrackKind::Midi)
        });
        let source = pinned.filter(|id| {
            self.project.track(*id).is_some_and(|track| {
                matches!(
                    track.kind,
                    TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                )
            })
        });
        let can_send = source.is_some_and(|id| id != daw_core::MASTER_CHANNEL_ID);
        let mut action = None;
        let mut folders = false;
        let mut retry = None;
        let mut refresh = false;
        let mut edit = None;
        let mut diagnostic_request = None;
        let mut details_request = None;
        let mut summary_request = None;
        let mut open = true;
        let search_focused = state
            .search_id
            .is_some_and(|id| ctx.memory(|memory| memory.has_focus(id)));
        let device_keys = library_owns_keys(ctx);
        // This event belongs only to this frame's visible selection. Scans must
        // never retain Enter and later apply it to newly discovered devices.
        let activate = ctx
            .data_mut(|data| data.remove_temp::<bool>(egui::Id::new("instrument_chooser_enter")))
            .unwrap_or(false)
            || (!egui::Popup::is_any_open(ctx)
                && device_keys
                && ctx.input(|input| input.key_pressed(egui::Key::Enter)));
        let scanning = self.plugin_sync.scanning();
        let status = self.plugin_sync.status();
        daw_ui::page("Device browser").id(egui::Id::new("library")).open(&mut open).show(ctx, |ui| {
            let compact_height = ui.max_rect().height() < 500.0;
            if compact_height { ui.spacing_mut().item_spacing.y = daw_ui::tokens::SPACE_XS; }
            let mut kind = state.kind;
            let pages = [
                (LibrarySection::Instrument, "Instruments", "Browse instruments that turn MIDI notes into sound"),
                (LibrarySection::Effect, "Effects", "Browse effects for processing audio on a channel"),
                (LibrarySection::Sample, "Samples", "Find, preview, and use sounds from your sample library or project"),
                (LibrarySection::Unknown, "Unverified", "Inspect plug-ins whose instrument or effect type has not been verified"),
            ];
            let manage = |ui: &mut egui::Ui| {
                if state.kind == LibrarySection::Sample { return; }
                if scanning || status.phase == SyncPhase::Capturing {
                    ui.spinner().on_hover_text(if scanning { "Scanning plug-ins…" } else { "Capturing optional preview images…" });
                }
                daw_ui::view_menu(ui, "Manage plug-ins", |ui| {
                    if ui.add_enabled(!scanning, daw_ui::button_widget("Refresh")).on_hover_text("Rescan plug-in folders and capture missing previews").on_disabled_hover_text("A plug-in scan is already in progress").clicked() { refresh = true; ui.close(); }
                    if daw_ui::button(ui, "Plug-in folders…").on_hover_text("Choose the folders RILL scans for installed plug-ins").clicked() { folders = true; ui.close(); }
                }).response.on_hover_text("Manage plug-in folders and scan for installed devices");
            };
            if compact_height {
                daw_ui::action_row(ui, manage, |ui| {
                    for (value, label, hint) in pages {
                        if daw_ui::tab_choice(ui, kind == value, label).on_hover_text(hint).clicked() { kind = value; }
                    }
                });
            } else { daw_ui::popup_tabs_with_hints(ui, &mut kind, &pages, manage); }
            if kind != state.kind {
                if state.kind == LibrarySection::Sample {
                    self.reset_sample_browser_context();
                    self.stop_sample_preview();
                }
                state.kind = kind; state.selected = None;
                state.developer.clear(); state.developer_query.clear();
                state.category.clear(); state.category_query.clear();
            }
            if state.kind == LibrarySection::Sample {
                self.sample_browser_contents(ui);
                return;
            }
            let search = daw_ui::device_browser_search(ui, &mut self.plugin_filter, match state.kind {
                LibrarySection::Instrument => "Search instruments", LibrarySection::Effect => "Search effects", LibrarySection::Unknown => "Search unverified devices", LibrarySection::Sample => unreachable!("Samples uses the shared sample browser"),
            }).search.on_hover_text("Search name, category, developer or format; combine words in any order");
            if self.plugin_search_focus {
                if search.has_focus() { self.plugin_search_focus = false; } else { search.request_focus(); }
            }
            state.search_id = Some(search.id);
            let kind_items: Vec<_> = crate::builtin_devices::catalog().iter().chain(&self.vst_catalog)
                .filter(|entry| state.kind.accepts(entry)).map(|entry| item(&self.plugin_library, entry)).collect();
            let width = ui.available_width();
            let sidebar = if width >= SIDEBAR_WIDTH + PANE_GAP + CATALOG_MIN_WIDTH { SIDEBAR_WIDTH } else { 0.0 };
            let details = if width >= sidebar + DETAILS_WIDTH + 2.0 * PANE_GAP + CATALOG_MIN_WIDTH { DETAILS_WIDTH } else { 0.0 };
            let categories: BTreeSet<_> = kind_items.iter().flat_map(|item| &item.categories).collect();
            let developers: BTreeSet<_> = kind_items.iter().map(|item| &item.entry.plugin.vendor).filter(|value| !value.is_empty()).collect();
            ui.horizontal_wrapped(|ui| {
                ui.set_min_height(daw_ui::tokens::CONTROL_HEIGHT);
                ui.spacing_mut().item_spacing.x = SPACE_SM;
                if sidebar == 0.0 {
                    daw_ui::toolbar_group_sized(ui, daw_ui::tokens::SELECT_WIDTH, |ui| {
                        daw_ui::view_menu(ui, "Collection", |ui| {
                            for collection in Collection::ALL {
                                if daw_ui::selectable_value(ui, &mut state.collection, collection, collection.label()).clicked() { ui.close(); }
                            }
                        }).response.on_hover_text(state.collection.label());
                    });
                }
                daw_ui::toolbar_group_sized(ui, daw_ui::tokens::SELECT_WIDTH, |ui| {
                    daw_ui::searchable_select(ui, "library_category", &mut state.category, &mut state.category_query, "All categories", "Search categories", categories).on_hover_text("Filter devices by their instrument or effect category");
                });
                daw_ui::toolbar_group_sized(ui, 175.0, |ui| {
                    daw_ui::searchable_select(ui, "library_developer", &mut state.developer, &mut state.developer_query, "All developers", "Search developers", developers).on_hover_text("Filter devices by the developer or manufacturer");
                });
                daw_ui::toolbar_group_sized(ui, 175.0, |ui| {
                    ui.add_enabled_ui(state.collection != Collection::RecentlyUsed, |ui| {
                        daw_ui::choice_menu(ui, if state.collection == Collection::RecentlyUsed { "Latest use first".to_owned() } else { format!("Sort: {}", state.sort.label()) }, |ui| {
                            for sort in Sort::ALL { if daw_ui::selectable_value(ui, &mut state.sort, sort, sort.label()).clicked() { ui.close(); } }
                        }).response.on_hover_text("Sort devices by name, developer, or recently added").on_disabled_hover_text("Recently used devices are listed latest first");
                    });
                });
                daw_ui::view_menu(ui, "View", |ui| {
                    if daw_ui::selectable_value(ui, &mut state.list, true, "List").clicked()
                        || daw_ui::selectable_value(ui, &mut state.list, false, "Grid").clicked() { ui.close(); }
                }).response.on_hover_text("Switch between grid and list view");
            });
            let filter = self.plugin_filter.to_lowercase();
            let terms: Vec<_> = filter.split_whitespace().collect();
            let mut matches: Vec<&Item> = kind_items.iter().filter(|item| {
                state.collection.accepts(item)
                    && (state.developer.is_empty() || item.entry.plugin.vendor == state.developer)
                    && (state.category.is_empty() || item.categories.contains(&state.category))
                    && (terms.is_empty() || {
                        let searchable = format!("{} {} {} {} {}", item.entry.plugin.name, item.entry.plugin.vendor, item.entry.plugin.category, item.categories.join(" "), crate::builtin_devices::format_label(&item.entry.plugin)).to_lowercase();
                        terms.iter().all(|term| searchable.contains(term))
                    })
            }).collect();
            let name = |item: &Item| item.entry.plugin.name.to_lowercase();
            if state.collection == Collection::RecentlyUsed { matches.sort_by_cached_key(|item| (item.recent_position, name(item))); }
            else { match state.sort {
                Sort::Name => matches.sort_by_cached_key(|item| name(item)),
                Sort::Developer => matches.sort_by_cached_key(|item| (item.entry.plugin.vendor.to_lowercase(), name(item))),
                Sort::Recent => matches.sort_by_cached_key(|item| (std::cmp::Reverse(item.entry.added_unix_ms), name(item))),
            } }
            let signature = (state.kind, self.plugin_filter.clone(), state.collection, state.developer.clone(), state.category.clone(), state.sort, state.list);
            let filter_changed = state.grid_filter.as_ref() != Some(&signature);
            state.grid_filter = Some(signature);
            let selection_changed = !matches.iter().any(|item| state.selected.as_ref() == Some(&item.entry.fingerprint));
            if selection_changed {
                state.selected = matches.first().map(|item| item.entry.fingerprint.clone());
                state.organize = false;
            }
            let primary = match state.kind {
                LibrarySection::Instrument if assign => DeviceAction::Assign,
                LibrarySection::Instrument => DeviceAction::Instrument,
                LibrarySection::Effect if matches!(self.library_target, LibraryTarget::Send(_)) => DeviceAction::Send,
                LibrarySection::Effect => DeviceAction::Insert,
                LibrarySection::Unknown if assign => DeviceAction::Assign,
                LibrarySection::Unknown if matches!(self.library_target, LibraryTarget::Send(_)) => DeviceAction::Send,
                LibrarySection::Unknown if matches!(self.library_target, LibraryTarget::Insert(_)) => DeviceAction::Insert,
                LibrarySection::Unknown => DeviceAction::Instrument,
                LibrarySection::Sample => unreachable!("Samples uses the shared sample browser"),
            };
            let enabled = state.kind != LibrarySection::Unknown && match primary {
                DeviceAction::Assign => assignment_target.is_some(), DeviceAction::Instrument => state.kind != LibrarySection::Unknown,
                DeviceAction::Insert => source.is_some(), DeviceAction::Send => can_send,
            };
            let primary_label = match primary {
                DeviceAction::Instrument => "Add instrument track",
                DeviceAction::Assign if matches!(self.library_target, LibraryTarget::Instrument { name_new_track: true, .. }) => "Set instrument",
                DeviceAction::Assign => "Replace instrument",
                DeviceAction::Insert => "Add to effect chain", DeviceAction::Send => "Create send track",
            };
            let destination = match primary {
                DeviceAction::Instrument => "New MIDI track".to_owned(),
                DeviceAction::Assign => assignment_target.and_then(|id| self.project.track(id)).map(|track| {
                    if matches!(self.library_target, LibraryTarget::Instrument { name_new_track: true, .. }) { track.name.clone() }
                    else { format!("{} · {}", track.name, instrument_model(&self.vst_catalog, &track.instrument).unwrap_or_else(|| "No instrument".into())) }
                }).unwrap_or_else(|| "Destination no longer available".into()),
                DeviceAction::Insert | DeviceAction::Send => source.and_then(|id| self.project.track(id)).map(|track| {
                    if primary == DeviceAction::Send {
                        if can_send { format!("Send from {}", track.name) } else { "Master cannot send to a return".into() }
                    } else { format!("Insert on {}", track.name) }
                }).unwrap_or_else(|| if pinned.is_some() { "Destination no longer available".into() } else { "Choose a channel to add an effect".into() }),
            };
            let scan_status = if scanning { Some("Scanning plug-ins…".to_owned()) } else if status.phase == SyncPhase::Capturing {
                Some(format!("Capturing images · {} left", status.queued + usize::from(status.current.is_some())))
            } else if state.collection == Collection::RecentlyUsed { Some("Most recently used first".to_owned()) } else { None };
            let (singular, plural) = match state.kind { LibrarySection::Instrument => ("instrument", "instruments"), LibrarySection::Effect => ("effect", "effects"), LibrarySection::Unknown => ("device", "devices"), LibrarySection::Sample => unreachable!("Samples uses the shared sample browser") };
            let has_filters = state.collection != Collection::All || !state.developer.is_empty() || !state.category.is_empty();
            if has_filters {
                ui.horizontal_wrapped(|ui| {
                    if compact_height { daw_ui::text::label(ui, daw_ui::text::count(matches.len(), singular, plural)); }

                    if state.collection != Collection::All && daw_ui::filter_chip(ui, state.collection.label()).clicked() { state.collection = Collection::All; }
                    if !state.developer.is_empty() && daw_ui::filter_chip(ui, &state.developer).clicked() { state.developer.clear(); state.developer_query.clear(); }
                    if !state.category.is_empty() && daw_ui::filter_chip(ui, &state.category).clicked() { state.category.clear(); state.category_query.clear(); }
                    if daw_ui::quiet_button(ui, "Reset filters").on_hover_text("Clear the search, collection, developer, and category filters; keep the device type").clicked() {
                        reset_filters(&mut state, &mut self.plugin_filter);
                        self.plugin_search_focus = true;
                    }

                });
            }
            if !compact_height || !has_filters { daw_ui::device_browser_results_heading(ui, matches.len(), singular, plural, scan_status.as_deref()); }
            let catalog_width = width - if sidebar > 0.0 { sidebar + PANE_GAP } else { 0.0 } - if details > 0.0 { details + PANE_GAP } else { 0.0 };
            let layout = if state.list { daw_ui::device_list_layout(catalog_width) } else { daw_ui::device_grid_layout(catalog_width) };
            let reflowed = state.grid_columns != layout.columns;
            state.grid_columns = layout.columns;
            let popup_open = egui::Popup::is_any_open(ctx);
            let step = ctx.input_mut(|input| {
                if popup_open || !device_keys { return 0; }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) { layout.columns as isize }
                else if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) { -(layout.columns as isize) }
                else if !search_focused && input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight) { 1 }
                else if !search_focused && input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft) { -1 } else { 0 }
            });
            let mut scroll_to = (filter_changed || reflowed || selection_changed).then(|| matches.iter().position(|item| state.selected.as_ref() == Some(&item.entry.fingerprint))).flatten();
            if step != 0 && !matches.is_empty() {
                let index = matches.iter().position(|item| state.selected.as_ref() == Some(&item.entry.fingerprint)).unwrap_or(0);
                let next = if (step == -(layout.columns as isize) && index < layout.columns) || (step == layout.columns as isize && index / layout.columns == (matches.len() - 1) / layout.columns) { index }
                    else { (index as isize + step).clamp(0, matches.len() as isize - 1) as usize };
                state.selected = Some(matches[next].entry.fingerprint.clone()); scroll_to = Some(next);
            }
            if activate && enabled && let Some(item) = matches.iter().find(|item| state.selected.as_ref() == Some(&item.entry.fingerprint)) {
                action = Some((item.entry.plugin.clone(), primary));
            }
            let icon = if state.kind == LibrarySection::Instrument { daw_ui::Icon::Instrument } else { daw_ui::Icon::Effect };
            let mut footer_slot = daw_ui::popup_footer(ui, "library_actions");
            let window_ui = ui;
            let ui = footer_slot.body();
            let body_height = daw_ui::popup_scroll_height(ui, 0.0);
            let mut grid = None;
            let mut sidebar_rect = None;
            let mut details_rect = None;
            daw_ui::bounded_panes(ui, "library_panes", body_height, [sidebar, details], |pane, ui| match pane {
                0 => {
                    sidebar_rect = Some(ui.max_rect());
                    egui::ScrollArea::vertical().id_salt("library_sidebar").auto_shrink([false, false]).show(ui, |ui| {
                        daw_ui::text::section_heading(ui, "Library");
                        for collection in Collection::ALL {
                            let count = kind_items.iter().filter(|item| collection.accepts(item)).count();
                            if daw_ui::count_item(ui, state.collection == collection, collection.label(), count).clicked() { state.collection = collection; }
                        }
                    });
                }
                1 => {
                    let mut render = |ui: &mut egui::Ui, index: usize, width: f32| {
                        let item = matches[index]; let entry = item.entry;
                        let texture = preview(ctx, &state.previews, entry);
                        let selected = state.selected.as_ref() == Some(&entry.fingerprint);
                        let device = daw_ui::DeviceCard { id: device_id(&entry.fingerprint), name: &entry.plugin.name, producer: &entry.plugin.vendor, preview: texture.as_ref(), selected, favorite: item.favorite, icon, format: crate::builtin_devices::format_label(&entry.plugin) };
                        let card = if state.list { let detail = if item.categories.is_empty() { kind_label(&entry.plugin).to_owned() } else { item.categories.join(" · ") }; daw_ui::device_row(ui, &device, &detail, width) } else { daw_ui::device_card(ui, &device, width) };
                        if step != 0 && selected { card.request_focus(); }
                        if card.response.clicked() { state.selected = Some(entry.fingerprint.clone()); }
                        card.response.context_menu(|ui| {
                            if daw_ui::button(ui, if item.favorite { "Remove from favorites" } else { "Add to favorites" }).clicked() { edit = Some(LibraryEdit::Favorite(entry.plugin.clone(), !item.favorite)); ui.close(); }
                            if daw_ui::button(ui, "Details").clicked() { summary_request = Some(entry.clone()); ui.close(); }
                        });
                        if card.activated && enabled { action = Some((entry.plugin.clone(), primary)); }
                        let rect = card.response.rect; let image = card.preview_rect;
                        serde_json::json!({"fingerprint":entry.fingerprint,"name":entry.plugin.name,"producer":if entry.plugin.vendor.is_empty(){"Unknown developer"}else{&entry.plugin.vendor},"rect":{"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]},"preview_rect":{"min":[image.min.x,image.min.y],"max":[image.max.x,image.max.y]},"preview":texture.is_some(),"texture_id":texture.as_ref().map(|texture|format!("{:?}",texture.id())),"favorite":item.favorite,"format":crate::builtin_devices::format_label(&entry.plugin)})
                    };
                    let height = ui.available_height();
                    let result = if state.list { daw_ui::device_list(ui, "device_catalog", matches.len(), height, scroll_to, &mut render) } else { daw_ui::device_grid(ui, "device_catalog", matches.len(), height, scroll_to, &mut render) };
                    if matches.is_empty() {
                        let mut empty = ui.new_child(egui::UiBuilder::new().max_rect(result.rect));
                        let filtered = !self.plugin_filter.is_empty() || !state.category.is_empty() || !state.developer.is_empty();
                        let title = if filtered { format!("No matching {plural}") }
                            else if state.collection == Collection::Favorites { format!("No favorite {plural} yet") }
                            else if state.collection == Collection::RecentlyUsed { format!("No recently used {plural}") }
                            else if state.kind == LibrarySection::Unknown { "No unverified devices".into() }
                            else if scanning { "Scanning plug-ins…".into() } else { format!("No {plural} found") };
                        empty.vertical_centered(|ui| {
                            ui.add_space(daw_ui::tokens::SPACE_LG);
                            daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Subtitle, title, 2);
                            if filtered {
                                daw_ui::text::note(ui, "Try another search or reset the filters.");
                                if !self.plugin_filter.is_empty() && daw_ui::button(ui, "Clear search").clicked() { self.plugin_filter.clear(); self.plugin_search_focus = true; }
                                if daw_ui::button(ui, "Reset filters").on_hover_text("Clear the search, collection, developer, and category filters; keep the device type").clicked() { reset_filters(&mut state, &mut self.plugin_filter); self.plugin_search_focus = true; }
                            } else if matches!(state.collection, Collection::Favorites | Collection::RecentlyUsed) {
                                daw_ui::text::note(ui, if state.collection == Collection::Favorites { "Favorite a device from Details or its context menu." } else { "Devices you add appear here." });
                                if daw_ui::button(ui, "Browse all").clicked() { state.collection = Collection::All; }
                            } else if state.kind != LibrarySection::Unknown && !scanning {
                                daw_ui::text::note(ui, "Check your plug-in folders, then refresh.");
                                if daw_ui::button(ui, "Plug-in folders…").on_hover_text("Choose the folders RILL scans for installed plug-ins").clicked() { folders = true; }
                                if daw_ui::button(ui, "Refresh").clicked() { refresh = true; }
                            }
                        });
                    }
                    grid = Some(result);
                }
                _ => {
                    details_rect = Some(ui.max_rect());
                    if let Some(item) = matches.iter().find(|item| state.selected.as_ref() == Some(&item.entry.fingerprint)) {
                        egui::ScrollArea::vertical().id_salt("library_details").auto_shrink([false, false]).show(ui, |ui| {
                            let result = inspector(ui, &mut state, item, true);
                            if result.edit.is_some() { edit = result.edit; }
                            if result.technical { details_request = Some(item.entry.clone()); }
                        });
                    }
                }
            });
            let grid = grid.expect("catalog pane");
            let mut cards: Vec<_> = grid.cards.into_iter().map(|(_, card)| card).collect();
            for card in &mut cards { card["selected"] = serde_json::json!(card["fingerprint"].as_str() == state.selected.as_deref()); }
            let footer = footer_slot.show(window_ui, |ui| {
                let selected = matches.iter().find(|item| state.selected.as_ref() == Some(&item.entry.fingerprint)).copied();
                daw_ui::dialog_actions(ui, |ui| {
                    if state.kind != LibrarySection::Unknown {
                        ui.add_enabled_ui(enabled && selected.is_some(), |ui| {
                            if daw_ui::dialog_primary(ui, primary_label).on_hover_text(action_hint(primary)).on_disabled_hover_text(if selected.is_none() { "Select a device first" } else { &destination }).clicked() && let Some(item) = selected { action = Some((item.entry.plugin.clone(), primary)); }
                        });
                    }
                    ui.add_enabled_ui(selected.is_some(), |ui| {
                        daw_ui::action_menu(ui, "More", |ui| {
                            let Some(item) = selected else { return; }; let entry = item.entry;
                            if state.kind == LibrarySection::Unknown {
                                if ui.add_enabled(!assign || assignment_target.is_some(), daw_ui::button_widget(if assign { "Try replacing instrument" } else if matches!(self.library_target, LibraryTarget::Insert(_) | LibraryTarget::Send(_)) { "Try as new instrument track" } else { "Try as instrument" })).on_hover_text("Type is unverified; the isolated host will validate the device").clicked() { action = Some((entry.plugin.clone(), if assign { DeviceAction::Assign } else { DeviceAction::Instrument })); ui.close(); }
                                let send = matches!(self.library_target, LibraryTarget::Send(_));
                                if ui.add_enabled(if send { can_send } else { source.is_some() }, daw_ui::button_widget(if send { "Try as send effect" } else { "Try as effect" })).on_hover_text("Try this unverified device on the destination channel; the isolated host will validate its type").clicked() { action = Some((entry.plugin.clone(), if send { DeviceAction::Send } else { DeviceAction::Insert })); ui.close(); }
                                if daw_ui::button(ui, "Inspect device").on_hover_text("Show identification and validation details for the unverified device").clicked() { diagnostic_request = Some(device_diagnostics(entry)); ui.close(); }
                            }
                            if state.kind == LibrarySection::Instrument && !assign && let Some(track) = assignment_target.and_then(|id| self.project.track(id)) && daw_ui::button(ui, format!("Replace instrument on {}", track.name)).on_hover_text(action_hint(DeviceAction::Assign)).clicked() { action = Some((entry.plugin.clone(), DeviceAction::Assign)); ui.close(); }
                            if state.kind == LibrarySection::Effect && source.is_some() {
                                if primary == DeviceAction::Send {
                                    if daw_ui::button(ui, "Add to effect chain").on_hover_text(action_hint(DeviceAction::Insert)).clicked() { action = Some((entry.plugin.clone(), DeviceAction::Insert)); ui.close(); }
                                } else if can_send && daw_ui::button(ui, "Create send track").on_hover_text(action_hint(DeviceAction::Send)).clicked() { action = Some((entry.plugin.clone(), DeviceAction::Send)); ui.close(); }
                            }
                            if daw_ui::button(ui, if item.favorite { "Remove from favorites" } else { "Add to favorites" }).clicked() { edit = Some(LibraryEdit::Favorite(entry.plugin.clone(), !item.favorite)); ui.close(); }
                            if !crate::builtin_devices::is_builtin(&entry.plugin) && daw_ui::button(ui, if entry.thumbnail.is_some() { "Refresh preview" } else { "Capture preview" }).on_hover_text(PREVIEW_IMAGE_HELP).clicked() { retry = Some(entry.plugin.clone()); ui.close(); }
                        }).response.on_hover_text("More ways to use the selected device, manage favorites, and capture its preview").on_disabled_hover_text("Select a device first");
                        if daw_ui::dialog_button(ui, "Details").on_hover_text("View device information and organize favorites and categories").on_disabled_hover_text("Select a device first").clicked() && let Some(item) = selected { summary_request = Some(item.entry.clone()); }
                    });
                }, |ui| {
                    let summary_destination = if state.kind == LibrarySection::Unknown { format!("Unverified · {destination}") } else { destination.clone() };
                    daw_ui::device_browser_selection_summary(ui, selected.map(|item| item.entry.plugin.name.as_str()).unwrap_or("No device selected"), &summary_destination);
                });
            }).response.rect;
            let rect = |rect: egui::Rect| serde_json::json!({"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]});
            ctx.data_mut(|data| data.insert_temp(egui::Id::new("library_diagnostics"), serde_json::json!({
                "catalog":rect(grid.rect),"footer":rect(footer),"sidebar":sidebar_rect.map(rect),"details":details_rect.map(rect),"scroll_offset":grid.scroll_offset,"selected":state.selected,"columns":grid.layout.columns,"card_size":[grid.layout.card_width,grid.layout.row_height-if state.list{SPACE_SM}else{SPACE_MD}],"view":if state.list{"list"}else{"grid"},"collection":state.collection.label(),"developer":state.developer,"category":state.category,"query":self.plugin_filter,"sort":if state.collection==Collection::RecentlyUsed{"Recently used"}else{state.sort.label()},"total":matches.len(),"cards":cards,"order":matches.iter().map(|item|&item.entry.plugin.name).collect::<Vec<_>>(),"pinned_source":pinned,"destination":destination,"primary_action":primary_label,"primary_enabled":enabled && state.selected.is_some(),"sync":{"scanning":scanning,"phase":format!("{:?}",status.phase),"captured":status.captured,"queued":status.queued,"current":status.current,"synced_unix_ms":status.synced_unix_ms}
            })));
        });
        if let Some(window) = ctx.memory(|memory| memory.area_rect(egui::Id::new("library"))) {
            ctx.data_mut(|data| { let id=egui::Id::new("library_diagnostics"); if let Some(mut diagnostic)=data.get_temp::<serde_json::Value>(id) { diagnostic["window"]=serde_json::json!({"min":[window.min.x,window.min.y],"max":[window.max.x,window.max.y]}); data.insert_temp(id,diagnostic); } });
        }
        self.library_open &= open;
        if let Some((summary, report)) = diagnostic_request {
            self.show_error_details("Device details", summary, report);
        }
        if let Some(entry) = details_request {
            crate::plugin_details::open(ctx, &entry);
        }
        if let Some(entry) = summary_request {
            let backdrop = Some(daw_ui::ViewBackdrop::capture_dialog_context(
                ctx,
                ctx.content_rect(),
            ));
            ctx.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("library_details_popup"),
                    LibraryDetails {
                        entry,
                        state: LibraryUi::default(),
                        error: None,
                        backdrop,
                    },
                )
            });
        }
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
        if let Some(edit) = edit {
            let categories = matches!(edit, LibraryEdit::Categories(..));
            match self.edit_plugin_library(edit) {
                Ok(()) if categories => state.category_draft = None,
                Ok(()) => {}
                Err(error) => self.message(Err(error), ""),
            }
        }
        if let Some((plugin, action)) = action {
            self.add_library_device(plugin, action, source);
        }
        let targets = if state.kind == LibrarySection::Sample {
            [None, None]
        } else {
            [state.search_id, state.selected.as_deref().map(device_id)]
        };
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new(KEY_TARGETS), targets);
            data.insert_temp(key, state);
        });
        if details_open(ctx) {
            self.library_details_window(ctx);
        }
        crate::plugin_details::show_window(ctx);
    }

    pub(crate) fn library_details_window(&mut self, ctx: &egui::Context) {
        let key = egui::Id::new("library_details_popup");
        let Some(mut dialog) = ctx.data_mut(|data| {
            let dialog = data.get_temp::<LibraryDetails>(key);
            data.remove::<LibraryDetails>(key);
            dialog
        }) else {
            return;
        };
        if let Some(backdrop) = &dialog.backdrop {
            egui::CentralPanel::default()
                .frame(daw_ui::inset(ctx).inner_margin(0).corner_radius(0))
                .show(ctx, |ui| backdrop.paint(ui));
        }
        let mut result = InspectorResponse::default();
        let modal = daw_ui::modal(
            ctx,
            egui::Id::new("library_details_modal"),
            &format!("Details · {}", dialog.entry.plugin.name),
            |ui| {
                if let Some(error) = &dialog.error {
                    daw_ui::notice(
                        ui,
                        daw_ui::NoticeKind::Error,
                        "Could not save device organization",
                        |_| {},
                    )
                    .response
                    .on_hover_text(error);
                }
                let item = item(&self.plugin_library, &dialog.entry);
                let height = (ctx.content_rect().height()
                    - 180.0
                    - if dialog.error.is_some() {
                        daw_ui::tokens::CONTROL_HEIGHT + SPACE_MD
                    } else {
                        0.0
                    })
                .clamp(0.0, 610.0);
                egui::ScrollArea::vertical()
                    .id_salt("library_details_popup_scroll")
                    .max_height(height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        result = inspector(ui, &mut dialog.state, &item, false);
                        ui.add_space(SPACE_SM);
                        daw_ui::text::section_heading(ui, "Preview image");
                        daw_ui::text::note(
                            ui,
                            match &dialog.entry.status {
                                _ if crate::builtin_devices::is_builtin(&dialog.entry.plugin) => {
                                    "Included"
                                }
                                ThumbnailStatus::Ready => "Captured",
                                ThumbnailStatus::Missing => "Not captured",
                                ThumbnailStatus::Failed { .. } => "Unavailable",
                            },
                        );
                        if let ThumbnailStatus::Failed { message } = &dialog.entry.status {
                            daw_ui::text::note(ui, "Optional image; assignment remains available.")
                                .on_hover_text(PREVIEW_IMAGE_HELP);
                            daw_ui::text::bounded_block(
                                ui,
                                daw_ui::text::TextRole::Note,
                                message,
                                3,
                            )
                            .on_hover_text(message);
                        }
                        daw_ui::text::section_heading(ui, "Install path");
                        daw_ui::path_row(ui, &dialog.entry.plugin.path, |ui| {
                            if daw_ui::button(ui, "Copy path").clicked() {
                                ctx.copy_text(dialog.entry.plugin.path.display().to_string());
                            }
                        });
                    });
                daw_ui::button(ui, "Close").clicked()
            },
        );
        let rect = modal.response.rect;
        ctx.data_mut(|data| {
            let id=egui::Id::new("library_diagnostics");
            if let Some(mut value)=data.get_temp::<serde_json::Value>(id) { value["details_popup"]=serde_json::json!({"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]}); data.insert_temp(id,value); }
        });
        if let Some(edit) = result.edit {
            let categories = matches!(edit, LibraryEdit::Categories(..));
            match self.edit_plugin_library(edit) {
                Ok(()) => {
                    dialog.error = None;
                    if categories {
                        dialog.state.category_draft = None;
                    }
                }
                Err(error) => dialog.error = Some(format!("{error:#}")),
            }
        }
        if result.technical {
            crate::plugin_details::open(ctx, &dialog.entry);
        } else if !modal.inner && !modal.should_close() {
            ctx.data_mut(|data| data.insert_temp(key, dialog));
        }
        crate::plugin_details::show_window(ctx);
    }

    fn add_library_device(
        &mut self,
        plugin: daw_plugins::PluginInfo,
        action: DeviceAction,
        source: Option<u64>,
    ) {
        if matches!(action, DeviceAction::Insert | DeviceAction::Send) {
            let Some(source) = source.filter(|id| {
                self.project.track(*id).is_some_and(|track| {
                    matches!(
                        track.kind,
                        TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                    )
                }) && (action != DeviceAction::Send || *id != daw_core::MASTER_CHANNEL_ID)
            }) else {
                return;
            };
            self.begin_edit();
            let track = if action == DeviceAction::Send {
                self.project.add_bus(format!("{} return", plugin.name))
            } else {
                source
            };
            let slot = EffectSlot::new(&plugin.name, crate::builtin_devices::effect(&plugin));
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
            self.record_plugin_use(&plugin);
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
                    track.name == "New instrument"
                        && (track.instrument == Instrument::None
                            || track.instrument == Instrument::new_track_default())
                });
        let instrument_name = self
            .available_instrument_name(&plugin.name, if name_new_track { target } else { None });
        self.begin_edit();
        let track = if replace {
            target.unwrap()
        } else {
            let id = self.project.add_midi_track(&instrument_name);
            if let Some(after) = self.new_track_after.take() {
                // A track removed while the chooser was open leaves the new
                // track at the end.
                let _ = self.project.reorder_track(id, after, TrackPlacement::After);
            }
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
        let result = self
            .project
            .set_instrument(track, crate::builtin_devices::instrument(&plugin));
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
        let assigned = result.is_ok();
        self.message(
            result,
            if replace {
                "Instrument assigned"
            } else {
                "Instrument track added"
            },
        );
        if assigned {
            self.record_plugin_use(&plugin);
        }
    }
}

/// "Producer · Name" of whatever an insert holds.
pub(crate) fn effect_model(catalog: &[CatalogEntry], effect: &Effect) -> String {
    match effect {
        Effect::Vst3 { path, class_id } => plugin_model(catalog, path, class_id),
        Effect::Builtin { device } => crate::builtin_devices::model(device),
    }
}

pub(crate) fn instrument_model(
    catalog: &[CatalogEntry],
    instrument: &Instrument,
) -> Option<String> {
    match instrument {
        Instrument::None => None,
        Instrument::Vst3 { path, class_id } => Some(plugin_model(catalog, path, class_id)),
        Instrument::Builtin { device, .. } => Some(crate::builtin_devices::model(device)),
    }
}

pub(crate) fn plugin_model(
    catalog: &[CatalogEntry],
    path: &str,
    class_id: &Option<String>,
) -> String {
    catalog
        .iter()
        .find(|entry| {
            entry.plugin.path == std::path::Path::new(path) && entry.plugin.class_id == *class_id
        })
        .map(|entry| {
            if entry.plugin.vendor.is_empty() {
                entry.plugin.name.clone()
            } else {
                format!("{} · {}", entry.plugin.vendor, entry.plugin.name)
            }
        })
        .unwrap_or_else(|| {
            format!(
                "{} · {}",
                daw_plugins::PluginFormat::from_path(std::path::Path::new(path)).label(),
                std::path::Path::new(path)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
            )
        })
}

pub(crate) fn device_diagnostics(entry: &CatalogEntry) -> (String, String) {
    let plugin = &entry.plugin;
    if crate::builtin_devices::is_builtin(plugin) {
        return (
            "Built-in device details.".into(),
            format!(
                "Device: {}\nDeveloper: {}\nType: {}\nCategories: {}\nID: {}\nVersion: {}\n\nBuilt into Rill: no scan, metadata file or preview capture is needed.",
                plugin.name,
                plugin.vendor,
                kind_label(plugin),
                plugin.category.replace('|', " · "),
                plugin.path.display(),
                plugin.version
            ),
        );
    }
    let kind = if plugin.is_instrument() == Some(true) {
        "Instrument"
    } else if plugin.is_effect() == Some(true) {
        "Effect"
    } else {
        "Unverified"
    };
    let metadata = [
        plugin.path.join("Contents/Resources/moduleinfo.json"),
        plugin.path.join("moduleinfo.json"),
    ]
    .into_iter()
    .find(|path| path.is_file());
    let metadata_status = metadata.as_ref().map_or_else(|| "No moduleinfo.json found. The bundle name is used as a fallback; its type and developer cannot be verified without valid metadata.".into(), |path| {
        match std::fs::metadata(path).and_then(|info| if info.len() <= 16 * 1024 * 1024 { std::fs::read_to_string(path) } else { Err(std::io::Error::other("Metadata exceeds 16 MiB")) }) {
            Ok(contents) => match serde_json::from_str::<serde_json::Value>(&contents) {
                Ok(_) => format!("Metadata parsed: {}", path.display()),
                Err(error) => format!("Invalid JSON in {}: {error}", path.display()),
            },
            Err(error) => format!("Cannot read {}: {error}", path.display()),
        }
    });
    let preview = match &entry.status {
        ThumbnailStatus::Missing => {
            "No preview has been captured. Use More → Capture preview.".into()
        }
        ThumbnailStatus::Ready => format!(
            "Preview captured: {}",
            entry.thumbnail.as_ref().map_or_else(
                || "Cached image is missing".into(),
                |path| path.display().to_string()
            )
        ),
        ThumbnailStatus::Failed { message } => format!(
            "Preview capture failed:\n{message}\n\nUse More → Capture preview to retry after correcting the error. A missing isolation helper requires reinstalling Rill (development builds: cargo build --workspace)."
        ),
    };
    let summary = if kind == "Unverified" {
        "This bundle does not declare a verified instrument or effect type."
    } else {
        "Plugin metadata and preview diagnostics."
    };
    (
        summary.into(),
        format!(
            "Device: {}\nDeveloper: {}\nType: {kind}\nDeclared categories: {}\nProcessor class: {}\nBundle: {}\n\n{metadata_status}\n\n{preview}\n\nMissing metadata does not prove a plugin is broken. Install or update the vendor's VST3 build, then Scan again. Unverified devices are kept separate from Instruments and Effects; More offers an explicit manual loading attempt.",
            plugin.name,
            if plugin.vendor.is_empty() {
                "Not declared"
            } else {
                &plugin.vendor
            },
            if plugin.category.is_empty() {
                "Not declared"
            } else {
                &plugin.category
            },
            plugin.class_id.as_deref().unwrap_or("Not declared"),
            plugin.path.display()
        ),
    )
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
    fn enter_with_no_visible_result_cannot_activate_a_later_result() {
        let ctx = egui::Context::default();
        let mut app = DawApp::with_context(&ctx, Project::new(), None);
        app.library_open = true;
        app.plugin_filter = "no matching device at keypress".into();
        let frame = |app: &mut DawApp| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.plugin_library(ctx),
            )
        };
        let _ = frame(&mut app);
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("instrument_chooser_enter"), true));
        let _ = frame(&mut app);
        assert!(app.project.tracks.is_empty());
        app.plugin_filter = "R1 Mini".into();
        for _ in 0..3 {
            let _ = frame(&mut app);
        }
        assert!(
            app.project.tracks.is_empty(),
            "A later catalogue result must not consume an earlier Enter"
        );
        assert!(app.library_open);
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
            app.library_target = LibraryTarget::Instrument {
                track: Some(target),
                name_new_track: false,
            };
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
                        target: app.library_target,
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
            app.library_target = LibraryTarget::Instrument {
                track: Some(target),
                name_new_track: false,
            };
            let loading = frame(&mut app, vec![]);
            let assign = action_position(&loading, "Replace instrument");
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
            assert!(action_position(&ready, "Replace instrument").distance(assign) <= 1.0);
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

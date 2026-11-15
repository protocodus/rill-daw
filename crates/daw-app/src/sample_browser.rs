//! Source navigation, audition and explicit use of a sound share one library.
//! The browser owns presentation state; catalog, decoding and import services
//! remain independent of the selected collection or calling instrument.
use crate::{
    app::{DawApp, PopupKind},
    audio_import::{ImportOutcome, ImportTicket},
    library::LibraryTarget,
    sample_index::SampleIndex,
};
use daw_ui::{SampleAuditionState, SampleRegion, SampleResult, SampleSourceNode, tokens};
use eframe::egui;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum SourceScope {
    #[default]
    All,
    Favorites,
    Project,
    Folder(PathBuf),
}
impl SourceScope {
    fn key(&self) -> String {
        match self {
            Self::All => "all".into(),
            Self::Favorites => "favorites".into(),
            Self::Project => "project".into(),
            Self::Folder(path) => format!("folder:{}", path.display()),
        }
    }
    fn label(&self) -> String {
        match self {
            Self::All => "All samples".into(),
            Self::Favorites => "Favorites".into(),
            Self::Project => "Project samples".into(),
            Self::Folder(path) => path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "all" => Some(Self::All),
            "favorites" => Some(Self::Favorites),
            "project" => Some(Self::Project),
            _ => key
                .strip_prefix("folder:")
                .map(|path| Self::Folder(path.into())),
        }
    }
}

#[derive(Clone)]
struct BrowserItem {
    row: SampleResult,
    root: Option<PathBuf>,
    project: bool,
}
impl BrowserItem {
    fn source(&self) -> daw_ui::SampleDrop {
        daw_ui::SampleDrop {
            path: self.row.path.clone().into(),
            name: self.row.source_name.clone(),
        }
    }
}
#[derive(Clone, Copy)]
enum UseKind {
    Project,
    Device,
    Track,
}
struct PendingUse {
    ticket: ImportTicket,
    name: String,
    kind: UseKind,
    return_to: Option<(u64, u64)>,
    context: u64,
}

#[derive(Default)]
pub(crate) struct SampleBrowser {
    pub requested: bool,
    index: SampleIndex,
    metadata_index_revision: u64,
    library: crate::sample_library::SampleLibrary,
    metadata: crate::sample_metadata::SampleMetadataState,
    visual: crate::sample_visual::SampleVisualState,
    visual_bounds: Option<egui::Rect>,
    visual_waveform: Option<egui::Rect>,
    visual_spectrum: Option<egui::Rect>,
    scope: SourceScope,
    tag_draft: Option<FavoriteDraft>,
    session: u64,
    context: u64,
    captured: bool,
    target: Option<(u64, u32)>,
    query: String,
    selected: Option<String>,
    folders: bool,
    scan_details: bool,
    destination: bool,
    destination_load: bool,
    destination_original: Option<Option<(u64, u32)>>,
    root_draft: String,
    items: Vec<BrowserItem>,
    results: Vec<SampleResult>,
    result_lookup: HashMap<String, usize>,
    item_lookup: HashMap<String, usize>,
    audition_index: Option<usize>,
    source_folders: Vec<SampleSourceNode>,
    tree_key: Option<BrowserTreeKey>,
    pending: Option<PendingUse>,
    notice: Option<(String, bool)>,
    memberships: BTreeMap<(PathBuf, Option<String>), u64>,
}
#[derive(PartialEq)]
struct BrowserTreeKey {
    index: u64,
    session: u64,
    project: u64,
    library: u64,
    scope: SourceScope,
    query: String,
    roots: Vec<PathBuf>,
}
struct FavoriteDraft {
    id: String,
    value: daw_ui::SampleFavorite,
}

#[derive(Default)]
struct FolderBranch {
    children: BTreeMap<String, FolderBranch>,
}
impl FolderBranch {
    fn insert(&mut self, parts: &[String]) {
        if let Some((head, tail)) = parts.split_first() {
            self.children.entry(head.clone()).or_default().insert(tail);
        }
    }
    fn nodes(self, parent: &Path) -> Vec<SampleSourceNode> {
        self.children
            .into_iter()
            .map(|(name, children)| {
                let path = parent.join(&name);
                SampleSourceNode {
                    id: SourceScope::Folder(path.clone()).key(),
                    name,
                    icon: daw_ui::Icon::Folder,
                    detail: None,
                    children: children.nodes(&path),
                    drop_target: false,
                }
            })
            .collect()
    }
}
fn matches(query: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    query
        .split_whitespace()
        .all(|word| text.contains(&word.to_lowercase()))
}

impl DawApp {
    pub(crate) fn reset_sample_browser_context(&mut self) {
        self.sample_browser.context = self.sample_browser.context.wrapping_add(1);
        self.sample_browser.captured = false;
        self.sample_browser.target = None;
        self.sample_browser.destination = false;
        self.sample_browser.destination_original = None;
        self.sample_browser.folders = false;
        self.sample_browser.scan_details = false;
    }
    pub(crate) fn open_sample_browser(&mut self, target: Option<(u64, u32)>) {
        self.sample_browser.context = self.sample_browser.context.wrapping_add(1);
        self.sample_browser.captured = target.is_some();
        self.sample_browser.target = target;
        self.sample_browser.session = self.song_session.id;
        self.sample_browser.notice = None;
        self.sample_browser.requested = true;
        self.library_target = LibraryTarget::Browse;
        self.open_popup(PopupKind::Library);
    }

    pub(crate) fn prepare_sample_browser_screenshot(&mut self, ctx: &egui::Context) {
        self.sample_library_tree();
        if let Some(id) = self
            .sample_browser
            .results
            .first()
            .map(|row| row.id.clone())
        {
            self.select_browser_sample(id, ctx);
        }
    }
    pub(crate) fn sample_browser_metadata_pending(&self) -> bool {
        self.sample_browser
            .selected
            .as_deref()
            .and_then(|id| self.sample_source(id))
            .is_some_and(|source| {
                matches!(
                    self.sample_browser.metadata.selected(&source.path),
                    Some(crate::sample_metadata::SampleMetadataStatus::Loading)
                ) || matches!(
                    self.sample_browser.visual.selected(&source.path),
                    crate::sample_visual::SampleVisualStatus::Idle
                        | crate::sample_visual::SampleVisualStatus::Loading
                )
            })
    }
    pub(crate) fn sample_browser_keyboard(&mut self, ctx: &egui::Context) -> bool {
        if !(self.library_open || self.sample_player_editor.open) || egui::Popup::is_any_open(ctx) {
            return false;
        }
        if (self.sample_browser.folders
            || self.sample_browser.destination
            || self.sample_browser.scan_details)
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.sample_browser.folders = false;
            if let Some(original) = self.sample_browser.destination_original.take() {
                self.sample_browser.target = original;
            }
            self.sample_browser.destination = false;
            self.sample_browser.scan_details = false;
            return true;
        }
        false
    }
    pub(crate) fn poll_sample_browser(&mut self, ctx: &egui::Context) {
        self.sample_browser.index.poll();
        self.sample_browser.library.poll(ctx);
        self.sample_browser.metadata.poll(ctx);
        self.sample_browser.visual.poll(ctx);
        if !self.library_open || !crate::library::samples_visible(ctx) {
            self.sample_browser.visual.clear_selection();
            self.sample_browser.visual_bounds = None;
            self.sample_browser.visual_waveform = None;
            self.sample_browser.visual_spectrum = None;
        }
        if self.sample_browser.metadata_index_revision != self.sample_browser.index.revision {
            self.sample_browser.metadata_index_revision = self.sample_browser.index.revision;
            self.sample_browser.metadata.invalidate();
            self.sample_browser.visual.invalidate();
            if let Some(source) = self
                .sample_browser
                .selected
                .as_deref()
                .and_then(|id| self.sample_source(id))
            {
                self.sample_browser.metadata.request(source.path, ctx);
            }
        }
        for error in self.sample_browser.library.take_errors() {
            self.sample_browser.notice = Some((error, true));
        }
        if self.sample_browser.session != self.song_session.id {
            self.sample_browser.target = None;
            self.sample_browser.captured = false;
            self.sample_browser.selected = None;
            self.sample_browser.tag_draft = None;
            self.sample_browser.pending = None;
            self.sample_browser.notice = None;
            self.sample_browser.memberships.clear();
            self.sample_browser.metadata.invalidate();
            self.sample_browser.visual.invalidate();
            self.sample_browser.session = self.song_session.id;
            self.sample_browser.context = self.sample_browser.context.wrapping_add(1);
        }
        let completed = self
            .sample_browser
            .pending
            .as_ref()
            .and_then(|pending| self.audio_import_outcome(pending.ticket).cloned());
        if let Some(outcome) = completed {
            let pending = self.sample_browser.pending.take().unwrap();
            let current = pending.context == self.sample_browser.context;
            match outcome {
                ImportOutcome::Succeeded(receipt) => {
                    for source in receipt.sources {
                        if let Some(id) = source.project_sample_id {
                            self.sample_browser
                                .memberships
                                .insert((source.source.path, source.source.name), id);
                        }
                    }
                    let verb = match pending.kind {
                        UseKind::Project => "Added to Project Samples",
                        UseKind::Device => "Loaded",
                        UseKind::Track => "Added audio track",
                    };
                    if current {
                        self.sample_browser.notice =
                            Some((format!("{verb} · {}", pending.name), false));
                    }
                    if let Some((context, track)) = pending.return_to
                        && context == self.sample_browser.context
                        && self.library_open
                        && crate::library::samples_visible(ctx)
                    {
                        self.finish_sample_browser_load(track);
                    }
                }
                ImportOutcome::Failed(error) if current => {
                    self.sample_browser.notice = Some((format!("{} · {error}", pending.name), true))
                }
                ImportOutcome::Cancelled if current => {
                    self.sample_browser.notice = Some(("Import canceled".into(), false))
                }
                _ => (),
            }
        }
    }

    /// Build the catalog once per revision; querying never traverses the filesystem.
    fn sample_library_tree(&mut self) {
        let key = BrowserTreeKey {
            index: self.sample_browser.index.revision,
            session: self.song_session.id,
            project: self.song_session.revision,
            library: self.sample_browser.library.revision(),
            scope: self.sample_browser.scope.clone(),
            query: self.sample_browser.query.clone(),
            roots: self.settings.sample_roots.clone(),
        };
        if self.sample_browser.tree_key.as_ref() == Some(&key) {
            return;
        }
        let mut items = Vec::new();
        let mut represented = HashSet::new();
        for sample in &self.project.samples {
            let favorite = self
                .sample_browser
                .library
                .favorite_for(Path::new(&sample.path), Some(&sample.name));
            if let Some(entry) = favorite {
                represented.insert(entry.id.clone());
            }
            items.push(BrowserItem {
                row: SampleResult {
                    id: format!("project:{}", sample.id),
                    name: sample.name.clone(),
                    provenance: "Project samples".into(),
                    path: sample.path.clone(),
                    source_name: Some(sample.name.clone()),
                    duration_seconds: self.sample_duration(&sample.path),
                    favorite: favorite.map(|entry| favorite_view(&entry.favorite)),
                    ..Default::default()
                },
                root: None,
                project: true,
            });
        }
        let mut folders = Vec::new();
        let mut by_root = BTreeMap::<&Path, Vec<&crate::sample_index::SampleEntry>>::new();
        for entry in self.sample_browser.index.entries() {
            by_root.entry(&entry.root).or_default().push(entry);
        }
        for root in &self.settings.sample_roots {
            let mut branch = FolderBranch::default();
            if let Some(entries) = by_root.get(root.as_path()) {
                for entry in entries {
                    let path = entry.path();
                    let favorite = self.sample_browser.library.favorite_for(&path, None);
                    if let Some(entry) = favorite {
                        represented.insert(entry.id.clone());
                    }
                    let parts: Vec<_> = entry
                        .relative
                        .parent()
                        .unwrap_or(Path::new(""))
                        .components()
                        .map(|part| part.as_os_str().to_string_lossy().into_owned())
                        .collect();
                    branch.insert(&parts);
                    let parent = entry.relative.parent().unwrap_or(Path::new(""));
                    let root_name = root
                        .file_name()
                        .unwrap_or(root.as_os_str())
                        .to_string_lossy();
                    let provenance = if parent.as_os_str().is_empty() {
                        root_name.into_owned()
                    } else {
                        format!("{root_name} / {}", parent.display())
                    };
                    items.push(BrowserItem {
                        row: SampleResult {
                            id: path.to_string_lossy().into_owned(),
                            name: entry
                                .relative
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                            provenance,
                            path: path.to_string_lossy().into_owned(),
                            source_name: None,
                            duration_seconds: self.sample_duration(&path.to_string_lossy()),
                            favorite: favorite.map(|entry| favorite_view(&entry.favorite)),
                            ..Default::default()
                        },
                        root: Some(root.clone()),
                        project: false,
                    });
                }
            }
            folders.push(SampleSourceNode {
                id: SourceScope::Folder(root.clone()).key(),
                name: root
                    .file_name()
                    .unwrap_or(root.as_os_str())
                    .to_string_lossy()
                    .into_owned(),
                icon: daw_ui::Icon::Folder,
                detail: Some(if self.sample_browser.index.roots().contains(root) {
                    root.display().to_string()
                } else {
                    format!("{} · Not scanned", root.display())
                }),
                children: branch.nodes(root),
                drop_target: false,
            });
        }
        for favorite in self.sample_browser.library.entries() {
            let id = format!("favorite:{}", favorite.id);
            let keep = self.sample_browser.scope == SourceScope::Favorites
                || !represented.contains(&favorite.id)
                || self.sample_browser.selected.as_deref() == Some(&id);
            if keep {
                items.push(BrowserItem {
                    row: SampleResult {
                        id,
                        name: favorite.name.clone(),
                        provenance: "Favorites library".into(),
                        path: favorite.path.to_string_lossy().into_owned(),
                        source_name: Some(favorite.name.clone()),
                        duration_seconds: self.sample_duration(&favorite.path.to_string_lossy()),
                        favorite: Some(favorite_view(&favorite.favorite)),
                        ..Default::default()
                    },
                    root: None,
                    project: false,
                });
            }
        }
        let scope = &self.sample_browser.scope;
        let query = &self.sample_browser.query;
        let mut results: Vec<_> = items
            .iter()
            .filter(|item| match scope {
                SourceScope::All => true,
                SourceScope::Project => item.project,
                SourceScope::Favorites => item.row.id.starts_with("favorite:"),
                SourceScope::Folder(folder) => {
                    item.root.is_some() && Path::new(&item.row.path).starts_with(folder)
                }
            })
            .filter(|item| {
                matches(
                    query,
                    &format!(
                        "{} {} {} {}",
                        item.row.name,
                        item.row.path,
                        item.row.provenance,
                        item.row
                            .favorite
                            .as_ref()
                            .map_or("", |favorite| favorite.tag.as_str())
                    ),
                )
            })
            .map(|item| item.row.clone())
            .collect();
        results.sort_by_cached_key(|row| {
            (
                row.name.to_lowercase(),
                row.provenance.to_lowercase(),
                row.id.clone(),
            )
        });
        if self
            .sample_browser
            .selected
            .as_deref()
            .is_some_and(|id| !results.iter().any(|row| row.id == id))
        {
            self.stop_sample_preview();
            self.sample_browser.selected = None;
            self.sample_browser.tag_draft = None;
        }
        self.sample_browser.item_lookup = items
            .iter()
            .enumerate()
            .map(|(index, item)| (item.row.id.clone(), index))
            .collect();
        self.sample_browser.result_lookup = results
            .iter()
            .enumerate()
            .map(|(index, row)| (row.id.clone(), index))
            .collect();
        self.sample_browser.audition_index = None;
        self.sample_browser.items = items;
        self.sample_browser.results = results;
        self.sample_browser.source_folders = folders;
        self.sample_browser.tree_key = Some(key);
    }

    fn sample_duration(&self, path: &str) -> Option<f64> {
        match self.sample_browser.metadata.selected(Path::new(path)) {
            Some(crate::sample_metadata::SampleMetadataStatus::Ready(metadata)) => {
                metadata.duration_seconds
            }
            _ => None,
        }
    }
    fn sample_collections(&self) -> Vec<SampleSourceNode> {
        [
            ("all", "All samples", daw_ui::Icon::Library),
            ("favorites", "Favorites", daw_ui::Icon::Star),
            ("project", "Project samples", daw_ui::Icon::Folder),
        ]
        .into_iter()
        .map(|(id, name, icon)| SampleSourceNode {
            id: id.into(),
            name: name.into(),
            icon,
            detail: None,
            children: Vec::new(),
            drop_target: id == "project",
        })
        .collect()
    }
    fn set_sample_scope(&mut self, key: &str) {
        if let Some(scope) = SourceScope::from_key(key) {
            self.sample_browser.scope = scope;
            self.sample_library_tree();
        }
    }

    pub(crate) fn sample_browser_contents(&mut self, ui: &mut egui::Ui) {
        self.sample_library_tree();
        let height = ui.available_height();
        let mut result_response = daw_ui::SampleResultsResponse::default();
        let mut source_response = daw_ui::SampleSourcesResponse::default();
        daw_ui::sample_browser_workspace(
            ui,
            egui::Id::new("sample_browser_workspace"),
            height,
            |pane, ui| match pane {
                0 => {
                    let status_height = if self.sample_scan_notice().is_some() {
                        24.0 + if self.sample_browser.index.error.is_some()
                            || !self.sample_browser.index.warnings().is_empty()
                        {
                            42.0
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    };
                    daw_ui::sample_regions(
                        ui,
                        egui::Id::new("sample_source_regions"),
                        ui.available_height(),
                        0.0,
                        2.0 * tokens::CONTROL_HEIGHT + tokens::SPACE_SM + status_height,
                        |region, ui| match region {
                            SampleRegion::Body => {
                                source_response = daw_ui::sample_sources(
                                    ui,
                                    egui::Id::new("sample_sources"),
                                    &self.sample_collections(),
                                    &self.sample_browser.source_folders,
                                    &self.sample_browser.scope.key(),
                                    ui.available_height(),
                                );
                                if let Some(scope) = &source_response.selected {
                                    self.set_sample_scope(scope);
                                }
                            }
                            SampleRegion::Footer => self.sample_source_footer(ui),
                            SampleRegion::Header => (),
                        },
                    );
                }
                1 => {
                    if daw_ui::sample_search(
                        ui,
                        &mut self.sample_browser.query,
                        &self.sample_browser.scope.label(),
                        self.sample_browser.results.len(),
                    ) {
                        self.sample_library_tree();
                    }
                    if self.sample_browser.scope == SourceScope::Project
                        && daw_ui::quiet_button(ui, "Bounce…").clicked()
                    {
                        self.open_bounce_to_project_samples();
                    }
                    if self.sample_browser.results.is_empty() {
                        self.sample_empty_state(ui);
                    }
                    self.update_sample_row_status();
                    result_response = daw_ui::sample_results(
                        ui,
                        egui::Id::new("sample_results_full"),
                        &self.sample_browser.results,
                        self.sample_browser.selected.as_deref(),
                        ui.available_height(),
                    );
                    self.apply_sample_results(&result_response, ui.ctx());
                }
                2 => {
                    let footer = if self.sample_browser.captured {
                        156.0
                    } else {
                        130.0
                    };
                    daw_ui::sample_regions(
                        ui,
                        egui::Id::new("sample_inspector"),
                        ui.available_height(),
                        120.0,
                        footer,
                        |region, ui| match region {
                            SampleRegion::Header => self.sample_inspector_header(ui),
                            SampleRegion::Body => {
                                self.sample_inspector_backdrop(ui);
                                egui::ScrollArea::vertical()
                                    .id_salt("sample_inspector_scroll")
                                    .show(ui, |ui| self.sample_selection_details(ui));
                            }
                            SampleRegion::Footer => self.sample_use_actions(ui),
                        },
                    );
                }
                _ => (),
            },
        );
        if let Some((_, source)) = &source_response.dropped {
            self.add_browser_source_to_project(source.clone());
        }
        self.sample_browser_diagnostics(ui, &result_response, &source_response, false);
        self.sample_browser_dialogs(ui.ctx());
    }

    pub(crate) fn sample_palette(&mut self, ui: &mut egui::Ui) {
        self.sample_library_tree();
        let mut result_response = daw_ui::SampleResultsResponse::default();
        let mut drop_response = None;
        daw_ui::sample_regions(
            ui,
            egui::Id::new("sample_palette_regions"),
            ui.available_height(),
            172.0,
            142.0,
            |region, ui| match region {
                SampleRegion::Header => {
                    daw_ui::control_row(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = tokens::SPACE_SM;
                        ui.spacing_mut().button_padding.x = tokens::SPACE_SM;
                        daw_ui::select("palette_scope", self.sample_browser.scope.label())
                            .width(
                                (ui.available_width() - tokens::CONTROL_HEIGHT - tokens::SPACE_SM)
                                    .max(1.0),
                            )
                            .truncate()
                            .show_ui(ui, |ui| {
                                let mut choices = self.sample_collections();
                                fn flatten(
                                    nodes: &[SampleSourceNode],
                                    prefix: &str,
                                    choices: &mut Vec<SampleSourceNode>,
                                ) {
                                    for node in nodes {
                                        let name = if prefix.is_empty() {
                                            node.name.clone()
                                        } else {
                                            format!("{prefix} / {}", node.name)
                                        };
                                        let mut copy = node.clone();
                                        copy.name = name.clone();
                                        copy.children.clear();
                                        choices.push(copy);
                                        flatten(&node.children, &name, choices);
                                    }
                                }
                                flatten(&self.sample_browser.source_folders, "", &mut choices);
                                for node in choices {
                                    if daw_ui::selectable(
                                        ui,
                                        node.id == self.sample_browser.scope.key(),
                                        &node.name,
                                    )
                                    .clicked()
                                    {
                                        self.set_sample_scope(&node.id);
                                    }
                                }
                            });
                        if ui
                            .add_sized(
                                egui::Vec2::splat(tokens::CONTROL_HEIGHT),
                                daw_ui::IconButton::icon_only(daw_ui::Icon::Library, "Details…")
                                    .presentation()
                                    .quiet(),
                            )
                            .on_hover_text("Open sample details")
                            .clicked()
                        {
                            let target = self.sample_player_browser_target();
                            self.open_sample_browser(target);
                        }
                    });
                    if daw_ui::sample_search(
                        ui,
                        &mut self.sample_browser.query,
                        &self.sample_browser.scope.label(),
                        self.sample_browser.results.len(),
                    ) {
                        self.sample_library_tree();
                    }
                    let (response, source) =
                        daw_ui::sample_project_drop_target(ui, "Project samples");
                    if response.clicked() {
                        self.set_sample_scope("project");
                    }
                    if let Some(source) = source {
                        self.add_browser_source_to_project(source);
                    }
                    drop_response = Some(response);
                }
                SampleRegion::Body => {
                    if self.sample_browser.results.is_empty() {
                        daw_ui::text::bounded_block(
                            ui,
                            daw_ui::text::TextRole::Note,
                            "No samples in this view. Open Details to choose folders or files.",
                            3,
                        );
                    }
                    self.update_sample_row_status();
                    result_response = daw_ui::sample_results(
                        ui,
                        egui::Id::new("sample_results_palette"),
                        &self.sample_browser.results,
                        self.sample_browser.selected.as_deref(),
                        ui.available_height(),
                    );
                    self.apply_sample_results(&result_response, ui.ctx());
                }
                SampleRegion::Footer => {
                    if let Some(row) = self.selected_sample_row() {
                        daw_ui::text::truncated(ui, daw_ui::text::TextRole::Label, &row.name);
                    } else {
                        daw_ui::text::note(ui, "Select a sample");
                    }
                    self.sample_preview_controls(ui);
                    if self.sample_browser.library.busy() {
                        daw_ui::text::note(ui, "Updating favorites…");
                    } else {
                        self.sample_preview_notice(ui);
                    }
                    let source = self
                        .sample_browser
                        .selected
                        .as_deref()
                        .and_then(|id| self.sample_source(id));
                    let contained = source
                        .as_ref()
                        .is_some_and(|source| self.source_in_project(source));
                    if self.audio_import_busy() {
                        if daw_ui::quiet_button(ui, "Cancel import").clicked() {
                            self.cancel_audio_import();
                        }
                    } else if ui
                        .add_enabled(
                            source.is_some() && !contained,
                            daw_ui::button_widget(if contained {
                                "In Project Samples"
                            } else {
                                "Add to Project Samples"
                            }),
                        )
                        .clicked()
                        && let Some(source) = source
                    {
                        self.add_browser_source_to_project(source);
                    }
                }
            },
        );
        let mut sources = daw_ui::SampleSourcesResponse::default();
        if let Some(response) = drop_response {
            sources.drop_targets.push(("project".into(), response));
        }
        self.sample_browser_diagnostics(ui, &result_response, &sources, true);
    }

    /// Audition and metadata decorate one cached row without rebuilding or
    /// cloning the catalog. The virtual list borrows the cached result slice.
    fn update_sample_row_status(&mut self) {
        if let Some(index) = self.sample_browser.audition_index.take() {
            if let Some(row) = self.sample_browser.results.get_mut(index) {
                row.audition = SampleAuditionState::Idle;
            }
        }
        let Some(id) = self.sample_browser.selected.as_deref() else {
            return;
        };
        let Some(index) = self.sample_browser.result_lookup.get(id).copied() else {
            return;
        };
        let row = &self.sample_browser.results[index];
        let duration = self.sample_duration(&row.path);
        let status = match self.sample_preview_status(Path::new(&row.path)) {
            crate::sample_preview::SamplePreviewStatus::Idle => SampleAuditionState::Idle,
            crate::sample_preview::SamplePreviewStatus::Loading => SampleAuditionState::Loading,
            crate::sample_preview::SamplePreviewStatus::Playing => SampleAuditionState::Playing,
            crate::sample_preview::SamplePreviewStatus::Failed(_) => SampleAuditionState::Failed,
        };
        let row = &mut self.sample_browser.results[index];
        row.audition = status;
        row.duration_seconds = duration;
        self.sample_browser.audition_index = Some(index);
    }
    fn apply_sample_results(
        &mut self,
        response: &daw_ui::SampleResultsResponse,
        ctx: &egui::Context,
    ) {
        if let Some(id) = &response.selected {
            self.select_browser_sample(id.clone(), ctx);
        }
        if let Some(id) = &response.preview {
            if self.sample_browser.selected.as_deref() != Some(id) {
                self.select_browser_sample(id.clone(), ctx);
            }
            if let Some(source) = self.sample_source(id) {
                self.preview_sample(source.path);
            }
        }
        if response.toggle_preview {
            if self.sample_preview_active() {
                self.stop_sample_preview();
            } else if let Some(source) = self
                .sample_browser
                .selected
                .as_deref()
                .and_then(|id| self.sample_source(id))
            {
                self.preview_sample(source.path);
            }
        }
        if let Some(id) = &response.favorite_toggled {
            self.toggle_sample_favorite(id, ctx);
        }
    }

    fn selected_sample_row(&self) -> Option<&SampleResult> {
        self.sample_browser
            .selected
            .as_deref()
            .and_then(|id| {
                self.sample_browser
                    .item_lookup
                    .get(id)
                    .and_then(|index| self.sample_browser.items.get(*index))
            })
            .map(|item| &item.row)
    }
    fn sample_source(&self, id: &str) -> Option<daw_ui::SampleDrop> {
        self.sample_browser
            .item_lookup
            .get(id)
            .and_then(|index| self.sample_browser.items.get(*index))
            .map(BrowserItem::source)
    }
    fn select_browser_sample(&mut self, id: String, ctx: &egui::Context) {
        if self.sample_browser.selected.as_deref() == Some(&id) {
            return;
        }
        self.stop_sample_preview();
        self.sample_browser.selected = Some(id.clone());
        self.sample_browser.tag_draft = None;
        if let Some(source) = self.sample_source(&id) {
            self.sample_browser
                .metadata
                .request(source.path.clone(), ctx);
            if id.starts_with("project:") {
                self.sample_browser.library.resolve(source.clone(), ctx);
            }
            if self.settings.sample_auto_play {
                self.preview_sample(source.path);
            }
        }
    }
    fn toggle_sample_favorite(&mut self, id: &str, ctx: &egui::Context) {
        if self.sample_browser.library.busy() {
            return;
        }
        let Some(source) = self.sample_source(id) else {
            return;
        };
        if let Some(existing) = self
            .sample_browser
            .library
            .favorite_for(&source.path, source.name.as_deref())
            .map(|entry| entry.id.clone())
        {
            if let Err(error) = self.sample_browser.library.remove(&existing) {
                self.sample_browser.notice = Some((error.to_string(), true));
            }
            self.sample_browser.tag_draft = None;
        } else {
            self.sample_browser
                .library
                .request_add(source, id.starts_with("project:"), ctx);
        }
    }
    fn sample_inspector_header(&mut self, ui: &mut egui::Ui) {
        if let Some(row) = self.selected_sample_row() {
            daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Label, &row.name, 2)
                .on_hover_text(&row.name);
        } else {
            daw_ui::text::note(ui, "Select a sample to hear it.");
        }
        self.sample_preview_controls(ui);
        self.sample_preview_notice(ui);
    }
    fn sample_preview_controls(&mut self, ui: &mut egui::Ui) {
        daw_ui::control_row(ui, |ui| {
            if self.sample_preview_active() {
                if daw_ui::button(ui, "Stop preview").clicked() {
                    self.stop_sample_preview();
                }
            } else if ui
                .add_enabled(
                    self.sample_browser.selected.is_some(),
                    daw_ui::button_widget("Preview"),
                )
                .clicked()
                && let Some(source) = self
                    .sample_browser
                    .selected
                    .as_deref()
                    .and_then(|id| self.sample_source(id))
            {
                self.preview_sample(source.path);
            }
            let mut auto_play = self.settings.sample_auto_play;
            if daw_ui::checkbox(ui, &mut auto_play, "Auto-play").changed() {
                let mut next = self.settings.clone();
                next.sample_auto_play = auto_play;
                match next.save() {
                    Ok(()) => {
                        self.settings = next;
                        self.setup_draft.sample_auto_play = auto_play;
                        if auto_play
                            && let Some(source) = self
                                .sample_browser
                                .selected
                                .as_deref()
                                .and_then(|id| self.sample_source(id))
                        {
                            self.preview_sample(source.path);
                        } else {
                            self.stop_sample_preview();
                        }
                    }
                    Err(error) => self.sample_browser.notice = Some((error.to_string(), true)),
                }
            }
        });
    }
    fn sample_preview_notice(&self, ui: &mut egui::Ui) {
        if let Some(source) = self
            .sample_browser
            .selected
            .as_deref()
            .and_then(|id| self.sample_source(id))
        {
            let message = match self.sample_preview_status(&source.path) {
                crate::sample_preview::SamplePreviewStatus::Idle => "Ready to preview",
                crate::sample_preview::SamplePreviewStatus::Loading => "Loading preview…",
                crate::sample_preview::SamplePreviewStatus::Playing => "Playing preview",
                crate::sample_preview::SamplePreviewStatus::Failed(error) => error,
            };
            daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note, message, 2);
        }
    }
    /// Only the visible selected-source inspector requests analysis. Painting
    /// is behind the scrolling metadata and allocates no interaction targets.
    fn sample_inspector_backdrop(&mut self, ui: &egui::Ui) {
        self.sample_browser.visual_bounds = None;
        self.sample_browser.visual_waveform = None;
        self.sample_browser.visual_spectrum = None;
        let Some(source) = self
            .sample_browser
            .selected
            .as_deref()
            .and_then(|id| self.sample_source(id))
        else {
            self.sample_browser.visual.clear_selection();
            return;
        };
        self.sample_browser.visual.request(
            source.path.clone(),
            self.settings.scratch_dir.clone(),
            ui.ctx(),
        );
        let bounds = ui.max_rect().intersect(ui.clip_rect());
        self.sample_browser.visual_bounds = Some(bounds);
        if let crate::sample_visual::SampleVisualStatus::Ready(data) =
            self.sample_browser.visual.selected(&source.path)
        {
            let painted = daw_ui::paint_audio_preview_backdrop(
                ui.painter(),
                bounds,
                daw_ui::AudioPreviewBackdrop {
                    waveform: &data.waveform,
                    spectrum: &data.spectrum,
                    state: daw_ui::AudioPreviewState::Ready,
                },
                daw_ui::theme::palette(ui.ctx()).bg,
            );
            self.sample_browser.visual_waveform = painted.waveform;
            self.sample_browser.visual_spectrum = painted.spectrum;
        }
    }

    fn sample_selection_details(&mut self, ui: &mut egui::Ui) {
        let Some(row) = self.selected_sample_row().cloned() else {
            return;
        };
        let mut details = daw_ui::SampleDetails {
            name: "",
            location: &row.provenance,
            ..Default::default()
        };
        match self.sample_browser.metadata.selected(Path::new(&row.path)) {
            Some(crate::sample_metadata::SampleMetadataStatus::Ready(metadata)) => {
                details.duration_seconds = metadata.duration_seconds;
                details.format = Some(&metadata.format);
                details.codec = Some(&metadata.codec);
                details.sample_rate = metadata.sample_rate;
                details.channels = metadata.channels.and_then(|n| u16::try_from(n).ok());
                details.bit_depth = metadata.bits_per_sample.and_then(|n| u16::try_from(n).ok());
                details.size_bytes = Some(metadata.file_bytes);
            }
            Some(crate::sample_metadata::SampleMetadataStatus::Failed(error)) => {
                details.status = Some(error)
            }
            _ => details.status = Some("Reading sample details…"),
        }
        daw_ui::sample_details(ui, &details).on_hover_text(&row.path);
        if matches!(
            self.sample_browser.metadata.selected(Path::new(&row.path)),
            Some(crate::sample_metadata::SampleMetadataStatus::Failed(_))
        ) && daw_ui::quiet_button(ui, "Retry details").clicked()
        {
            self.sample_browser
                .metadata
                .retry(row.path.clone().into(), ui.ctx());
            self.sample_browser.visual.retry(
                row.path.clone().into(),
                self.settings.scratch_dir.clone(),
                ui.ctx(),
            );
        }
        daw_ui::choice_menu(ui, "File details…", |ui| {
            daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note, &row.path, 8);
            if let crate::sample_visual::SampleVisualStatus::Failed(error) =
                self.sample_browser.visual.selected(Path::new(&row.path))
            {
                daw_ui::text::bounded_block(
                    ui,
                    daw_ui::text::TextRole::Note,
                    format!("Visual analysis unavailable · {error}"),
                    4,
                );
                if daw_ui::quiet_button(ui, "Retry visual analysis").clicked() {
                    self.sample_browser.visual.retry(
                        row.path.clone().into(),
                        self.settings.scratch_dir.clone(),
                        ui.ctx(),
                    );
                    ui.close();
                }
            }
            if daw_ui::button(ui, "Copy path").clicked() {
                ui.ctx().copy_text(row.path.clone());
                ui.close();
            }
        });
        ui.add_space(tokens::SPACE_SM);
        let favorite = self
            .sample_browser
            .library
            .favorite_for(Path::new(&row.path), row.source_name.as_deref())
            .map(|entry| (entry.id.clone(), favorite_view(&entry.favorite)));
        let mut toggle = false;
        let mut edit = false;
        daw_ui::control_row_wrapped(ui, |ui| {
            ui.add_enabled_ui(!self.sample_browser.library.busy(), |ui| {
                toggle = daw_ui::sample_favorite_button(
                    ui,
                    favorite.as_ref().map(|(_, favorite)| favorite),
                )
                .clicked();
                if let Some((_, favorite)) = &favorite {
                    daw_ui::sample_tag(ui, favorite);
                    edit = daw_ui::quiet_button(ui, "Edit tag…").clicked();
                }
            });
        });
        if self.sample_browser.library.busy() {
            daw_ui::text::note(ui, "Updating favorites…");
        }
        if toggle {
            self.toggle_sample_favorite(&row.id, ui.ctx());
        }
        if edit && let Some((id, value)) = favorite {
            self.sample_browser.tag_draft = Some(FavoriteDraft { id, value });
        }
        let mut apply = None;
        let mut cancel = false;
        if let Some(draft) = &mut self.sample_browser.tag_draft {
            daw_ui::sample_favorite_editor(ui, &mut draft.value);
            daw_ui::control_row_wrapped(ui, |ui| {
                if daw_ui::button(ui, "Apply tag").clicked() {
                    apply = Some((draft.id.clone(), favorite_model(&draft.value)));
                }
                cancel = daw_ui::quiet_button(ui, "Cancel tag").clicked();
            });
        }
        if let Some((id, value)) = apply {
            match self.sample_browser.library.update(&id, value) {
                Ok(()) => self.sample_browser.tag_draft = None,
                Err(error) => self.sample_browser.notice = Some((error.to_string(), true)),
            }
        }
        if cancel {
            self.sample_browser.tag_draft = None;
        }
    }

    fn source_in_project(&self, source: &daw_ui::SampleDrop) -> bool {
        self.project.samples.iter().any(|sample| {
            Path::new(&sample.path) == source.path
                && source.name.as_ref().is_none_or(|name| *name == sample.name)
        }) || self
            .sample_browser
            .memberships
            .get(&(source.path.clone(), source.name.clone()))
            .is_some_and(|id| self.project.samples.iter().any(|sample| sample.id == *id))
    }
    fn sample_use_actions(&mut self, ui: &mut egui::Ui) {
        let source = self
            .sample_browser
            .selected
            .as_deref()
            .and_then(|id| self.sample_source(id));
        let busy = self.audio_import_busy();
        if self.sample_browser.captured {
            let label = self
                .sample_target_label()
                .unwrap_or_else(|| "Choose a destination".into());
            daw_ui::text::truncated(ui, daw_ui::text::TextRole::Note, &label).on_hover_text(&label);
            daw_ui::control_row(ui, |ui| {
                if ui
                    .add_enabled(
                        source.is_some() && self.valid_sample_target().is_some() && !busy,
                        daw_ui::button_widget("Load sample"),
                    )
                    .clicked()
                    && let Some(source) = source.clone()
                {
                    self.load_browser_source(source);
                }
                if ui
                    .add_enabled(!busy, daw_ui::quiet_button_widget("Change…"))
                    .clicked()
                {
                    self.open_sample_destination(false);
                }
            });
        } else {
            let contained = source
                .as_ref()
                .is_some_and(|source| self.source_in_project(source));
            if ui
                .add_enabled(
                    source.is_some() && !contained && !busy,
                    daw_ui::button_widget(if contained {
                        "In Project Samples"
                    } else {
                        "Add to Project Samples"
                    }),
                )
                .clicked()
                && let Some(source) = source.clone()
            {
                self.add_browser_source_to_project(source);
            }
        }
        daw_ui::control_row(ui, |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                daw_ui::choice_menu(ui, "Use…", |ui| {
                    if self.sample_browser.captured {
                        if ui
                            .add_enabled(
                                source.is_some()
                                    && !source
                                        .as_ref()
                                        .is_some_and(|source| self.source_in_project(source)),
                                daw_ui::quiet_button_widget("Add to Project Samples"),
                            )
                            .clicked()
                        {
                            if let Some(source) = source.clone() {
                                self.add_browser_source_to_project(source);
                            }
                            ui.close();
                        }
                        if daw_ui::menu_item(ui, "Load file…", None).clicked() {
                            self.use_browser_file();
                            ui.close();
                        }
                    } else {
                        if ui
                            .add_enabled(
                                source.is_some(),
                                daw_ui::quiet_button_widget("Load into instrument…"),
                            )
                            .clicked()
                        {
                            self.open_sample_destination(true);
                            ui.close();
                        }
                        if daw_ui::menu_item(ui, "Use file…", None).clicked() {
                            self.use_browser_file();
                            ui.close();
                        }
                    }
                    if ui
                        .add_enabled(
                            source.is_some(),
                            daw_ui::quiet_button_widget("Add audio track"),
                        )
                        .clicked()
                    {
                        if let Some(source) = source.clone() {
                            self.add_browser_audio_track(source);
                        }
                        ui.close();
                    }
                });
            });
            if busy && daw_ui::quiet_button(ui, "Cancel import").clicked() {
                self.cancel_audio_import();
            }
        });
        if let Some(pending) = &self.sample_browser.pending {
            daw_ui::text::bounded_block(
                ui,
                daw_ui::text::TextRole::Note,
                format!("Loading {}…", pending.name),
                2,
            );
        } else if self.sample_browser.library.busy() {
            daw_ui::text::note(ui, "Updating favorites…");
        } else if let Some((notice, _)) = &self.sample_browser.notice {
            daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note, notice, 2)
                .on_hover_text(notice);
        }
    }
    fn remember_sample_use(
        &mut self,
        ticket: ImportTicket,
        source: &daw_ui::SampleDrop,
        kind: UseKind,
        return_to: Option<(u64, u64)>,
    ) {
        let name = source.name.clone().unwrap_or_else(|| {
            source
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
        self.sample_browser.pending = Some(PendingUse {
            ticket,
            name,
            kind,
            return_to,
            context: self.sample_browser.context,
        });
        self.sample_browser.notice = None;
    }
    fn add_browser_source_to_project(&mut self, source: daw_ui::SampleDrop) {
        if self.audio_import_busy() || self.source_in_project(&source) {
            return;
        }
        let ticket = self.queue_project_sample_import_tracked(vec![source.clone()]);
        self.remember_sample_use(ticket, &source, UseKind::Project, None);
    }
    fn add_browser_audio_track(&mut self, source: daw_ui::SampleDrop) {
        let ticket =
            self.queue_audio_source_import_tracked(vec![source.clone()], None, self.cursor);
        self.remember_sample_use(ticket, &source, UseKind::Track, None);
    }
    fn load_browser_source(&mut self, source: daw_ui::SampleDrop) {
        let Some((track, slot)) = self.valid_sample_target() else {
            self.sample_browser.notice =
                Some(("Choose a Sampler or Drums destination".into(), true));
            return;
        };
        let ticket = self.queue_sample_source_import_tracked(source.clone(), track, slot);
        let return_to = self
            .sample_browser
            .captured
            .then_some((self.sample_browser.context, track));
        self.remember_sample_use(ticket, &source, UseKind::Device, return_to);
        self.stop_sample_preview();
    }
    fn use_browser_file(&mut self) {
        if !self.import_picker_ready() {
            return;
        }
        if let Some(path) = self.pick_document_path(crate::document_ui::DocumentDialog::ImportAudio)
        {
            if self.sample_browser.captured {
                self.load_browser_source(path.into());
            } else {
                self.add_browser_source_to_project(path.into());
            }
        }
    }
    fn valid_sample_target(&self) -> Option<(u64, u32)> {
        let (id, slot) = self.sample_browser.target?;
        let track = self.project.track(id)?;
        let descriptor = track
            .instrument
            .builtin_device()
            .and_then(daw_devices::descriptor)?;
        sample_destinations(&track.instrument, descriptor.sample_slots)
            .iter()
            .any(|(candidate, _)| *candidate == slot)
            .then_some((id, slot))
    }
    fn sample_target_label(&self) -> Option<String> {
        let (id, slot) = self.valid_sample_target()?;
        let track = self.project.track(id)?;
        let descriptor = track
            .instrument
            .builtin_device()
            .and_then(daw_devices::descriptor)?;
        let (_, leaf) = sample_destinations(&track.instrument, descriptor.sample_slots)
            .into_iter()
            .find(|(pool, _)| *pool == slot)?;
        Some(format!("{} / {leaf}", track.name))
    }
    fn open_sample_destination(&mut self, load: bool) {
        self.sample_browser.destination_original = Some(self.sample_browser.target);
        if self.valid_sample_target().is_none() {
            self.sample_browser.target = self.project.tracks.iter().find_map(|track| {
                let descriptor = track
                    .instrument
                    .builtin_device()
                    .and_then(daw_devices::descriptor)?;
                sample_destinations(&track.instrument, descriptor.sample_slots)
                    .first()
                    .map(|(slot, _)| (track.id, *slot))
            });
        }
        self.sample_browser.destination_load = load;
        self.sample_browser.destination = true;
    }

    fn sample_scan_notice(&self) -> Option<String> {
        if self.sample_browser.index.scanning() {
            Some("Scanning folders…".into())
        } else if let Some(error) = &self.sample_browser.index.error {
            Some(if error.to_lowercase().contains("cancel") {
                "Scan canceled".into()
            } else {
                "Scan could not finish".into()
            })
        } else if !self.sample_browser.index.warnings().is_empty() {
            Some("Some folders unavailable".into())
        } else if self.sample_browser.index.roots() != self.settings.sample_roots.as_slice() {
            Some("Folders need scanning".into())
        } else {
            None
        }
    }
    fn sample_source_footer(&mut self, ui: &mut egui::Ui) {
        if daw_ui::quiet_button(ui, "Folders…").clicked() {
            self.sample_browser.folders = true;
        }
        if self.sample_browser.index.scanning() {
            if daw_ui::quiet_button(ui, "Cancel scan").clicked() {
                self.sample_browser.index.cancel();
            }
        } else if daw_ui::button(ui, "Scan folders").clicked() {
            self.sample_browser
                .index
                .scan(self.settings.sample_roots.clone(), ui.ctx().clone());
        }
        if let Some(notice) = self.sample_scan_notice() {
            daw_ui::text::truncated(ui, daw_ui::text::TextRole::Note, notice);
            if (self.sample_browser.index.error.is_some()
                || !self.sample_browser.index.warnings().is_empty())
                && daw_ui::quiet_button(ui, "Details").clicked()
            {
                self.sample_browser.scan_details = true;
            }
        }
    }
    fn sample_empty_state(&mut self, ui: &mut egui::Ui) {
        ui.add_space(tokens::SPACE_LG);
        if !self.sample_browser.query.trim().is_empty() {
            daw_ui::text::section_heading(ui, "No matching samples");
            daw_ui::text::note(
                ui,
                format!(
                    "Try another search in {}.",
                    self.sample_browser.scope.label()
                ),
            );
            if daw_ui::button(ui, "Clear search").clicked() {
                self.sample_browser.query.clear();
                self.sample_library_tree();
            }
            if self.sample_browser.scope != SourceScope::All
                && daw_ui::quiet_button(ui, "Search all samples").clicked()
            {
                self.set_sample_scope("all");
            }
        } else {
            match &self.sample_browser.scope {
                SourceScope::Project => {
                    daw_ui::text::section_heading(ui, "Keep sounds with this project");
                    daw_ui::text::bounded_block(
                        ui,
                        daw_ui::text::TextRole::Note,
                        "Drop samples onto Project samples, or bounce a sound from the song.",
                        3,
                    );
                    if daw_ui::button(ui, "Browse all samples").clicked() {
                        self.set_sample_scope("all");
                    }
                }
                SourceScope::Favorites => {
                    daw_ui::text::section_heading(ui, "Your favorite sounds live here");
                    daw_ui::text::bounded_block(
                        ui,
                        daw_ui::text::TextRole::Note,
                        "Star any sample to keep it available across projects.",
                        3,
                    );
                    if daw_ui::button(ui, "Browse all samples").clicked() {
                        self.set_sample_scope("all");
                    }
                }
                _ if self.settings.sample_roots.is_empty() => {
                    daw_ui::text::section_heading(ui, "Find your first sound");
                    daw_ui::text::bounded_block(
                        ui,
                        daw_ui::text::TextRole::Note,
                        "Choose sample folders, or add an audio file to this project.",
                        3,
                    );
                    if daw_ui::button(ui, "Add folder…").clicked() {
                        self.sample_browser.folders = true;
                    }
                }
                _ if self.sample_browser.index.roots() != self.settings.sample_roots.as_slice() => {
                    daw_ui::text::section_heading(ui, "Ready to index your folders");
                    daw_ui::text::bounded_block(
                        ui,
                        daw_ui::text::TextRole::Note,
                        "Use Scan folders to build the library. Rill scans only when you ask.",
                        3,
                    );
                }
                _ => {
                    daw_ui::text::section_heading(ui, "No samples in this folder");
                    daw_ui::text::note(ui, "Choose another source or scan after adding files.");
                }
            }
            if daw_ui::quiet_button(
                ui,
                if self.sample_browser.captured {
                    "Load file…"
                } else {
                    "Use file…"
                },
            )
            .clicked()
            {
                self.use_browser_file();
            }
        }
    }
    fn sample_browser_dialogs(&mut self, ctx: &egui::Context) {
        if self.sample_browser.folders {
            let mut open = true;
            let mut add = None;
            let mut remove = None;
            daw_ui::popup("Sample folders", ctx)
                .id(egui::Id::new("sample_folder_manager"))
                .open(&mut open)
                .show(ctx, |ui| {
                    daw_ui::text::note(ui, "Add folders here, then scan when you are ready.");
                    for (index, root) in self.settings.sample_roots.iter().enumerate() {
                        ui.push_id(index, |ui| {
                            daw_ui::path_row(ui, root, |ui| {
                                if daw_ui::quiet_button(ui, "Remove folder").clicked() {
                                    remove = Some(index);
                                }
                            });
                        });
                    }
                    let field = daw_ui::text_field(
                        ui,
                        &mut self.sample_browser.root_draft,
                        "Sample folder path",
                        ui.available_width(),
                    );
                    daw_ui::name_keyboard_button(ui, field.id, &mut self.sample_browser.root_draft);
                    daw_ui::control_row_wrapped(ui, |ui| {
                        if daw_ui::button(ui, "Add folder").clicked() {
                            add = Some(PathBuf::from(self.sample_browser.root_draft.trim()));
                        }
                        if daw_ui::quiet_button(ui, "Browse folder…").clicked() {
                            add = daw_ui::with_native_dialog_menu(|| {
                                rfd::FileDialog::new().pick_folder()
                            });
                        }
                    });
                    if daw_ui::button(ui, "Done").clicked() {
                        self.sample_browser.folders = false;
                    }
                });
            self.sample_browser.folders &= open;
            if let Some(path) = add {
                self.add_sample_root(path);
            }
            if let Some(index) = remove {
                let mut next = self.settings.clone();
                next.sample_roots.remove(index);
                match next.save() {
                    Ok(()) => {
                        self.settings = next;
                        self.setup_draft.sample_roots = self.settings.sample_roots.clone();
                        if matches!(&self.sample_browser.scope, SourceScope::Folder(path) if !self.settings.sample_roots.iter().any(|root| path.starts_with(root)))
                        {
                            self.sample_browser.scope = SourceScope::All;
                        }
                    }
                    Err(error) => self.sample_browser.notice = Some((error.to_string(), true)),
                }
            }
        }
        if self.sample_browser.scan_details {
            let mut open = true;
            daw_ui::popup("Sample scan details", ctx)
                .id(egui::Id::new("sample_scan_details"))
                .open(&mut open)
                .show(ctx, |ui| {
                    if let Some(error) = &self.sample_browser.index.error {
                        daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note, error, 10);
                    }
                    for warning in self.sample_browser.index.warnings() {
                        daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note, warning, 10);
                    }
                    if daw_ui::button(ui, "Done").clicked() {
                        self.sample_browser.scan_details = false;
                    }
                });
            self.sample_browser.scan_details &= open;
        }
        if self.sample_browser.destination {
            let mut open = true;
            let mut load = false;
            let mut accepted = false;
            daw_ui::popup("Sample destination", ctx)
                .id(egui::Id::new("sample_destination"))
                .open(&mut open)
                .show(ctx, |ui| {
                    if self.sample_browser.destination_load
                        && let Some(row) = self.selected_sample_row()
                    {
                        daw_ui::text::bounded_block(
                            ui,
                            daw_ui::text::TextRole::Label,
                            &row.name,
                            2,
                        );
                    }
                    let tracks: Vec<_> = self
                        .project
                        .tracks
                        .iter()
                        .filter_map(|track| {
                            let descriptor = track
                                .instrument
                                .builtin_device()
                                .and_then(daw_devices::descriptor)?;
                            (descriptor.sample_slots > 0).then_some((
                                track.id,
                                track.name.clone(),
                                descriptor.sample_slots,
                            ))
                        })
                        .collect();
                    if tracks.is_empty() {
                        daw_ui::text::note(
                            ui,
                            "Add a Sampler or Drums instrument to choose a destination.",
                        );
                    }
                    daw_ui::text::label(ui, "Instrument");
                    let label = self
                        .sample_browser
                        .target
                        .and_then(|(id, _)| tracks.iter().find(|(track, _, _)| *track == id))
                        .map_or("Choose instrument", |(_, name, _)| name.as_str())
                        .to_owned();
                    daw_ui::select("sample_target_track", label)
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            for (id, name, groups) in &tracks {
                                if daw_ui::selectable(
                                    ui,
                                    self.sample_browser
                                        .target
                                        .is_some_and(|(track, _)| track == *id),
                                    name,
                                )
                                .clicked()
                                {
                                    self.sample_browser.target =
                                        self.project.track(*id).and_then(|track| {
                                            sample_destinations(&track.instrument, *groups)
                                                .first()
                                                .map(|(slot, _)| (*id, *slot))
                                        });
                                }
                            }
                        });
                    if let Some((track, slot)) = self.sample_browser.target
                        && let Some(model) = self.project.track(track)
                        && let Some(descriptor) = model
                            .instrument
                            .builtin_device()
                            .and_then(daw_devices::descriptor)
                    {
                        let choices =
                            sample_destinations(&model.instrument, descriptor.sample_slots);
                        daw_ui::text::label(ui, "Player / sample");
                        let label = choices
                            .iter()
                            .find(|(pool, _)| *pool == slot)
                            .map_or("Choose sample", |(_, label)| label.as_str())
                            .to_owned();
                        daw_ui::select("sample_target_leaf", label)
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                for (pool, label) in choices {
                                    if daw_ui::selectable(ui, pool == slot, label).clicked() {
                                        self.sample_browser.target = Some((track, pool));
                                    }
                                }
                            });
                    }
                    daw_ui::control_row(ui, |ui| {
                        if ui
                            .add_enabled(
                                self.valid_sample_target().is_some() && !self.audio_import_busy(),
                                daw_ui::button_widget(if self.sample_browser.destination_load {
                                    "Load sample"
                                } else {
                                    "Use destination"
                                }),
                            )
                            .clicked()
                        {
                            load = self.sample_browser.destination_load;
                            accepted = true;
                            self.sample_browser.destination = false;
                        }
                        if daw_ui::quiet_button(ui, "Cancel").clicked() {
                            self.sample_browser.destination = false;
                        }
                    });
                });
            self.sample_browser.destination &= open;
            if !self.sample_browser.destination
                && let Some(original) = self.sample_browser.destination_original.take()
            {
                if !accepted {
                    self.sample_browser.target = original;
                }
            }
            if load
                && let Some(source) = self
                    .sample_browser
                    .selected
                    .as_deref()
                    .and_then(|id| self.sample_source(id))
            {
                self.load_browser_source(source);
            }
        }
    }
    fn add_sample_root(&mut self, path: PathBuf) {
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(path.is_dir(), "Choose an existing sample folder");
            let path = path.canonicalize()?;
            if self.settings.sample_roots.contains(&path) {
                return Ok(());
            }
            anyhow::ensure!(
                self.settings.sample_roots.len() < 64,
                "Choose at most 64 sample folders"
            );
            let mut next = self.settings.clone();
            next.sample_roots.push(path);
            next.save()?;
            self.settings = next;
            self.setup_draft.sample_roots = self.settings.sample_roots.clone();
            self.sample_browser.root_draft.clear();
            Ok(())
        })();
        if let Err(error) = result {
            self.sample_browser.notice = Some((error.to_string(), true));
        }
    }
    fn sample_metadata_diagnostics(&self) -> serde_json::Value {
        let Some(source) = self
            .sample_browser
            .selected
            .as_deref()
            .and_then(|id| self.sample_source(id))
        else {
            return serde_json::Value::Null;
        };
        match self.sample_browser.metadata.selected(&source.path) {
            Some(crate::sample_metadata::SampleMetadataStatus::Ready(metadata)) => {
                serde_json::json!({"state":"ready","duration_seconds":metadata.duration_seconds,"format":metadata.format,"codec":metadata.codec,"sample_rate":metadata.sample_rate,"channels":metadata.channels,"bits_per_sample":metadata.bits_per_sample,"file_bytes":metadata.file_bytes})
            }
            Some(crate::sample_metadata::SampleMetadataStatus::Failed(error)) => {
                serde_json::json!({"state":"failed","error":error})
            }
            _ => serde_json::json!({"state":"loading"}),
        }
    }
    fn sample_visual_diagnostics(&self) -> serde_json::Value {
        let Some(source) = self
            .sample_browser
            .selected
            .as_deref()
            .and_then(|id| self.sample_source(id))
        else {
            return serde_json::Value::Null;
        };
        let rect = |rect: egui::Rect| serde_json::json!({"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]});
        let mut value = match self.sample_browser.visual.selected(&source.path) {
            crate::sample_visual::SampleVisualStatus::Idle => serde_json::json!({"state":"idle"}),
            crate::sample_visual::SampleVisualStatus::Loading => {
                serde_json::json!({"state":"loading"})
            }
            crate::sample_visual::SampleVisualStatus::Failed(error) => {
                serde_json::json!({"state":"failed","error":error})
            }
            crate::sample_visual::SampleVisualStatus::Ready(data) => serde_json::json!({
                "state":"ready","waveform":data.waveform,"spectrum":data.spectrum,
                "min_frequency":data.min_frequency,"max_frequency":data.max_frequency,
                "sample_rate":data.sample_rate,"source_frames":data.source_frames,
            }),
        };
        value["path"] = serde_json::json!(source.path);
        value["bounds"] = self.sample_browser.visual_bounds.map(rect).into();
        value["waveform_rect"] = self.sample_browser.visual_waveform.map(rect).into();
        value["spectrum_rect"] = self.sample_browser.visual_spectrum.map(rect).into();
        value
    }
    fn sample_browser_diagnostics(
        &self,
        ui: &egui::Ui,
        results: &daw_ui::SampleResultsResponse,
        sources: &daw_ui::SampleSourcesResponse,
        palette: bool,
    ) {
        let rect = |rect: egui::Rect| serde_json::json!({"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]});
        let samples: Vec<_> = results.rows.iter().filter_map(|(id, response)| self.sample_source(id).map(|source| serde_json::json!({"id":id,"path":source.path,"rect":rect(response.rect),"min":[response.rect.min.x,response.rect.min.y],"max":[response.rect.max.x,response.rect.max.y]}))).collect();
        let audition = self
            .sample_browser
            .selected
            .as_deref()
            .and_then(|id| self.sample_source(id))
            .map(|source| {
                let (state, error) = match self.sample_preview_status(&source.path) {
                    crate::sample_preview::SamplePreviewStatus::Idle => ("idle", None),
                    crate::sample_preview::SamplePreviewStatus::Loading => ("loading", None),
                    crate::sample_preview::SamplePreviewStatus::Playing => ("playing", None),
                    crate::sample_preview::SamplePreviewStatus::Failed(error) => {
                        ("failed", Some(error))
                    }
                };
                serde_json::json!({"state":state,"path":source.path,"error":error})
            });
        let data = serde_json::json!({
            "audition":audition,"results_focused":daw_ui::sample_results_owns_keys(ui.ctx()),
            "bounds":rect(ui.max_rect().intersect(ui.clip_rect())),"view":"samples","scanning":self.sample_browser.index.scanning(),"total":self.sample_browser.results.len(),"indexed":self.sample_browser.index.entries().len(),"roots":self.settings.sample_roots,
            "scope":self.sample_browser.scope.key(),"scope_label":self.sample_browser.scope.label(),"query":self.sample_browser.query,"selected":self.sample_browser.selected,"target":self.sample_browser.target,"captured":self.sample_browser.captured,
            "auto_play":self.settings.sample_auto_play,"favorites_only":self.sample_browser.scope==SourceScope::Favorites,
            "favorites":self.sample_browser.library.entries().iter().map(|entry| serde_json::json!({"id":entry.id,"path":entry.path,"name":entry.name,"favorite":entry.favorite})).collect::<Vec<_>>(),"favorite_busy":self.sample_browser.library.busy(),
            "visual_analysis":self.sample_visual_diagnostics(),"metadata":self.sample_metadata_diagnostics(),"notice":self.sample_browser.notice,"pending_import":self.sample_browser.pending.as_ref().map(|pending| pending.ticket.id),
            "drop_targets":sources.drop_targets.iter().map(|(id,response)|serde_json::json!({"id":id,"rect":rect(response.rect),"min":[response.rect.min.x,response.rect.min.y],"max":[response.rect.max.x,response.rect.max.y]})).collect::<Vec<_>>(),
            "sources":sources.rows.iter().map(|(id,response)|serde_json::json!({"id":id,"rect":rect(response.rect)})).collect::<Vec<_>>(),"samples":samples,
        });
        ui.ctx().data_mut(|storage| {
            storage.insert_temp(
                egui::Id::new(if palette {
                    "sample_palette_diagnostics"
                } else {
                    "library_diagnostics"
                }),
                data,
            )
        });
    }
}

/// A tree path names its destination; the source-pool index stays an internal
/// address and never pretends to be an ordered layer number.
fn sample_destinations(instrument: &daw_core::Instrument, groups: u32) -> Vec<(u32, String)> {
    fn leaves(
        instrument: &daw_core::Instrument,
        node: &daw_core::PlayerNode,
        prefix: &str,
        output: &mut Vec<(u32, String)>,
    ) {
        let fallback = match &node.kind {
            daw_core::PlayerKind::Sample { slot, .. } => instrument
                .sample(*slot)
                .map_or("Empty sample", |sample| sample.name.as_str()),
            daw_core::PlayerKind::All { .. } => "All at once",
            daw_core::PlayerKind::RoundRobin { .. } => "Round robin",
            daw_core::PlayerKind::Random { .. } => "Random",
            daw_core::PlayerKind::Velocity { .. } => "Velocity",
        };
        let name = if node.name.trim().is_empty() {
            fallback
        } else {
            &node.name
        };
        let label = format!("{prefix} / {name}");
        match &node.kind {
            daw_core::PlayerKind::Sample { slot, .. } => output.push((*slot, label)),
            daw_core::PlayerKind::All { children }
            | daw_core::PlayerKind::RoundRobin { children }
            | daw_core::PlayerKind::Random { children } => {
                for child in children {
                    leaves(instrument, child, &label, output);
                }
            }
            daw_core::PlayerKind::Velocity { children } => {
                for child in children {
                    leaves(instrument, &child.child, &label, output);
                }
            }
        }
    }
    let mut output = Vec::new();
    for group in 0..groups {
        let label = instrument
            .sample_name(group)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if groups == 1 {
                    "Sampler".into()
                } else {
                    format!("Pad {}", group + 1)
                }
            });
        if let Some(player) = instrument.sample_player(group) {
            leaves(instrument, player, &label, &mut output);
        } else {
            output.push((group, label));
        }
    }
    output
}

fn favorite_view(value: &daw_core::SampleFavorite) -> daw_ui::SampleFavorite {
    let icon = match value.icon {
        daw_core::SampleTagIcon::Star => daw_ui::SampleTagIcon::Star,
        daw_core::SampleTagIcon::Drum => daw_ui::SampleTagIcon::Drum,
        daw_core::SampleTagIcon::Percussion => daw_ui::SampleTagIcon::Percussion,
        daw_core::SampleTagIcon::Tone => daw_ui::SampleTagIcon::Tone,
        daw_core::SampleTagIcon::Texture => daw_ui::SampleTagIcon::Texture,
        daw_core::SampleTagIcon::Voice => daw_ui::SampleTagIcon::Voice,
        daw_core::SampleTagIcon::Foley => daw_ui::SampleTagIcon::Foley,
    };
    daw_ui::SampleFavorite {
        tag: value.tag.clone(),
        color: value.color,
        icon,
    }
}
fn favorite_model(value: &daw_ui::SampleFavorite) -> daw_core::SampleFavorite {
    let icon = match value.icon {
        daw_ui::SampleTagIcon::Star => daw_core::SampleTagIcon::Star,
        daw_ui::SampleTagIcon::Drum => daw_core::SampleTagIcon::Drum,
        daw_ui::SampleTagIcon::Percussion => daw_core::SampleTagIcon::Percussion,
        daw_ui::SampleTagIcon::Tone => daw_core::SampleTagIcon::Tone,
        daw_ui::SampleTagIcon::Texture => daw_core::SampleTagIcon::Texture,
        daw_ui::SampleTagIcon::Voice => daw_core::SampleTagIcon::Voice,
        daw_ui::SampleTagIcon::Foley => daw_core::SampleTagIcon::Foley,
    };
    daw_core::SampleFavorite {
        tag: value.tag.clone(),
        color: value.color,
        icon,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn find_sample<'a>(rows: &'a [SampleResult], id: &str) -> Option<&'a SampleResult> {
        rows.iter().find(|row| row.id == id)
    }

    fn finish_library(library: &mut crate::sample_library::SampleLibrary, ctx: &egui::Context) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while library.busy() {
            library.poll(ctx);
            assert!(
                std::time::Instant::now() < deadline,
                "favorite job timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(library.take_errors().is_empty());
    }

    #[test]
    fn late_project_resolution_preserves_selected_global_favorite_and_tag_draft() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original.wav");
        let reopened = directory.path().join("reopened.wav");
        for path in [&original, &reopened] {
            std::fs::write(path, b"same immutable sample asset").unwrap();
        }
        let ctx = egui::Context::default();
        let mut library =
            crate::sample_library::SampleLibrary::load(directory.path().join("config"));
        library.request_add(
            daw_ui::SampleDrop {
                path: original,
                name: Some("Kick".into()),
            },
            true,
            &ctx,
        );
        finish_library(&mut library, &ctx);
        let favorite = library.entries()[0].clone();
        let mut project = daw_core::Project::new();
        let project_id = project
            .add_project_sample("Kick", reopened.to_string_lossy())
            .unwrap();
        let mut app = DawApp::with_context(&ctx, project, None);
        app.settings.sample_roots.clear();
        app.sample_browser.library = library;
        app.sample_library_tree();
        let favorite_id = format!("favorite:{}", favorite.id);
        assert!(find_sample(&app.sample_browser.results, &favorite_id).is_some());
        assert!(
            find_sample(
                &app.sample_browser.results,
                &format!("project:{project_id}")
            )
            .unwrap()
            .favorite
            .is_none()
        );

        // A selection in the reopened project starts content lookup. Before
        // its result is polled, the user moves to the global row and edits a tag.
        app.sample_browser.library.resolve(
            daw_ui::SampleDrop {
                path: reopened,
                name: Some("Kick".into()),
            },
            &ctx,
        );
        app.sample_browser.selected = Some(favorite_id.clone());
        let mut draft = favorite_view(&favorite.favorite);
        draft.tag = "Unapplied edit".into();
        app.sample_browser.tag_draft = Some(FavoriteDraft {
            id: favorite.id,
            value: draft,
        });
        finish_library(&mut app.sample_browser.library, &ctx);
        app.sample_library_tree();

        assert!(
            find_sample(
                &app.sample_browser.results,
                &format!("project:{project_id}")
            )
            .unwrap()
            .favorite
            .is_some()
        );
        assert!(find_sample(&app.sample_browser.results, &favorite_id).is_some());
        assert_eq!(
            app.sample_browser.selected.as_deref(),
            Some(favorite_id.as_str())
        );
        assert_eq!(
            app.sample_browser.tag_draft.as_ref().unwrap().value.tag,
            "Unapplied edit"
        );

        // An explicit filter still clears a selection that no longer matches.
        app.sample_browser.query = "unmatched filter".into();
        app.sample_library_tree();
        assert!(app.sample_browser.selected.is_none());
        assert!(app.sample_browser.tag_draft.is_none());
    }
}

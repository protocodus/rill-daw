//! Bounded sample navigation, virtual results, and selected-source presentation.
//! File discovery, source identity, audition, and loading belong to callers.
use crate::{Icon, IconButton, SampleDrop, text, theme, tokens};
use egui::{Id, Response, Sense, Ui};

/// Presentation metadata only. The host owns persistence and source identity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleFavorite {
    pub tag: String,
    pub color: Option<[u8; 3]>,
    pub icon: SampleTagIcon,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SampleTagIcon {
    #[default]
    Star,
    Drum,
    Percussion,
    Tone,
    Texture,
    Voice,
    Foley,
}

impl SampleTagIcon {
    pub const ALL: [Self; 7] = [
        Self::Star,
        Self::Drum,
        Self::Percussion,
        Self::Tone,
        Self::Texture,
        Self::Voice,
        Self::Foley,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Star => "Star",
            Self::Drum => "Drum",
            Self::Percussion => "Percussion",
            Self::Tone => "Tone",
            Self::Texture => "Texture",
            Self::Voice => "Voice",
            Self::Foley => "Foley",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::Star => Icon::StarFilled,
            Self::Drum => Icon::Drum,
            Self::Percussion => Icon::Percussion,
            Self::Tone => Icon::Note,
            Self::Texture => Icon::Waveform,
            Self::Voice => Icon::Microphone,
            Self::Foley => Icon::Footsteps,
        }
    }
}

/// A constant star action shared by result rows and selected details.
pub fn sample_favorite_button(ui: &mut Ui, favorite: Option<&SampleFavorite>) -> Response {
    let label = if favorite.is_some() {
        "Remove favorite"
    } else {
        "Add favorite"
    };
    let icon = if favorite.is_some() {
        Icon::StarFilled
    } else {
        Icon::Star
    };
    let color = if favorite.is_some() {
        theme::palette(ui.ctx()).action
    } else {
        theme::palette(ui.ctx()).muted
    };
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().button_padding.x = tokens::SPACE_SM;
            ui.add_sized(
                egui::Vec2::splat(tokens::CONTROL_HEIGHT),
                IconButton::icon_only(icon, label)
                    .presentation()
                    .quiet()
                    .small()
                    .label_color(color),
            )
        })
        .inner;
    response.on_hover_text(
        favorite
            .filter(|favorite| !favorite.tag.is_empty())
            .map_or_else(
                || label.to_owned(),
                |favorite| format!("{} · {}\n{label}", favorite.tag, favorite.icon.label()),
            ),
    )
}

/// An on-demand editor; callers decide when to reveal it and persist changes.
/// Every choice has a named mouse control, including clearing a custom color.
pub fn sample_favorite_editor(ui: &mut Ui, favorite: &mut SampleFavorite) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        let label = text::label(ui, "Favorite tag");
        let response = crate::text_field(
            ui,
            &mut favorite.tag,
            "Optional label",
            (ui.available_width() - 2.0 * tokens::BUTTON_PADDING_X).max(1.0),
        )
        .labelled_by(label.id);
        if response.changed() {
            favorite.tag = favorite
                .tag
                .chars()
                .filter(|character| !character.is_control())
                .take(64)
                .collect();
            changed = true;
        }
        crate::control_row(ui, |ui| {
            text::label(ui, "Tag icon");
            crate::select("sample_tag_icon", favorite.icon.label())
                .width((ui.available_width() - tokens::SPACE_SM).max(80.0))
                .show_ui(ui, |ui| {
                    for icon in SampleTagIcon::ALL {
                        if crate::selectable(ui, favorite.icon == icon, icon.label()).clicked() {
                            changed |= favorite.icon != icon;
                            favorite.icon = icon;
                        }
                    }
                });
        });
        crate::control_row_wrapped(ui, |ui| {
            text::label(ui, "Tag color");
            let mut color = favorite.color.unwrap_or([128, 128, 128]);
            if crate::color_edit(ui, &mut color)
                .on_hover_text("Tag color")
                .changed()
            {
                favorite.color = Some(color);
                changed = true;
            }
            if ui
                .add_enabled(
                    favorite.color.is_some(),
                    crate::quiet_button_widget("Clear color"),
                )
                .clicked()
            {
                favorite.color = None;
                changed = true;
            }
        });
    });
    changed
}

/// Source metadata prepared off the UI thread by the host. Missing values are
/// unknown, never invented from the decoded playback representation.
#[derive(Clone, Debug, Default)]
pub struct SampleDetails<'a> {
    pub name: &'a str,
    pub location: &'a str,
    pub duration_seconds: Option<f64>,
    pub format: Option<&'a str>,
    pub codec: Option<&'a str>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub bit_depth: Option<u16>,
    pub size_bytes: Option<u64>,
    pub status: Option<&'a str>,
}

/// Bounded identity and a wrapping metadata summary, suitable for a details
/// pane or a 260-point Player palette footer. No filesystem access or probing.
pub fn sample_details(ui: &mut Ui, details: &SampleDetails<'_>) -> Response {
    ui.vertical(|ui| {
        if !details.name.is_empty() {
            text::truncated(ui, text::TextRole::Label, details.name).on_hover_text(details.name);
        }
        if !details.location.is_empty() {
            text::truncated(ui, text::TextRole::Note, details.location)
                .on_hover_text(details.location);
        }
        let duration = details
            .duration_seconds
            .map(sample_duration)
            .unwrap_or_else(|| "Length unknown".into());
        let format = details.format.unwrap_or("Format unknown");
        let codec = details.codec.filter(|codec| {
            !codec.eq_ignore_ascii_case(format) && !codec.to_ascii_uppercase().starts_with("PCM_")
        });
        text::note(
            ui,
            match codec {
                Some(codec) => format!("{duration} · {format} · {codec}"),
                None => format!("{duration} · {format}"),
            },
        );
        let mut audio = Vec::new();
        if let Some(rate) = details.sample_rate {
            let rate = format!("{:.3}", rate as f64 / 1000.0);
            audio.push(format!(
                "{} kHz",
                rate.trim_end_matches('0').trim_end_matches('.')
            ));
        }
        if let Some(channels) = details.channels {
            audio.push(match channels {
                1 => "Mono".into(),
                2 => "Stereo".into(),
                channels => format!("{channels} channels"),
            });
        }
        let pcm = details
            .codec
            .map(str::to_ascii_uppercase)
            .unwrap_or_default();
        if pcm.starts_with("PCM_F") {
            audio.push(details.bit_depth.map_or_else(
                || "Floating point PCM".into(),
                |depth| format!("{depth}-bit float"),
            ));
        } else if pcm.starts_with("PCM_") {
            audio.push(
                details
                    .bit_depth
                    .map_or_else(|| "PCM".into(), |depth| format!("{depth}-bit PCM")),
            );
        } else if let Some(depth) = details.bit_depth {
            audio.push(format!("{depth}-bit"));
        }
        if let Some(size) = details.size_bytes {
            audio.push(sample_size(size));
        }
        if !audio.is_empty() {
            text::note(ui, audio.join(" · "));
        }
        if let Some(status) = details.status {
            text::bounded_block(ui, text::TextRole::Note, status, 3);
        }
    })
    .response
}

fn sample_duration(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "Unknown".into();
    }
    if seconds < 60.0 {
        format!("{seconds:.2} s")
    } else {
        let seconds = seconds.round() as u64;
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

fn sample_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

/// One selectable source scope. Folder children come from the cached index;
/// this control never reads directories or filters the source hierarchy.
#[derive(Clone, Debug)]
pub struct SampleSourceNode {
    pub id: String,
    pub name: String,
    pub icon: Icon,
    pub detail: Option<String>,
    pub children: Vec<SampleSourceNode>,
    pub drop_target: bool,
}

#[derive(Default)]
pub struct SampleSourcesResponse {
    pub selected: Option<String>,
    pub dropped: Option<(String, SampleDrop)>,
    pub rows: Vec<(String, Response)>,
    pub drop_targets: Vec<(String, Response)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SampleAuditionState {
    #[default]
    Idle,
    Loading,
    Playing,
    Failed,
}

#[derive(Clone, Debug, Default)]
pub struct SampleResult {
    pub id: String,
    pub name: String,
    pub provenance: String,
    pub path: String,
    pub source_name: Option<String>,
    pub duration_seconds: Option<f64>,
    pub favorite: Option<SampleFavorite>,
    pub audition: SampleAuditionState,
}

#[derive(Default)]
pub struct SampleResultsResponse {
    pub selected: Option<String>,
    pub preview: Option<String>,
    pub toggle_preview: bool,
    pub favorite_toggled: Option<String>,
    pub dragged: Option<String>,
    /// Stable source IDs and the name/provenance drag region.
    pub rows: Vec<(String, Response)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleRegion {
    Header,
    Body,
    Footer,
}

/// Bounded header, middle, and footer. The owner chooses middle scrolling;
/// sources and result lists already own their virtualized scroll area.
pub fn sample_regions(
    ui: &mut Ui,
    id: Id,
    height: f32,
    header_height: f32,
    footer_height: f32,
    mut contents: impl FnMut(SampleRegion, &mut Ui),
) {
    let height = height.max(0.0).min(ui.available_height().max(0.0));
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let header_height = header_height.max(0.0).min(height);
    let footer_height = footer_height
        .max(0.0)
        .min((height - header_height).max(0.0));
    let gap_top = if header_height > 0.0 {
        tokens::SPACE_SM
    } else {
        0.0
    };
    let gap_bottom = if footer_height > 0.0 {
        tokens::SPACE_SM
    } else {
        0.0
    };
    let footer_top = rect.bottom() - footer_height;
    let body_top = (rect.top() + header_height + gap_top).min(footer_top);
    let body_bottom = (footer_top - gap_bottom).max(body_top);
    for (region, region_rect) in [
        (
            SampleRegion::Header,
            egui::Rect::from_x_y_ranges(rect.x_range(), rect.top()..=rect.top() + header_height),
        ),
        (
            SampleRegion::Body,
            egui::Rect::from_x_y_ranges(rect.x_range(), body_top..=body_bottom),
        ),
        (
            SampleRegion::Footer,
            egui::Rect::from_x_y_ranges(rect.x_range(), footer_top..=rect.bottom()),
        ),
    ] {
        if region_rect.height() <= 0.0 {
            continue;
        }
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(id.with(format!("{region:?}")))
                .max_rect(region_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(ui.clip_rect().intersect(region_rect));
        child.set_width(region_rect.width());
        child.spacing_mut().item_spacing.y = tokens::SPACE_SM;
        contents(region, &mut child);
    }
}

/// Folder tree, results, and sample inspector with draggable dividers.
pub fn sample_browser_workspace(
    ui: &mut Ui,
    id: Id,
    height: f32,
    mut contents: impl FnMut(usize, &mut Ui),
) -> [egui::Rect; 3] {
    crate::BrowserPanes::new(ui, id, [196.0, 280.0], tokens::SEARCH_CHOICE_WIDTH).show(
        ui,
        height,
        |index, ui| {
            ui.spacing_mut().item_spacing.y = tokens::SPACE_SM;
            contents(index, ui);
        },
    )
}

/// Search stays in one bounded row. The second line states the current scope
/// and count; changing the query never changes source navigation.
pub fn sample_search(ui: &mut Ui, query: &mut String, scope: &str, count: usize) -> bool {
    let mut changed = false;
    crate::control_row(ui, |ui| {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_SM;
        ui.spacing_mut().button_padding.x = tokens::SPACE_SM;
        let clear_width = if query.is_empty() {
            0.0
        } else {
            tokens::CONTROL_HEIGHT + tokens::SPACE_SM
        };
        // TextEdit's desired width excludes its horizontal content margins.
        let field_width =
            (ui.available_width() - clear_width - 2.0 * tokens::BUTTON_PADDING_X).max(1.0);
        let response = crate::text_field(ui, query, "Search samples", field_width);
        changed |= response.changed();
        if !query.is_empty()
            && ui
                .add_sized(
                    egui::Vec2::splat(tokens::CONTROL_HEIGHT),
                    IconButton::icon_only(Icon::Close, "Clear search").quiet(),
                )
                .on_hover_text("Clear search")
                .clicked()
        {
            query.clear();
            changed = true;
        }
    });
    crate::control_row(ui, |ui| {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_SM;
        let count = text::count(count, "sample", "samples");
        let count_width = ui
            .painter()
            .layout_no_wrap(
                count.clone(),
                text::TextRole::Note.font(),
                theme::palette(ui.ctx()).muted,
            )
            .size()
            .x;
        let scope_width = (ui.available_width() - count_width - tokens::SPACE_SM).max(0.0);
        ui.allocate_ui_with_layout(
            egui::vec2(scope_width, tokens::CONTROL_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(scope_width);
                text::truncated(ui, text::TextRole::Label, scope);
            },
        );
        text::note(ui, count);
    });
    changed
}

/// Global collections stay visible while only the cached folder tree scrolls.
pub fn sample_sources(
    ui: &mut Ui,
    id: Id,
    collections: &[SampleSourceNode],
    folders: &[SampleSourceNode],
    selected: &str,
    height: f32,
) -> SampleSourcesResponse {
    let mut output = SampleSourcesResponse::default();
    let heading = tokens::CONTROL_HEIGHT;
    let header =
        collections.len() as f32 * (tokens::CONTROL_HEIGHT + ui.spacing().item_spacing.y) + heading;
    sample_regions(ui, id, height, header, 0.0, |region, ui| match region {
        SampleRegion::Header => {
            for node in collections {
                source_row(ui, id, node, selected, None, &mut output);
            }
            text::section_heading(ui, "Folders");
        }
        SampleRegion::Body => {
            let mut visible = Vec::new();
            visible_source_rows(ui, id, folders, 0, &mut visible);
            egui::ScrollArea::vertical()
                .id_salt(id.with("folders"))
                .auto_shrink([false, false])
                .show_rows(ui, tokens::CONTROL_HEIGHT, visible.len(), |ui, range| {
                    for index in range {
                        let (node, depth) = visible[index];
                        source_row(ui, id, node, selected, Some(depth), &mut output);
                    }
                });
        }
        SampleRegion::Footer => (),
    });
    output
}

fn source_open(ui: &Ui, id: Id, node: &SampleSourceNode) -> bool {
    ui.data(|data| data.get_temp::<bool>(id.with(("folder", &node.id))))
        .unwrap_or(false)
}
fn visible_source_rows<'a>(
    ui: &Ui,
    id: Id,
    nodes: &'a [SampleSourceNode],
    depth: usize,
    output: &mut Vec<(&'a SampleSourceNode, usize)>,
) {
    for node in nodes {
        output.push((node, depth));
        if source_open(ui, id, node) {
            visible_source_rows(ui, id, &node.children, depth + 1, output);
        }
    }
}
fn source_row(
    ui: &mut Ui,
    id: Id,
    node: &SampleSourceNode,
    selected: &str,
    depth: Option<usize>,
    output: &mut SampleSourcesResponse,
) {
    ui.push_id(&node.id, |ui| {
        let response = crate::control_row(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.spacing_mut().button_padding.x = tokens::SPACE_SM;
            if let Some(depth) = depth {
                ui.add_space(
                    (depth as f32 * tokens::SPACE_SM).min((ui.available_width() - 120.0).max(0.0)),
                );
                if node.children.is_empty() {
                    ui.allocate_exact_size(
                        egui::Vec2::splat(tokens::CONTROL_HEIGHT),
                        Sense::hover(),
                    );
                } else {
                    let open = source_open(ui, id, node);
                    let label = if open {
                        format!("Collapse {}", node.name)
                    } else {
                        format!("Expand {}", node.name)
                    };
                    if ui
                        .add_sized(
                            egui::Vec2::splat(tokens::CONTROL_HEIGHT),
                            IconButton::icon_only(
                                if open {
                                    Icon::ChevronDown
                                } else {
                                    Icon::ChevronRight
                                },
                                label,
                            )
                            .presentation()
                            .quiet(),
                        )
                        .clicked()
                    {
                        ui.data_mut(|data| data.insert_temp(id.with(("folder", &node.id)), !open));
                    }
                }
            }
            ui.add_sized(
                [ui.available_width(), tokens::CONTROL_HEIGHT],
                IconButton::new(&node.name)
                    .icon(node.icon)
                    .presentation()
                    .left_aligned()
                    .truncate()
                    .selected(node.id == selected)
                    .frame_when_inactive(node.id == selected),
            )
        })
        .inner;
        let response = response.on_hover_text(node.detail.as_deref().unwrap_or(&node.name));
        if response.clicked() {
            output.selected = Some(node.id.clone());
        }
        if node.drop_target {
            paint_sample_drop(ui, &response);
            if let Some(source) = response.dnd_release_payload::<SampleDrop>() {
                output.dropped = Some((node.id.clone(), (*source).clone()));
            }
            output
                .drop_targets
                .push((node.id.clone(), response.clone()));
        }
        output.rows.push((node.id.clone(), response));
    });
}

pub fn sample_project_drop_target(ui: &mut Ui, label: &str) -> (Response, Option<SampleDrop>) {
    let response = ui.add_sized(
        [ui.available_width(), tokens::CONTROL_HEIGHT],
        IconButton::new(label)
            .icon(Icon::Library)
            .presentation()
            .left_aligned()
            .truncate(),
    );
    paint_sample_drop(ui, &response);
    let dropped = response
        .dnd_release_payload::<SampleDrop>()
        .map(|source| (*source).clone());
    (
        response.on_hover_text("Drop a sample to add it to Project samples"),
        dropped,
    )
}
fn paint_sample_drop(ui: &Ui, response: &Response) {
    if response.dnd_hover_payload::<SampleDrop>().is_some() {
        ui.painter().rect_stroke(
            response.rect,
            tokens::RADIUS,
            egui::Stroke::new(tokens::FOCUS_WIDTH, theme::palette(ui.ctx()).action),
            egui::StrokeKind::Inside,
        );
    }
}

fn result_focus_key() -> Id {
    Id::new("sample_result_focus_ids")
}
/// Used before the workspace is drawn, so global Enter/Space handling cannot
/// steal a key from the focused result list. Text editors and menus own theirs.
pub fn sample_results_owns_keys(ctx: &egui::Context) -> bool {
    !egui::Popup::is_any_open(ctx)
        && ctx
            .data(|data| data.get_temp::<Vec<Id>>(result_focus_key()))
            .is_some_and(|ids| ctx.memory(|memory| ids.iter().any(|id| memory.has_focus(*id))))
}

pub fn sample_result_row_height(ui: &Ui) -> f32 {
    let text_height = ui.fonts_mut(|fonts| {
        fonts.row_height(&text::TextRole::Body.font()).ceil()
            + fonts.row_height(&text::TextRole::Note.font()).ceil()
    });
    (tokens::CONTROL_HEIGHT + 2.0 * tokens::SPACE_XS).max(text_height + 2.0 * tokens::SPACE_XS)
}

/// Flat fixed-height rows. Only visible rows become widgets, regardless of
/// collection size. Selection and preview are distinct from import actions.
pub fn sample_results(
    ui: &mut Ui,
    id: Id,
    rows: &[SampleResult],
    selected: Option<&str>,
    height: f32,
) -> SampleResultsResponse {
    let mut output = SampleResultsResponse::default();
    let focus = id.with("focus");
    ui.ctx().data_mut(|data| {
        let mut ids = data
            .get_temp::<Vec<Id>>(result_focus_key())
            .unwrap_or_default();
        if !ids.contains(&focus) {
            ids.push(focus);
        }
        data.insert_temp(result_focus_key(), ids);
    });
    let viewport = egui::Rect::from_min_size(
        ui.cursor().min,
        egui::vec2(ui.available_width(), height.max(0.0)),
    );
    ui.interact(viewport, focus, Sense::focusable_noninteractive());
    lock_result_arrows(ui, focus);
    let focused =
        ui.memory(|memory| memory.has_focus(focus)) && !egui::Popup::is_any_open(ui.ctx());
    let mut reveal = None;
    let mut selected = selected;
    if focused {
        let (up, down, enter, space) = ui.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Space),
            )
        });
        if up || down {
            // The first arrow after a pointer focus can reach egui before its
            // lock filter is installed; keep native directional navigation
            // from moving focus to a row's favorite button at end of frame.
            ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        }
        if (up || down) && !rows.is_empty() {
            let current = selected.and_then(|id| rows.iter().position(|row| row.id == id));
            let index = match current {
                Some(index) if down => (index + 1).min(rows.len() - 1),
                Some(index) => index.saturating_sub(1),
                None => 0,
            };
            output.selected = Some(rows[index].id.clone());
            selected = Some(rows[index].id.as_str());
            reveal = Some(index);
        }
        if enter {
            output.preview = selected.map(str::to_owned);
        }
        output.toggle_preview = space && selected.is_some();
    }
    let row_height = sample_result_row_height(ui);
    let scroll_id = ui.make_persistent_id(id.with("scroll"));
    let current_offset =
        egui::scroll_area::State::load(ui.ctx(), scroll_id).map_or(0.0, |state| state.offset.y);
    let mut area = egui::ScrollArea::vertical()
        .id_salt(id.with("scroll"))
        .max_height(height.max(0.0))
        .auto_shrink([false, false]);
    if let Some(index) = reveal {
        let top = index as f32 * row_height;
        let bottom = top + row_height;
        let offset = if top < current_offset {
            top
        } else if bottom > current_offset + height {
            bottom - height
        } else {
            current_offset
        };
        area = area.vertical_scroll_offset(offset.max(0.0));
    }
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        area.show_rows(ui, row_height, rows.len(), |ui, range| {
            for index in range {
                result_row(
                    ui,
                    id,
                    focus,
                    &rows[index],
                    selected,
                    row_height,
                    &mut output,
                );
            }
        });
    });
    output
}

fn result_row(
    ui: &mut Ui,
    id: Id,
    focus: Id,
    row: &SampleResult,
    selected: Option<&str>,
    height: f32,
    output: &mut SampleResultsResponse,
) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let colors = theme::palette(ui.ctx());
    let favorite_rect = egui::Rect::from_min_size(
        egui::pos2(
            rect.right() - tokens::CONTROL_HEIGHT,
            rect.center().y - tokens::CONTROL_HEIGHT * 0.5,
        ),
        egui::Vec2::splat(tokens::CONTROL_HEIGHT),
    );
    let duration_width = if rect.width() >= 360.0 { 64.0 } else { 0.0 };
    let duration_rect = egui::Rect::from_x_y_ranges(
        favorite_rect.left() - duration_width..=favorite_rect.left(),
        rect.y_range(),
    );
    let name_rect = egui::Rect::from_x_y_ranges(rect.left()..=duration_rect.left(), rect.y_range());
    let response = ui.interact(
        name_rect,
        id.with(("row", &row.id)),
        Sense::click_and_drag(),
    );
    let chosen = selected == Some(row.id.as_str());
    if chosen || response.hovered() {
        ui.painter().rect_filled(
            rect,
            tokens::RADIUS,
            if chosen {
                colors.track_selected
            } else {
                colors.control_hover
            },
        );
    }
    if chosen && ui.memory(|memory| memory.has_focus(focus)) {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            tokens::RADIUS,
            egui::Stroke::new(tokens::FOCUS_WIDTH, colors.accent),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            chosen,
            &row.name,
        )
    });
    let icon = match row.audition {
        SampleAuditionState::Idle => Icon::Waveform,
        SampleAuditionState::Loading => Icon::Refresh,
        SampleAuditionState::Playing => Icon::Play,
        SampleAuditionState::Failed => Icon::Warning,
    };
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(name_rect.left() + tokens::SPACE_MD, name_rect.center().y),
        egui::Vec2::splat(tokens::ICON_SIZE),
    );
    icon.paint(
        ui.painter(),
        icon_rect,
        if row.audition == SampleAuditionState::Idle {
            colors.muted
        } else {
            colors.action
        },
    );
    let text_left = icon_rect.right() + tokens::SPACE_SM;
    let (body_height, note_height) = ui.fonts_mut(|fonts| {
        (
            fonts.row_height(&text::TextRole::Body.font()).ceil(),
            fonts.row_height(&text::TextRole::Note.font()).ceil(),
        )
    });
    let text_top = rect.center().y - (body_height + note_height) * 0.5;
    let mut title = egui::Rect::from_x_y_ranges(
        text_left..=(name_rect.right() - tokens::SPACE_SM).max(text_left),
        text_top..=text_top + body_height,
    );
    let mut subtitle = egui::Rect::from_x_y_ranges(
        title.x_range(),
        title.bottom()..=title.bottom() + note_height,
    );
    if let Some(favorite) = &row.favorite {
        let tag_rect = egui::Rect::from_center_size(
            egui::pos2(title.right() - tokens::ICON_SIZE * 0.5, rect.center().y),
            egui::Vec2::splat(tokens::ICON_SIZE),
        );
        if name_rect.width() >= 180.0
            && (favorite.icon != SampleTagIcon::Star
                || !favorite.tag.is_empty()
                || favorite.color.is_some())
        {
            favorite
                .icon
                .icon()
                .paint(ui.painter(), tag_rect, favorite_color(ui.ctx(), favorite));
            title.max.x = tag_rect.left() - tokens::SPACE_SM;
            subtitle.max.x = title.max.x;
        }
    }
    text::paint_row(ui.painter(), title, text::TextRole::Body, &row.name);
    let provenance = match row.audition {
        SampleAuditionState::Loading => format!("Loading · {}", row.provenance),
        SampleAuditionState::Playing => format!("Playing · {}", row.provenance),
        SampleAuditionState::Failed => format!("Preview failed · {}", row.provenance),
        SampleAuditionState::Idle => row.provenance.clone(),
    };
    let provenance = row
        .favorite
        .as_ref()
        .filter(|favorite| !favorite.tag.is_empty())
        .map_or(provenance.clone(), |favorite| {
            format!("{provenance} · {}", favorite.tag)
        });
    text::paint_row(ui.painter(), subtitle, text::TextRole::Note, &provenance);
    if duration_width > 0.0 {
        ui.painter().with_clip_rect(duration_rect).text(
            egui::pos2(
                duration_rect.right() - tokens::SPACE_XS,
                duration_rect.center().y,
            ),
            egui::Align2::RIGHT_CENTER,
            row.duration_seconds
                .map(sample_duration)
                .unwrap_or_else(|| "—".into()),
            text::TextRole::Note.font(),
            colors.muted,
        );
    }
    let mut favorite_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id.with(("favorite", &row.id)))
            .max_rect(favorite_rect),
    );
    if sample_favorite_button(&mut favorite_ui, row.favorite.as_ref()).clicked() {
        output.favorite_toggled = Some(row.id.clone());
    }
    crate::context_menu(&response, |ui| {
        if crate::button(ui, "Preview sample").clicked() {
            output.preview = Some(row.id.clone());
            ui.close();
        }
        let favorite = if row.favorite.is_some() {
            "Remove from favorites"
        } else {
            "Add to favorites"
        };
        if crate::button(ui, favorite).clicked() {
            output.favorite_toggled = Some(row.id.clone());
            ui.close();
        }
        if crate::button(ui, "Copy path").clicked() {
            ui.ctx().copy_text(row.path.clone());
            ui.close();
        }
    });
    response.dnd_set_drag_payload(SampleDrop {
        path: row.path.clone().into(),
        name: row.source_name.clone(),
    });
    if response.clicked() {
        ui.memory_mut(|memory| memory.request_focus(focus));
        lock_result_arrows(ui, focus);
        output.selected = Some(row.id.clone());
    }
    if response.double_clicked() {
        output.preview = Some(row.id.clone());
    }
    if response.drag_started() {
        output.dragged = Some(row.id.clone());
    }
    output.rows.push((
        row.id.clone(),
        response.on_hover_text(format!("{}\n{}", row.name, row.path)),
    ));
}

fn lock_result_arrows(ui: &Ui, focus: Id) {
    ui.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            focus,
            egui::EventFilter {
                vertical_arrows: true,
                ..Default::default()
            },
        );
    });
}

fn favorite_color(ctx: &egui::Context, favorite: &SampleFavorite) -> egui::Color32 {
    favorite.color.map_or_else(
        || theme::palette(ctx).muted,
        |color| theme::readable_track_color(ctx, color),
    )
}

/// Category decoration is distinct from the membership star. The text stays in
/// theme colors, while the optional icon alone carries the chosen tag color.
pub fn sample_tag(ui: &mut Ui, favorite: &SampleFavorite) -> Response {
    crate::control_row(ui, |ui| {
        let (rect, response) =
            ui.allocate_exact_size(egui::Vec2::splat(tokens::CONTROL_HEIGHT), Sense::hover());
        favorite.icon.icon().paint(
            ui.painter(),
            rect.shrink(tokens::SPACE_SM),
            favorite_color(ui.ctx(), favorite),
        );
        text::truncated(
            ui,
            text::TextRole::Note,
            if favorite.tag.is_empty() {
                favorite.icon.label()
            } else {
                &favorite.tag
            },
        );
        response.on_hover_text(favorite.icon.label())
    })
    .response
}

//! The frame, preset widgets and generated panel shared by every device.
//!
//! A device panel is a `device_panel` frame (name, preset bar, bypass, meters
//! and the Mix/Output or Volume knobs) around titled `device_section`s.
//! `generic_device_panel` builds the whole panel from plain parameter rows, so
//! this crate needs no knowledge of the device crates. See `docs/UI.md` and
//! `docs/R1_DEVICES.md` for the panel grammar.
use crate::{
    AutomationState, Icon, IconButton, KnobSize, labeled_knob, segmented, stepped_selector,
    text::TextRole, theme, toggle_led, tokens, value_text,
};
use egui::{Align, Layout, Response, Ui, Vec2, vec2};
use std::ops::RangeInclusive;

/// Borrowed text parser of a parameter.
pub type ParseRef<'a> = dyn Fn(&str) -> Option<f64> + 'a;

/// The two widths a device panel may have. Nothing in between.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DevicePanelWidth {
    #[default]
    Medium,
    Large,
}

impl DevicePanelWidth {
    pub fn points(self) -> f32 {
        match self {
            Self::Medium => tokens::POPUP_WIDTH_MD,
            Self::Large => tokens::POPUP_WIDTH_LG,
        }
    }
}

/// One preset in the browser.
#[derive(Clone, Copy, Debug)]
pub struct PresetEntry<'a> {
    pub name: &'a str,
    pub category: &'a str,
    pub tags: &'a [&'a str],
    pub description: &'a str,
    pub factory: bool,
    pub favorite: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PresetAction {
    Previous,
    Next,
    /// Index into the entries given to the panel.
    Load(usize),
    ToggleFavorite(usize),
    SaveAs(String),
    Reset,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PresetSource {
    #[default]
    All,
    Factory,
    User,
}

/// Search and filter state of a [`preset_browser`]; keep one per panel.
#[derive(Clone, Debug, Default)]
pub struct PresetBrowserState {
    pub search: String,
    pub category: Option<String>,
    pub tag: Option<String>,
    pub source: PresetSource,
    pub favorites_only: bool,
    pub save_name: String,
}

impl PresetBrowserState {
    /// Whether an entry passes every active filter. The search is a
    /// case-insensitive substring of the name, category, tags or description.
    pub fn matches(&self, entry: &PresetEntry<'_>) -> bool {
        let search = self.search.trim().to_lowercase();
        (search.is_empty()
            || [entry.name, entry.category, entry.description]
                .into_iter()
                .chain(entry.tags.iter().copied())
                .any(|text| text.to_lowercase().contains(&search)))
            && self
                .category
                .as_deref()
                .is_none_or(|category| category == entry.category)
            && self
                .tag
                .as_deref()
                .is_none_or(|tag| entry.tags.contains(&tag))
            && match self.source {
                PresetSource::All => true,
                PresetSource::Factory => entry.factory,
                PresetSource::User => !entry.factory,
            }
            && (!self.favorites_only || entry.favorite)
    }
}

fn distinct<'a>(values: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut values: Vec<&str> = values.filter(|value| !value.is_empty()).collect();
    values.sort_unstable();
    values.dedup();
    values
}

/// Searchable preset list with category and tag filters, Factory/User,
/// favorites, **Save as…** and **Reset**. Identical for every device.
pub fn preset_browser(
    ui: &mut Ui,
    entries: &[PresetEntry<'_>],
    current: Option<usize>,
    state: &mut PresetBrowserState,
) -> Option<PresetAction> {
    let mut action = None;
    ui.set_width(tokens::POPUP_WIDTH_SM + tokens::SPACE_XL);
    ui.horizontal(|ui| {
        crate::text_field(ui, &mut state.search, "Search presets", 180.0);
        let mut source = state.source as usize;
        if segmented(ui, &mut source, &["All", "Factory", "User"]).changed() {
            state.source = [PresetSource::All, PresetSource::Factory, PresetSource::User][source];
        }
        crate::toggle(ui, &mut state.favorites_only, "Favorites");
    });
    ui.add_space(tokens::SPACE_SM);
    for (title, values, selected) in [
        (
            "Category",
            distinct(entries.iter().map(|entry| entry.category)),
            &mut state.category,
        ),
        (
            "Tag",
            distinct(entries.iter().flat_map(|entry| entry.tags.iter().copied())),
            &mut state.tag,
        ),
    ] {
        if values.is_empty() {
            continue;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(tokens::SPACE_XS, tokens::SPACE_XS);
            crate::text::note(ui, title);
            if crate::selectable(ui, selected.is_none(), "All").clicked() {
                *selected = None;
            }
            for value in values {
                if crate::selectable(ui, selected.as_deref() == Some(value), value).clicked() {
                    *selected = Some(value.to_owned());
                }
            }
        });
    }
    ui.add_space(tokens::SPACE_SM);
    let shown: Vec<usize> = (0..entries.len())
        .filter(|index| state.matches(&entries[*index]))
        .collect();
    egui::ScrollArea::vertical()
        .id_salt("preset_list")
        .max_height(tokens::SPACE_XL * 7.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for index in &shown {
                let entry = &entries[*index];
                ui.horizontal(|ui| {
                    let star = ui.add(
                        IconButton::icon_only(
                            Icon::Star,
                            if entry.favorite {
                                "Remove favorite"
                            } else {
                                "Add favorite"
                            },
                        )
                        .selected(entry.favorite),
                    );
                    if star.clicked() {
                        action = Some(PresetAction::ToggleFavorite(*index));
                    }
                    let name = crate::selectable(ui, current == Some(*index), entry.name);
                    if name.clicked() {
                        action = Some(PresetAction::Load(*index));
                    }
                    if !entry.description.is_empty() {
                        crate::hover_text(name, entry.description);
                    }
                    crate::text::note(
                        ui,
                        if entry.factory {
                            entry.category.to_owned()
                        } else {
                            format!("{} · User", entry.category)
                        },
                    );
                });
            }
            if shown.is_empty() {
                crate::text::note(ui, "No presets match.");
            }
        });
    ui.add_space(tokens::SPACE_SM);
    ui.separator();
    ui.horizontal(|ui| {
        crate::text_field(ui, &mut state.save_name, "Name", 180.0);
        let name = state.save_name.trim().to_owned();
        if ui
            .add_enabled(!name.is_empty(), crate::button_widget("Save as…"))
            .clicked()
        {
            action = Some(PresetAction::SaveAs(name));
            state.save_name.clear();
        }
        if crate::button(ui, "Reset to default").clicked() {
            action = Some(PresetAction::Reset);
        }
    });
    action
}

/// Preset data the panel header shows.
#[derive(Clone, Copy)]
pub struct PresetInfo<'a> {
    pub name: &'a str,
    pub modified: bool,
    pub entries: &'a [PresetEntry<'a>],
    pub current: Option<usize>,
}

/// Previous, current name (opens the browser) and next. Returns the name
/// button's response and a Previous/Next action.
pub fn preset_bar(ui: &mut Ui, name: &str, modified: bool) -> (Response, Option<PresetAction>) {
    let mut action = None;
    let mut name_response = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_XS;
        if ui
            .add(IconButton::icon_only(Icon::Back, "Previous preset"))
            .clicked()
        {
            action = Some(PresetAction::Previous);
        }
        let label = if modified {
            format!("{name} •")
        } else {
            name.to_owned()
        };
        let button = ui.add_sized(
            vec2(
                tokens::SELECT_WIDTH + tokens::SPACE_MD,
                tokens::CONTROL_HEIGHT,
            ),
            IconButton::with_icon(Icon::ChevronDown, label).wrap_mode(egui::TextWrapMode::Truncate),
        );
        name_response = Some(button.on_hover_text(if modified {
            "Modified since loading · click to browse presets"
        } else {
            "Click to browse presets"
        }));
        if ui
            .add(IconButton::icon_only(Icon::ChevronRight, "Next preset"))
            .clicked()
        {
            action = Some(PresetAction::Next);
        }
    });
    (
        name_response.expect("the preset bar always draws its name button"),
        action,
    )
}

/// A header knob: Mix, Output or Volume, always dense.
pub struct HeaderKnob<'a> {
    pub caption: &'a str,
    pub value: &'a mut f64,
    pub range: RangeInclusive<f64>,
    pub default: f64,
    pub unit: &'a str,
    pub format: Option<&'a dyn Fn(f64) -> String>,
    pub parse: Option<&'a ParseRef<'a>>,
    pub automation: Option<AutomationState>,
}

pub struct DevicePanelOutput {
    pub preset: Option<PresetAction>,
    /// One response per header knob, in order, for gesture tracking.
    pub header: Vec<Response>,
    /// Parallel to `header`: **Automate** was chosen on that knob.
    pub header_automate: Vec<bool>,
}

const NAME_WIDTH: f32 = 130.0;
const BYPASS_WIDTH: f32 = 42.0;
const METER_SIZE: Vec2 = vec2(13.0, tokens::CONTROL_HEIGHT);

/// The frame every device shares. `body` draws the sections.
#[must_use = "Call show"]
pub struct DevicePanel<'a> {
    name: &'a str,
    width: DevicePanelWidth,
    bypassed: &'a mut bool,
    presets: Option<PresetInfo<'a>>,
    input: Option<[f32; 2]>,
    output: Option<[f32; 2]>,
    knobs: Vec<HeaderKnob<'a>>,
}

pub fn device_panel<'a>(
    name: &'a str,
    width: DevicePanelWidth,
    bypassed: &'a mut bool,
) -> DevicePanel<'a> {
    DevicePanel {
        name,
        width,
        bypassed,
        presets: None,
        input: None,
        output: None,
        knobs: Vec::new(),
    }
}

impl<'a> DevicePanel<'a> {
    pub fn presets(mut self, presets: PresetInfo<'a>) -> Self {
        self.presets = Some(presets);
        self
    }
    /// Peak readings (linear). Instruments pass only `output`.
    pub fn meters(mut self, input: Option<[f32; 2]>, output: Option<[f32; 2]>) -> Self {
        self.input = input;
        self.output = output;
        self
    }
    pub fn knob(mut self, knob: HeaderKnob<'a>) -> Self {
        self.knobs.push(knob);
        self
    }

    pub fn show(self, ui: &mut Ui, body: impl FnOnce(&mut Ui)) -> DevicePanelOutput {
        let colors = theme::palette(ui.ctx());
        let total = self.width.points();
        let frame = crate::panel(ui.ctx());
        let inner_width = total - 2.0 * tokens::SPACE_LG;
        let mut output = DevicePanelOutput {
            preset: None,
            header: Vec::new(),
            header_automate: Vec::new(),
        };
        let Self {
            name,
            bypassed,
            presets,
            input,
            output: out_peaks,
            knobs,
            ..
        } = self;
        frame.show(ui, |ui| {
            ui.set_width(inner_width);
            ui.spacing_mut().item_spacing = vec2(tokens::SPACE_SM, tokens::SPACE_SM);
            let height = if knobs.is_empty() {
                tokens::CONTROL_HEIGHT
            } else {
                KnobSize::Dense.cell_height()
            };
            let meters = usize::from(input.is_some()) + usize::from(out_peaks.is_some());
            let right = BYPASS_WIDTH
                + meters as f32 * (METER_SIZE.x * 2.0 + tokens::SPACE_XS + tokens::SPACE_SM)
                + knobs.len() as f32 * (KnobSize::Dense.cell_width() + tokens::SPACE_SM);
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    vec2((inner_width - right - tokens::SPACE_MD).max(0.0), height),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.allocate_ui_with_layout(
                            vec2(NAME_WIDTH, height),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.add(
                                    egui::Label::new(TextRole::Subtitle.rich(ui.ctx(), name))
                                        .truncate(),
                                );
                            },
                        );
                        if let Some(presets) = presets {
                            let (button, action) = preset_bar(ui, presets.name, presets.modified);
                            output.preset = action;
                            let browser_id = button.id.with("browser");
                            let mut state = ui
                                .data(|data| data.get_temp::<PresetBrowserState>(browser_id))
                                .unwrap_or_default();
                            egui::Popup::menu(&button)
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                .id(browser_id.with("popup"))
                                .show(|ui| {
                                    if let Some(action) = preset_browser(
                                        ui,
                                        presets.entries,
                                        presets.current,
                                        &mut state,
                                    ) {
                                        output.preset = Some(action);
                                    }
                                });
                            ui.data_mut(|data| data.insert_temp(browser_id, state));
                        }
                    },
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.allocate_ui_with_layout(
                        vec2(right, height),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            let mut active = !*bypassed;
                            if toggle_led(ui, &mut active, "On")
                                .on_hover_text("Bypass the device when off")
                                .changed()
                            {
                                *bypassed = !active;
                            }
                            for (title, peaks) in [("Input", input), ("Output", out_peaks)] {
                                if let Some(peaks) = peaks {
                                    crate::stereo_meter(
                                        ui,
                                        peaks,
                                        vec2(METER_SIZE.x * 2.0 + tokens::SPACE_XS, METER_SIZE.y),
                                    )
                                    .on_hover_text(format!("{title} level"));
                                }
                            }
                            for knob in knobs {
                                let mut labeled =
                                    labeled_knob(knob.caption, knob.value, knob.range)
                                        .size(KnobSize::Dense)
                                        .unit(knob.unit)
                                        .default_value(knob.default)
                                        .automation(knob.automation);
                                if let Some(format) = knob.format {
                                    labeled = labeled.format(format);
                                }
                                if let Some(parse) = knob.parse {
                                    labeled = labeled.parse(parse);
                                }
                                let shown = labeled.show(ui);
                                output.header.push(shown.response);
                                output.header_automate.push(shown.automate);
                            }
                        },
                    );
                });
            });
            ui.add_space(tokens::SPACE_XS);
            let line = ui.available_rect_before_wrap();
            ui.painter().hline(
                line.x_range(),
                line.top(),
                egui::Stroke::new(tokens::BORDER_WIDTH, colors.line),
            );
            ui.add_space(tokens::SPACE_MD);
            body(ui);
        });
        output
    }
}

/// A titled group of controls: a one-word heading and the controls under it,
/// separated from its neighbors by spacing rather than an outline.
pub fn device_section<R>(ui: &mut Ui, title: &str, contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = tokens::SPACE_SM;
        device_section_title(ui, title);
        contents(ui)
    })
    .inner
}

/// The heading of a device section: its name in tracked capitals and a rule
/// to the section's right edge. Distinct from the sentence-case
/// [`control_caption`]s under it, so the panel's two levels never read alike.
pub fn device_section_title(ui: &mut Ui, title: &str) -> Response {
    let colors = crate::theme::palette(ui.ctx());
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &title.to_uppercase(),
        0.0,
        egui::TextFormat {
            font_id: crate::text::TextRole::Label.font(),
            color: colors.text,
            extra_letter_spacing: 1.0,
            ..Default::default()
        },
    );
    let galley = ui.painter().layout_job(job);
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, galley.size().y), egui::Sense::hover());
    let rule = rect.left() + galley.size().x + tokens::SPACE_SM;
    ui.painter().galley(rect.left_top(), galley, colors.text);
    if rule < rect.right() {
        ui.painter().hline(
            rule..=rect.right(),
            rect.center().y,
            egui::Stroke::new(tokens::BORDER_WIDTH, colors.line),
        );
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, title));
    response
}

/// The font of a [`control_caption`], for callers that size cells to it.
pub fn control_caption_font() -> egui::FontId {
    crate::text::TextRole::Note.font()
}

/// The name above one control of a device section: smaller and lighter than
/// the section's title, in the text color so it stays readable.
pub fn control_caption(ui: &mut Ui, text: &str) -> Response {
    let color = crate::theme::palette(ui.ctx()).text;
    ui.add(egui::Label::new(
        egui::RichText::new(text)
            .font(control_caption_font())
            .color(color),
    ))
}

/// How the host stores and edits parameter values for a generated panel.
/// Values are normalized (0 to 1). One drag is `begin_edit`, any number of
/// `set_value`s, then `end_edit`; hosts make each gesture one Undo step and
/// drop a gesture without a `set_value`.
pub trait ParameterEdits {
    fn value(&self, id: u32) -> f64;
    fn begin_edit(&mut self, id: u32);
    fn set_value(&mut self, id: u32, normalized: f64);
    fn end_edit(&mut self, id: u32);
    fn automation(&self, _id: u32) -> Option<AutomationState> {
        None
    }
    /// The user chose **Automate** on a parameter: create its arranger lane
    /// (or reveal it when `automation` already reports one).
    fn automate(&mut self, _id: u32) {}
}

pub enum ParamKind<'a> {
    Continuous { default: f64, bipolar: bool },
    Choice(&'a [&'a str]),
    Toggle,
}

/// One parameter of a device, in plain terms. `format` and `parse` map the
/// normalized value to text ("1.2 kHz") and back; without them the value
/// reads as a percentage.
pub struct ParamRow<'a> {
    pub id: u32,
    pub name: &'a str,
    pub group: &'a str,
    pub unit: &'a str,
    pub kind: ParamKind<'a>,
    pub format: Option<&'a dyn Fn(f64) -> String>,
    pub parse: Option<&'a ParseRef<'a>>,
}

/// Everything `generic_device_panel` needs besides the host.
pub struct GenericDevice<'a> {
    pub name: &'a str,
    pub rows: &'a [ParamRow<'a>],
    pub width: DevicePanelWidth,
    /// Rows shown as header knobs instead of in the body.
    pub mix: Option<u32>,
    pub output: Option<u32>,
    pub presets: Option<PresetInfo<'a>>,
    pub input_peaks: Option<[f32; 2]>,
    pub output_peaks: Option<[f32; 2]>,
}

/// Report one control's edit to the host as a gesture.
fn report(edits: &mut dyn ParameterEdits, id: u32, response: &Response, value: f64) {
    let during_drag = response.dragged() || response.drag_started() || response.drag_stopped();
    let changed = response.changed();
    if response.drag_started() || (changed && !during_drag) {
        edits.begin_edit(id);
    }
    if changed {
        edits.set_value(id, value);
    }
    if response.drag_stopped() || (changed && !during_drag) {
        edits.end_edit(id);
    }
}

const GROUP_GAP: f32 = tokens::SPACE_LG;

fn choice_index(value: f64, count: usize) -> usize {
    (value.clamp(0.0, 1.0) * count.saturating_sub(1) as f64).round() as usize
}

/// A complete panel generated from parameter rows: one section per group
/// (in order of appearance) with a knob, selector or toggle per row by kind,
/// flowing left to right and wrapping within the panel width.
pub fn generic_device_panel(
    ui: &mut Ui,
    device: &GenericDevice<'_>,
    edits: &mut dyn ParameterEdits,
    bypassed: &mut bool,
) -> DevicePanelOutput {
    let header_rows: Vec<&ParamRow<'_>> = [device.mix, device.output]
        .into_iter()
        .flatten()
        .filter_map(|id| device.rows.iter().find(|row| row.id == id))
        .collect();
    let mut header_values: Vec<f64> = header_rows.iter().map(|row| edits.value(row.id)).collect();
    let before = header_values.clone();
    let mut panel = device_panel(device.name, device.width, bypassed)
        .meters(device.input_peaks, device.output_peaks);
    if let Some(presets) = device.presets {
        panel = panel.presets(presets);
    }
    for (row, value) in header_rows.iter().zip(header_values.iter_mut()) {
        let ParamKind::Continuous { default, .. } = row.kind else {
            continue;
        };
        panel = panel.knob(HeaderKnob {
            caption: row.name,
            value,
            range: 0.0..=1.0,
            default,
            unit: row.unit,
            format: row.format,
            parse: row.parse,
            automation: edits.automation(row.id),
        });
    }
    let inner = device.width.points() - 2.0 * tokens::SPACE_LG;
    let body_rows: Vec<&ParamRow<'_>> = device
        .rows
        .iter()
        .filter(|row| !header_rows.iter().any(|header| header.id == row.id))
        .collect();
    let output = panel.show(ui, |ui| {
        let mut groups: Vec<(&str, Vec<&ParamRow<'_>>)> = Vec::new();
        for row in &body_rows {
            match groups.iter_mut().find(|(name, _)| *name == row.group) {
                Some((_, rows)) => rows.push(row),
                None => groups.push((row.group, vec![row])),
            }
        }
        let cell = KnobSize::Standard.cell_width();
        let widths: Vec<f32> = groups
            .iter()
            .map(|(_, rows)| {
                let knobs = rows
                    .iter()
                    .filter(|row| matches!(row.kind, ParamKind::Continuous { .. }))
                    .count();
                let columns = knobs.min(4) as f32;
                let selectors = rows
                    .iter()
                    .any(|row| !matches!(row.kind, ParamKind::Continuous { .. }));
                let knob_width = columns * cell + (columns - 1.0).max(0.0) * tokens::SPACE_SM;
                knob_width
                    .max(if selectors { 2.0 * cell } else { 0.0 })
                    .max(cell)
            })
            .collect();
        let mut start = 0;
        while start < groups.len() {
            let mut end = start + 1;
            let mut used = widths[start];
            while end < groups.len() && used + GROUP_GAP + widths[end] <= inner {
                used += GROUP_GAP + widths[end];
                end += 1;
            }
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = GROUP_GAP;
                for index in start..end {
                    let (title, rows) = &groups[index];
                    ui.allocate_ui_with_layout(
                        vec2(widths[index], 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(widths[index]);
                            device_section(ui, title, |ui| generic_group(ui, rows, edits));
                        },
                    );
                }
            });
            ui.add_space(GROUP_GAP);
            start = end;
        }
    });
    for (index, ((row, value), before)) in header_rows
        .iter()
        .zip(&header_values)
        .zip(&before)
        .enumerate()
    {
        if let Some(response) = output.header.get(index) {
            let mut response = response.clone();
            if value != before {
                response.mark_changed();
            }
            report(edits, row.id, &response, *value);
        }
        if output.header_automate.get(index).copied().unwrap_or(false) {
            edits.automate(row.id);
        }
    }
    output
}

fn generic_group(ui: &mut Ui, rows: &[&ParamRow<'_>], edits: &mut dyn ParameterEdits) {
    for row in rows {
        let value = edits.value(row.id);
        match &row.kind {
            ParamKind::Choice(options) => {
                ui.vertical(|ui| {
                    crate::text::note(ui, row.name);
                    let mut index = choice_index(value, options.len());
                    let response = if options.len() <= 5 && options.len() >= 2 {
                        segmented(ui, &mut index, options)
                    } else {
                        stepped_selector(ui, &mut index, options)
                    };
                    if response.changed() {
                        let next = index as f64 / options.len().saturating_sub(1).max(1) as f64;
                        edits.begin_edit(row.id);
                        edits.set_value(row.id, next);
                        edits.end_edit(row.id);
                    }
                });
            }
            ParamKind::Toggle => {
                let mut on = value >= 0.5;
                if toggle_led(ui, &mut on, row.name).changed() {
                    edits.begin_edit(row.id);
                    edits.set_value(row.id, if on { 1.0 } else { 0.0 });
                    edits.end_edit(row.id);
                }
            }
            ParamKind::Continuous { .. } => {}
        }
    }
    let knobs: Vec<&&ParamRow<'_>> = rows
        .iter()
        .filter(|row| matches!(row.kind, ParamKind::Continuous { .. }))
        .collect();
    for line in knobs.chunks(4) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = tokens::SPACE_SM;
            for row in line {
                let ParamKind::Continuous { default, bipolar } = row.kind else {
                    continue;
                };
                let mut value = edits.value(row.id);
                let mut knob = labeled_knob(row.name, &mut value, 0.0..=1.0)
                    .unit(row.unit)
                    .default_value(default)
                    .automation(edits.automation(row.id));
                if bipolar {
                    knob = knob.bipolar();
                }
                if let Some(format) = row.format {
                    knob = knob.format(format);
                } else {
                    knob = knob.format(|normalized| value_text(normalized * 100.0, "%"));
                }
                if let Some(parse) = row.parse {
                    knob = knob.parse(parse);
                }
                let shown = knob.show(ui);
                report(edits, row.id, &shown.response, value);
                if shown.automate {
                    edits.automate(row.id);
                }
            }
        });
    }
}

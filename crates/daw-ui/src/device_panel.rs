//! The frame, preset widgets and generated panel shared by every device.
//!
//! A device panel is a `device_panel` frame (name, preset bar, bypass, meters
//! and the Mix/Output or Volume knobs) around titled `device_section`s.
//! `generic_device_panel` builds the whole panel from plain parameter rows, so
//! this crate needs no knowledge of the device crates. See `docs/UI.md` and
//! `docs/R1_DEVICES.md` for the panel grammar.
use crate::{
    AutomationMark, AutomationState, Icon, IconButton, KnobSize, labeled_knob, segmented,
    stepped_selector, text::TextRole, theme, toggle_led, tokens, value_text,
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

/// One entry of a [`preset_group`]'s menu.
#[derive(Clone, Copy, Debug)]
pub struct PresetChoice<'a> {
    pub label: &'a str,
    /// Shown on hover; may be empty.
    pub description: &'a str,
}

/// Every preset control of a device in one bordered group, set apart from
/// the device's own controls: previous and next, the preset menu, **Save
/// as…** and **Reset**. `draft` is the name being typed while saving; the
/// group opens and closes it. Identical wherever presets appear.
pub fn preset_group(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash,
    name: &str,
    choices: &[PresetChoice<'_>],
    current: Option<usize>,
    draft: &mut Option<String>,
) -> Option<PresetAction> {
    let mut action = None;
    // The device popup scrolls only vertically, so where one row does not
    // fit the actions go on a second row instead of off its right edge. The
    // row's width is known once it has been drawn.
    let row_id = egui::Id::new((&id_salt, "preset_row_width"));
    let row_width = ui.data(|data| data.get_temp::<f32>(row_id));
    let one_row = ui.available_width() + 0.5
        >= row_width.unwrap_or(tokens::POPUP_WIDTH_SM + tokens::SPACE_XL * 4.0);
    crate::inset(ui.ctx())
        .inner_margin(egui::Margin::symmetric(
            tokens::SPACE_SM as i8,
            tokens::SPACE_XS as i8,
        ))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::splat(tokens::SPACE_XS);
            let navigation = |ui: &mut Ui, action: &mut Option<PresetAction>| {
                let title = group_title(ui, "Preset");
                let (rect, _) = ui.allocate_exact_size(
                    title.size() + vec2(tokens::SPACE_XS, 0.0),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .galley(rect.left_top(), title, egui::Color32::PLACEHOLDER);
                if ui
                    .add(IconButton::icon_only(Icon::Back, "Previous preset"))
                    .on_hover_text("Previous preset")
                    .clicked()
                {
                    *action = Some(PresetAction::Previous);
                }
                crate::select(id_salt, name)
                    .width(tokens::SELECT_WIDTH + tokens::SPACE_XL)
                    .truncate()
                    .show_ui(ui, |ui| {
                        for (index, choice) in choices.iter().enumerate() {
                            let response =
                                crate::selectable(ui, current == Some(index), choice.label);
                            let response = if choice.description.is_empty() {
                                response
                            } else {
                                response.on_hover_text(choice.description)
                            };
                            if response.clicked() {
                                *action = Some(PresetAction::Load(index));
                            }
                        }
                    });
                if ui
                    .add(IconButton::icon_only(Icon::ChevronRight, "Next preset"))
                    .on_hover_text("Next preset")
                    .clicked()
                {
                    *action = Some(PresetAction::Next);
                }
            };
            let mut actions = |ui: &mut Ui, action: &mut Option<PresetAction>| {
                if let Some(mut name) = draft.take() {
                    crate::text_field(ui, &mut name, "Preset name", tokens::SELECT_WIDTH);
                    let valid = !name.trim().is_empty() && name.len() <= 128;
                    let mut keep = true;
                    if ui
                        .add_enabled(valid, crate::button_widget("Save"))
                        .clicked()
                    {
                        *action = Some(PresetAction::SaveAs(name.trim().to_owned()));
                        keep = false;
                    }
                    if crate::small_button(ui, "Cancel").clicked() {
                        keep = false;
                    }
                    if keep {
                        *draft = Some(name);
                    }
                } else {
                    if crate::small_button(ui, "Save as…").clicked() {
                        *draft = Some(String::new());
                    }
                    if crate::small_button(ui, "Reset")
                        .on_hover_text("Return every parameter to its default")
                        .clicked()
                    {
                        *action = Some(PresetAction::Reset);
                    }
                }
            };
            if one_row {
                let row = ui.horizontal(|ui| {
                    navigation(ui, &mut action);
                    ui.add_space(tokens::SPACE_XS);
                    ui.separator();
                    ui.add_space(tokens::SPACE_XS);
                    actions(ui, &mut action);
                });
                // Saving swaps the actions for a wider name field.
                if draft.is_none() {
                    // With the group's own margin and border.
                    let width = row.response.rect.width() + 2.0 * (tokens::SPACE_SM + 1.0);
                    ui.data_mut(|data| data.insert_temp(row_id, width));
                }
            } else {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| navigation(ui, &mut action));
                    ui.horizontal(|ui| actions(ui, &mut action));
                });
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
        let mut output = DevicePanelOutput {
            preset: None,
            header: Vec::new(),
            header_automate: Vec::new(),
        };
        let Self {
            name,
            width,
            bypassed,
            presets,
            input,
            output: out_peaks,
            knobs,
        } = self;
        ui.set_width(width.points().min(ui.available_width()));
        let mut frame = device_frame(ui, name);
        if let Some(presets) = presets {
            let bar = &mut frame.bar;
            let (button, action) = preset_bar(bar, presets.name, presets.modified);
            output.preset = action;
            let browser_id = button.id.with("browser");
            let mut state = bar
                .data(|data| data.get_temp::<PresetBrowserState>(browser_id))
                .unwrap_or_default();
            egui::Popup::menu(&button)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .id(browser_id.with("popup"))
                .show(|ui| {
                    if let Some(action) =
                        preset_browser(ui, presets.entries, presets.current, &mut state)
                    {
                        output.preset = Some(action);
                    }
                });
            bar.data_mut(|data| data.insert_temp(browser_id, state));
        }
        frame.body(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(tokens::SPACE_SM, tokens::SPACE_SM);
            let height = if knobs.is_empty() {
                tokens::CONTROL_HEIGHT
            } else {
                KnobSize::Dense.cell_height()
            };
            // Bypass, meters and the Mix/Output or Volume knobs, flush right.
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), height),
                Layout::right_to_left(Align::Center),
                |ui| {
                    for knob in knobs.into_iter().rev() {
                        let mut labeled = labeled_knob(knob.caption, knob.value, knob.range)
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
                        output.header.insert(0, shown.response);
                        output.header_automate.insert(0, shown.automate);
                    }
                    for (title, peaks) in [("Output", out_peaks), ("Input", input)] {
                        if let Some(peaks) = peaks {
                            crate::stereo_meter(
                                ui,
                                peaks,
                                vec2(METER_SIZE.x * 2.0 + tokens::SPACE_XS, METER_SIZE.y),
                            )
                            .on_hover_text(format!("{title} level"));
                        }
                    }
                    let mut active = !*bypassed;
                    if toggle_led(ui, &mut active, "On")
                        .on_hover_text("Bypass the device when off")
                        .changed()
                    {
                        *bypassed = !active;
                    }
                },
            );
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

/// Thickness of a [`device_frame`]'s border.
pub const DEVICE_FRAME_BORDER: f32 = 3.0;
/// The width a [`device_frame`] takes from its contents, both sides together.
pub const DEVICE_FRAME_INSET: f32 = 2.0 * (DEVICE_FRAME_BORDER + tokens::SPACE_SM);
const DEVICE_FRAME_RADIUS: u8 = 10;

/// The frame every device sits in, so each one reads as one unit: a thick
/// rounded border, and a bar in the border's color across the top with the
/// device's name at the left and its presets at the right. Draw the presets
/// into [`DeviceFrame::bar`], then the device with [`DeviceFrame::body`],
/// [`DEVICE_FRAME_INSET`] narrower than the frame. Nothing in the body
/// repeats the name or the presets.
pub struct DeviceFrame {
    /// Draw the presets here, left to right; the frame keeps them flush right.
    pub bar: Ui,
    top: egui::Pos2,
    width: f32,
    bar_rect: egui::Rect,
    background: egui::layers::ShapeIdx,
    width_id: egui::Id,
    previous: Option<f32>,
}

pub fn device_frame(ui: &mut Ui, name: &str) -> DeviceFrame {
    let colors = theme::palette(ui.ctx());
    let width = ui.available_width();
    let top = ui.cursor().min;
    // The frame goes under its contents, once their height is known.
    let background = ui.painter().add(egui::Shape::Noop);
    // The preset group and a little air above and below it.
    let bar_height = tokens::CONTROL_HEIGHT + 2.0 * tokens::SPACE_XS + 2.0 * 3.0;
    let bar_rect = egui::Rect::from_min_size(top, vec2(width, bar_height));
    let inner = bar_rect.shrink2(vec2(tokens::SPACE_SM + DEVICE_FRAME_BORDER, 0.0));
    let mut title = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("device_frame_name")
            .max_rect(inner)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let color = theme::contrast_text(colors.device);
    let name_width = title
        .add(egui::Label::new(TextRole::Subtitle.rich(title.ctx(), name).color(color)).truncate())
        .rect
        .width();
    // The presets keep their own order, flush right: placed by the width
    // they took on the previous pass.
    // Per device: another device's presets may take another width.
    let width_id = ui.id().with(("device_frame_bar", name));
    let previous = ui.data(|data| data.get_temp::<f32>(width_id));
    let room = (inner.width() - name_width - tokens::SPACE_MD).max(0.0);
    let left = inner.right() - previous.unwrap_or(room).min(room);
    let bar = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("device_frame_bar")
            .max_rect(egui::Rect::from_x_y_ranges(
                left..=inner.right(),
                inner.y_range(),
            ))
            .layout(Layout::left_to_right(Align::Center)),
    );
    DeviceFrame {
        bar,
        top,
        width,
        bar_rect,
        background,
        width_id,
        previous,
    }
}

impl DeviceFrame {
    /// Draws the device under the bar and closes the frame around it.
    pub fn body<R>(self, ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> R {
        let colors = theme::palette(ui.ctx());
        let used = self.bar.min_rect().width();
        if self.previous != Some(used) {
            ui.data_mut(|data| data.insert_temp(self.width_id, used));
            ui.ctx().request_discard("device frame bar width");
        }
        let side = DEVICE_FRAME_BORDER + tokens::SPACE_SM;
        let body_rect = egui::Rect::from_min_max(
            egui::pos2(self.top.x + side, self.bar_rect.bottom() + tokens::SPACE_SM),
            egui::pos2(self.top.x + self.width - side, f32::INFINITY),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("device_frame_body")
                .max_rect(body_rect)
                .layout(*ui.layout()),
        );
        child.set_width(body_rect.width());
        let inner = body(&mut child);
        let bottom = child.min_rect().bottom() + side;
        let frame = egui::Rect::from_min_max(self.top, egui::pos2(self.top.x + self.width, bottom));
        ui.allocate_rect(frame, egui::Sense::hover());
        let radius = egui::CornerRadius::same(DEVICE_FRAME_RADIUS);
        ui.painter().set(
            self.background,
            egui::Shape::Vec(vec![
                egui::Shape::rect_filled(
                    self.bar_rect,
                    egui::CornerRadius {
                        sw: 0,
                        se: 0,
                        ..radius
                    },
                    colors.device,
                ),
                egui::Shape::rect_stroke(
                    frame,
                    radius,
                    egui::Stroke::new(DEVICE_FRAME_BORDER, colors.device),
                    egui::StrokeKind::Inside,
                ),
            ]),
        );
        inner
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

/// A group's name in small tracked capitals, dimmed so it frames the
/// controls without competing with them.
fn group_title(ui: &Ui, title: &str) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &title.to_uppercase(),
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::new(tokens::FONT_SMALL, TextRole::Label.font().family),
            color: theme::palette(ui.ctx()).muted,
            extra_letter_spacing: 1.0,
            ..Default::default()
        },
    );
    ui.painter().layout_job(job)
}

/// The heading of a device section: its dimmed name on top and a rule that
/// separates it from the controls. Distinct from the sentence-case
/// [`control_caption`]s under it, so the panel's two levels never read alike.
pub fn device_section_title(ui: &mut Ui, title: &str) -> Response {
    device_section_header(ui, title, None, None)
}

const HEADER_LAMP: f32 = 11.0;

/// A [`device_section_title`] that switches its section when `enable` is
/// given: a lamp stands in front of the name, and a click on either flips
/// it. As tall as the plain title, so cards with and without a lamp share
/// a row. The response is `changed` when the click flipped the value.
/// `mark` shows the switch's automation on its lamp, as [`crate::automated`].
pub fn device_section_header(
    ui: &mut Ui,
    title: &str,
    enable: Option<&mut bool>,
    mark: Option<AutomationMark>,
) -> Response {
    let galley = group_title(ui, title);
    let (rect, response) = ui.allocate_exact_size(
        vec2(
            ui.available_width(),
            galley.size().y + tokens::SPACE_SM + tokens::BORDER_WIDTH,
        ),
        egui::Sense::hover(),
    );
    header_title(ui, rect, rect.top(), galley, title, enable, mark, response)
}

/// A [`device_section_header`] whose section comes in a few models: the
/// choice among them stands at the right end of the title line, as toggle
/// buttons, so the card's own lines stay free for its controls. Taller than
/// the plain title: give it to every card of a row or to none. While the
/// section is switched off the choice is disabled in its place. Returns the
/// title's response, `changed` when its lamp was flipped, and the choice's,
/// `changed` when another option was picked. `marks` show the automation of
/// the switch and of the choice, as [`crate::automated`].
pub fn device_section_header_choice(
    ui: &mut Ui,
    title: &str,
    enable: Option<&mut bool>,
    selected: &mut usize,
    options: &[&str],
    marks: [Option<AutomationMark>; 2],
) -> (Response, Response) {
    let galley = group_title(ui, title);
    let (rect, response) = ui.allocate_exact_size(
        vec2(
            ui.available_width(),
            crate::TOGGLE_ROW_HEIGHT + tokens::SPACE_SM + tokens::BORDER_WIDTH,
        ),
        egui::Sense::hover(),
    );
    let band = egui::Rect::from_min_size(rect.min, vec2(rect.width(), crate::TOGGLE_ROW_HEIGHT));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt((title, "model"))
            .max_rect(band)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    // A section that is off has no model to pick.
    if enable.as_deref() == Some(&false) {
        child.disable();
    }
    // Less padding than a button's: the name left of it needs its room.
    child.spacing_mut().button_padding.x = tokens::SPACE_SM;
    let indexed: Vec<(usize, &str)> = options.iter().copied().enumerate().collect();
    let choice = crate::automated(&mut child, marks[1], |child| {
        crate::toggle_grid(child, selected, &indexed, options.len().max(1))
    });
    let top = band.center().y - galley.size().y * 0.5;
    let header = header_title(ui, rect, top, galley, title, enable, marks[0], response);
    (header, choice)
}

/// The name of a section in `rect`, its text starting at `top`, with the
/// lamp of its switch in front where it has one, and the rule below.
#[expect(clippy::too_many_arguments)]
fn header_title(
    ui: &Ui,
    rect: egui::Rect,
    top: f32,
    galley: std::sync::Arc<egui::Galley>,
    title: &str,
    enable: Option<&mut bool>,
    mark: Option<AutomationMark>,
    mut response: Response,
) -> Response {
    let colors = theme::palette(ui.ctx());
    let mut text_left = rect.left();
    let on = enable.as_deref().copied();
    if let Some(value) = enable {
        text_left += HEADER_LAMP + tokens::SPACE_SM;
        // The lamp and the name; the rest of the line stays free.
        let target = egui::Rect::from_min_max(
            rect.left_top(),
            egui::pos2(
                text_left + galley.size().x,
                rect.bottom() - tokens::BORDER_WIDTH,
            ),
        );
        let mut hit = ui.interact(target, response.id.with("enable"), egui::Sense::click());
        if hit.clicked() {
            *value = !*value;
            hit.mark_changed();
        }
        crate::device_controls::paint_lamp(
            ui.painter(),
            egui::pos2(rect.left() + HEADER_LAMP * 0.5, top + galley.size().y * 0.5),
            HEADER_LAMP,
            *value,
            hit.has_focus(),
            mark.filter(|_| ui.is_enabled()),
        );
        response = hit.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    ui.painter()
        .galley(egui::pos2(text_left, top), galley, colors.muted);
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - tokens::BORDER_WIDTH * 0.5,
        egui::Stroke::new(tokens::BORDER_WIDTH, colors.line),
    );
    response.widget_info(|| match on {
        Some(on) => egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, title),
        None => egui::WidgetInfo::labeled(egui::WidgetType::Label, true, title),
    });
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
                    let response = if options.len() >= 2 {
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

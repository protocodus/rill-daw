//! Panels for Rill's built-in devices, composed only from `daw-ui` components.
//!
//! A panel talks to whatever owns the parameter values through [`DeviceHost`],
//! so the same panel runs in the app, in the gallery and in tests. See
//! `docs/R1_DEVICES.md#panel-host`.
use daw_devices::{
    DeviceDescriptor, DeviceKind, ParamRange, ParamSpec, Preset, SpectrumAnalyzer, Telemetry,
    factory_presets,
};
use daw_ui::{AutomationControl, AutomationMark, AutomationNavigation, AutomationState};
use egui::Ui;
use std::sync::Arc;

mod panels;
mod pictures;

/// What a sample slot of a device holds, as its panel shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SampleSummary {
    pub name: String,
    pub seconds: f32,
    /// Min and max of each column of the waveform, −1 to 1; empty while the
    /// host has not read the recording.
    pub peaks: Vec<(f32, f32)>,
}

/// What a panel asks of its host for a sample slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SampleRequest {
    /// Choose a file with the host's file dialog.
    Pick,
    /// A file dropped on the panel.
    File(std::path::PathBuf),
    Clear,
}

/// The panel's view of the parameter owner.
pub trait DeviceHost {
    /// Normalized 0–1.
    fn value(&self, id: u32) -> f64;
    /// One gesture on one control. Consecutive changes of the same parameter
    /// form one Undo step.
    fn set_value(&mut self, id: u32, normalized: f64);
    /// Several parameters at once. Every parameter (a preset, Reset) is one
    /// Undo step; a smaller set is one gesture on those controls, and
    /// consecutive calls with the same IDs form one Undo step: a node dragged
    /// across the response moves its frequency and gain together.
    fn set_values(&mut self, values: &[(u32, f64)]);
    /// The automation lane and its state, when the parameter is automated.
    fn automation(&self, id: u32) -> Option<(u64, AutomationState)>;
    /// Where the parameter's lane is at the song position, normalized 0–1,
    /// whatever holds the parameter now. `None` without a lane.
    fn lane_value(&self, _id: u32) -> Option<f64> {
        None
    }
    /// Create, edit, override or resume automation. `Create` carries
    /// [`AutomationControl::DeviceParameter`].
    fn navigate(&mut self, action: AutomationNavigation);
    /// Presets the user saved for this device.
    fn user_presets(&self) -> Vec<Preset>;
    fn save_preset(&mut self, preset: Preset);
    /// Instruments: sound one note (MIDI pitch, velocity 0–1) from the panel
    /// keyboard until the next call; `None` releases it.
    fn audition(&mut self, note: Option<(u8, f32)>);
    /// Instruments: the panel's pitch wheel (−1–1) and mod wheel (0–1).
    fn wheels(&mut self, pitch: f32, modulation: f32);
    /// The sounding instance's live values and signal, for spectra and
    /// meters. `None` while nothing plays the device: the panel then draws
    /// from the parameter values alone.
    fn telemetry(&self) -> Option<Arc<Telemetry>> {
        None
    }
    /// The song's tempo in beats per minute, for the picture of a
    /// tempo-synced time. `None` when the host has no song (a gallery, a
    /// test table).
    fn tempo(&self) -> Option<f64> {
        None
    }
    /// Names of the channels routed to the device's sidechain input.
    /// `None` when the host cannot say (a gallery, a test table).
    fn sidechain_sources(&self) -> Option<Vec<String>> {
        None
    }
    /// Opens the host's routing view for this device.
    fn route_sidechain(&mut self) {}
    /// What sample slot `slot` holds; `None` while it is empty.
    fn sample(&self, _slot: u32) -> Option<&SampleSummary> {
        None
    }
    /// Loads or clears a sample slot, as one Undo step.
    fn request_sample(&mut self, _slot: u32, _request: SampleRequest) {}
}

/// Per-panel UI state the host keeps between frames.
#[derive(Debug, Default)]
pub struct PanelState {
    /// Name of the last loaded or saved preset.
    pub preset: Option<String>,
    /// Draft name while "Save as…" is open.
    pub save_name: Option<String>,
    /// Pitch wheel, −1–1; springs back to 0 on release.
    pub pitch_wheel: f32,
    /// Mod wheel, 0–1; stays where it is left.
    pub mod_wheel: f32,
    /// Last drawn pitch wheel, mod wheel and keyboard, for host diagnostics.
    pub performance_rects: Option<[egui::Rect; 3]>,
    /// The selected response node and table column.
    pub selected: Option<usize>,
    /// The response's gain axis while a node is dragged, so the axis does
    /// not rescale under the pointer.
    held_range: Option<f32>,
    spectrum: SpectrumState,
    /// Whether the device's telemetry still moves.
    activity: Activity,
    /// The largest gain reduction since the meter was last clicked, in dB.
    pub reduction_hold: f32,
    /// The level history as last read, oldest first.
    history: Vec<[f32; 3]>,
    /// The time span of a delay picture while one of its handles is
    /// dragged, so the axis does not rescale under the pointer.
    held_span: Option<f32>,
    /// What a drag across an LFO picture has turned of the next step of a
    /// synced rate.
    rate_drag: f32,
    /// Where the response, the parts of another picture and each control
    /// were last drawn, and which controls were disabled.
    #[cfg(test)]
    response_rect: Option<egui::Rect>,
    #[cfg(test)]
    picture_rects: Vec<egui::Rect>,
    #[cfg(test)]
    control_rects: Vec<(u32, egui::Rect)>,
    /// The toggle grids themselves, inside their places.
    #[cfg(test)]
    grid_rects: Vec<(u32, egui::Rect)>,
    #[cfg(test)]
    disabled: Vec<u32>,
}

/// Tells a device that plays from one whose telemetry holds what last
/// played: a ring nothing has written to for a while is stale.
#[derive(Debug, Default)]
struct Activity {
    /// Where the ring's writer was at the last look, and for how long it
    /// has not moved.
    position: usize,
    idle_seconds: f32,
}

impl Activity {
    /// Whether the writer, now at `position`, moved within
    /// [`IDLE_SECONDS`]; `seconds` have passed since the previous call.
    fn live(&mut self, position: usize, seconds: f32) -> bool {
        self.idle_seconds = if position == self.position {
            self.idle_seconds + seconds
        } else {
            0.0
        };
        self.position = position;
        self.idle_seconds <= IDLE_SECONDS
    }
}

/// The analyzer behind a response, created when a spectrum first shows.
#[derive(Default)]
struct SpectrumState {
    analyzer: Option<(f32, SpectrumAnalyzer)>,
    frames: Vec<f32>,
    activity: Activity,
}

/// A ring nothing has written to for this long holds what last played, not
/// what sounds: a spectrum then falls as for silence and a meter rests.
const IDLE_SECONDS: f32 = 0.25;

impl std::fmt::Debug for SpectrumState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SpectrumState")
    }
}

impl SpectrumState {
    /// The spectrum of the latest frames of `tap`, in dB per bin, `seconds`
    /// after the previous call.
    fn update(
        &mut self,
        tap: panels::Tap,
        telemetry: &Telemetry,
        sample_rate: f32,
        seconds: f32,
    ) -> &[f32] {
        if self
            .analyzer
            .as_ref()
            .is_none_or(|(rate, _)| *rate != sample_rate)
        {
            self.analyzer = Some((sample_rate, SpectrumAnalyzer::new(sample_rate)));
        }
        let (_, analyzer) = self.analyzer.as_mut().expect("created above");
        self.frames.resize(analyzer.size(), 0.0);
        let position = match tap {
            panels::Tap::Input => telemetry.input_position(),
            panels::Tap::Output => telemetry.output_position(),
        };
        if !self.activity.live(position, seconds) {
            self.frames.fill(0.0);
        } else {
            match tap {
                panels::Tap::Input => telemetry.read_input(&mut self.frames),
                panels::Tap::Output => telemetry.read_output(&mut self.frames),
            }
        }
        analyzer.update(&self.frames, seconds)
    }
}

/// The hero knob's diameter from the panel grammar.
pub const KNOB_HERO: f32 = 64.0;
/// Space between neighbouring controls of one card, so captions never touch.
const CELL_GAP: f32 = daw_ui::tokens::SPACE_SM;

/// Extra space between the lines of a cell-grid card, so a value reads with
/// the knob above it and not with the caption below.
const LINE_GAP: f32 = daw_ui::tokens::SPACE_SM;

/// The panel's width with its frame, for the window that hosts it.
pub fn panel_width(descriptor: &DeviceDescriptor) -> f32 {
    panels::layout(descriptor).width + daw_ui::DEVICE_FRAME_INSET
}

/// Draws a device's panel in its frame: name and presets on the bar, the
/// response, then the sections as cards in signal order, side by side where
/// they share a row.
/// Instruments end with an audition keyboard and pitch and mod wheels.
pub fn show(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    host: &mut dyn DeviceHost,
    state: &mut PanelState,
) {
    let layout = panels::layout(descriptor);
    ui.set_width(panel_width(descriptor).min(ui.available_width()));
    #[cfg(test)]
    {
        state.control_rects.clear();
        state.grid_rects.clear();
        state.picture_rects.clear();
        state.disabled.clear();
    }
    let mut frame = daw_ui::device_frame(ui, descriptor.name);
    preset_bar(&mut frame.bar, descriptor, host, state);
    frame.body(ui, |ui| {
        if layout.picture_place == panels::PicturePlace::Above && layout.picture.is_some() {
            picture(ui, descriptor, &layout, host, state);
            daw_ui::section_gap(ui);
        }
        let rows = layout.sections.iter().map(|section| section.row).max();
        for row in 0..=rows.unwrap_or(0) {
            let sections: Vec<_> = layout
                .sections
                .iter()
                .filter(|section| section.row == row)
                .collect();
            let drawn = if layout.cells {
                // The picture's columns, when it stands in this row.
                let beside = match layout.picture_place {
                    panels::PicturePlace::Row { row: place, span } if place == row => Some(span),
                    _ => None,
                };
                cell_row(ui, descriptor, &sections, beside, host, state, &layout);
                true
            } else {
                natural_row(ui, descriptor, &sections, host, state, &layout)
            };
            if drawn {
                // The gutter between rows equals the one between cards.
                ui.add_space((daw_ui::tokens::SPACE_SM - ui.spacing().item_spacing.y).max(0.0));
            }
        }
        if descriptor.kind == DeviceKind::Instrument {
            daw_ui::section_gap(ui);
            performance(ui, host, state);
        }
    });
}

/// The device's picture, in the space the caller gave it.
fn picture(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    layout: &panels::Layout,
    host: &mut dyn DeviceHost,
    state: &mut PanelState,
) {
    match &layout.picture {
        Some(panels::Picture::Response(view)) => {
            response(ui, descriptor, view, layout, host, state);
        }
        Some(panels::Picture::Saturation { height }) => {
            pictures::saturation(ui, *height, host, state);
        }
        Some(panels::Picture::Dynamics { height }) => {
            pictures::dynamics(ui, *height, host, state);
        }
        Some(panels::Picture::Modulation { height }) => {
            pictures::modulation(ui, *height, host, state);
        }
        Some(panels::Picture::Space) => pictures::space(ui, host, state),
        Some(panels::Picture::Sample { height }) => {
            pictures::sample(ui, descriptor, *height, host, state);
        }
        None => {}
    }
}

/// The parameter a slot of several shows: the one the layout names, else
/// the first that applies, or the first when none does.
fn shown(ids: &[u32], host: &dyn DeviceHost, layout: &panels::Layout) -> usize {
    let values = |id| host.value(id);
    ids.iter()
        .position(|id| (layout.swap)(*id, &values))
        .or_else(|| ids.iter().position(|id| (layout.visible)(*id, &values)))
        .unwrap_or(0)
}

/// A change of a choice from the panel, with whatever the layout sends
/// along as one edit.
fn edit(host: &mut dyn DeviceHost, layout: &panels::Layout, id: u32, value: f64) {
    let mut values = vec![(id, value)];
    values.extend((layout.companions)(id, value, &|id| host.value(id)));
    match values[..] {
        [(id, value)] => host.set_value(id, value),
        _ => host.set_values(&values),
    }
}

/// One row of cards whose controls take the widths their captions and
/// values need; a table card stands among them as it is. `false` when none
/// of the row's controls shows.
fn natural_row(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    sections: &[&panels::Section],
    host: &mut dyn DeviceHost,
    state: &mut PanelState,
    layout: &panels::Layout,
) -> bool {
    let cards: Vec<_> = sections
        .iter()
        .filter_map(|section| card(ui, descriptor, section, host, layout))
        .collect();
    if cards.is_empty() {
        return false;
    }
    // One center line per line of the row, so knobs, choices and
    // switches in neighbouring cards line up.
    let lines = cards.iter().map(|card| card.lines.len()).max().unwrap_or(0);
    let heights: Vec<f32> = (0..lines)
        .map(|line| {
            cards
                .iter()
                .filter(|card| card.table.is_none())
                .filter_map(|card| card.lines.get(line))
                .flatten()
                .map(|(spec, _)| control_height(spec, layout))
                .fold(daw_ui::tokens::CONTROL_HEIGHT, f32::max)
        })
        .collect();
    let spans: Vec<u8> = cards.iter().map(|card| card.span).collect();
    daw_ui::grid_row(ui, &spans, |ui, index| {
        let card = &cards[index];
        daw_ui::device_section_title(ui, card.title);
        if let Some((section, table)) = card.table {
            table_body(ui, descriptor, section, table, host, state, layout);
            return;
        }
        ui.add_space(daw_ui::tokens::SPACE_XS);
        let available = ui.available_width();
        let lines = wrap(&card.lines, available);
        for (index, line) in lines.iter().enumerate() {
            let height = heights
                .get(index)
                .or(heights.last())
                .copied()
                .unwrap_or(0.0);
            // Lines with as many controls share their columns, as in
            // a table; a line of another length spreads on its own.
            let peers: Vec<_> = lines
                .iter()
                .filter(|peer| peer.len() == line.len())
                .cloned()
                .collect();
            let columns = columns(&peers, available);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for ((spec, natural), width) in line.iter().zip(&columns) {
                    let cell = Cell {
                        width: width.unwrap_or(*natural),
                        natural: *natural,
                        height,
                        lead: 0.0,
                        fader: false,
                    };
                    control(ui, descriptor, spec, host, state, cell, layout);
                }
            });
        }
    });
    true
}

/// One row of cards on the cell grid: a card `span` cells wide holds that
/// many equal cells per line, so controls stand in columns within a card
/// and on shared center lines across the row. With `beside`, the picture
/// is the row's first cell, that many columns wide, without a frame.
fn cell_row(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    sections: &[&panels::Section],
    beside: Option<u8>,
    host: &mut dyn DeviceHost,
    state: &mut PanelState,
    layout: &panels::Layout,
) {
    let lines = sections
        .iter()
        .map(|section| section.lines.len())
        .max()
        .unwrap_or(0);
    let heights: Vec<f32> = (0..lines)
        .map(|line| {
            sections
                .iter()
                .filter_map(|section| section.lines.get(line))
                .flatten()
                .filter_map(|slot| match slot {
                    panels::Slot::Control(id, _) => descriptor.parameter(*id),
                    // Whichever shows: the line must not change height.
                    panels::Slot::Swap(ids, _) => ids
                        .iter()
                        .filter_map(|id| descriptor.parameter(*id))
                        .max_by(|a, b| {
                            control_height(a, layout).total_cmp(&control_height(b, layout))
                        }),
                    _ => None,
                })
                .map(|spec| control_height(spec, layout))
                .fold(daw_ui::tokens::CONTROL_HEIGHT, f32::max)
        })
        .collect();
    // What a line adds to its control: the caption above, the value below
    // and the spacing around them. A fader spans the lines it stands beside.
    let text = ui.fonts_mut(|fonts| fonts.row_height(&daw_ui::control_caption_font()));
    let between = 2.0 * text + 3.0 * ui.spacing().item_spacing.y + LINE_GAP;
    let tall = heights.iter().sum::<f32>() + between * lines.saturating_sub(1) as f32;
    // Faders on a row of one line still get the travel of two, so an
    // envelope reads and drags precisely.
    let faders = sections.iter().any(|section| {
        section
            .lines
            .iter()
            .flatten()
            .any(|slot| matches!(slot, panels::Slot::Faders(..)))
    });
    let tall = if faders {
        tall.max(2.0 * daw_ui::tokens::CONTROL_HEIGHT + between)
    } else {
        tall
    };
    let cells: Vec<daw_ui::GridCell> = beside
        .map(|span| daw_ui::GridCell {
            span,
            framed: false,
        })
        .into_iter()
        .chain(sections.iter().map(|section| daw_ui::GridCell {
            span: section.span,
            framed: true,
        }))
        .collect();
    daw_ui::grid_row_cells(ui, &cells, |ui, index| {
        let Some(index) = index.checked_sub(usize::from(beside.is_some())) else {
            picture(ui, descriptor, layout, host, state);
            return;
        };
        let section = sections[index];
        section_header(ui, descriptor, section, host, state, layout);
        ui.add_space(daw_ui::tokens::SPACE_XS);
        let width = ui.available_width() / f32::from(section.span);
        for (line, slots) in section.lines.iter().enumerate() {
            if line > 0 {
                ui.add_space(LINE_GAP);
            }
            // In a card of one line beside taller ones, faders span every
            // line and a knob stands on the last, level with its neighbours.
            let alone = section.lines.len() == 1;
            let last = heights.last().copied().unwrap_or(0.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for slot in slots {
                    let (ids, cells, fader) = match slot {
                        panels::Slot::Control(id, cells) => {
                            (std::slice::from_ref(id), cells, false)
                        }
                        panels::Slot::Faders(ids, cells) => (*ids, cells, true),
                        panels::Slot::Swap(ids, cells) => {
                            let at = shown(ids, host, layout);
                            (&ids[at..=at], cells, false)
                        }
                        panels::Slot::Blank(cells) => {
                            ui.add_space(width * f32::from(*cells));
                            continue;
                        }
                    };
                    let width = width * f32::from(*cells) / ids.len() as f32;
                    for spec in ids.iter().filter_map(|id| descriptor.parameter(*id)) {
                        let cell = Cell {
                            width,
                            natural: width,
                            height: match (alone, fader) {
                                (true, true) => tall,
                                (true, false) => last,
                                (false, _) => heights[line],
                            },
                            lead: if alone && !fader { tall - last } else { 0.0 },
                            fader,
                        };
                        control(ui, descriptor, spec, host, state, cell, layout);
                    }
                }
            });
        }
    });
}

/// How a parameter's automation shows on its control: its state, and the
/// lane's place in the control's own terms, which `position` finds from the
/// lane's normalized value.
fn mark(
    host: &dyn DeviceHost,
    spec: &ParamSpec,
    position: impl Fn(f64) -> Option<f64>,
) -> Option<AutomationMark> {
    host.automation(spec.id).map(|(_, state)| AutomationMark {
        state,
        lane: host.lane_value(spec.id).and_then(position),
    })
}

/// A switch's lane as the lamp shows it: on or off.
fn switch_position(normalized: f64) -> Option<f64> {
    Some(if normalized >= 0.5 { 1.0 } else { 0.0 })
}

/// A card's title; with the lamp of the toggle that switches the section
/// and the choice of its model, where it has them.
fn section_header(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    section: &panels::Section,
    host: &mut dyn DeviceHost,
    // Only tests look at where controls were drawn.
    #[cfg_attr(not(test), expect(unused_variables))] state: &mut PanelState,
    layout: &panels::Layout,
) {
    let parameter = |id: Option<u32>| id.and_then(|id| descriptor.parameter(id));
    let (enable, model) = (parameter(section.enable), parameter(section.choice));
    let mut on = enable.is_some_and(|spec| host.value(spec.id) >= 0.5);
    let lamp = match model {
        Some(spec) => {
            let ParamRange::Choice(labels) = spec.range else {
                unreachable!("a section's model is a choice");
            };
            let mut selected = spec.to_plain(host.value(spec.id) as f32).round() as usize;
            let (lamp, choice) = daw_ui::device_section_header_choice(
                ui,
                section.title,
                enable.is_some().then_some(&mut on),
                &mut selected,
                labels,
                [
                    enable.and_then(|enable| mark(host, enable, switch_position)),
                    mark(host, spec, |normalized| {
                        Some(f64::from(spec.to_plain(normalized as f32).round()))
                    }),
                ],
            );
            if choice.changed() {
                let value = f64::from(spec.to_normalized(selected as f32));
                edit(host, layout, spec.id, value);
            }
            #[cfg(test)]
            {
                state.control_rects.push((spec.id, choice.rect));
                if !active(spec, host, layout) {
                    state.disabled.push(spec.id);
                }
            }
            let navigation = daw_ui::automatable_parameter_context(
                ui,
                &choice,
                AutomationControl::DeviceParameter { id: spec.id },
                host.automation(spec.id),
            );
            if let Some(action) = navigation {
                host.navigate(action);
            }
            lamp
        }
        None => daw_ui::device_section_header(
            ui,
            section.title,
            enable.is_some().then_some(&mut on),
            enable.and_then(|enable| mark(host, enable, switch_position)),
        ),
    };
    let Some(spec) = enable else {
        return;
    };
    if lamp.changed() {
        host.set_value(spec.id, f64::from(u8::from(on)));
    }
    let navigation = daw_ui::automatable_parameter_context(
        ui,
        &lamp,
        AutomationControl::DeviceParameter { id: spec.id },
        host.automation(spec.id),
    );
    #[cfg(test)]
    state.control_rects.push((spec.id, lamp.rect));
    lamp.on_hover_text(spec.name);
    if let Some(action) = navigation {
        host.navigate(action);
    }
}

/// A section's visible controls, line by line, with their natural widths.
struct Card<'a> {
    title: &'static str,
    lines: Vec<Vec<(&'a ParamSpec, f32)>>,
    /// The card's width in grid columns.
    span: u8,
    /// The section and its table, when the card is one.
    table: Option<(&'a panels::Section, &'a panels::Table)>,
}

fn card<'a>(
    ui: &Ui,
    descriptor: &'a DeviceDescriptor,
    section: &'a panels::Section,
    host: &dyn DeviceHost,
    layout: &panels::Layout,
) -> Option<Card<'a>> {
    let lines: Vec<Vec<_>> = section
        .lines
        .iter()
        .map(|line| {
            line.iter()
                .flat_map(|slot| slot.ids())
                .filter_map(|id| descriptor.parameter(*id))
                .filter(|spec| layout.keep_inactive || active(spec, host, layout))
                .map(|spec| (spec, cell_width(ui, spec, layout)))
                .collect::<Vec<_>>()
        })
        .filter(|line| !line.is_empty())
        .collect();
    (!lines.is_empty()).then_some(Card {
        title: section.title,
        lines,
        span: section.span,
        table: section.table.as_ref().map(|table| (section, table)),
    })
}

/// Whether the control does anything in the device's current mode.
fn active(spec: &ParamSpec, host: &dyn DeviceHost, layout: &panels::Layout) -> bool {
    (layout.visible)(spec.id, &|id| host.value(id))
}

/// The columns of a choice whose toggle buttons form a grid.
fn toggle_columns(spec: &ParamSpec, layout: &panels::Layout) -> Option<usize> {
    layout
        .toggle_columns
        .iter()
        .find(|(id, _)| *id == spec.id && toggles(spec))
        .map(|(_, columns)| *columns)
}

/// Whether a parameter's options are few enough to show as toggle buttons.
fn toggles(spec: &ParamSpec) -> bool {
    matches!(spec.range, ParamRange::Choice(labels) if labels.len() < daw_ui::TOGGLE_OPTION_LIMIT)
}

/// Lines too wide for `width` continue on the next, so a device without
/// planned lines still fits its panel.
fn wrap<T: Copy>(lines: &[Vec<(T, f32)>], width: f32) -> Vec<Vec<(T, f32)>> {
    let mut wrapped = Vec::new();
    for line in lines {
        let mut current: Vec<(T, f32)> = Vec::new();
        let mut used = 0.0;
        for &(item, cell) in line {
            if !current.is_empty() && used + cell > width {
                wrapped.push(std::mem::take(&mut current));
                used = 0.0;
            }
            current.push((item, cell));
            used += cell;
        }
        wrapped.push(current);
    }
    wrapped
}

/// Columns as wide as their widest control, as in a table.
fn grid_width<T>(lines: &[Vec<(T, f32)>]) -> f32 {
    let columns = lines.iter().map(Vec::len).max().unwrap_or(0);
    (0..columns)
        .map(|column| {
            lines
                .iter()
                .filter_map(|line| line.get(column))
                .map(|(_, width)| *width)
                .fold(0.0, f32::max)
        })
        .sum()
}

/// Column widths that align the lines of a card and share its leftover
/// width evenly. `None` when the aligned columns do not fit: each control
/// then keeps its natural width.
fn columns<T>(lines: &[Vec<(T, f32)>], available: f32) -> Vec<Option<f32>> {
    let count = lines.iter().map(Vec::len).max().unwrap_or(0);
    let used = grid_width(lines);
    if used > available {
        return vec![None; count];
    }
    let extra = (available - used) / count.max(1) as f32;
    (0..count)
        .map(|column| {
            let natural = lines
                .iter()
                .filter_map(|line| line.get(column))
                .map(|(_, width)| *width)
                .fold(0.0, f32::max);
            Some(natural + extra)
        })
        .collect()
}

/// The captions a parameter takes with the options of the choice it
/// follows, and that choice.
fn mode_captions(
    spec: &ParamSpec,
    layout: &panels::Layout,
) -> Option<(u32, &'static [&'static str])> {
    layout
        .mode_captions
        .iter()
        .find(|(id, _, _)| *id == spec.id)
        .map(|(_, choice, captions)| (*choice, *captions))
}

/// The name the panel shows for a parameter: its own, unless the layout
/// moved it to a card where another name reads right or names it after
/// what it does in the current mode.
fn caption(
    descriptor: &DeviceDescriptor,
    spec: &ParamSpec,
    host: &dyn DeviceHost,
    layout: &panels::Layout,
) -> &'static str {
    if let Some((choice, captions)) = mode_captions(spec, layout)
        && let Some(choice) = descriptor.parameter(choice)
    {
        let option = choice.to_plain(host.value(choice.id) as f32).round() as usize;
        if let Some(caption) = captions.get(option) {
            return caption;
        }
    }
    layout
        .captions
        .iter()
        .find(|(id, _)| *id == spec.id)
        .map_or(spec.name, |(_, caption)| caption)
}

/// A parameter's value as the panel prints it: the device's own text, less
/// the plus sign where the range has no negative side to tell it from.
fn value_text(spec: &ParamSpec, normalized: f32) -> String {
    let text = spec.format(normalized);
    match text.strip_prefix('+') {
        Some(unsigned) if spec.plain_min() >= 0.0 => unsigned.to_owned(),
        _ => text,
    }
}

/// The value at one end of a knob's range, short enough to print under the
/// arc: "20", "20k", "+24". The unit stays only where the two ends differ
/// in more than its prefix ("1ms", "2s"); the full value is under the knob.
fn end_label(spec: &ParamSpec, normalized: f32) -> String {
    let split = |normalized: f32| {
        let text = value_text(spec, normalized);
        // "20 Hz", or a unit that joins its number: "4.0:1".
        let (number, unit) = text
            .split_once(' ')
            .or_else(|| {
                (!spec.unit.is_empty())
                    .then(|| text.strip_suffix(spec.unit))
                    .flatten()
                    .map(|number| (number, spec.unit))
            })
            .unwrap_or((&text, ""));
        let number = if number.contains('.') {
            number.trim_end_matches('0').trim_end_matches('.')
        } else {
            number
        };
        let number = if number == "+0" { "0" } else { number };
        (number.to_owned(), unit.to_owned())
    };
    let ((number, unit), (_, low), (_, high)) = (split(normalized), split(0.0), split(1.0));
    if low == high {
        number
    } else if high.strip_prefix('k') == Some(low.as_str()) {
        // "20" to "20k": the kilo prefix alone tells the ends apart.
        format!("{number}{}", if unit == high { "k" } else { "" })
    } else {
        format!("{number}{unit}")
    }
}

fn knob_diameter(spec: &ParamSpec, layout: &panels::Layout) -> f32 {
    if layout.hero == Some(spec.id) {
        KNOB_HERO
    } else {
        layout.knob.diameter()
    }
}

fn is_knob(spec: &ParamSpec) -> bool {
    matches!(
        spec.range,
        ParamRange::Linear { .. } | ParamRange::Log { .. } | ParamRange::Integer { .. }
    )
}

/// Whether the layout draws a choice as a detented knob.
fn dial(spec: &ParamSpec, layout: &panels::Layout) -> bool {
    layout.dials.contains(&spec.id) && matches!(spec.range, ParamRange::Choice(_))
}

/// Height of the control itself, between its caption and its value.
fn control_height(spec: &ParamSpec, layout: &panels::Layout) -> f32 {
    if layout.steppers.contains(&spec.id) {
        daw_ui::VARIATION_STEPPER_SIZE.y
    } else if is_knob(spec) || dial(spec, layout) {
        knob_diameter(spec, layout)
    } else if let (Some(columns), ParamRange::Choice(labels)) =
        (toggle_columns(spec, layout), spec.range)
    {
        daw_ui::TOGGLE_ROW_HEIGHT * labels.len().div_ceil(columns) as f32
    } else if led(spec, layout).is_some() {
        daw_ui::tokens::COMPACT_HEIGHT
    } else {
        daw_ui::tokens::CONTROL_HEIGHT
    }
}

/// The label of a toggle the layout draws as a lamp.
fn led(spec: &ParamSpec, layout: &panels::Layout) -> Option<&'static str> {
    layout
        .leds
        .iter()
        .find(|(id, _)| *id == spec.id)
        .map(|(_, label)| *label)
}

/// How the layout draws a choice's options as toggle segments.
fn faces<'a>(spec: &ParamSpec, layout: &'a panels::Layout) -> Option<&'a panels::Faces> {
    layout
        .faces
        .iter()
        .find(|faces| faces.ids.contains(&spec.id))
}

/// The narrowest cell that shows a control's caption, and its widest value
/// whole, with a gap to its neighbours.
fn cell_width(ui: &Ui, spec: &ParamSpec, layout: &panels::Layout) -> f32 {
    let text = |value: &str, font: egui::FontId| {
        ui.painter()
            .layout_no_wrap(value.to_owned(), font, egui::Color32::PLACEHOLDER)
            .size()
            .x
    };
    let button = egui::TextStyle::Button.resolve(ui.style());
    if let Some(label) = led(spec, layout) {
        return daw_ui::toggle_led_width(ui, label) + CELL_GAP;
    }
    // The widest caption the control ever takes, so its cell never changes.
    let caption = match mode_captions(spec, layout) {
        Some((_, captions)) => captions
            .iter()
            .map(|caption| text(caption, daw_ui::control_caption_font()))
            .fold(0.0, f32::max),
        None => text(
            layout
                .captions
                .iter()
                .find(|(id, _)| *id == spec.id)
                .map_or(spec.name, |(_, caption)| caption),
            daw_ui::control_caption_font(),
        ),
    };
    let widest = |values: &mut dyn Iterator<Item = String>| {
        values
            .map(|value| text(&value, button.clone()))
            .fold(0.0, f32::max)
    };
    // Text, padding on both sides and the chevron, as the choice draws them.
    let spacing = ui.spacing();
    let choice = |text: f32| {
        text + spacing.icon_spacing + spacing.icon_width + 2.0 * daw_ui::tokens::SPACE_SM
    };
    let control = match spec.range {
        // The knob, or its value below it at either end of the range.
        ParamRange::Linear { .. } | ParamRange::Log { .. } | ParamRange::Integer { .. } => {
            [0.0, 1.0]
                .into_iter()
                .map(|end| text(&value_text(spec, end), daw_ui::text::TextRole::Note.font()))
                .fold(knob_diameter(spec, layout), f32::max)
        }
        ParamRange::Choice(labels) if toggles(spec) => {
            let widths = labels
                .iter()
                .map(|label| text(label, button.clone()) + 2.0 * daw_ui::tokens::SPACE_SM);
            match toggle_columns(spec, layout) {
                Some(columns) => widths.fold(0.0, f32::max) * columns as f32,
                None => widths.sum(),
            }
        }
        ParamRange::Choice(labels) => choice(widest(&mut labels.iter().map(|l| l.to_string()))),
        ParamRange::Toggle => daw_ui::tokens::SWITCH_WIDTH,
    };
    caption.max(control) + CELL_GAP
}

const KEYBOARD_FIRST: u8 = 36;
const KEYBOARD_OCTAVES: u8 = 4;
const KEYBOARD_HEIGHT: f32 = 55.0;
const WHEEL_WIDTH: f32 = 21.0;

/// Pitch and mod wheels beside an audition keyboard. The wheels apply to
/// every note the instrument plays while it is previewed, not only to the
/// keyboard's.
fn performance(ui: &mut Ui, host: &mut dyn DeviceHost, state: &mut PanelState) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_XS;
        let size = egui::vec2(WHEEL_WIDTH, KEYBOARD_HEIGHT);
        let pitch = daw_ui::wheel(ui, &mut state.pitch_wheel, true, "Pitch", size)
            .on_hover_text("Pitch wheel: drag to bend; springs back on release");
        let modulation = daw_ui::wheel(ui, &mut state.mod_wheel, false, "Mod", size)
            .on_hover_text("Mod wheel: drag to set; double-click to reset");
        if pitch.changed() || modulation.changed() {
            host.wheels(state.pitch_wheel, state.mod_wheel);
        }
        let width = ui.available_width();
        let keyboard =
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, KEYBOARD_HEIGHT));
        state.performance_rects = Some([pitch.rect, modulation.rect, keyboard]);
        let keys = daw_ui::keyboard_strip(
            ui,
            KEYBOARD_FIRST,
            KEYBOARD_OCTAVES,
            &[],
            Some(60),
            egui::vec2(width, KEYBOARD_HEIGHT),
        );
        if let Some(note) = keys.pressed {
            host.audition(Some(note));
        } else if keys.released.is_some() {
            host.audition(None);
        }
    });
}

/// Tilt and smoothing of the spectrum behind a response, as on analyzers
/// behind an equalizer curve: pink-ish material draws level, and the trace
/// is steady enough to read against the curve.
const SPECTRUM_TILT: f32 = 4.5;
const SPECTRUM_SMOOTHING: f32 = 1.0 / 6.0;
/// Frames per second of a moving spectrum.
const SPECTRUM_REFRESH: f32 = 30.0;
/// Wheel notches for a continuous value's whole range.
const WHEEL_NOTCHES: f64 = 40.0;

/// The device's response with its nodes. Dragging a node is one gesture on
/// its parameters, like turning their dials; the wheel, a double-click and
/// the context menu reach the rest of what the node stands for.
fn response(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    view: &panels::ResponseView,
    layout: &panels::Layout,
    host: &mut dyn DeviceHost,
    state: &mut PanelState,
) {
    let telemetry = host.telemetry();
    // The curve the device sounds at its own rate.
    let rate = view
        .rate_slot
        .zip(telemetry.as_ref())
        .map(|(slot, telemetry)| telemetry.value(slot))
        .filter(|rate| *rate >= 8000.0)
        .unwrap_or(panels::RESPONSE_RATE);
    let size = egui::vec2(ui.available_width(), view.height);
    // A point per point of width: a narrow bell or notch is not missed.
    let points = (size.x as usize).max(2);
    let frequencies: Vec<f32> = (0..points)
        .map(|index| daw_ui::log_grid_frequency(index, points))
        .collect();
    let selected = state.selected.filter(|node| *node < view.nodes.len());
    let (mut db, mut own) = (vec![0.0; points], vec![0.0; points]);
    let (range, edits) = {
        let values = |id: u32| host.value(id);
        (view.curve)(&values, rate, &frequencies, &mut db);
        let mut curves = Vec::new();
        if let (Some(curve), Some(node)) = (view.node_curve, selected) {
            curve(&values, node, rate, &frequencies, &mut own);
            curves.push(daw_ui::FrequencyCurve {
                db: &own,
                emphasis: false,
                low: None,
            });
        }
        curves.push(daw_ui::FrequencyCurve {
            db: &db,
            emphasis: true,
            low: None,
        });
        let plain = |id: u32| {
            descriptor
                .parameter(id)
                .map(|spec| spec.to_plain(values(id) as f32))
        };
        let readouts: Vec<Option<String>> = (0..view.nodes.len())
            .map(|node| view.readout.map(|readout| readout(node, &values)))
            .collect();
        let nodes: Vec<daw_ui::FrequencyNode> = view
            .nodes
            .iter()
            .zip(&readouts)
            .enumerate()
            .map(|(index, (node, readout))| {
                let gain = node.gain.filter(|id| (layout.visible)(*id, &values));
                daw_ui::FrequencyNode {
                    frequency: plain(node.frequency).unwrap_or(1000.0),
                    gain_db: gain.and_then(plain).unwrap_or(0.0),
                    selected: selected == Some(index),
                    color: None,
                    gain_editable: gain.is_some(),
                    label: node.label,
                    enabled: node.enable.is_none_or(|id| values(id) >= 0.5),
                    readout: readout.as_deref(),
                }
            })
            .collect();
        let range = state.held_range.unwrap_or_else(|| (view.range_db)(&values));
        let bins = (view.spectrum)(&values)
            .zip(telemetry.as_ref())
            .map(|(tap, telemetry)| {
                let seconds = ui.input(|input| input.stable_dt).min(0.1);
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f32(
                        1.0 / SPECTRUM_REFRESH,
                    ));
                state
                    .spectrum
                    .update(tap, telemetry, rate, seconds)
                    .to_vec()
            });
        let output = daw_ui::frequency_plot(
            ui,
            &daw_ui::FrequencyPlot {
                curves: &curves,
                spectrum: bins.as_deref().map(|bins| daw_ui::Spectrum {
                    bins_db: bins,
                    sample_rate: rate,
                    tilt_db_per_octave: SPECTRUM_TILT,
                    smoothing_octaves: SPECTRUM_SMOOTHING,
                }),
                nodes: &nodes,
                range_db: range,
                limit_db: view.limit_db,
                spans: &[],
                markers: &[],
                size,
            },
        );
        // What the gestures ask for, applied once the host is free again.
        let normalized = |id: u32, plain: f32| {
            descriptor
                .parameter(id)
                .map(|spec| (id, f64::from(spec.to_normalized(plain))))
        };
        let mut edits: Vec<Vec<(u32, f64)>> = Vec::new();
        for drag in &output.drags {
            let node = &view.nodes[drag.index];
            let gain = node.gain.filter(|_| nodes[drag.index].gain_editable);
            edits.push(
                normalized(node.frequency, drag.frequency)
                    .into_iter()
                    .chain(gain.and_then(|id| normalized(id, drag.gain_db)))
                    .collect(),
            );
        }
        let toggled = |id: u32| vec![(id, if values(id) >= 0.5 { 0.0 } else { 1.0 })];
        if let Some(id) = output
            .double_clicked
            .and_then(|node| view.nodes[node].enable)
        {
            edits.push(toggled(id));
        }
        if let Some((node, notches)) = output.scrolled
            && let Some(spec) = view.nodes[node]
                .wheel
                .and_then(|id| descriptor.parameter(id))
        {
            let value = values(spec.id);
            let next = match spec.steps() {
                // A choice steps one option per notch.
                Some(steps) => f64::from(
                    spec.to_normalized(
                        (spec.to_plain(value as f32).round() + notches.signum())
                            .clamp(0.0, steps.saturating_sub(1) as f32),
                    ),
                ),
                None => {
                    let fine = ui.input(|input| if input.modifiers.shift { 0.1 } else { 1.0 });
                    (value + f64::from(notches) / WHEEL_NOTCHES * fine).clamp(0.0, 1.0)
                }
            };
            edits.push(vec![(spec.id, next)]);
            state.selected = Some(node);
        }
        for (node, hit) in view.nodes.iter().zip(&output.nodes) {
            if node.enable.is_none() && node.menu.is_none() {
                continue;
            }
            hit.context_menu(|ui| {
                if let Some(id) = node.enable {
                    let label = if values(id) >= 0.5 {
                        "Disable"
                    } else {
                        "Enable"
                    };
                    if daw_ui::menu_item(ui, label, None).clicked() {
                        edits.push(toggled(id));
                        ui.close();
                    }
                    ui.separator();
                }
                if let Some(spec) = node.menu.and_then(|id| descriptor.parameter(id))
                    && let ParamRange::Choice(labels) = spec.range
                {
                    let current = spec.to_plain(values(spec.id) as f32).round() as usize;
                    for (index, label) in labels.iter().enumerate() {
                        if daw_ui::selectable(ui, index == current, *label).clicked() {
                            edits.extend(normalized(spec.id, index as f32).map(|edit| vec![edit]));
                            ui.close();
                        }
                    }
                    ui.separator();
                }
                if daw_ui::menu_item(ui, "Reset band", None).clicked() {
                    edits.push(
                        node.ids()
                            .filter_map(|id| descriptor.parameter(id))
                            .map(|spec| (spec.id, f64::from(spec.default_normalized())))
                            .collect(),
                    );
                    ui.close();
                }
            });
        }
        if let Some(node) = output.clicked {
            state.selected = Some(node);
        }
        #[cfg(test)]
        {
            state.response_rect = Some(output.response.rect);
        }
        (output.dragging.map(|_| range), edits)
    };
    state.held_range = range;
    for edit in edits {
        match edit[..] {
            [(id, value)] => host.set_value(id, value),
            _ => host.set_values(&edit),
        }
    }
}

/// Space between the lines of a table card.
const TABLE_GAP: f32 = daw_ui::tokens::SPACE_SM;
/// What a table column keeps free beside its widest control.
const TABLE_CELL_GAP: f32 = 2.0;

/// Height of a control as a table shows it: bare, on one line.
fn table_height(spec: &ParamSpec) -> f32 {
    match spec.range {
        ParamRange::Choice(_) => daw_ui::tokens::CONTROL_HEIGHT,
        ParamRange::Toggle => daw_ui::tokens::COMPACT_HEIGHT,
        _ => daw_ui::MINI_KNOB_DIAMETER,
    }
}

/// A card that is a table, below its title: a caption column, then equal
/// columns with one control per line. The selected column is marked; using
/// any control of a column selects it.
fn table_body(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    section: &panels::Section,
    table: &panels::Table,
    host: &mut dyn DeviceHost,
    state: &mut PanelState,
    layout: &panels::Layout,
) {
    ui.add_space(daw_ui::tokens::SPACE_SM);
    ui.spacing_mut().item_spacing.y = 0.0;
    let columns = section.lines.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let column = (ui.available_width() - table.caption_width) / columns as f32;
    let heights: Vec<f32> = section
        .lines
        .iter()
        .map(|line| {
            line.iter()
                .flat_map(|slot| slot.ids())
                .filter_map(|id| descriptor.parameter(*id))
                .map(table_height)
                .fold(daw_ui::tokens::COMPACT_HEIGHT, f32::max)
        })
        .collect();
    let origin = ui.cursor().min + egui::vec2(table.caption_width, 0.0);
    if let Some(selected) = state.selected.filter(|column| *column < columns) {
        let height =
            heights.iter().sum::<f32>() + TABLE_GAP * heights.len().saturating_sub(1) as f32;
        daw_ui::column_highlight(
            ui,
            egui::Rect::from_min_size(
                origin + egui::vec2(column * selected as f32, 0.0),
                egui::vec2(column, height),
            )
            .expand2(egui::vec2(0.0, daw_ui::tokens::SPACE_XS)),
        );
    }
    for (line, (slots, height)) in section.lines.iter().zip(&heights).enumerate() {
        if line > 0 {
            ui.add_space(TABLE_GAP);
        }
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), *height),
            egui::Sense::hover(),
        );
        if let Some(caption) = table.captions.get(line) {
            ui.painter().text(
                row.left_center(),
                egui::Align2::LEFT_CENTER,
                caption,
                daw_ui::control_caption_font(),
                daw_ui::theme::palette(ui.ctx()).text,
            );
        }
        for (index, slot) in slots.iter().enumerate() {
            let Some(spec) = slot.ids().first().and_then(|id| descriptor.parameter(*id)) else {
                continue;
            };
            let cell = egui::Rect::from_center_size(
                egui::pos2(origin.x + column * (index as f32 + 0.5), row.center().y),
                egui::vec2(column, table_height(spec)),
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("table", line, index))
                    .max_rect(cell)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            if table_control(&mut child, spec, host, layout, column) {
                state.selected = Some(index);
            }
            #[cfg(test)]
            state.control_rects.push((spec.id, child.min_rect()));
        }
    }
}

/// One bare control of a table cell: an inline dial, a lamp or a row of
/// toggle segments. `true` when it was clicked, dragged or changed.
fn table_control(
    ui: &mut Ui,
    spec: &ParamSpec,
    host: &mut dyn DeviceHost,
    layout: &panels::Layout,
    column: f32,
) -> bool {
    if !active(spec, host, layout) {
        ui.disable();
    }
    let normalized = host.value(spec.id);
    let automation = host.automation(spec.id);
    let control = AutomationControl::DeviceParameter { id: spec.id };
    // The row's caption abbreviates and the column's heading is a number.
    let name = format!("{} {}", spec.group, spec.name);
    let lane = mark(host, spec, |normalized| {
        Some(match spec.range {
            ParamRange::Choice(_) => f64::from(spec.to_plain(normalized as f32).round()),
            ParamRange::Toggle => f64::from(u8::from(normalized >= 0.5)),
            _ => normalized,
        })
    });
    let (response, navigation) = daw_ui::automated(ui, lane, |ui| match spec.range {
        ParamRange::Linear { .. } | ParamRange::Log { .. } | ParamRange::Integer { .. } => {
            let mut value = normalized;
            let mut knob = daw_ui::inline_knob(
                &name,
                &mut value,
                0.0..=1.0,
                value_text(spec, normalized as f32),
            )
            .width(column - TABLE_CELL_GAP)
            .automation(automation.map(|(_, state)| state));
            if spec.plain_min() == -spec.plain_max() {
                knob = knob.bipolar();
            }
            let response = ui.add(knob);
            let navigation = knob_edits(
                ui, spec, host, &response, value, normalized, automation, false,
            );
            (response, navigation)
        }
        ParamRange::Choice(labels) => {
            let mut current = spec.to_plain(normalized as f32);
            let response = match faces(spec, layout) {
                Some(faces) => {
                    let options: Vec<_> = faces
                        .faces
                        .iter()
                        .enumerate()
                        .map(|(index, (face, name))| (index as f32, *face, *name))
                        .collect();
                    daw_ui::glyph_toggle(ui, &mut current, &options, faces.segment_width)
                }
                None => {
                    let options: Vec<_> = labels
                        .iter()
                        .enumerate()
                        .map(|(index, label)| (index as f32, *label))
                        .collect();
                    daw_ui::toggle_group(ui, &mut current, &options)
                }
            };
            if response.changed() {
                host.set_value(spec.id, f64::from(spec.to_normalized(current)));
            }
            let navigation =
                daw_ui::automatable_parameter_context(ui, &response, control, automation);
            (response, navigation)
        }
        ParamRange::Toggle => {
            let mut on = normalized >= 0.5;
            let response = match led(spec, layout) {
                Some(label) => daw_ui::toggle_led(ui, &mut on, label),
                None => daw_ui::switch(ui, &mut on, ""),
            };
            if response.changed() {
                host.set_value(spec.id, f64::from(u8::from(on)));
            }
            let navigation =
                daw_ui::automatable_parameter_context(ui, &response, control, automation);
            (response, navigation)
        }
    });
    let used = response.clicked() || response.drag_started() || response.changed();
    response.on_hover_text(name);
    if let Some(action) = navigation {
        host.navigate(action);
    }
    used
}

/// What every continuous control offers once drawn: the exact value and
/// automation from its context menu, Reset by double-click, and the edit
/// itself. `value` is what the control left after this frame's gesture.
/// A `stepper` has no Reset: two quick clicks on a button are two steps.
#[expect(clippy::too_many_arguments)]
fn knob_edits(
    ui: &Ui,
    spec: &ParamSpec,
    host: &mut dyn DeviceHost,
    response: &egui::Response,
    value: f64,
    normalized: f64,
    automation: Option<(u64, AutomationState)>,
    stepper: bool,
) -> Option<AutomationNavigation> {
    let (action, exact) = daw_ui::automatable_parameter_value_context_named(
        ui,
        response,
        AutomationControl::DeviceParameter { id: spec.id },
        automation,
        f64::from(spec.to_plain(normalized as f32)),
        f64::from(spec.plain_min())..=f64::from(spec.plain_max()),
        spec.unit,
        spec.name,
    );
    if let Some(plain) = exact {
        host.set_value(spec.id, f64::from(spec.to_normalized(plain as f32)));
    } else if response.double_clicked() && !stepper {
        host.set_value(spec.id, f64::from(spec.default_normalized()));
    } else if response.changed() {
        host.set_value(spec.id, value.clamp(0.0, 1.0));
    }
    action
}

/// A control's cell: its width in the line, its own natural width, which a
/// choice keeps, the height of the line's control band, the space above
/// its caption, and whether a continuous value is a fader filling that band
/// instead of a knob.
struct Cell {
    width: f32,
    natural: f32,
    height: f32,
    lead: f32,
    fader: bool,
}

/// One control chosen by the parameter's range, centered in its cell, on the
/// line's center line below its caption.
fn control(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    spec: &ParamSpec,
    host: &mut dyn DeviceHost,
    // Only tests look at where controls were drawn.
    #[cfg_attr(not(test), expect(unused_variables))] state: &mut PanelState,
    cell: Cell,
    layout: &panels::Layout,
) {
    let Cell {
        width,
        natural,
        height,
        lead,
        fader,
    } = cell;
    let normalized = host.value(spec.id);
    let automation = host.automation(spec.id);
    let id = egui::Id::new((descriptor.id, spec.id));
    let control = AutomationControl::DeviceParameter { id: spec.id };
    let caption = caption(descriptor, spec, host, layout);
    let applies = active(spec, host, layout);
    #[cfg(test)]
    if !applies {
        state.disabled.push(spec.id);
    }
    let _drawn = ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            if !applies {
                ui.disable();
            }
            // Compact horizontal padding leaves a choice's text the room of its cell.
            ui.spacing_mut().button_padding.x = daw_ui::tokens::SPACE_SM;
            ui.add_space(lead);
            // A lamp carries its own label.
            let named = led(spec, layout).is_none();
            daw_ui::control_caption(ui, if named { caption } else { " " });
            let own = if fader {
                height
            } else {
                control_height(spec, layout)
            };
            let offset = (height - own) * 0.5;
            if offset > 0.0 {
                ui.add_space(offset);
            }
            let lane = mark(host, spec, |normalized| {
                Some(match spec.range {
                    // A knob over the options turns with the lane.
                    ParamRange::Choice(_) if dial(spec, layout) => normalized,
                    ParamRange::Choice(labels) => {
                        let plain = spec.to_plain(normalized as f32).round() as usize;
                        if faces(spec, layout).is_some() {
                            plain as f64
                        } else {
                            // The option's place among those offered now.
                            let offered =
                                (layout.options)(spec.id, &|id| host.value(id)).unwrap_or(labels);
                            let label = labels.get(plain)?;
                            offered.iter().position(|option| option == label)? as f64
                        }
                    }
                    ParamRange::Toggle => f64::from(u8::from(normalized >= 0.5)),
                    _ => normalized,
                })
            });
            let navigation = daw_ui::automated(ui, lane, |ui| match spec.range {
                ParamRange::Linear { .. } | ParamRange::Log { .. } | ParamRange::Integer { .. } => {
                    let mut value = normalized;
                    let state = automation.map(|(_, state)| state);
                    let response = if fader {
                        let size = egui::vec2(daw_ui::FADER_CAP_WIDTH, height);
                        daw_ui::fader(ui, &mut value, spec.name, size, state)
                    } else if let (true, ParamRange::Integer { min, max }) =
                        (layout.steppers.contains(&spec.id), spec.range)
                    {
                        let mut whole = spec.to_plain(normalized as f32).round() as i32;
                        let response =
                            daw_ui::variation_stepper(ui, &mut whole, min..=max, caption);
                        value = f64::from(spec.to_normalized(whole as f32));
                        response
                    } else {
                        let mut knob = daw_ui::Knob::new(&mut value, 0.0..=1.0)
                            .compact()
                            .pointer()
                            .diameter(knob_diameter(spec, layout))
                            .text(caption)
                            .endpoint_labels(end_label(spec, 0.0), end_label(spec, 1.0))
                            .automation(state);
                        if spec.plain_min() == -spec.plain_max() {
                            // A range around zero fills from its center.
                            knob = knob.bipolar();
                        }
                        if let ParamRange::Integer { min, max } = spec.range {
                            // One detent per whole value.
                            knob = knob.step_by(1.0 / f64::from((max - min).max(1)));
                        }
                        ui.add(knob)
                    };
                    // What the device uses, where that is not the knob's value.
                    let own = value_text(spec, normalized as f32);
                    let replaced = (layout.readout)(spec.id, &|id| host.value(id));
                    // Values share one baseline across the line.
                    ui.add_space(offset);
                    match replaced {
                        Some(text) if text != own => {
                            let note = daw_ui::text::note(ui, &text);
                            // The pointer and the number disagree: say where
                            // the knob is. Another wording of the knob's own
                            // value ("+50 %") needs no such note.
                            let same = spec
                                .parse(&text)
                                .is_some_and(|value| (f64::from(value) - normalized).abs() < 1e-3);
                            if !same {
                                note.on_hover_text(format!("The knob is at {own}"));
                            }
                        }
                        _ => {
                            daw_ui::text::note(ui, &own);
                        }
                    }
                    let hint = (layout.hint)(spec.id, &|id| host.value(id));
                    let response = match hint {
                        Some(hint) => response.on_hover_text(hint),
                        None => response,
                    };
                    let stepper = layout.steppers.contains(&spec.id);
                    knob_edits(
                        ui, spec, host, &response, value, normalized, automation, stepper,
                    )
                }
                ParamRange::Choice(labels) => {
                    // Option indices with their labels.
                    let offered = (layout.options)(spec.id, &|id| host.value(id)).unwrap_or(labels);
                    let options: Vec<(f32, String)> = offered
                        .iter()
                        .map(|option| {
                            let index = labels.iter().position(|o| o == option);
                            (index.unwrap_or(0) as f32, (*option).to_string())
                        })
                        .collect();
                    let mut current = spec.to_plain(normalized as f32);
                    let label = spec.format(normalized as f32);
                    let response = if dial(spec, layout) {
                        // One detent per option; the value below names the
                        // option and lists them all.
                        let last = labels.len().saturating_sub(1).max(1);
                        // The ends of a scale ("1/32 T" to "1/1 D") stand
                        // under the arc; names that are no scale ("Single",
                        // "Golden") would only collide there.
                        let scale = labels
                            .iter()
                            .all(|label| label.starts_with(|first: char| first.is_ascii_digit()));
                        let ends = if scale {
                            (labels[0], labels[last])
                        } else {
                            ("", "")
                        };
                        let mut value = normalized;
                        let response = ui.add(
                            daw_ui::Knob::new(&mut value, 0.0..=1.0)
                                .compact()
                                .pointer()
                                .diameter(layout.knob.diameter())
                                .text(caption)
                                .endpoint_labels(ends.0, ends.1)
                                .step_by(1.0 / last as f64)
                                .automation(automation.map(|(_, state)| state)),
                        );
                        if response.double_clicked() {
                            edit(host, layout, spec.id, f64::from(spec.default_normalized()));
                        } else if response.changed() {
                            edit(host, layout, spec.id, value.clamp(0.0, 1.0));
                        }
                        ui.add_space(offset);
                        let text = (layout.readout)(spec.id, &|id| host.value(id)).unwrap_or(label);
                        let (_, picked) = daw_ui::value_menu(
                            ui,
                            &text,
                            labels,
                            current.round() as usize,
                            width - CELL_GAP,
                        );
                        if let Some(index) = picked {
                            let value = f64::from(spec.to_normalized(index as f32));
                            edit(host, layout, spec.id, value);
                        }
                        response
                    } else if let Some(faces) = faces(spec, layout) {
                        let options: Vec<_> = faces
                            .faces
                            .iter()
                            .enumerate()
                            .map(|(index, (face, name))| (index as f32, *face, *name))
                            .collect();
                        let response =
                            daw_ui::glyph_toggle(ui, &mut current, &options, faces.segment_width);
                        if response.changed() {
                            let value = f64::from(spec.to_normalized(current));
                            edit(host, layout, spec.id, value);
                        }
                        response
                    } else if toggles(spec) {
                        let options: Vec<(f32, &str)> = options
                            .iter()
                            .map(|(plain, option)| (*plain, option.as_str()))
                            .collect();
                        let response = match toggle_columns(spec, layout) {
                            Some(columns) => {
                                // Columns as wide as the longest name: less
                                // padding where the place is narrower.
                                let padding = ui.spacing().button_padding.x;
                                let widest = options
                                    .iter()
                                    .map(|(_, option)| {
                                        daw_ui::toggle_group_width(ui, std::iter::once(*option))
                                    })
                                    .fold(0.0, f32::max);
                                let over = widest - (width - CELL_GAP) / columns as f32;
                                ui.spacing_mut().button_padding.x =
                                    (padding - 0.5 * over.max(0.0)).max(0.0);
                                let response =
                                    daw_ui::toggle_grid(ui, &mut current, &options, columns);
                                ui.spacing_mut().button_padding.x = padding;
                                #[cfg(test)]
                                state.grid_rects.push((spec.id, response.rect));
                                response
                            }
                            None => daw_ui::toggle_group(ui, &mut current, &options),
                        };
                        if response.changed() {
                            let value = f64::from(spec.to_normalized(current));
                            edit(host, layout, spec.id, value);
                        }
                        response
                    } else {
                        let size = egui::vec2(natural - CELL_GAP, daw_ui::tokens::CONTROL_HEIGHT);
                        // A child of the choice's own width, which the cell centers.
                        ui.allocate_ui_with_layout(
                            size,
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                daw_ui::select(id, &label)
                                    .width(size.x)
                                    // Long names ("Transistor Ladder") must not push the
                                    // next control out of its cell; the list shows them whole.
                                    .truncate()
                                    .show_ui(ui, |ui| {
                                        for (plain, option) in &options {
                                            if daw_ui::selectable(ui, *plain == current, option)
                                                .clicked()
                                            {
                                                let value = f64::from(spec.to_normalized(*plain));
                                                edit(host, layout, spec.id, value);
                                            }
                                        }
                                    })
                                    .response
                            },
                        )
                        .inner
                        .on_hover_text(&label)
                    };
                    daw_ui::automatable_parameter_context(ui, &response, control, automation)
                }
                ParamRange::Toggle => {
                    let mut on = normalized >= 0.5;
                    let response = match led(spec, layout) {
                        Some(label) => daw_ui::toggle_led(ui, &mut on, label),
                        None => daw_ui::switch(ui, &mut on, ""),
                    };
                    if response.changed() {
                        host.set_value(spec.id, f64::from(u8::from(on)));
                    }
                    daw_ui::automatable_parameter_context(ui, &response, control, automation)
                }
            });
            if !is_knob(spec) && !dial(spec, layout) {
                // A value line, empty unless the layout has something to
                // say there, so every cell of a line is as tall as a knob's
                // and the next line starts level across cards.
                ui.add_space(offset);
                let source = (layout.sidechain == Some(spec.id) && applies && normalized >= 0.5)
                    .then(|| host.sidechain_sources())
                    .flatten();
                match source {
                    Some(sources) => {
                        let text = match &sources[..] {
                            [] => "No source".to_owned(),
                            [only] => only.clone(),
                            [first, rest @ ..] => format!("{first} +{}", rest.len()),
                        };
                        let link =
                            daw_ui::value_link(ui, &text, sources.is_empty(), width - CELL_GAP)
                                .on_hover_text(if sources.is_empty() {
                                    "Nothing is routed to the sidechain input. Open Routing"
                                        .to_owned()
                                } else {
                                    format!("Sidechain from {}. Open Routing", sources.join(", "))
                                });
                        if link.clicked() {
                            host.route_sidechain();
                        }
                    }
                    None => {
                        let readout = (layout.readout)(spec.id, &|id| host.value(id));
                        daw_ui::text::note(ui, readout.as_deref().unwrap_or(" "));
                    }
                }
            }
            if let Some(action) = navigation {
                host.navigate(action);
            }
        },
    );
    #[cfg(test)]
    state.control_rects.push((spec.id, _drawn.response.rect));
}

/// The device's `daw_ui::preset_group`, for its frame's bar. Identical for
/// every device.
fn preset_bar(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    host: &mut dyn DeviceHost,
    state: &mut PanelState,
) {
    let factory = factory_presets(descriptor);
    let user = host.user_presets();
    let presets: Vec<&Preset> = factory.iter().chain(&user).collect();
    let current = state
        .preset
        .as_ref()
        .and_then(|name| presets.iter().position(|preset| &preset.name == name));
    let modified = current.is_none_or(|index| {
        presets[index].resolve(descriptor).is_ok_and(|resolved| {
            resolved
                .values
                .iter()
                .any(|(id, value)| (host.value(*id) - f64::from(*value)).abs() > 1e-4)
        })
    });
    let load = |host: &mut dyn DeviceHost, state: &mut PanelState, preset: &Preset| {
        if let Ok(resolved) = preset.resolve(descriptor) {
            let values: Vec<_> = resolved
                .values
                .iter()
                .map(|(id, value)| (*id, f64::from(*value)))
                .collect();
            host.set_values(&values);
            state.preset = Some(preset.name.clone());
        }
    };
    let name = state.preset.clone().unwrap_or_else(|| "Custom".into());
    let name = if modified && state.preset.is_some() {
        format!("{name} *")
    } else {
        name
    };
    let labels: Vec<String> = presets
        .iter()
        .map(|preset| {
            if preset.category.is_empty() {
                preset.name.clone()
            } else {
                format!("{} · {}", preset.category, preset.name)
            }
        })
        .collect();
    let choices: Vec<_> = presets
        .iter()
        .zip(&labels)
        .map(|(preset, label)| daw_ui::PresetChoice {
            label,
            description: &preset.description,
        })
        .collect();
    let step = |offset: isize| {
        let count = presets.len() as isize;
        let from = current.map_or(if offset > 0 { -1 } else { 0 }, |index| index as isize);
        (count > 0).then(|| (from + offset).rem_euclid(count) as usize)
    };
    let saving = state.save_name.is_some();
    let action = daw_ui::preset_group(
        ui,
        egui::Id::new((descriptor.id, "preset")),
        &name,
        &choices,
        current,
        &mut state.save_name,
    );
    if !saving && let Some(draft) = &mut state.save_name {
        // Saving starts from the loaded preset's name.
        *draft = state.preset.clone().unwrap_or_default();
    }
    match action {
        Some(daw_ui::PresetAction::Previous) => {
            if let Some(index) = step(-1) {
                load(host, state, presets[index]);
            }
        }
        Some(daw_ui::PresetAction::Next) => {
            if let Some(index) = step(1) {
                load(host, state, presets[index]);
            }
        }
        Some(daw_ui::PresetAction::Load(index)) => load(host, state, presets[index]),
        Some(daw_ui::PresetAction::SaveAs(name)) => {
            let preset = Preset::capture(descriptor, &name, |id| host.value(id) as f32);
            host.save_preset(preset);
            state.preset = Some(name);
        }
        Some(daw_ui::PresetAction::Reset) => {
            let values: Vec<_> = descriptor
                .parameters
                .iter()
                .map(|spec| (spec.id, f64::from(spec.default_normalized())))
                .collect();
            host.set_values(&values);
            state.preset = None;
        }
        Some(daw_ui::PresetAction::Randomize(amount)) => {
            // The time of the click picks the roll.
            let seed = ui.input(|input| input.time).to_bits();
            let values = random_roll(descriptor, host, amount, seed);
            host.set_values(&values);
            state.preset = None;
        }
        Some(daw_ui::PresetAction::ToggleFavorite(_)) | None => {}
    }
}

/// A roll of the dice over the whole device, every parameter included as a
/// preset sets them: a host takes a whole device as one Undo step, never
/// merged with the roll before it.
fn random_roll(
    descriptor: &DeviceDescriptor,
    host: &dyn DeviceHost,
    amount: f32,
    seed: u64,
) -> Vec<(u32, f64)> {
    let rolled = daw_devices::randomized(descriptor, |id| host.value(id) as f32, amount, seed);
    descriptor
        .parameters
        .iter()
        .map(|spec| {
            let value = rolled
                .iter()
                .find(|(id, _)| *id == spec.id)
                .map_or(host.value(spec.id), |(_, value)| f64::from(*value));
            (spec.id, value)
        })
        .collect()
}

#[cfg(test)]
mod tests;

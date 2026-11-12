//! Per-device layout rules. A device without an entry here gets the generic
//! layout: one section per parameter group, every control visible.
use daw_devices::DeviceDescriptor;
use daw_devices::devices::{compressor, equalizer, filter, mini, modulator, saturator, spacer};
use daw_ui::{Glyph, ToggleFace};

pub type Values<'a> = &'a dyn Fn(u32) -> f64;

/// The rate a curve is drawn at when the device does not report its own;
/// at other host rates the curve differs only near Nyquist.
pub const RESPONSE_RATE: f32 = 48_000.0;

/// What stands at one place of a section's line.
#[derive(Clone, Copy)]
pub enum Slot {
    /// One control, this many cells wide.
    Control(u32, u8),
    /// Cells left empty, so the controls after them keep their column.
    Blank(u8),
    /// Vertical faders sharing this many cells, as tall as all the lines of
    /// the row: the stages of an envelope.
    Faders(&'static [u32], u8),
    /// One place for several parameters of which one applies at a time: the
    /// first that does is drawn (the first, disabled, when none does).
    Swap(&'static [u32], u8),
}

impl Slot {
    pub fn ids(&self) -> &[u32] {
        match self {
            Self::Control(id, _) => std::slice::from_ref(id),
            Self::Blank(_) => &[],
            Self::Faders(ids, _) | Self::Swap(ids, _) => ids,
        }
    }
}

pub struct Section {
    pub title: &'static str,
    /// Controls line by line; the lines of a card share their columns.
    pub lines: Vec<Vec<Slot>>,
    /// Sections sharing a row sit side by side, in signal order.
    pub row: usize,
    /// Width in columns of `daw_ui::grid_row`; every row's spans add up to
    /// the same number. On a cell grid, also the card's width in cells.
    pub span: u8,
    /// Whether the card is a table instead of captioned controls.
    pub table: Option<Table>,
    /// The toggle that switches the section, drawn as a lamp in its title.
    pub enable: Option<u32>,
    /// The choice of the section's model, drawn as toggle buttons at the
    /// right end of its title line. Every card of a row has one or none.
    pub choice: Option<u32>,
}

/// A card of like values in equal columns, one slot per column on every
/// line: continuous values are inline dials, toggles and choices stand bare.
/// What a line holds is named once, at its left. A click in a column selects
/// it, together with the response node of the same position.
pub struct Table {
    /// The caption of each line; empty where the controls name themselves.
    pub captions: &'static [&'static str],
    /// Width of the caption column.
    pub caption_width: f32,
}

impl Section {
    #[cfg(test)]
    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.lines
            .iter()
            .flatten()
            .flat_map(|slot| slot.ids().iter().copied())
            .chain(self.enable)
            .chain(self.choice)
    }
}

pub struct Layout {
    /// One of the two panel widths of the panel grammar.
    pub width: f32,
    pub sections: Vec<Section>,
    /// The one large knob of the panel, if any.
    pub hero: Option<u32>,
    /// Every other knob: standard, or dense for a panel with many controls.
    pub knob: daw_ui::KnobSize,
    /// Captions that differ from the parameter's name.
    pub captions: &'static [(u32, &'static str)],
    /// Whether every card divides into equal cells, one control each unless
    /// a slot says otherwise, as on a hardware front panel. Otherwise a
    /// control takes the width its caption and value need.
    pub cells: bool,
    /// Short choices whose toggle buttons form a grid of this many columns
    /// instead of one row, where a card is short of width.
    pub toggle_columns: &'static [(u32, usize)],
    /// Whether a control does anything in the current mode.
    pub visible: fn(u32, Values<'_>) -> bool,
    /// Whether a parameter is the one its [`Slot::Swap`] shows, where that
    /// is not simply the first that does something: a slot whose whole
    /// card is switched off still shows the one in use.
    pub swap: fn(u32, Values<'_>) -> bool,
    /// What happens to a control that does nothing: hidden, or, on a dense
    /// panel whose cards would otherwise reflow, disabled in its place.
    pub keep_inactive: bool,
    /// Choice options that apply in the current mode, when not all do.
    pub options: fn(u32, Values<'_>) -> Option<&'static [&'static str]>,
    /// Choices drawn as `daw_ui::glyph_toggle` instead of words.
    pub faces: &'static [Faces],
    /// Toggles drawn as `daw_ui::toggle_led` with this label instead of a
    /// captioned switch.
    pub leds: &'static [(u32, &'static str)],
    /// Captions that follow a choice: the caption of the first parameter is
    /// the entry at the option index of the second, a choice. The cell is as
    /// wide as the widest entry, so a card never reflows when the caption
    /// changes; the parameter's own name stays what lanes and tools show.
    pub mode_captions: &'static [(u32, u32, &'static [&'static str])],
    /// Text for a control's value line: what stands under a control that is
    /// not a knob, and what replaces a knob's value where the device uses
    /// another ("Off" at a minimum, a time a character holds up).
    pub readout: fn(u32, Values<'_>) -> Option<String>,
    /// The choice whose value line names what the host routed to the
    /// device's sidechain input, as a link to the routing.
    pub sidechain: Option<u32>,
    /// The device's picture.
    pub picture: Option<Picture>,
    /// Where it stands.
    pub picture_place: PicturePlace,
    /// Choices drawn as a detented knob over a value that lists the options
    /// when clicked: a list too long for toggles in a place one cell wide.
    pub dials: &'static [u32],
    /// What else a change of a control from the panel sets, sent with it as
    /// one edit: the new value of the control, and the values as they are.
    /// Presets, automation, tools and Undo never ask.
    pub companions: fn(u32, f64, Values<'_>) -> Vec<(u32, f64)>,
    /// Whole numbers drawn as `daw_ui::variation_stepper` instead of a
    /// knob: a numbered variation, where no value is more than another.
    pub steppers: &'static [u32],
    /// What hover says about a control whose meaning follows the mode.
    pub hint: fn(u32, Values<'_>) -> Option<&'static str>,
}

/// Where a panel shows its picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PicturePlace {
    /// Over the whole width, above the cards.
    Above,
    /// As the first cell of a row of a cell grid, without a frame, this
    /// many columns wide and as tall as the row's cards.
    Row { row: usize, span: u8 },
}

/// The picture of a device, fed by the same functions as its audio path.
pub enum Picture {
    /// A frequency response with draggable nodes.
    Response(ResponseView),
    /// A shaper: its curve beside the harmonics it adds.
    Saturation { height: f32 },
    /// Dynamics: the static curve, levels and reduction over time, and the
    /// reduction meter.
    Dynamics { height: f32 },
    /// A modulation effect: its LFO beside the response it sweeps.
    Modulation { height: f32 },
    /// Echoes and a reverb's decay over time.
    Space,
}

impl Layout {
    #[cfg(test)]
    pub fn response(&self) -> Option<&ResponseView> {
        match &self.picture {
            Some(Picture::Response(view)) => Some(view),
            _ => None,
        }
    }
}

/// How the options of some choices look as toggle segments.
pub struct Faces {
    pub ids: &'static [u32],
    /// One per option, in option order, with the name hover reads.
    pub faces: &'static [(ToggleFace<'static>, &'static str)],
    pub segment_width: f32,
}

/// Which telemetry ring a spectrum reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tap {
    Input,
    Output,
}

/// Fills dB values for one node (by index), the frequencies and the rate.
pub type NodeCurve = fn(Values<'_>, usize, f32, &[f32], &mut [f32]);

pub struct ResponseView {
    /// Fills dB values for the given frequencies at the given sample rate.
    pub curve: fn(Values<'_>, f32, &[f32], &mut [f32]),
    /// The same for one node alone, drawn muted for the selected node.
    pub node_curve: Option<NodeCurve>,
    pub nodes: &'static [NodeView],
    /// A node's values in words, shown beside it under the pointer.
    pub readout: Option<fn(usize, Values<'_>) -> String>,
    /// The spectrum drawn behind the curves, if the device has one switched on.
    pub spectrum: fn(Values<'_>) -> Option<Tap>,
    /// The telemetry value that holds the device's sample rate.
    pub rate_slot: Option<usize>,
    pub height: f32,
    /// The gain axis runs from minus to plus this.
    pub range_db: fn(Values<'_>) -> f32,
    /// How far a node's gain can be dragged, beyond the axis if need be.
    pub limit_db: f32,
}

/// One draggable node of the response and the parameters it edits.
pub struct NodeView {
    /// Printed at the node; its number when `None`.
    pub label: Option<&'static str>,
    pub frequency: u32,
    /// The node moves along the frequency axis only without one, and while
    /// the parameter does not apply.
    pub gain: Option<u32>,
    /// What the scroll wheel changes: a continuous value, or a choice it steps.
    pub wheel: Option<u32>,
    /// The toggle a double-click flips; the node is hollow while it is off.
    pub enable: Option<u32>,
    /// A choice whose options the node's context menu offers.
    pub menu: Option<u32>,
}

impl NodeView {
    pub fn ids(&self) -> impl Iterator<Item = u32> {
        [
            self.enable,
            self.menu,
            Some(self.frequency),
            self.gain,
            self.wheel,
        ]
        .into_iter()
        .flatten()
    }
}

fn section(title: &'static str, row: usize, span: u8, lines: &[&[u32]]) -> Section {
    let lines: Vec<Vec<Slot>> = lines
        .iter()
        .map(|line| line.iter().map(|id| Slot::Control(*id, 1)).collect())
        .collect();
    Section {
        title,
        lines,
        row,
        span,
        table: None,
        enable: None,
        choice: None,
    }
}

fn cells(title: &'static str, row: usize, span: u8, lines: &[&[Slot]]) -> Section {
    Section {
        title,
        lines: lines.iter().map(|line| line.to_vec()).collect(),
        row,
        span,
        table: None,
        enable: None,
        choice: None,
    }
}

pub fn layout(descriptor: &DeviceDescriptor) -> Layout {
    let mut layout = generic(descriptor);
    if descriptor.id == filter::DESCRIPTOR.id {
        use filter::*;
        // The filter on top, its character before its frequency; below it
        // the modulation sources, and the output last.
        layout.width = daw_ui::tokens::POPUP_WIDTH_LG;
        layout.sections = vec![
            section(
                "Filter",
                0,
                12,
                &[&[MODEL, TYPE, SLOPE, CUTOFF, RESONANCE, DRIVE, MORPH]],
            ),
            section(
                "LFO",
                1,
                6,
                &[&[LFO_RATE, LFO_SYNC, LFO_SHAPE, LFO_DEPTH, LFO_STEREO]],
            ),
            section("Envelope", 1, 4, &[&[ENV_AMOUNT, ENV_ATTACK, ENV_RELEASE]]),
            section("Output", 1, 2, &[&[MIX, OUTPUT]]),
        ];
        layout.hero = Some(CUTOFF);
        layout.toggle_columns = &[(SLOPE, 1)];
        layout.visible = |id, values| applies(id, &filter_settings(values));
        layout.options =
            |id, values| (id == TYPE).then(|| type_options(filter_settings(values).model));
        layout.picture = Some(Picture::Response(ResponseView {
            curve: |values, rate, frequencies, out| {
                let settings = filter_settings(values);
                // The drawn curve is static; the sounding cutoff also moves with the LFO and envelope.
                response_db(&settings, settings.cutoff, rate, frequencies, out);
            },
            node_curve: None,
            nodes: &[NodeView {
                label: None,
                frequency: CUTOFF,
                gain: None,
                wheel: None,
                enable: None,
                menu: None,
            }],
            readout: None,
            spectrum: |_| None,
            rate_slot: None,
            height: 160.0,
            range_db: |_| 24.0,
            limit_db: 24.0,
        }));
    }
    if descriptor.id == equalizer::DESCRIPTOR.id {
        layout = equalizer_layout(layout);
    }
    if descriptor.id == saturator::DESCRIPTOR.id {
        layout = saturator_layout(layout);
    }
    if descriptor.id == compressor::DESCRIPTOR.id {
        layout = compressor_layout(layout);
    }
    if descriptor.id == modulator::DESCRIPTOR.id {
        layout = modulator_layout(layout);
    }
    if descriptor.id == spacer::DESCRIPTOR.id {
        layout = spacer_layout(layout);
    }
    if descriptor.id == mini::DESCRIPTOR.id {
        use mini::*;
        // A hardware front panel on one grid of equal cells, twelve to a
        // row, in signal order: the sources, their mix and the master
        // controls; the filter and the two envelopes; modulation and the
        // voices. Every row breaks after its eighth cell. Both oscillators
        // share their columns, so the second one's extra controls stand
        // out; an envelope is four faders, read as its shape, and a fifth
        // for how much velocity drives it.
        use Slot::{Blank, Control as C, Faders};
        layout.width = daw_ui::tokens::POPUP_WIDTH_LG;
        layout.cells = true;
        layout.sections = vec![
            cells(
                "Oscillators",
                0,
                8,
                &[
                    &[
                        C(OSC1_MODEL, 1),
                        C(OSC1_WAVE, 2),
                        C(OSC1_OCTAVE, 1),
                        C(OSC1_WIDTH, 1),
                        Blank(3),
                    ],
                    &[
                        C(OSC2_MODEL, 1),
                        C(OSC2_WAVE, 2),
                        C(OSC2_OCTAVE, 1),
                        C(OSC2_WIDTH, 1),
                        C(OSC2_SEMITONES, 1),
                        C(OSC2_DETUNE, 1),
                        C(OSC2_SYNC, 1),
                    ],
                ],
            ),
            cells(
                "Mixer",
                0,
                2,
                &[
                    &[C(MIX_OSC1, 1), C(MIX_SUB, 1)],
                    &[C(MIX_OSC2, 1), C(MIX_NOISE, 1)],
                ],
            ),
            cells(
                "Master",
                0,
                2,
                &[
                    &[C(VOLUME, 1), C(GLIDE, 1)],
                    &[C(BEND_RANGE, 1), C(DRIFT, 1)],
                ],
            ),
            cells(
                "Filter",
                1,
                4,
                &[
                    &[C(FILTER_MODEL, 3), C(CUTOFF, 1)],
                    &[
                        C(RESONANCE, 1),
                        C(DRIVE, 1),
                        C(FILTER_ENV, 1),
                        C(KEY_TRACKING, 1),
                    ],
                ],
            ),
            cells(
                "Filter envelope",
                1,
                4,
                &[&[
                    Faders(&[FENV_ATTACK, FENV_DECAY, FENV_SUSTAIN, FENV_RELEASE], 3),
                    Faders(&[VELOCITY_CUTOFF], 1),
                ]],
            ),
            cells(
                "Amp envelope",
                1,
                4,
                &[&[
                    Faders(&[AENV_ATTACK, AENV_DECAY, AENV_SUSTAIN, AENV_RELEASE], 3),
                    Faders(&[VELOCITY_AMP], 1),
                ]],
            ),
            cells(
                "LFO",
                2,
                8,
                &[&[
                    C(LFO_SHAPE, 2),
                    C(LFO_RATE, 1),
                    C(LFO_SYNC, 1),
                    C(LFO_PITCH, 1),
                    C(LFO_CUTOFF, 1),
                    C(LFO_WIDTH, 1),
                    C(LFO_WHEEL, 1),
                ]],
            ),
            cells("Voices", 2, 4, &[&[C(VOICE_MODE, 3), C(VOICES, 1)]]),
        ];
        layout.toggle_columns = &[
            (OSC1_MODEL, 1),
            (OSC2_MODEL, 1),
            (OSC1_WAVE, 2),
            (OSC2_WAVE, 2),
        ];
        layout.captions = &[
            (OSC1_MODEL, "Osc 1"),
            (OSC2_MODEL, "Osc 2"),
            (VELOCITY_CUTOFF, "Velocity"),
            (VELOCITY_AMP, "Velocity"),
            // As engraved on hardware; four full names do not fit side by side.
            (FENV_ATTACK, "A"),
            (FENV_DECAY, "D"),
            (FENV_SUSTAIN, "S"),
            (FENV_RELEASE, "R"),
            (AENV_ATTACK, "A"),
            (AENV_DECAY, "D"),
            (AENV_SUSTAIN, "S"),
            (AENV_RELEASE, "R"),
        ];
        layout.hero = Some(CUTOFF);
        layout.visible = |id, values| applies(id, &mini_settings(values));
        layout.keep_inactive = true;
    }
    layout
}

/// The six nodes of the equalizer's picture, low to high; the Bands table
/// has one column for each, in the same order.
static EQUALIZER_NODES: [NodeView; 6] = {
    use equalizer::*;
    const fn band(index: usize, label: &'static str) -> NodeView {
        NodeView {
            label: Some(label),
            frequency: BANDS[index] + BAND_FREQUENCY,
            gain: Some(BANDS[index] + BAND_GAIN),
            wheel: Some(BANDS[index] + BAND_Q),
            enable: Some(BANDS[index] + BAND_ON),
            menu: Some(BANDS[index] + BAND_TYPE),
        }
    }
    const fn cut(label: &'static str, on: u32, frequency: u32, slope: u32) -> NodeView {
        NodeView {
            label: Some(label),
            frequency,
            gain: None,
            wheel: Some(slope),
            enable: Some(on),
            menu: Some(slope),
        }
    }
    [
        cut("HP", HP_ON, HP_FREQUENCY, HP_SLOPE),
        band(0, "1"),
        band(1, "2"),
        band(2, "3"),
        band(3, "4"),
        cut("LP", LP_ON, LP_FREQUENCY, LP_SLOPE),
    ]
};

/// Width of a Bands column's toggle track: four type glyphs or three slopes.
const BAND_TRACK: f32 = 84.0;

fn equalizer_layout(mut layout: Layout) -> Layout {
    use Slot::{Blank, Control as C};
    use equalizer::*;
    // The graph is the editor; under it every band's numbers as a table in
    // the order of the nodes, and beside that what applies to the whole
    // device. The wide panel, so that all of it shows without scrolling in
    // the effect window; the graph gains from the width.
    layout.width = daw_ui::tokens::POPUP_WIDTH_LG;
    let band = |offset: u32| BANDS.map(|base| C(base + offset, 1));
    let line = |first: Slot, bands: [Slot; 4], last: Slot| {
        let mut line = vec![first];
        line.extend(bands);
        line.push(last);
        line
    };
    layout.sections = vec![
        Section {
            title: "Bands",
            lines: vec![
                line(C(HP_ON, 1), band(BAND_ON), C(LP_ON, 1)),
                line(C(HP_SLOPE, 1), band(BAND_TYPE), C(LP_SLOPE, 1)),
                line(C(HP_FREQUENCY, 1), band(BAND_FREQUENCY), C(LP_FREQUENCY, 1)),
                line(Blank(1), band(BAND_GAIN), Blank(1)),
                line(Blank(1), band(BAND_Q), Blank(1)),
            ],
            row: 0,
            span: 8,
            table: Some(Table {
                // "Frequency" would take the room of a column, and the columns
                // need theirs: "20.00 kHz" beside its arc is 88 points of 98.
                captions: &["", "", "Freq", "Gain", "Q"],
                caption_width: 36.0,
            }),
            enable: None,
            choice: None,
        },
        section("Global", 0, 4, &[&[MODE, ANALYZER], &[AUTO_GAIN, OUTPUT]]),
    ];
    layout.leds = &[
        (HP_ON, "HP"),
        (BANDS[0] + BAND_ON, "1"),
        (BANDS[1] + BAND_ON, "2"),
        (BANDS[2] + BAND_ON, "3"),
        (BANDS[3] + BAND_ON, "4"),
        (LP_ON, "LP"),
        (AUTO_GAIN, "Auto gain"),
    ];
    layout.faces = &[
        Faces {
            ids: &[
                BANDS[0] + BAND_TYPE,
                BANDS[1] + BAND_TYPE,
                BANDS[2] + BAND_TYPE,
                BANDS[3] + BAND_TYPE,
            ],
            faces: &[
                (ToggleFace::Glyph(Glyph::Bell), "Bell"),
                (ToggleFace::Glyph(Glyph::LowShelf), "Low shelf"),
                (ToggleFace::Glyph(Glyph::HighShelf), "High shelf"),
                (ToggleFace::Glyph(Glyph::Notch), "Notch"),
            ],
            segment_width: BAND_TRACK / 4.0,
        },
        Faces {
            ids: &[HP_SLOPE, LP_SLOPE],
            faces: &[
                (ToggleFace::Text("12"), "12 dB/oct"),
                (ToggleFace::Text("24"), "24 dB/oct"),
                (ToggleFace::Text("48"), "48 dB/oct"),
            ],
            segment_width: BAND_TRACK / 3.0,
        },
    ];
    layout.visible = |id, values| applies(id, &equalizer_settings(values));
    // A notch's Gain stays in its cell, so the table never moves.
    layout.keep_inactive = true;
    layout.readout = |id, values| {
        if id != AUTO_GAIN || values(AUTO_GAIN) < 0.5 {
            return None;
        }
        // What Auto gain applies, printed like Output beside it; nothing
        // while it is off. The rate matters only next to Nyquist.
        let spec = DESCRIPTOR.parameter(OUTPUT)?;
        let db = auto_gain_db(&equalizer_settings(values), RESPONSE_RATE);
        Some(spec.format(spec.to_normalized(db)))
    };
    layout.picture = Some(Picture::Response(ResponseView {
        curve: |values, rate, frequencies, out| {
            response_db(&equalizer_settings(values), rate, frequencies, out);
        },
        node_curve: Some(|values, node, rate, frequencies, out| {
            band_response_db(&equalizer_settings(values), node, rate, frequencies, out);
        }),
        nodes: &EQUALIZER_NODES,
        readout: Some(|node, values| {
            let node = &EQUALIZER_NODES[node];
            let text = |id: u32| {
                DESCRIPTOR
                    .parameter(id)
                    .map(|spec| spec.format(values(id) as f32))
                    .unwrap_or_default()
            };
            let frequency = text(node.frequency);
            match (node.gain, node.wheel) {
                (Some(gain), Some(q)) if applies(gain, &equalizer_settings(values)) => {
                    format!("{frequency}   {}   Q {}", text(gain), text(q))
                }
                (Some(_), Some(q)) => format!("{frequency}   Q {}", text(q)),
                (_, Some(slope)) => format!("{frequency}   {}/oct", text(slope)),
                _ => frequency,
            }
        }),
        spectrum: |values| match equalizer_settings(values).analyzer {
            Analyzer::Off => None,
            Analyzer::Input => Some(Tap::Input),
            Analyzer::Output => Some(Tap::Output),
        },
        rate_slot: Some(TELEMETRY_SAMPLE_RATE),
        height: 200.0,
        // Twelve decibels read finely; the axis doubles once a band needs it.
        range_db: |values| {
            let settings = equalizer_settings(values);
            let beyond = settings
                .bands
                .iter()
                .any(|band| band.on && band.kind != BandType::Notch && band.gain_db.abs() > 12.0);
            if beyond { 24.0 } else { 12.0 }
        },
        limit_db: 24.0,
    }));
    layout
}

/// A value printed as another parameter of the device prints its own.
fn formatted(descriptor: &DeviceDescriptor, like: u32, plain: f32) -> Option<String> {
    let spec = descriptor.parameter(like)?;
    Some(spec.format(spec.to_normalized(plain)))
}

fn saturator_layout(mut layout: Layout) -> Layout {
    use saturator::*;
    // The curve and what it adds on top; below, in the order of the signal:
    // the model and how hard it is hit, what colours it, then the cost
    // setting and the output. The wide panel: at the narrow width two rows
    // of cards leave the picture 110 points.
    layout.width = daw_ui::tokens::POPUP_WIDTH_LG;
    layout.sections = vec![
        section(
            "Saturation",
            0,
            6,
            &[&[MODEL, DRIVE, CHARACTER, TONE, FOCUS]],
        ),
        section("Quality", 0, 2, &[&[QUALITY]]),
        section("Output", 0, 4, &[&[AUTO_GAIN, MIX, OUTPUT]]),
    ];
    layout.hero = Some(DRIVE);
    layout.leds = &[(AUTO_GAIN, "Auto gain")];
    // What Character does in each model.
    layout.mode_captions = &[(CHARACTER, MODEL, &["Hardness", "Bias", "Compress"])];
    layout.readout = |id, values| match id {
        FOCUS if values(FOCUS) <= 0.0 => Some("Off".into()),
        // What Auto gain applies, printed like Output beside it; nothing
        // while it is off.
        AUTO_GAIN if values(AUTO_GAIN) >= 0.5 => formatted(
            &DESCRIPTOR,
            OUTPUT,
            auto_gain_db(&saturator_settings(values)),
        ),
        _ => None,
    };
    layout.picture = Some(Picture::Saturation { height: 233.0 });
    layout
}

fn compressor_layout(mut layout: Layout) -> Layout {
    use Slot::Control as C;
    use compressor::*;
    // One row of four cards on a grid of twelve cells, read as the signal
    // and the convention go: the style; threshold, ratio and knee over the
    // timing; the detector's own path, kept small; makeup and output last.
    layout.width = daw_ui::tokens::POPUP_WIDTH_LG;
    layout.cells = true;
    layout.sections = vec![
        cells("Mode", 0, 4, &[&[C(MODE, 4)], &[C(CHARACTER, 4)]]),
        cells(
            "Dynamics",
            0,
            4,
            &[
                &[C(THRESHOLD, 1), C(RATIO, 1), C(KNEE, 1), C(CEILING, 1)],
                &[
                    C(ATTACK, 1),
                    C(RELEASE, 1),
                    C(AUTO_RELEASE, 1),
                    C(LOOKAHEAD, 1),
                ],
            ],
        ),
        cells(
            "Sidechain",
            0,
            2,
            &[
                &[C(SIDECHAIN, 2)],
                &[C(SIDECHAIN_HIGH_PASS, 1), C(STEREO_LINK, 1)],
            ],
        ),
        cells(
            "Output",
            0,
            2,
            &[
                &[C(MAKEUP, 1), C(AUTO_MAKEUP, 1)],
                &[C(MIX, 1), C(OUTPUT, 1)],
            ],
        ),
    ];
    layout.toggle_columns = &[(SIDECHAIN, 1)];
    layout.leds = &[(AUTO_RELEASE, "Auto"), (AUTO_MAKEUP, "Auto")];
    // The card names the sidechain already.
    layout.captions = &[(SIDECHAIN_HIGH_PASS, "High-pass"), (STEREO_LINK, "Link")];
    // In front of a limiter the same value is the gain that drives it.
    layout.mode_captions = &[(MAKEUP, MODE, &["Makeup", "Gain", "Makeup"])];
    layout.visible = |id, values| applies(id, &compressor_settings(values));
    // Hiding Ratio, Ceiling or Lookahead would reflow the cards on every
    // change of Mode.
    layout.keep_inactive = true;
    layout.readout = |id, values| {
        let settings = compressor_settings(values);
        let effective = effective(&settings);
        let limiter = settings.mode == Mode::Limiter;
        match id {
            RATIO if limiter => Some("∞:1".into()),
            // A character that holds a value up shows what it uses.
            ATTACK if !limiter && effective.attack_ms != settings.attack_ms => {
                formatted(&DESCRIPTOR, ATTACK, effective.attack_ms)
            }
            KNEE if effective.curve.knee_db != settings.knee_db => {
                formatted(&DESCRIPTOR, KNEE, effective.curve.knee_db)
            }
            RELEASE if !applies(RELEASE, &settings) => Some("Auto".into()),
            LOOKAHEAD if settings.lookahead_ms <= 0.0 => Some(if limiter {
                formatted(&DESCRIPTOR, LOOKAHEAD, effective.lookahead_ms)?
            } else {
                "Off".into()
            }),
            SIDECHAIN_HIGH_PASS if !settings.high_pass_on => Some("Off".into()),
            // What Auto makeup applies, printed like Makeup beside it;
            // nothing while it is off.
            AUTO_MAKEUP if settings.auto_makeup && applies(AUTO_MAKEUP, &settings) => {
                formatted(&DESCRIPTOR, MAKEUP, auto_makeup_db(&settings))
            }
            _ => None,
        }
    };
    layout.sidechain = Some(SIDECHAIN);
    layout.picture = Some(Picture::Dynamics { height: 208.0 });
    layout
}

fn modulator_layout(mut layout: Layout) -> Layout {
    use Slot::{Control as C, Swap};
    use modulator::*;
    // Cause and effect on top: the LFO beside the response it sweeps. Below,
    // as the signal goes: what kind of effect, the LFO that moves it, the
    // wet path it moves, the mix. Thirteen controls fill ten columns by two
    // lines; on twelve, two cells would stay empty.
    layout.width = daw_ui::tokens::POPUP_WIDTH_LG;
    layout.cells = true;
    layout.sections = vec![
        cells("Mode", 0, 3, &[&[C(MODE, 3)], &[C(CHARACTER, 3)]]),
        cells(
            "LFO",
            0,
            3,
            &[
                &[C(SHAPE, 2), C(SYNC, 1)],
                &[Swap(&[RATE, DIVISION], 1), C(DEPTH, 1), C(STEREO, 1)],
            ],
        ),
        cells(
            "Wet",
            0,
            2,
            &[
                &[
                    Swap(&[CENTER_CHORUS, CENTER_FLANGER, CENTER_PHASER], 1),
                    C(FEEDBACK, 1),
                ],
                // A flanger has neither: Voices stays, disabled.
                &[Swap(&[VOICES, STAGES], 1), C(LOW_CUT, 1)],
            ],
        ),
        cells("Output", 0, 2, &[&[C(MIX, 2)], &[C(OUTPUT, 2)]]),
    ];
    layout.faces = &[Faces {
        ids: &[SHAPE],
        faces: &[
            (ToggleFace::Glyph(Glyph::Sine), "Sine"),
            (ToggleFace::Glyph(Glyph::Triangle), "Triangle"),
            (ToggleFace::Glyph(Glyph::Wander), "Random"),
        ],
        segment_width: 34.0,
    }];
    layout.leds = &[(SYNC, "Sync")];
    layout.dials = &[DIVISION, STAGES];
    // The card and the mode name the rest.
    layout.captions = &[
        (CENTER_CHORUS, "Center"),
        (CENTER_FLANGER, "Center"),
        (CENTER_PHASER, "Center"),
        (LOW_CUT, "Low cut"),
    ];
    layout.visible = |id, values| applies(id, &modulator_settings(values));
    layout.keep_inactive = true;
    layout.readout = |id, values| {
        let settings = modulator_settings(values);
        match id {
            LOW_CUT if !settings.low_cut_on => Some("Off".into()),
            FEEDBACK => Some(format!("{:+.0} %", settings.feedback * 100.0)),
            STEREO => Some(format!("{:.0}°", settings.stereo_degrees)),
            VOICES if settings.voices == 1 => Some("1 voice".into()),
            VOICES => Some(format!("{} voices", settings.voices)),
            STAGES => Some(format!("{} stages", settings.stages)),
            _ => None,
        }
    };
    // A mode starts from its own Feedback, Center and count.
    layout.companions = |id, value, _| {
        if id != MODE {
            return Vec::new();
        }
        let plain = DESCRIPTOR
            .parameter(MODE)
            .map_or(0.0, |spec| spec.to_plain(value as f32));
        let mode = match plain as usize {
            0 => Mode::Chorus,
            1 => Mode::Flanger,
            _ => Mode::Phaser,
        };
        mode_defaults(mode)
            .into_iter()
            .filter_map(|(id, plain)| {
                let spec = DESCRIPTOR.parameter(id)?;
                Some((id, f64::from(spec.to_normalized(plain))))
            })
            .collect()
    };
    layout.picture = Some(Picture::Modulation { height: 208.0 });
    layout
}

fn spacer_layout(mut layout: Layout) -> Layout {
    use Slot::{Control as C, Swap};
    use spacer::*;
    // The display with the controls of the whole device beside it, as on
    // delays and reverbs; below it the two sections side by side in the
    // order of the signal, each with its model on the title line and
    // twelve cells: time and loop, structure, tone, then Duck and Level.
    // Every control applies in every model, so nothing moves on a change.
    layout.width = daw_ui::tokens::POPUP_WIDTH_LG;
    layout.cells = true;
    layout.picture = Some(Picture::Space);
    layout.picture_place = PicturePlace::Row { row: 0, span: 8 };
    // The two routings stacked: side by side they would take three cells.
    layout.toggle_columns = &[(ROUTING, 1)];
    layout.sections = vec![
        cells("Output", 0, 4, &[&[C(ROUTING, 2), C(MIX, 1), C(OUTPUT, 1)]]),
        Section {
            enable: Some(ECHO_ON),
            choice: Some(ECHO_MODEL),
            ..cells(
                "Echo",
                1,
                6,
                &[
                    &[
                        C(ECHO_SYNC, 1),
                        Swap(&[ECHO_TIME, ECHO_DIVISION], 1),
                        C(ECHO_FEEDBACK, 1),
                        C(ECHO_TAPS, 1),
                        C(ECHO_STEREO, 2),
                    ],
                    &[
                        C(ECHO_DIFFUSION, 1),
                        C(ECHO_COLOR, 1),
                        C(ECHO_WOBBLE, 1),
                        C(ECHO_LOW_CUT, 1),
                        C(ECHO_DUCK, 1),
                        C(ECHO_LEVEL, 1),
                    ],
                ],
            )
        },
        Section {
            enable: Some(REVERB_ON),
            choice: Some(REVERB_MODEL),
            ..cells(
                "Reverb",
                1,
                6,
                &[
                    &[
                        C(PRE_DELAY, 1),
                        C(SIZE, 1),
                        C(DECAY, 1),
                        C(DAMPING, 1),
                        C(DIFFUSION, 1),
                        C(MOVEMENT, 1),
                    ],
                    &[
                        C(SEED, 1),
                        C(REVERB_COLOR, 1),
                        C(REVERB_LOW_CUT, 1),
                        C(WIDTH, 1),
                        C(REVERB_DUCK, 1),
                        C(REVERB_LEVEL, 1),
                    ],
                ],
            )
        },
    ];
    layout.faces = &[Faces {
        ids: &[ECHO_STEREO],
        faces: &[
            (ToggleFace::Text("Mono"), "Mono"),
            (ToggleFace::Text("Ping"), "Ping-pong"),
            (ToggleFace::Text("Wide"), "Wide"),
        ],
        segment_width: 48.0,
    }];
    layout.leds = &[(ECHO_SYNC, "Sync")];
    layout.dials = &[ECHO_DIVISION, ECHO_TAPS];
    layout.steppers = &[SEED];
    // The cards name the section.
    layout.captions = &[
        (ECHO_TIME, "Time"),
        (ECHO_DIVISION, "Division"),
        (ECHO_FEEDBACK, "Feedback"),
        (ECHO_TAPS, "Taps"),
        (ECHO_STEREO, "Stereo"),
        (ECHO_DIFFUSION, "Diffusion"),
        (ECHO_COLOR, "Color"),
        (ECHO_WOBBLE, "Wobble"),
        (ECHO_LOW_CUT, "Low cut"),
        (ECHO_DUCK, "Duck"),
        (ECHO_LEVEL, "Level"),
        (REVERB_COLOR, "Color"),
        (REVERB_LOW_CUT, "Low cut"),
        (REVERB_DUCK, "Duck"),
        (REVERB_LEVEL, "Level"),
    ];
    layout.visible = |id, values| applies(id, &spacer_settings(values));
    layout.swap = |id, values| match id {
        ECHO_TIME => !spacer_settings(values).echo_sync,
        ECHO_DIVISION => spacer_settings(values).echo_sync,
        _ => false,
    };
    layout.keep_inactive = true;
    layout.readout = |id, values| {
        let settings = spacer_settings(values);
        let bottom = values(id) <= 0.0;
        let off = match id {
            ECHO_LEVEL | REVERB_LEVEL if bottom => return Some("−∞ dB".into()),
            SEED => return Some(format!("#{}", settings.reverb.seed)),
            DECAY if settings.reverb.decay_seconds < 1.0 => {
                return Some(format!("{:.0} ms", settings.reverb.decay_seconds * 1000.0));
            }
            // In Digital, Color is the corner of the cut in the loop.
            ECHO_COLOR if settings.echo.model == EchoModel::Digital => {
                return Some(
                    match daw_devices::dsp::echo::digital_cut_hz(settings.echo.color, RESPONSE_RATE)
                    {
                        Some(hz) => format!("{:.1} kHz", hz / 1000.0),
                        None => "Off".into(),
                    },
                );
            }
            ECHO_DUCK | REVERB_DUCK | REVERB_LOW_CUT => bottom,
            // Bucket-brigade and tape loops must not pass DC: their low
            // cut still filters at the end of its range.
            ECHO_LOW_CUT => bottom && settings.echo.model == EchoModel::Digital,
            _ => false,
        };
        off.then(|| "Off".into())
    };
    // Color and Wobble are each model's own.
    layout.hint = |id, values| {
        let model = spacer_settings(values).echo.model;
        Some(match (id, model) {
            (ECHO_COLOR, EchoModel::Digital) => "High cut of the repeats: darker left, open right",
            (ECHO_COLOR, EchoModel::Bbd) => {
                "Clock rate: fewer, noisier stages left; more, cleaner right"
            }
            (ECHO_COLOR, EchoModel::Tape) => {
                "Tape and heads: worn and hot left, fresh and clean right"
            }
            (ECHO_WOBBLE, EchoModel::Digital) => "Slow drift of the time",
            (ECHO_WOBBLE, EchoModel::Bbd) => "Clock drift",
            (ECHO_WOBBLE, EchoModel::Tape) => "Wow, flutter and drift",
            (REVERB_COLOR, _) => "Tone of the reverb: dark left, bright right",
            (SEED, _) => "Another room of the same size and decay",
            _ => return None,
        })
    };
    layout
}

pub fn modulator_settings(values: Values<'_>) -> modulator::Settings {
    modulator::Settings::from_normalized(|id| values(id) as f32)
}

pub fn spacer_settings(values: Values<'_>) -> spacer::Settings {
    spacer::Settings::from_normalized(|id| values(id) as f32)
}

pub fn saturator_settings(values: Values<'_>) -> saturator::Settings {
    saturator::Settings::from_normalized(|id| values(id) as f32)
}

pub fn compressor_settings(values: Values<'_>) -> compressor::Settings {
    compressor::Settings::from_normalized(|id| values(id) as f32)
}

fn equalizer_settings(values: Values<'_>) -> equalizer::Settings {
    equalizer::Settings::from_normalized(|id| values(id) as f32)
}

fn mini_settings(values: Values<'_>) -> mini::Settings {
    mini::Settings::from_normalized(|id| values(id) as f32)
}

fn filter_settings(values: Values<'_>) -> filter::Settings {
    filter::Settings::from_normalized(|id| values(id) as f32)
}

/// Groups in the order their first parameter appears in the descriptor,
/// which devices keep in signal order, one line each.
fn generic(descriptor: &DeviceDescriptor) -> Layout {
    let mut sections: Vec<Section> = Vec::new();
    for spec in descriptor.parameters {
        match sections
            .iter_mut()
            .find(|section| section.title == spec.group)
        {
            Some(section) => section.lines[0].push(Slot::Control(spec.id, 1)),
            None => sections.push(Section {
                title: spec.group,
                lines: vec![vec![Slot::Control(spec.id, 1)]],
                row: sections.len(),
                span: 1,
                table: None,
                enable: None,
                choice: None,
            }),
        }
    }
    Layout {
        width: daw_ui::tokens::POPUP_WIDTH_MD,
        sections,
        hero: None,
        knob: daw_ui::KnobSize::Standard,
        captions: &[],
        cells: false,
        toggle_columns: &[],
        visible: |_, _| true,
        swap: |_, _| false,
        keep_inactive: false,
        options: |_, _| None,
        faces: &[],
        leds: &[],
        mode_captions: &[],
        readout: |_, _| None,
        sidechain: None,
        picture: None,
        picture_place: PicturePlace::Above,
        dials: &[],
        companions: |_, _, _| Vec::new(),
        steppers: &[],
        hint: |_, _| None,
    }
}

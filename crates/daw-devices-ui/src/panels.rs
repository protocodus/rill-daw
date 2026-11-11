//! Per-device layout rules. A device without an entry here gets the generic
//! layout: one section per parameter group, every control visible.
use daw_devices::DeviceDescriptor;
use daw_devices::devices::{filter, mini};

pub type Values<'a> = &'a dyn Fn(u32) -> f64;

/// Drawing only; at other host rates the curve differs only near Nyquist.
const RESPONSE_RATE: f32 = 48_000.0;

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
}

impl Slot {
    pub fn ids(&self) -> &[u32] {
        match self {
            Self::Control(id, _) => std::slice::from_ref(id),
            Self::Blank(_) => &[],
            Self::Faders(ids, _) => ids,
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
}

impl Section {
    #[cfg(test)]
    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.lines
            .iter()
            .flatten()
            .flat_map(|slot| slot.ids().iter().copied())
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
    /// What happens to a control that does nothing: hidden, or, on a dense
    /// panel whose cards would otherwise reflow, disabled in its place.
    pub keep_inactive: bool,
    /// Choice options that apply in the current mode, when not all do.
    pub options: fn(u32, Values<'_>) -> Option<&'static [&'static str]>,
    /// The response curve drawn above the controls.
    pub response: Option<ResponseView>,
}

pub struct ResponseView {
    /// Fills dB values for the given frequencies.
    pub curve: fn(Values<'_>, &[f32], &mut [f32]),
    /// The frequency parameter whose node moves along the curve's axis.
    pub node: u32,
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
    }
}

fn cells(title: &'static str, row: usize, span: u8, lines: &[&[Slot]]) -> Section {
    Section {
        title,
        lines: lines.iter().map(|line| line.to_vec()).collect(),
        row,
        span,
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
        layout.response = Some(ResponseView {
            curve: |values, frequencies, out| {
                let settings = filter_settings(values);
                // The drawn curve is static; the sounding cutoff also moves with the LFO and envelope.
                response_db(&settings, settings.cutoff, RESPONSE_RATE, frequencies, out);
            },
            node: CUTOFF,
        });
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
        keep_inactive: false,
        options: |_, _| None,
        response: None,
    }
}

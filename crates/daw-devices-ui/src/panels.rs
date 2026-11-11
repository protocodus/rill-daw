//! Per-device layout rules. A device without an entry here gets the generic
//! layout: one section per parameter group, every control visible.
use daw_devices::DeviceDescriptor;
use daw_devices::devices::{filter, mini};

pub type Values<'a> = &'a dyn Fn(u32) -> f64;

/// Drawing only; at other host rates the curve differs only near Nyquist.
const RESPONSE_RATE: f32 = 48_000.0;

pub struct Section {
    pub title: &'static str,
    /// Controls line by line; the lines of a card share their columns.
    pub lines: Vec<Vec<u32>>,
    /// Sections sharing a row sit side by side, in signal order.
    pub row: usize,
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
    /// Controls that do nothing in the current mode are hidden, not disabled.
    pub visible: fn(u32, Values<'_>) -> bool,
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

fn section(title: &'static str, row: usize, lines: &[&[u32]]) -> Section {
    Section {
        title,
        lines: lines.iter().map(|line| line.to_vec()).collect(),
        row,
    }
}

pub fn layout(descriptor: &DeviceDescriptor) -> Layout {
    let mut layout = generic(descriptor);
    if descriptor.id == filter::DESCRIPTOR.id {
        use filter::*;
        // The audio path on top, the filter's character before its
        // frequency and the output last; its modulation sources below.
        layout.width = daw_ui::tokens::POPUP_WIDTH_LG;
        layout.sections = vec![
            section(
                "Filter",
                0,
                &[&[MODEL, TYPE, SLOPE, CUTOFF, RESONANCE, DRIVE, MORPH]],
            ),
            section("Output", 0, &[&[MIX, OUTPUT]]),
            section(
                "LFO",
                1,
                &[&[LFO_RATE, LFO_SYNC, LFO_SHAPE, LFO_DEPTH, LFO_STEREO]],
            ),
            section("Envelope", 1, &[&[ENV_AMOUNT, ENV_ATTACK, ENV_RELEASE]]),
        ];
        layout.hero = Some(CUTOFF);
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
        // A classic subtractive front panel in signal order: the sources
        // and their mix, then the filter and the envelopes that shape the
        // sound, then modulation and how the voices play. An oscillator's
        // first line chooses its character, the second its pitch and pulse
        // width; each envelope's velocity amount sits under it.
        layout.width = daw_ui::tokens::POPUP_WIDTH_LG;
        layout.sections = vec![
            section(
                "Osc 1",
                0,
                &[&[OSC1_MODEL, OSC1_WAVE], &[OSC1_OCTAVE, OSC1_WIDTH]],
            ),
            section(
                "Osc 2",
                0,
                &[
                    &[OSC2_MODEL, OSC2_WAVE, OSC2_SYNC],
                    &[OSC2_OCTAVE, OSC2_SEMITONES, OSC2_DETUNE, OSC2_WIDTH],
                ],
            ),
            section("Mixer", 0, &[&[MIX_OSC1, MIX_OSC2], &[MIX_SUB, MIX_NOISE]]),
            section(
                "Filter",
                1,
                &[
                    &[FILTER_MODEL, CUTOFF, RESONANCE, DRIVE],
                    &[FILTER_ENV, KEY_TRACKING],
                ],
            ),
            section(
                "Filter envelope",
                1,
                &[
                    &[FENV_ATTACK, FENV_DECAY, FENV_SUSTAIN, FENV_RELEASE],
                    &[VELOCITY_CUTOFF],
                ],
            ),
            section(
                "Amp envelope",
                1,
                &[
                    &[AENV_ATTACK, AENV_DECAY, AENV_SUSTAIN, AENV_RELEASE],
                    &[VELOCITY_AMP],
                ],
            ),
            section(
                "LFO",
                2,
                &[&[
                    LFO_RATE, LFO_SYNC, LFO_SHAPE, LFO_PITCH, LFO_CUTOFF, LFO_WIDTH, LFO_WHEEL,
                ]],
            ),
            section(
                "Voice",
                2,
                &[&[VOICE_MODE, VOICES, GLIDE, BEND_RANGE, DRIFT, VOLUME]],
            ),
        ];
        layout.captions = &[(VELOCITY_CUTOFF, "Velocity"), (VELOCITY_AMP, "Velocity")];
        layout.hero = Some(CUTOFF);
        layout.knob = daw_ui::KnobSize::Dense;
        layout.visible = |id, values| applies(id, &mini_settings(values));
        layout.response = Some(ResponseView {
            curve: |values, frequencies, out| {
                response_db(&mini_settings(values), RESPONSE_RATE, frequencies, out);
            },
            node: CUTOFF,
        });
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
            Some(section) => section.lines[0].push(spec.id),
            None => sections.push(Section {
                title: spec.group,
                lines: vec![vec![spec.id]],
                row: sections.len(),
            }),
        }
    }
    Layout {
        width: daw_ui::tokens::POPUP_WIDTH_MD,
        sections,
        hero: None,
        knob: daw_ui::KnobSize::Standard,
        captions: &[],
        visible: |_, _| true,
        options: |_, _| None,
        response: None,
    }
}

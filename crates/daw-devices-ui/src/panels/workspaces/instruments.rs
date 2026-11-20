//! Instrument workspaces follow the task being performed while preserving a
//! shared output strip and the instrument's audition controls.
use super::*;

pub(super) fn build(
    layout: Layout,
    descriptor: &DeviceDescriptor,
    pad: usize,
    page: usize,
) -> Workspace {
    match descriptor.id {
        "r1-mini" => mini_workspace(layout, page),
        "r1-sampler" => sampler_workspace(layout, page),
        "r1-drums" => drums_workspace(layout, pad.min(drums::PADS - 1)),
        _ => unreachable!("instrument workspace requires an R1 instrument"),
    }
}

const LFO_WAVES: &[(ToggleFace<'static>, &str)] = &[
    (ToggleFace::Glyph(Glyph::Sine), "Sine"),
    (ToggleFace::Glyph(Glyph::Triangle), "Triangle"),
    (ToggleFace::Glyph(Glyph::Saw), "Saw"),
    (ToggleFace::Glyph(Glyph::Square), "Square"),
    (ToggleFace::Glyph(Glyph::SampleHold), "Sample and hold"),
];

fn mini_workspace(mut layout: Layout, page: usize) -> Workspace {
    use Slot::{Control as C, Faders};
    use mini::*;
    layout.knob = daw_ui::KnobSize::Dense;
    layout.hero = None;
    layout.fader_min_height = Some(56.0);
    layout.sections = match page {
        0 => vec![
            Section {
                choice: Some(OSC1_MODEL),
                ..cells(
                    "Oscillator 1",
                    0,
                    8,
                    &[&[C(OSC1_WAVE, 5), C(OSC1_WIDTH, 3)], &[C(OSC1_OCTAVE, 8)]],
                )
            },
            Section {
                choice: Some(OSC2_MODEL),
                ..cells(
                    "Oscillator 2",
                    0,
                    14,
                    &[
                        &[C(OSC2_WAVE, 6), C(OSC2_WIDTH, 4), C(OSC2_SYNC, 4)],
                        &[C(OSC2_OCTAVE, 6), C(OSC2_SEMITONES, 4), C(OSC2_DETUNE, 4)],
                    ],
                )
            },
            cells(
                "Source mix",
                0,
                6,
                &[&[Faders(&[MIX_OSC1, MIX_OSC2, MIX_SUB, MIX_NOISE], 6)]],
            ),
        ],
        1 => vec![
            cells(
                "Filter",
                0,
                9,
                &[
                    &[C(FILTER_MODEL, 5), C(DRIVE, 2), C(CUTOFF, 2)],
                    &[C(RESONANCE, 3), C(FILTER_ENV, 3), C(KEY_TRACKING, 3)],
                ],
            ),
            cells(
                "Filter envelope",
                0,
                8,
                &[&[Faders(
                    &[
                        FENV_ATTACK,
                        FENV_DECAY,
                        FENV_SUSTAIN,
                        FENV_RELEASE,
                        VELOCITY_CUTOFF,
                    ],
                    8,
                )]],
            ),
            cells(
                "Amp envelope",
                0,
                8,
                &[&[Faders(
                    &[
                        AENV_ATTACK,
                        AENV_DECAY,
                        AENV_SUSTAIN,
                        AENV_RELEASE,
                        VELOCITY_AMP,
                    ],
                    8,
                )]],
            ),
        ],
        2 => vec![
            cells(
                "LFO",
                0,
                6,
                &[&[C(LFO_SHAPE, 3), C(LFO_SYNC, 2), C(LFO_RATE, 1)]],
            ),
            cells(
                "Modulation depth",
                0,
                5,
                &[&[
                    C(LFO_PITCH, 1),
                    C(LFO_CUTOFF, 1),
                    C(LFO_WIDTH, 1),
                    C(LFO_WHEEL, 2),
                ]],
            ),
        ],
        _ => vec![
            cells(
                "Voice allocation",
                0,
                5,
                &[&[C(VOICE_MODE, 3), C(VOICES, 2)]],
            ),
            cells(
                "Pitch response",
                0,
                5,
                &[&[C(GLIDE, 2), C(BEND_RANGE, 2), C(DRIFT, 1)]],
            ),
        ],
    };
    layout.toggle_columns = &[];
    layout.faces = &[
        Faces {
            ids: &[OSC1_WAVE, OSC2_WAVE],
            faces: &[
                (ToggleFace::Glyph(Glyph::Saw), "Saw"),
                (ToggleFace::Glyph(Glyph::Square), "Pulse"),
                (ToggleFace::Glyph(Glyph::Triangle), "Triangle"),
                (ToggleFace::Glyph(Glyph::Sine), "Sine"),
            ],
            segment_width: 28.0,
        },
        Faces {
            ids: &[LFO_SHAPE],
            faces: LFO_WAVES,
            segment_width: 30.0,
        },
    ];
    layout.captions = &[
        (OSC2_SEMITONES, "Coarse"),
        (VELOCITY_CUTOFF, "Velocity"),
        (VELOCITY_AMP, "Velocity"),
        (LFO_WIDTH, "Pulse width"),
    ];
    layout.hint = |id, values| {
        let settings = mini::Settings::from_normalized(|id| values(id) as f32);
        if !mini::applies(id, &settings) {
            return Some(match id {
                OSC1_WIDTH | OSC2_WIDTH => "Choose Pulse to adjust its width.",
                LFO_WIDTH => "Choose Pulse on either oscillator to modulate pulse width.",
                LFO_RATE => "Sync sets the LFO rate to the song tempo.",
                VOICES => "Voice count applies in Poly mode.",
                _ => return None,
            });
        }
        None
    };
    Workspace {
        layout,
        output: vec![VOLUME],
        toolbar: vec![],
        body_height: 257.0,
    }
}

fn sampler_workspace(mut layout: Layout, page: usize) -> Workspace {
    use Slot::{Control as C, Faders};
    use sampler::*;
    layout.knob = daw_ui::KnobSize::Dense;
    layout.hero = None;
    layout.fader_min_height = Some(56.0);
    layout.picture = Some(Picture::Sample { height: 84.0 });
    layout.picture_place = PicturePlace::Above;
    layout.sections = match page {
        0 => vec![
            cells(
                "Region",
                0,
                4,
                &[&[C(START, 2), C(END, 2)], &[C(REVERSE, 2), C(GAIN, 2)]],
            ),
            cells(
                "Loop",
                0,
                6,
                &[
                    &[C(LOOP_MODE, 6)],
                    &[C(LOOP_START, 2), C(LOOP_END, 2), C(CROSSFADE, 2)],
                ],
            ),
        ],
        1 => vec![
            cells(
                "Pitch",
                0,
                4,
                &[
                    &[C(KEY_TRACKING, 2), C(ROOT, 2)],
                    &[C(TRANSPOSE, 2), C(FINE, 2)],
                ],
            ),
            cells(
                "Voices",
                0,
                6,
                &[
                    &[C(VOICE_MODE, 6)],
                    &[C(VOICES, 2), C(GLIDE, 2), C(BEND_RANGE, 2)],
                ],
            ),
        ],
        2 => vec![
            Section {
                enable: Some(FILTER_ON),
                ..cells(
                    "Filter",
                    0,
                    8,
                    &[
                        &[C(FILTER_TYPE, 5), C(FILTER_SLOPE, 3)],
                        &[
                            C(CUTOFF, 2),
                            C(RESONANCE, 2),
                            C(FILTER_ENV, 2),
                            C(FILTER_KEY_TRACKING, 2),
                        ],
                    ],
                )
            },
            cells(
                "Filter envelope",
                0,
                7,
                &[&[Faders(
                    &[
                        FENV_ATTACK,
                        FENV_DECAY,
                        FENV_SUSTAIN,
                        FENV_RELEASE,
                        VELOCITY_CUTOFF,
                    ],
                    7,
                )]],
            ),
            cells(
                "Amp envelope",
                0,
                8,
                &[&[Faders(
                    &[
                        AENV_ATTACK,
                        AENV_HOLD,
                        AENV_DECAY,
                        AENV_SUSTAIN,
                        AENV_RELEASE,
                        VELOCITY_LEVEL,
                    ],
                    8,
                )]],
            ),
        ],
        _ => vec![
            cells(
                "LFO",
                0,
                6,
                &[&[C(LFO_SHAPE, 3), C(LFO_SYNC, 2), C(LFO_RATE, 1)]],
            ),
            cells(
                "Modulation depth",
                0,
                5,
                &[&[
                    C(LFO_PITCH, 1),
                    C(LFO_CUTOFF, 1),
                    C(LFO_LEVEL, 1),
                    C(LFO_PAN, 2),
                ]],
            ),
        ],
    };
    layout.captions = &[
        (VELOCITY_CUTOFF, "Velocity"),
        (VELOCITY_LEVEL, "Velocity"),
        (FILTER_KEY_TRACKING, "Key track"),
        (LOOP_MODE, "Mode"),
        (LOOP_START, "Start"),
        (LOOP_END, "End"),
        (LFO_SHAPE, "Shape"),
        (LFO_RATE, "Rate"),
        (LFO_SYNC, "Sync"),
        (LFO_PITCH, "Pitch"),
        (LFO_CUTOFF, "Cutoff"),
        (LFO_LEVEL, "Level"),
        (LFO_PAN, "Pan"),
    ];
    layout.toggle_columns = &[];
    layout.faces = &[
        Faces {
            ids: &[FILTER_TYPE],
            faces: &[
                (ToggleFace::Glyph(Glyph::LowPass), "Low-pass"),
                (ToggleFace::Glyph(Glyph::HighPass), "High-pass"),
                (ToggleFace::Glyph(Glyph::BandPass), "Band-pass"),
            ],
            segment_width: 34.0,
        },
        Faces {
            ids: &[FILTER_SLOPE],
            faces: &[
                (ToggleFace::Text("12"), "12 dB/oct"),
                (ToggleFace::Text("24"), "24 dB/oct"),
            ],
            segment_width: 28.0,
        },
        Faces {
            ids: &[LFO_SHAPE],
            faces: LFO_WAVES,
            segment_width: 30.0,
        },
    ];
    layout.hint = |id, values| {
        let settings = sampler::Settings::from_normalized(|id| values(id) as f32);
        if !sampler::applies(id, &settings) {
            return Some(match id {
                ROOT => "Enable Key tracking to set the sample's original note.",
                LOOP_START | LOOP_END => "Choose Forward or Ping-pong to set a loop region.",
                CROSSFADE => "Crossfade applies to Forward loops.",
                FILTER_TYPE | FILTER_SLOPE | CUTOFF | RESONANCE | FILTER_ENV
                | FILTER_KEY_TRACKING | VELOCITY_CUTOFF | FENV_ATTACK | FENV_DECAY
                | FENV_SUSTAIN | FENV_RELEASE | LFO_CUTOFF => {
                    "Enable Filter to shape the sample with these controls."
                }
                AENV_RELEASE => "One-shot plays to End; note release does not shorten it.",
                LFO_RATE => "Sync sets the LFO rate to the song tempo.",
                VOICES => "Voice count applies in Poly mode.",
                _ => return None,
            });
        }
        (id == LOOP_MODE).then_some("Off follows note release without looping. One-shot plays to End. Forward and Ping-pong repeat the loop region.")
    };
    Workspace {
        layout,
        output: vec![PAN, VOLUME],
        toolbar: vec![],
        body_height: 392.0,
    }
}

/// Captions where the card names the rest: the Velocity card reads
/// "Velocity: Level, Pitch, Tone", the Pitch card "Env" for its envelope.
static PAD_CAPTIONS: [(u32, &str); 1 + PER_PAD * drums::PADS] = {
    let mut out = [(0, ""); 1 + PER_PAD * drums::PADS];
    out[0] = (drums::VOLUME, "Kit volume");
    let mut pad = 0;
    while pad < drums::PADS {
        let captions = [
            (drums::LEVEL, "Pad volume"),
            (drums::PITCH_ENV, "Env"),
            (drums::ENV_DECAY, "Env decay"),
            (drums::RESONANCE, "Reso"),
            (drums::VELOCITY, "Level"),
            (drums::VELOCITY_PITCH, "Pitch"),
            (drums::VELOCITY_TONE, "Tone"),
        ];
        let mut index = 0;
        while index < PER_PAD {
            let (offset, caption) = captions[index];
            out[1 + pad * PER_PAD + index] = (drums::id(pad, offset), caption);
            index += 1;
        }
        pad += 1;
    }
    out
};
const PER_PAD: usize = 7;

fn drums_workspace(mut layout: Layout, pad: usize) -> Workspace {
    use Slot::{Control as C, Faders};
    use drums::*;
    let at = |offset| id(pad, offset);
    layout.knob = daw_ui::KnobSize::Dense;
    layout.hero = None;
    layout.fader_min_height = Some(43.0);
    layout.captions = &PAD_CAPTIONS;
    layout.toggle_columns = &[];
    layout.picture = Some(Picture::Drums { height: 304.0 });
    layout.picture_place = PicturePlace::Above;
    // Two rows of three cards on shared columns, in signal order: what
    // plays, then how it is shaped. Velocity sits beside the amplifier it
    // mostly drives.
    layout.sections = vec![
        cells("Trigger", 0, 10, &[&[C(at(TRIGGER), 7), C(at(CHOKE), 3)]]),
        cells(
            "Region",
            0,
            7,
            &[&[C(at(START), 2), C(at(END), 2), C(at(REVERSE), 3)]],
        ),
        cells(
            "Pitch",
            0,
            9,
            &[&[
                C(at(PITCH), 2),
                C(at(FINE), 2),
                C(at(PITCH_ENV), 2),
                C(at(ENV_DECAY), 3),
            ]],
        ),
        cells(
            "Amp envelope",
            1,
            10,
            &[&[Faders(&DRUM_AMP_FADERS[pad], 10)]],
        ),
        cells(
            "Velocity",
            1,
            7,
            &[&[
                C(at(VELOCITY), 3),
                C(at(VELOCITY_PITCH), 2),
                C(at(VELOCITY_TONE), 2),
            ]],
        ),
        cells(
            "Tone",
            1,
            9,
            &[&[
                C(at(BITS), 3),
                C(at(FILTER), 2),
                C(at(RESONANCE), 2),
                C(at(DRIVE), 2),
            ]],
        ),
    ];
    layout.hint = |id, values| {
        let settings = drums::Settings::from_normalized(|id| values(id) as f32);
        if !drums::applies(id, &settings) {
            return Some(match drums::pad_of(id).map(|(_, offset)| offset) {
                Some(ENV_DECAY) => "Set a pitch envelope Amount to hear its decay.",
                Some(DECAY) => "Shorten Hold from Full or choose Gate to use the decay stage.",
                Some(RESONANCE) => "Move Filter away from Off to adjust its resonance.",
                _ => return None,
            });
        }
        match drums::pad_of(id).map(|(_, offset)| offset) {
            Some(VELOCITY) => Some(
                "How much quieter soft hits are. 0 % plays every hit at full level.",
            ),
            Some(VELOCITY_PITCH) => Some(
                "Pitch shift at the softest velocity. Harder hits shift less, reaching the pad's tuned pitch at full velocity.",
            ),
            Some(VELOCITY_TONE) => Some(
                "Darkens soft hits, as a softly struck drum sounds. Full velocity stays bright.",
            ),
            Some(PUNCH) => Some("Boosts the first milliseconds of each hit for a sharper attack."),
            Some(CHOKE) => {
                Some("Pads in the same choke group stop each other's hits. Off lets hits overlap.")
            }
            Some(TRIGGER) => {
                Some("One-shot plays to End. Gate releases the envelope when the note ends.")
            }
            _ => None,
        }
        .or((id == HUMANIZE).then_some(
            "Varies each hit's pitch and level a little at random, so repeated hits sound played.",
        ))
    };
    Workspace {
        layout,
        output: vec![at(LEVEL), at(PAN), at(MUTE), HUMANIZE, VOLUME],
        toolbar: vec![],
        body_height: 425.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_instrument_parameter_has_a_workspace_or_persistent_control() {
        for descriptor in [&mini::DESCRIPTOR, &sampler::DESCRIPTOR, &drums::DESCRIPTOR] {
            let pages = super::super::tabs(descriptor).len().max(1);
            let pads = if descriptor.id == drums::DESCRIPTOR.id {
                drums::PADS
            } else {
                1
            };
            let mut reachable = BTreeSet::new();
            for pad in 0..pads {
                for page in 0..pages {
                    let workspace = build(
                        crate::panels::layout_for(descriptor, pad),
                        descriptor,
                        pad,
                        page,
                    );
                    let mut on_page = BTreeSet::new();
                    for id in workspace
                        .layout
                        .sections
                        .iter()
                        .flat_map(Section::ids)
                        .chain(workspace.output)
                        .chain(workspace.toolbar)
                    {
                        assert!(
                            descriptor.parameter(id).is_some(),
                            "{}: unknown {id}",
                            descriptor.id
                        );
                        assert!(
                            on_page.insert(id),
                            "{}: duplicated control {id} on page {page}",
                            descriptor.id
                        );
                        reachable.insert(id);
                    }
                }
            }
            let expected: BTreeSet<_> = descriptor.parameters.iter().map(|spec| spec.id).collect();
            assert_eq!(
                reachable, expected,
                "{} has unreachable controls",
                descriptor.id
            );
        }
    }
}

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
        "r1-drums" => drums_workspace(layout, pad.min(drums::PADS - 1), page),
        _ => unreachable!("instrument workspace requires an R1 instrument"),
    }
}

fn mini_workspace(mut layout: Layout, page: usize) -> Workspace {
    use Slot::{Blank, Control as C, Faders};
    use mini::*;
    layout.knob = daw_ui::KnobSize::Dense;
    layout.hero = None;
    layout.fader_min_height = Some(56.0);
    layout.sections = if page == 0 {
        vec![
            cells(
                "Oscillators",
                0,
                10,
                &[
                    &[
                        C(OSC1_MODEL, 2),
                        C(OSC1_WAVE, 2),
                        C(OSC1_OCTAVE, 1),
                        C(OSC1_WIDTH, 1),
                        Blank(4),
                    ],
                    &[
                        C(OSC2_MODEL, 2),
                        C(OSC2_WAVE, 2),
                        C(OSC2_OCTAVE, 1),
                        C(OSC2_WIDTH, 1),
                        C(OSC2_SEMITONES, 1),
                        C(OSC2_DETUNE, 1),
                        C(OSC2_SYNC, 2),
                    ],
                ],
            ),
            cells(
                "Source mix",
                0,
                2,
                &[
                    &[C(MIX_OSC1, 1), C(MIX_SUB, 1)],
                    &[C(MIX_OSC2, 1), C(MIX_NOISE, 1)],
                ],
            ),
            cells(
                "Filter",
                1,
                7,
                &[&[
                    C(FILTER_MODEL, 2),
                    C(CUTOFF, 1),
                    C(RESONANCE, 1),
                    C(DRIVE, 1),
                    C(FILTER_ENV, 1),
                    C(KEY_TRACKING, 1),
                ]],
            ),
            cells(
                "Amp envelope",
                1,
                5,
                &[&[Faders(
                    &[AENV_ATTACK, AENV_DECAY, AENV_SUSTAIN, AENV_RELEASE],
                    5,
                )]],
            ),
        ]
    } else {
        vec![
            cells(
                "Filter envelope",
                0,
                4,
                &[&[Faders(
                    &[FENV_ATTACK, FENV_DECAY, FENV_SUSTAIN, FENV_RELEASE],
                    4,
                )]],
            ),
            cells(
                "Velocity",
                0,
                2,
                &[&[C(VELOCITY_CUTOFF, 1), C(VELOCITY_AMP, 1)]],
            ),
            cells(
                "Voices",
                0,
                6,
                &[&[
                    C(VOICE_MODE, 2),
                    C(VOICES, 1),
                    C(GLIDE, 1),
                    C(BEND_RANGE, 1),
                    C(DRIFT, 1),
                ]],
            ),
            cells(
                "LFO",
                1,
                12,
                &[&[
                    C(LFO_SHAPE, 3),
                    C(LFO_RATE, 1),
                    C(LFO_SYNC, 4),
                    C(LFO_PITCH, 1),
                    C(LFO_CUTOFF, 1),
                    C(LFO_WIDTH, 1),
                    C(LFO_WHEEL, 1),
                ]],
            ),
        ]
    };
    // Wave shapes retain their names on hover, while a single aligned row
    // leaves room for both oscillators without making the popup taller.
    layout.toggle_columns = &[];
    layout.faces = &[Faces {
        ids: &[OSC1_WAVE, OSC2_WAVE],
        faces: &[
            (ToggleFace::Glyph(Glyph::Saw), "Saw"),
            (ToggleFace::Glyph(Glyph::Square), "Pulse"),
            (ToggleFace::Glyph(Glyph::Triangle), "Triangle"),
            (ToggleFace::Glyph(Glyph::Sine), "Sine"),
        ],
        segment_width: 30.0,
    }];
    // In the velocity section these are two destinations, not two identically
    // named Velocity knobs. The descriptor names remain unchanged for automation.
    layout.captions = &[
        (OSC1_MODEL, "Osc 1"),
        (OSC2_MODEL, "Osc 2"),
        (VELOCITY_CUTOFF, "Filter"),
        (VELOCITY_AMP, "Amp"),
        (FENV_ATTACK, "Attack"),
        (FENV_DECAY, "Decay"),
        (FENV_SUSTAIN, "Sustain"),
        (FENV_RELEASE, "Release"),
        (AENV_ATTACK, "Attack"),
        (AENV_DECAY, "Decay"),
        (AENV_SUSTAIN, "Sustain"),
        (AENV_RELEASE, "Release"),
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
        body_height: 390.0,
    }
}

fn sampler_workspace(mut layout: Layout, page: usize) -> Workspace {
    use Slot::{Control as C, Faders};
    use sampler::*;
    layout.knob = daw_ui::KnobSize::Dense;
    layout.hero = None;
    layout.fader_min_height = Some(56.0);
    layout.picture = Some(Picture::Sample { height: 80.0 });
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
                "Playback",
                0,
                4,
                &[
                    &[C(LOOP_MODE, 4)],
                    &[C(LOOP_START, 1), C(LOOP_END, 1), C(CROSSFADE, 2)],
                ],
            ),
            cells(
                "Pitch",
                0,
                4,
                &[
                    &[C(ROOT, 2), C(KEY_TRACKING, 2)],
                    &[C(TRANSPOSE, 2), C(FINE, 2)],
                ],
            ),
        ],
        1 => {
            let mut filter = cells(
                "Filter",
                0,
                4,
                &[
                    &[C(FILTER_TYPE, 3), C(CUTOFF, 1)],
                    &[
                        C(FILTER_SLOPE, 1),
                        C(RESONANCE, 1),
                        C(FILTER_ENV, 1),
                        C(FILTER_KEY_TRACKING, 1),
                    ],
                ],
            );
            filter.enable = Some(FILTER_ON);
            vec![
                filter,
                cells(
                    "Filter envelope",
                    0,
                    3,
                    &[&[Faders(
                        &[
                            FENV_ATTACK,
                            FENV_DECAY,
                            FENV_SUSTAIN,
                            FENV_RELEASE,
                            VELOCITY_CUTOFF,
                        ],
                        3,
                    )]],
                ),
                cells(
                    "Amp envelope",
                    0,
                    5,
                    &[&[Faders(
                        &[
                            AENV_ATTACK,
                            AENV_HOLD,
                            AENV_DECAY,
                            AENV_SUSTAIN,
                            AENV_RELEASE,
                            VELOCITY_LEVEL,
                        ],
                        5,
                    )]],
                ),
            ]
        }
        _ => vec![
            cells(
                "LFO",
                0,
                8,
                &[
                    &[C(LFO_SHAPE, 3), C(LFO_RATE, 2), C(LFO_SYNC, 3)],
                    &[
                        C(LFO_PITCH, 2),
                        C(LFO_CUTOFF, 2),
                        C(LFO_LEVEL, 2),
                        C(LFO_PAN, 2),
                    ],
                ],
            ),
            cells(
                "Voices",
                0,
                4,
                &[
                    &[C(VOICE_MODE, 3), C(VOICES, 1)],
                    &[C(GLIDE, 2), C(BEND_RANGE, 2)],
                ],
            ),
        ],
    };
    layout.captions = &[
        (VELOCITY_CUTOFF, "Vel"),
        (VELOCITY_LEVEL, "Vel"),
        (FILTER_KEY_TRACKING, "Key track"),
        (KEY_TRACKING, "Key track"),
        (LOOP_MODE, "Mode"),
        (LOOP_START, "Start"),
        (LOOP_END, "End"),
        (FILTER_SLOPE, "Slope"),
        (FILTER_TYPE, "Type"),
        (VOICE_MODE, "Mode"),
        (LFO_SHAPE, "Shape"),
        (LFO_RATE, "Rate"),
        (LFO_SYNC, "Sync"),
        (LFO_PITCH, "Pitch"),
        (LFO_CUTOFF, "Cutoff"),
        (LFO_LEVEL, "Level"),
        (LFO_PAN, "Pan"),
        (FENV_ATTACK, "A"),
        (FENV_DECAY, "D"),
        (FENV_SUSTAIN, "S"),
        (FENV_RELEASE, "R"),
        (AENV_ATTACK, "A"),
        (AENV_HOLD, "H"),
        (AENV_DECAY, "D"),
        (AENV_SUSTAIN, "S"),
        (AENV_RELEASE, "R"),
    ];
    // A single line of named slope segments keeps the envelope workspace
    // compact without reducing the waveform or the editable fader travel.
    layout.toggle_columns = &[(LOOP_MODE, 2)];
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
        body_height: 390.5,
    }
}

static PAD_CAPTIONS: [(u32, &str); 1 + 5 * drums::PADS] = {
    let mut out = [(0, ""); 1 + 5 * drums::PADS];
    out[0] = (drums::VOLUME, "Kit volume");
    let mut pad = 0;
    while pad < drums::PADS {
        out[1 + pad * 5] = (drums::id(pad, drums::LEVEL), "Pad volume");
        out[2 + pad * 5] = (drums::id(pad, drums::PITCH_ENV), "Amount");
        out[3 + pad * 5] = (drums::id(pad, drums::ENV_DECAY), "Decay");
        out[4 + pad * 5] = (drums::id(pad, drums::RESONANCE), "Reso");
        out[5 + pad * 5] = (drums::id(pad, drums::VELOCITY), "Vel");
        pad += 1;
    }
    out
};

fn drums_workspace(mut layout: Layout, pad: usize, page: usize) -> Workspace {
    use Slot::{Control as C, Faders};
    use drums::*;
    let at = |offset| id(pad, offset);
    layout.knob = daw_ui::KnobSize::Dense;
    layout.hero = None;
    layout.fader_min_height = Some(48.0);
    layout.captions = &PAD_CAPTIONS;
    static TRIGGER_COLUMNS: [(u32, usize); drums::PADS] = {
        let mut columns = [(0, 1); drums::PADS];
        let mut pad = 0;
        while pad < drums::PADS {
            columns[pad] = (drums::id(pad, drums::TRIGGER), 1);
            pad += 1;
        }
        columns
    };
    layout.toggle_columns = &TRIGGER_COLUMNS;
    layout.picture = Some(Picture::Drums { height: 304.0 });
    layout.picture_place = PicturePlace::Above;
    layout.sections = if page == 0 {
        vec![
            cells(
                "Region",
                0,
                7,
                &[&[C(at(START), 2), C(at(END), 2), C(at(REVERSE), 3)]],
            ),
            cells(
                "Tuning",
                0,
                10,
                &[&[C(at(PITCH), 2), C(at(FINE), 3), C(at(VELOCITY_PITCH), 5)]],
            ),
            cells("Trigger", 0, 7, &[&[C(at(TRIGGER), 4), C(at(CHOKE), 3)]]),
        ]
    } else {
        vec![
            cells(
                "Pitch env",
                0,
                6,
                &[&[C(at(PITCH_ENV), 3), C(at(ENV_DECAY), 3)]],
            ),
            cells("Amp envelope", 0, 9, &[&[Faders(&DRUM_AMP_FADERS[pad], 9)]]),
            cells(
                "Tone",
                0,
                10,
                &[&[
                    C(at(FILTER), 3),
                    C(at(RESONANCE), 2),
                    C(at(DRIVE), 2),
                    C(at(BITS), 3),
                ]],
            ),
        ]
    };
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
            Some(VELOCITY_PITCH) => Some(
                "Pitch shift at the softest velocity. Harder hits shift less, reaching the pad's tuned pitch at full velocity.",
            ),
            Some(CHOKE) => {
                Some("Pads in the same choke group stop each other's hits. Off lets hits overlap.")
            }
            Some(TRIGGER) => {
                Some("One-shot plays to End. Gate releases the envelope when the note ends.")
            }
            _ => None,
        }
    };
    Workspace {
        layout,
        output: vec![at(LEVEL), at(PAN), at(MUTE), VOLUME],
        toolbar: vec![],
        body_height: 390.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_instrument_parameter_has_a_workspace_or_persistent_control() {
        for descriptor in [&mini::DESCRIPTOR, &sampler::DESCRIPTOR, &drums::DESCRIPTOR] {
            let pages = super::super::tabs(descriptor).len();
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

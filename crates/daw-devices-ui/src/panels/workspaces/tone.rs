use super::*;

pub(super) fn build(
    mut layout: Layout,
    descriptor: &DeviceDescriptor,
    page: usize,
    band: usize,
) -> Workspace {
    use Slot::Control as C;
    layout.cells = true;
    layout.picture_place = PicturePlace::Above;
    if descriptor.id == filter::DESCRIPTOR.id {
        use filter::*;
        layout.sections = if page == 1 {
            vec![
                cells(
                    "LFO",
                    0,
                    8,
                    &[&[
                        C(LFO_SHAPE, 2),
                        C(LFO_RATE, 1),
                        C(LFO_SYNC, 3),
                        C(LFO_DEPTH, 1),
                        C(LFO_STEREO, 1),
                    ]],
                ),
                cells(
                    "Envelope follower",
                    0,
                    4,
                    &[&[C(ENV_AMOUNT, 2), C(ENV_ATTACK, 1), C(ENV_RELEASE, 1)]],
                ),
            ]
        } else {
            vec![
                cells(
                    "Sound",
                    0,
                    5,
                    &[&[C(CUTOFF, 2), C(RESONANCE, 1), C(DRIVE, 1), C(MORPH, 1)]],
                ),
                cells("Circuit", 0, 7, &[&[C(MODEL, 2), C(TYPE, 4), C(SLOPE, 1)]]),
            ]
        };
        if let Some(Picture::Response(view)) = &mut layout.picture {
            view.height = 176.0;
            view.nodes = &[NodeView {
                label: Some("Cutoff"),
                frequency: CUTOFF,
                gain: None,
                wheel: None,
                enable: None,
                menu: None,
            }];
            view.readout = Some(|_, values| {
                DESCRIPTOR
                    .parameter(CUTOFF)
                    .unwrap()
                    .format(values(CUTOFF) as f32)
            });
        }
        layout.hint = |id, values| {
            if applies(id, &filter_settings(values)) {
                return None;
            }
            Some(match id {
                LFO_RATE => "Choose Off for Sync to set a free-running rate.",
                MORPH => "Morph is available for State variable, Comb and Vowel models.",
                TYPE => "This model has a fixed filter type.",
                SLOPE => "This model has a fixed filter slope.",
                DRIVE => "This model does not apply drive.",
                _ => "This control is inactive for the selected model.",
            })
        };
        Workspace {
            layout,
            output: vec![MIX, OUTPUT],
            toolbar: vec![],
            body_height: 360.0,
        }
    } else {
        use equalizer::*;
        let band = band.min(5);
        let mut inspector = match band {
            0 => cells(
                "High-pass",
                0,
                12,
                &[&[C(HP_FREQUENCY, 3), C(HP_SLOPE, 5), Slot::Blank(4)]],
            ),
            5 => cells(
                "Low-pass",
                0,
                12,
                &[&[C(LP_FREQUENCY, 3), C(LP_SLOPE, 5), Slot::Blank(4)]],
            ),
            _ => {
                let base = BANDS[band - 1];
                cells(
                    ["Band 1", "Band 2", "Band 3", "Band 4"][band - 1],
                    0,
                    12,
                    &[&[
                        C(base + BAND_FREQUENCY, 2),
                        C(base + BAND_GAIN, 2),
                        C(base + BAND_Q, 2),
                        C(base + BAND_TYPE, 6),
                    ]],
                )
            }
        };
        inspector.enable = Some(match band {
            0 => HP_ON,
            5 => LP_ON,
            _ => BANDS[band - 1] + BAND_ON,
        });
        layout.sections = vec![inspector];
        layout.hero = Some(match band {
            0 => HP_FREQUENCY,
            5 => LP_FREQUENCY,
            _ => BANDS[band - 1] + BAND_FREQUENCY,
        });
        layout.faces = &[];
        layout.leds = &[];
        layout.captions = &[
            (HP_FREQUENCY, "Frequency"),
            (LP_FREQUENCY, "Frequency"),
            (HP_SLOPE, "Slope"),
            (LP_SLOPE, "Slope"),
            (BANDS[0] + BAND_FREQUENCY, "Frequency"),
            (BANDS[1] + BAND_FREQUENCY, "Frequency"),
            (BANDS[2] + BAND_FREQUENCY, "Frequency"),
            (BANDS[3] + BAND_FREQUENCY, "Frequency"),
        ];
        if let Some(Picture::Response(view)) = &mut layout.picture {
            view.height = 190.0;
        }
        layout.hint = |id, values| {
            (!applies(id, &equalizer_settings(values)))
                .then_some("A Notch removes this frequency; Gain does not apply.")
        };
        Workspace {
            layout,
            output: vec![AUTO_GAIN, OUTPUT],
            toolbar: vec![MODE, ANALYZER],
            body_height: 450.0,
        }
    }
}

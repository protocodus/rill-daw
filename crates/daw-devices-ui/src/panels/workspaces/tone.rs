use super::*;

pub(super) fn build(mut layout: Layout, descriptor: &DeviceDescriptor, band: usize) -> Workspace {
    use Slot::Control as C;
    layout.cells = true;
    layout.picture_place = PicturePlace::Above;
    if descriptor.id == filter::DESCRIPTOR.id {
        use filter::*;
        layout.toggle_columns = &[(TYPE, 2), (SLOPE, 1), (ENV_SIDECHAIN, 1)];
        // Nine cells a row, so every caption fits beside its neighbours in a
        // narrow window.
        layout.sections = vec![
            // A model has a Type or a Morph, never both: Morph is the type of
            // the morphing models, so it takes the same place.
            cells(
                "Circuit",
                0,
                5,
                &[&[C(MODEL, 2), Slot::Swap(&[TYPE, MORPH], 2), C(SLOPE, 1)]],
            ),
            cells(
                "Filter",
                0,
                4,
                &[&[C(DRIVE, 1), C(CUTOFF, 1), C(RESONANCE, 1), C(SPREAD, 1)]],
            ),
            // Both sources move the same cutoff. Keep the source timings and
            // modulation amounts accessible while adjusting the filter.
            cells(
                "LFO",
                1,
                5,
                &[&[
                    C(LFO_SHAPE, 1),
                    C(LFO_SYNC, 1),
                    C(LFO_RATE, 1),
                    C(LFO_DEPTH, 1),
                    C(LFO_STEREO, 1),
                ]],
            ),
            // What the follower hears, how fast, then how far it moves the
            // cutoff.
            cells(
                "Envelope follower",
                1,
                4,
                &[&[
                    C(ENV_SIDECHAIN, 1),
                    C(ENV_ATTACK, 1),
                    C(ENV_RELEASE, 1),
                    C(ENV_AMOUNT, 1),
                ]],
            ),
        ];
        if let Some(Picture::Response(view)) = &mut layout.picture {
            view.height = 152.0;
            view.nodes = &[NodeView {
                label: Some("Cutoff"),
                frequency: CUTOFF,
                gain: None,
                wheel: Some(RESONANCE),
                enable: None,
                menu: None,
            }];
            view.readout = Some(|_, values| {
                let text = |id: u32| {
                    DESCRIPTOR
                        .parameter(id)
                        .map(|spec| spec.format(values(id) as f32))
                        .unwrap_or_default()
                };
                format!("{}   Res {}", text(CUTOFF), text(RESONANCE))
            });
        }
        layout.hint = |id, values| {
            if applies(id, &filter_settings(values)) {
                return None;
            }
            Some(match id {
                LFO_RATE => "Choose Off for Sync to set a free-running rate.",
                TYPE => "This model has a fixed filter type.",
                SLOPE => "This model has a fixed filter slope.",
                _ => "This control is inactive for the selected model.",
            })
        };
        Workspace {
            layout,
            output: vec![MIX, OUTPUT],
            toolbar: vec![],
            body_height: 0.0,
        }
    } else {
        use equalizer::*;
        let band = band.min(5);
        // Shape or slope first, then the same frequency dial for every band.
        // Cut filters share the other bands' geometry when gain and Q disappear.
        let mut inspector = match band {
            0 => cells(
                "High-pass",
                0,
                9,
                &[&[C(HP_SLOPE, 3), C(HP_FREQUENCY, 2), Slot::Blank(4)]],
            ),
            5 => cells(
                "Low-pass",
                0,
                9,
                &[&[C(LP_SLOPE, 3), C(LP_FREQUENCY, 2), Slot::Blank(4)]],
            ),
            _ => {
                let base = BANDS[band - 1];
                cells(
                    ["Band 1", "Band 2", "Band 3", "Band 4"][band - 1],
                    0,
                    9,
                    &[&[
                        C(base + BAND_TYPE, 3),
                        C(base + BAND_FREQUENCY, 2),
                        C(base + BAND_GAIN, 2),
                        C(base + BAND_Q, 2),
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
        if let Some(Picture::Response(view)) = &mut layout.picture {
            view.height = 212.0;
        }
        layout.hint = |id, values| {
            (!applies(id, &equalizer_settings(values)))
                .then_some("A Notch removes this frequency; Gain does not apply.")
        };
        Workspace {
            layout,
            output: vec![AUTO_GAIN, OUTPUT],
            toolbar: vec![MODE, ANALYZER],
            body_height: 0.0,
        }
    }
}

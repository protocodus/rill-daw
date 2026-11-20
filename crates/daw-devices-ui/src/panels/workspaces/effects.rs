//! Task layouts for dynamics, color, modulation and space. The base layouts
//! keep the parameter behavior; these workspaces own the information hierarchy.
use super::*;

pub(super) fn build(layout: Layout, descriptor: &DeviceDescriptor, page: usize) -> Workspace {
    match descriptor.id {
        "r1-saturator" => saturation(layout),
        "r1-compressor" => dynamics(layout),
        "r1-modulator" => modulation(layout),
        "r1-spacer" => space(layout, page),
        _ => unreachable!("effect workspace"),
    }
}

fn saturation(mut layout: Layout) -> Workspace {
    use Slot::Control as C;
    use saturator::*;
    layout.sections = vec![
        cells("Input", 0, 3, &[&[C(FOCUS, 1), C(DRIVE, 2)]]),
        cells(
            "Waveshaper",
            0,
            6,
            &[&[C(MODEL, 3), C(CHARACTER, 1), C(QUALITY, 2)]],
        ),
        cells("Wet signal", 0, 3, &[&[C(AUTO_GAIN, 2), C(TONE, 1)]]),
    ];
    layout.captions = &[(FOCUS, "Low bypass")];
    layout.hint = |id, _| match id {
        FOCUS => Some("Frequencies below this stay clean. Off processes the full signal."),
        QUALITY => Some("Higher oversampling reduces aliasing and uses more CPU."),
        AUTO_GAIN => {
            Some("Compensate for the level added by Drive; the readout shows the applied gain.")
        }
        _ => None,
    };
    layout.picture = Some(Picture::Saturation { height: 168.0 });
    layout.picture_place = PicturePlace::Above;
    Workspace {
        layout,
        output: vec![MIX, OUTPUT],
        toolbar: vec![],
        body_height: 0.0,
    }
}

fn dynamics(mut layout: Layout) -> Workspace {
    use Slot::Control as C;
    use compressor::*;
    layout.knob = daw_ui::KnobSize::Standard;
    layout.sections = vec![
        cells(
            "Sidechain",
            0,
            5,
            &[&[
                C(SIDECHAIN, 2),
                C(SIDECHAIN_HIGH_PASS, 1),
                C(STEREO_LINK, 1),
                C(LOOKAHEAD, 1),
            ]],
        ),
        cells(
            "Dynamics",
            0,
            4,
            &[&[C(THRESHOLD, 1), C(RATIO, 1), C(KNEE, 1), C(CEILING, 1)]],
        ),
        cells(
            "Timing",
            0,
            3,
            &[&[C(ATTACK, 1), C(RELEASE, 1), C(AUTO_RELEASE, 1)]],
        ),
    ];
    layout.toggle_columns = &[];
    layout.captions = &[
        (SIDECHAIN, "Source"),
        (SIDECHAIN_HIGH_PASS, "High-pass"),
        (STEREO_LINK, "Stereo link"),
        (AUTO_RELEASE, "Auto"),
    ];
    layout.hint = |id, values| {
        let settings = compressor_settings(values);
        if !applies(id, &settings) {
            return Some(match id {
                CHARACTER | AUTO_MAKEUP => "Available in Compressor mode.",
                THRESHOLD | RATIO | ATTACK => {
                    "Limiter uses its ceiling and automatic attack instead."
                }
                CEILING => "Available in Limiter mode.",
                RELEASE => "Auto release sets the release time. Turn it off to set a manual time.",
                AUTO_RELEASE => "Expander uses the manual Release time.",
                LOOKAHEAD => "Lookahead is available with Digital character, Limiter or Expander.",
                SIDECHAIN | SIDECHAIN_HIGH_PASS => "Limiter detects the main input directly.",
                _ => "Not used in the current mode.",
            });
        }
        match id {
            SIDECHAIN => {
                Some("External uses routed audio. Click the source below to edit routing.")
            }
            STEREO_LINK => {
                Some("Link the left and right detectors to keep the stereo image stable.")
            }
            LOOKAHEAD => Some("Listen ahead to catch transients; adds latency."),
            AUTO_MAKEUP => Some("Compensate for compression; the readout shows the applied gain."),
            _ => None,
        }
    };
    layout.picture = Some(Picture::Dynamics { height: 168.0 });
    layout.picture_place = PicturePlace::Above;
    Workspace {
        layout,
        output: vec![MAKEUP, AUTO_MAKEUP, MIX, OUTPUT],
        toolbar: vec![MODE, CHARACTER],
        body_height: 0.0,
    }
}

fn modulation(mut layout: Layout) -> Workspace {
    use Slot::{Control as C, Swap};
    use modulator::*;
    layout.knob = daw_ui::KnobSize::Standard;
    // Feel stacks its three options in one cell, beside the Division it
    // changes; Envelope sits beside the Center it moves.
    layout.sections = vec![
        cells(
            "LFO",
            0,
            7,
            &[&[
                C(SHAPE, 2),
                C(SYNC, 1),
                Swap(&[RATE, DIVISION], 1),
                C(FEEL, 1),
                C(DEPTH, 1),
                C(STEREO, 1),
            ]],
        ),
        cells(
            "Wet signal",
            0,
            5,
            &[&[
                Swap(&[CENTER_CHORUS, CENTER_FLANGER, CENTER_PHASER], 1),
                C(ENVELOPE, 1),
                Swap(&[VOICES, STAGES], 1),
                C(FEEDBACK, 1),
                C(LOW_CUT, 1),
            ]],
        ),
    ];
    layout.toggle_columns = &[(FEEL, 1)];
    layout.hint = |id, values| match id {
        VOICES if !applies(VOICES, &modulator_settings(values)) => {
            Some("Voices is available in Chorus mode; Flanger uses one delay.")
        }
        FEEL if !applies(FEEL, &modulator_settings(values)) => {
            Some("Feel applies while Sync follows the song tempo.")
        }
        SYNC => Some("Follow the song tempo with Division, or set a free-running Rate."),
        FEEL => Some("Stretch the division to a dotted length or squeeze it into a triplet."),
        ENVELOPE => {
            Some("Playing louder moves the sweep up in pitch; negative values move it down.")
        }
        DEPTH => Some("How far the modulation moves from the center."),
        LOW_CUT => {
            Some("Remove low frequencies from the effected signal; Off leaves it full range.")
        }
        _ => None,
    };
    layout.picture = Some(Picture::Modulation { height: 168.0 });
    layout.picture_place = PicturePlace::Above;
    Workspace {
        layout,
        output: vec![MIX, OUTPUT],
        toolbar: vec![MODE, CHARACTER],
        body_height: 0.0,
    }
}

fn space(mut layout: Layout, page: usize) -> Workspace {
    use Slot::{Control as C, Swap};
    use spacer::*;
    layout.knob = daw_ui::KnobSize::Standard;
    layout.sections = if page == 0 {
        vec![
            cells(
                "Repeats",
                0,
                5,
                &[
                    &[C(ECHO_MODEL, 3), C(ECHO_TAPS, 2)],
                    &[
                        C(ECHO_SYNC, 1),
                        Swap(&[ECHO_TIME, ECHO_DIVISION], 2),
                        C(ECHO_FEEDBACK, 2),
                    ],
                ],
            ),
            cells(
                "Delay line",
                0,
                4,
                &[
                    &[C(ECHO_DIFFUSION, 2), C(ECHO_WOBBLE, 2)],
                    &[C(ECHO_LOW_CUT, 2), C(ECHO_COLOR, 2)],
                ],
            ),
            cells(
                "Return",
                0,
                3,
                &[&[C(ECHO_STEREO, 3)], &[C(ECHO_DUCK, 1), C(ECHO_LEVEL, 2)]],
            ),
        ]
    } else {
        vec![
            cells(
                "Room",
                0,
                5,
                &[
                    &[C(REVERB_MODEL, 3), C(SEED, 2)],
                    &[C(PRE_DELAY, 1), C(SIZE, 2), C(DECAY, 2)],
                ],
            ),
            cells(
                "Reverb tail",
                0,
                4,
                &[
                    &[C(DIFFUSION, 2), C(MOVEMENT, 2)],
                    &[C(REVERB_LOW_CUT, 1), C(REVERB_COLOR, 1), C(DAMPING, 2)],
                ],
            ),
            cells(
                "Return",
                0,
                3,
                &[&[C(WIDTH, 3)], &[C(REVERB_DUCK, 1), C(REVERB_LEVEL, 2)]],
            ),
        ]
    };
    layout.picture = Some(Picture::Space { height: 150.0 });
    layout.picture_place = PicturePlace::Above;
    layout.hint = |id, values| {
        let settings = spacer_settings(values);
        if !applies(id, &settings) {
            return Some(match id {
                ROUTING => "Enable Echo and Reverb to choose how they connect.",
                ECHO_TIME | ECHO_DIVISION if !settings.echo_on => {
                    "Enable Echo to hear and edit these controls."
                }
                ECHO_TIME => "Turn off Sync to set a time in milliseconds.",
                ECHO_DIVISION => "Turn on Sync to choose a tempo division.",
                ECHO_MODEL..=ECHO_DUCK => "Enable Echo to hear and edit these controls.",
                REVERB_MODEL..=REVERB_DUCK => "Enable Reverb to hear and edit these controls.",
                _ => "Not used in the current mode.",
            });
        }
        Some(match (id, settings.echo.model) {
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
    Workspace {
        layout,
        output: vec![MIX, OUTPUT],
        toolbar: vec![ROUTING, ECHO_ON, REVERB_ON],
        body_height: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    struct ReadOnlyHost(&'static DeviceDescriptor);

    impl crate::DeviceHost for ReadOnlyHost {
        fn value(&self, id: u32) -> f64 {
            f64::from(self.0.parameter(id).unwrap().default_normalized())
        }
        fn set_value(&mut self, _: u32, _: f64) {
            panic!("navigation changed a parameter")
        }
        fn set_values(&mut self, _: &[(u32, f64)]) {
            panic!("navigation changed parameters")
        }
        fn automation(&self, _: u32) -> Option<(u64, daw_ui::AutomationState)> {
            None
        }
        fn navigate(&mut self, _: daw_ui::AutomationNavigation) {}
        fn user_presets(&self) -> Vec<daw_devices::Preset> {
            vec![]
        }
        fn save_preset(&mut self, _: daw_devices::Preset) {}
        fn audition(&mut self, _: Option<(u8, f32)>) {}
        fn wheels(&mut self, _: f32, _: f32) {}
    }

    #[test]
    fn every_effect_task_fits_and_keeps_output_in_place() {
        for descriptor in [
            &saturator::DESCRIPTOR,
            &compressor::DESCRIPTOR,
            &modulator::DESCRIPTOR,
            &spacer::DESCRIPTOR,
        ] {
            for theme in [egui::Theme::Light, egui::Theme::Dark] {
                let ctx = egui::Context::default();
                daw_ui::theme::apply(&ctx);
                ctx.set_theme(theme);
                let mut host = ReadOnlyHost(descriptor);
                let mut state = crate::PanelState::default();
                let mut previous_output = None;
                for page in 0..super::super::tabs(descriptor).len().max(1) {
                    state.page = page;
                    let mut used = egui::Rect::NOTHING;
                    for _ in 0..2 {
                        let _ = ctx.run(egui::RawInput::default(), |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                used = ui
                                    .scope(|ui| crate::show(ui, descriptor, &mut host, &mut state))
                                    .response
                                    .rect;
                            });
                        });
                    }
                    assert!(
                        used.height() <= 620.0,
                        "{} page {page}: height {}, body {:?}, picture {:?}, output {:?}, controls {:?}",
                        descriptor.id,
                        used.height(),
                        state.workspace_rect,
                        state.picture_rects,
                        state.output_rect,
                        state.control_rects
                    );
                    assert!(used.width() <= crate::panel_width(descriptor) + 1.0);
                    if let Some(previous) = previous_output {
                        assert_eq!(
                            state.output_rect,
                            Some(previous),
                            "{} output moved",
                            descriptor.id
                        );
                    }
                    previous_output = state.output_rect;
                }
            }
        }
    }

    #[test]
    fn every_effect_parameter_has_a_task_or_persistent_control() {
        for descriptor in [
            &saturator::DESCRIPTOR,
            &compressor::DESCRIPTOR,
            &modulator::DESCRIPTOR,
            &spacer::DESCRIPTOR,
        ] {
            let mut reachable = BTreeSet::new();
            for page in 0..super::super::tabs(descriptor).len().max(1) {
                let workspace = build(layout_for(descriptor, 0), descriptor, page);
                reachable.extend(workspace.layout.sections.iter().flat_map(Section::ids));
                reachable.extend(workspace.output);
                reachable.extend(workspace.toolbar);
            }
            let expected = descriptor.parameters.iter().map(|spec| spec.id).collect();
            assert_eq!(reachable, expected, "{} parameter access", descriptor.id);
        }
    }

    #[test]
    fn compressor_and_spacer_explain_their_inactive_controls() {
        for descriptor in [&compressor::DESCRIPTOR, &spacer::DESCRIPTOR] {
            for state in 0..4 {
                let values = |id| {
                    let spec = descriptor.parameter(id).unwrap();
                    let plain = match (descriptor.id, id) {
                        ("r1-compressor", compressor::MODE) => (state % 3) as f32,
                        ("r1-compressor", compressor::CHARACTER) => state as f32,
                        ("r1-compressor", compressor::AUTO_RELEASE) => 1.0,
                        ("r1-spacer", spacer::ECHO_ON) => (state & 1) as f32,
                        ("r1-spacer", spacer::REVERB_ON) => ((state >> 1) & 1) as f32,
                        _ => spec.default,
                    };
                    f64::from(spec.to_normalized(plain))
                };
                for page in 0..super::super::tabs(descriptor).len().max(1) {
                    let workspace = build(layout_for(descriptor, 0), descriptor, page);
                    let layout = &workspace.layout;
                    for id in layout
                        .sections
                        .iter()
                        .flat_map(Section::ids)
                        .chain(workspace.output)
                        .chain(workspace.toolbar)
                    {
                        if !(layout.visible)(id, &values) {
                            assert!(
                                (layout.hint)(id, &values).is_some(),
                                "{} parameter {id} needs an inactive explanation",
                                descriptor.id
                            );
                        }
                    }
                }
            }
        }
    }
}

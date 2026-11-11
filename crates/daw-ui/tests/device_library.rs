use daw_ui::{
    self as ui, DevicePanelWidth, EnvelopeShape, GenericDevice, KnobSize, ParamKind, ParamRow,
    ParameterEdits, PresetBrowserState, PresetEntry, PresetSource,
};
use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2, pos2, vec2};

fn run(ctx: &Context, events: Vec<Event>, mut add: impl FnMut(&mut egui::Ui)) -> Rect {
    let mut used = Rect::NOTHING;
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))),
            time: Some(ctx.cumulative_frame_nr() as f64 * 0.1),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                used = ui.vertical(|ui| add(ui)).response.rect;
            });
        },
    );
    used
}

fn button(at: Pos2, pressed: bool) -> Vec<Event> {
    vec![Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }]
}

/// Hover, then press and release in separate frames, as a real pointer does.
fn click(at: Pos2, times: usize, frame: &mut dyn FnMut(Vec<Event>)) {
    frame(vec![Event::PointerMoved(at)]);
    for _ in 0..times {
        frame(button(at, true));
        frame(button(at, false));
    }
    frame(vec![]);
}

fn themed() -> Context {
    let ctx = Context::default();
    ui::theme::apply(&ctx);
    ctx
}

#[test]
fn value_text_scales_precision_and_trims() {
    assert_eq!(ui::value_text(1.5, "dB"), "1.5 dB");
    assert_eq!(ui::value_text(12.34, ""), "12.3");
    assert_eq!(ui::value_text(1234.5, "Hz"), "1234 Hz");
    assert_eq!(ui::value_text(-0.001, ""), "0");
}

#[test]
fn log_grid_spans_audio_band() {
    assert!((ui::log_grid_frequency(0, 100) - 20.0).abs() < 1e-3);
    assert!((ui::log_grid_frequency(99, 100) - 20_000.0).abs() < 1.0);
    assert!(ui::log_grid_frequency(50, 100) < ui::log_grid_frequency(51, 100));
}

#[test]
fn knob_cells_have_fixed_sizes() {
    for size in [KnobSize::Dense, KnobSize::Standard, KnobSize::Hero] {
        let ctx = themed();
        let mut value = 0.5;
        let used = run(&ctx, vec![], |ui| {
            ui::labeled_knob("Cutoff", &mut value, 0.0..=1.0)
                .size(size)
                .show(ui);
        });
        assert_eq!(used.size(), vec2(size.cell_width(), size.cell_height()));
    }
    assert!(KnobSize::Dense.diameter() < KnobSize::Standard.diameter());
}

#[test]
fn double_click_resets_to_default() {
    let ctx = themed();
    let mut value = 0.9;
    let used = run(&ctx, vec![], |ui| {
        ui::labeled_knob("Mix", &mut value, 0.0..=1.0)
            .default_value(0.25)
            .show(ui);
    });
    let dial = pos2(used.center().x, used.top() + 18.0 + 24.0);
    click(dial, 2, &mut |events| {
        run(&ctx, events, |ui| {
            ui::labeled_knob("Mix", &mut value, 0.0..=1.0)
                .default_value(0.25)
                .show(ui);
        });
    });
    assert_eq!(value, 0.25);
}

#[test]
fn segmented_selects_exclusively() {
    let ctx = themed();
    let options = ["Digital", "Tube", "Opto", "FET"];
    let mut selected = 0;
    let used = run(&ctx, vec![], |ui| {
        ui::segmented(ui, &mut 0, &options);
    });
    let at = pos2(used.left() + used.width() * 0.3, used.center().y);
    click(at, 1, &mut |events| {
        run(&ctx, events, |ui| {
            ui::segmented(ui, &mut selected, &options);
        });
    });
    assert_ne!(selected, 0, "a click on another segment selects it");
}

#[test]
fn choice_is_toggle_buttons_below_five_options_and_a_drop_down_from_five() {
    let ctx = themed();
    let four = [(0, "Saw"), (1, "Pulse"), (2, "Triangle"), (3, "Sine")];
    let five = [(0, "A"), (1, "B"), (2, "C"), (3, "D"), (4, "E")];
    let mut selected = 0;
    let used = run(&ctx, vec![], |ui| {
        ui::choice(ui, "wave", &mut 0, &four);
    });
    // One click on a visible option picks it; nothing has to open first.
    let at = pos2(used.right() - 8.0, used.center().y);
    click(at, 1, &mut |events| {
        run(&ctx, events, |ui| {
            ui::choice(ui, "wave", &mut selected, &four);
        });
    });
    assert_eq!(selected, 3);
    let mut selected = 0;
    let used = run(&ctx, vec![], |ui| {
        ui::choice(ui, "letter", &mut 0, &five);
    });
    click(used.center(), 1, &mut |events| {
        run(&ctx, events, |ui| {
            ui::choice(ui, "letter", &mut selected, &five);
        });
    });
    assert_eq!(selected, 0, "a click on a drop-down only opens its list");
}

#[test]
fn toggle_grid_lays_four_options_out_two_by_two() {
    let ctx = themed();
    let four = [(0, "Saw"), (1, "Pulse"), (2, "Triangle"), (3, "Sine")];
    let mut selected = 0;
    let used = run(&ctx, vec![], |ui| {
        ui::toggle_grid(ui, &mut 0, &four, 2);
    });
    assert_eq!(used.height(), 2.0 * ui::TOGGLE_ROW_HEIGHT);
    click(used.right_bottom() - vec2(8.0, 8.0), 1, &mut |events| {
        run(&ctx, events, |ui| {
            ui::toggle_grid(ui, &mut selected, &four, 2);
        });
    });
    assert_eq!(selected, 3);
    click(used.left_bottom() + vec2(8.0, -8.0), 1, &mut |events| {
        run(&ctx, events, |ui| {
            ui::toggle_grid(ui, &mut selected, &four, 2);
        });
    });
    assert_eq!(selected, 2);
}

#[test]
fn fader_and_mini_knob_take_the_size_they_are_given() {
    let ctx = themed();
    let size = vec2(ui::FADER_CAP_WIDTH, 120.0);
    let mut value = 0.5;
    let used = run(&ctx, vec![], |ui| {
        ui::fader(ui, &mut value, "Attack", size, None);
    });
    assert_eq!(used.size(), size);
    assert_eq!(value, 0.5, "drawing a fader leaves its value alone");
    click(used.center(), 1, &mut |events| {
        run(&ctx, events, |ui| {
            ui::fader(ui, &mut value, "Attack", size, None);
        });
    });
    assert!(
        ui::parameter_owns_document_history(&ctx),
        "Undo and redo reach the song while a fader has focus"
    );
    let used = run(&ctx, vec![], |ui| {
        ui.add(ui::Knob::new(&mut value, 0.0..=1.0).mini());
    });
    assert_eq!(
        used.size(),
        vec2(ui::MINI_KNOB_DIAMETER, ui::MINI_KNOB_DIAMETER)
    );
}

#[test]
fn fader_release_applies_the_motion_since_the_last_held_frame() {
    // Height 111 less the 11-point cap: 100 points of travel.
    let size = vec2(ui::FADER_CAP_WIDTH, 111.0);
    for sampled in [8.0, 24.0] {
        let ctx = themed();
        let mut value = 0.5;
        let frame = |events: Vec<Event>, value: &mut f64| {
            run(&ctx, events, |ui| {
                ui::fader(ui, value, "Attack", size, None);
            })
        };
        let at = frame(vec![], &mut value).center();
        frame(vec![Event::PointerMoved(at)], &mut value);
        frame(button(at, true), &mut value);
        frame(
            vec![Event::PointerMoved(at - vec2(0.0, sampled))],
            &mut value,
        );
        frame(button(at - vec2(0.0, 30.0), false), &mut value);
        assert!((value - 0.8).abs() < 1e-6, "{sampled}: {value}");
        frame(vec![Event::PointerMoved(at - vec2(0.0, 50.0))], &mut value);
        assert!(
            (value - 0.8).abs() < 1e-6,
            "released motion is not replayed"
        );
    }
}

#[test]
fn toggle_led_flips() {
    let ctx = themed();
    let mut on = false;
    let used = run(&ctx, vec![], |ui| {
        ui::toggle_led(ui, &mut false, "Echo");
    });
    click(used.center(), 1, &mut |events| {
        run(&ctx, events, |ui| {
            ui::toggle_led(ui, &mut on, "Echo");
        });
    });
    assert!(on);
}

#[test]
fn pad_velocity_follows_click_height() {
    let pad = Rect::from_min_size(Pos2::ZERO, vec2(50.0, 50.0));
    let top = ui::pad_velocity(pad, 0.0);
    let bottom = ui::pad_velocity(pad, 50.0);
    assert!(top > bottom && top <= 1.0 && bottom >= 0.1);
}

#[test]
fn pad_press_reports_velocity_and_selection() {
    let ctx = themed();
    let pads = [ui::PadView::default(); 16];
    let size = vec2(233.0, 144.0);
    let used = run(&ctx, vec![], |ui| {
        ui::pad_grid(ui, &pads, 0, size);
    });
    // Bottom-left pad is index 0, top-left is 12.
    let at = pos2(used.left() + 10.0, used.bottom() - 10.0);
    let mut pressed = None;
    click(at, 1, &mut |events| {
        run(&ctx, events, |ui| {
            let out = ui::pad_grid(ui, &pads, 0, size);
            pressed = pressed.or(out.pressed);
        });
    });
    let (pad, velocity) = pressed.expect("pressed");
    assert_eq!(pad, 0);
    assert!(velocity < 0.5, "a press near the bottom is soft");
}

#[test]
fn envelope_shape_is_stable_and_editable() {
    let mut shape = EnvelopeShape::default();
    assert_eq!(ui::envelope_level(&shape, 0.0), 0.0);
    assert!((ui::envelope_level(&shape, 0.6) - shape.sustain).abs() < 1e-6);
    assert!(ui::envelope_level(&shape, 0.999) < 0.01);
    shape.apply(ui::EnvelopeEdit {
        param: ui::EnvelopeParam::Sustain,
        value: 0.2,
    });
    assert_eq!(shape.sustain, 0.2);
    for step in 0..=100 {
        let level = ui::envelope_level(&shape, step as f32 / 100.0);
        assert!((0.0..=1.0).contains(&level));
    }
    let ctx = themed();
    let output = {
        let mut edits = Vec::new();
        run(&ctx, vec![], |ui| {
            edits = ui::envelope_editor(ui, &shape, vec2(377.0, 144.0), &[]).edits
        });
        edits
    };
    assert!(output.is_empty(), "an idle editor requests no edits");
}

#[test]
fn preset_filters_combine() {
    let entries = [
        PresetEntry {
            name: "Warm Bass",
            category: "Bass",
            tags: &["warm"],
            description: "",
            factory: true,
            favorite: false,
        },
        PresetEntry {
            name: "My Lead",
            category: "Lead",
            tags: &["bright"],
            description: "Sharp",
            factory: false,
            favorite: true,
        },
    ];
    let mut state = PresetBrowserState::default();
    assert!(entries.iter().all(|entry| state.matches(entry)));
    state.search = "sharp".into();
    assert_eq!(
        entries.iter().filter(|entry| state.matches(entry)).count(),
        1
    );
    state.search.clear();
    state.source = PresetSource::Factory;
    assert!(state.matches(&entries[0]) && !state.matches(&entries[1]));
    state.source = PresetSource::All;
    state.favorites_only = true;
    state.category = Some("Lead".into());
    assert!(state.matches(&entries[1]) && !state.matches(&entries[0]));
    state.tag = Some("warm".into());
    assert!(!state.matches(&entries[1]));
}

#[derive(Default)]
struct Host {
    values: [f64; 4],
    log: Vec<String>,
}

impl ParameterEdits for Host {
    fn value(&self, id: u32) -> f64 {
        self.values[id as usize]
    }
    fn begin_edit(&mut self, id: u32) {
        self.log.push(format!("begin {id}"));
    }
    fn set_value(&mut self, id: u32, value: f64) {
        self.values[id as usize] = value;
        self.log.push(format!("set {id}"));
    }
    fn end_edit(&mut self, id: u32) {
        self.log.push(format!("end {id}"));
    }
}

fn rows() -> Vec<ParamRow<'static>> {
    let continuous = |id, name, group| ParamRow {
        id,
        name,
        group,
        unit: "",
        kind: ParamKind::Continuous {
            default: 0.5,
            bipolar: false,
        },
        format: None,
        parse: None,
    };
    vec![
        continuous(0, "Cutoff", "Filter"),
        continuous(1, "Resonance", "Filter"),
        ParamRow {
            id: 2,
            name: "Mode",
            group: "Filter",
            unit: "",
            kind: ParamKind::Choice(&["Low", "High"]),
            format: None,
            parse: None,
        },
        continuous(3, "Mix", "Output"),
    ]
}

#[test]
fn generic_panel_fits_declared_width_in_both_appearances() {
    let rows = rows();
    for width in [DevicePanelWidth::Medium, DevicePanelWidth::Large] {
        for appearance in [ui::theme::Appearance::Light, ui::theme::Appearance::Dark] {
            let ctx = themed();
            ui::theme::set_appearance(&ctx, appearance);
            let device = GenericDevice {
                name: "R1 Test",
                rows: &rows,
                width,
                mix: Some(3),
                output: None,
                presets: None,
                input_peaks: Some([0.1, 0.1]),
                output_peaks: Some([0.1, 0.1]),
            };
            let (mut host, mut bypassed) = (Host::default(), false);
            let used = run(&ctx, vec![], |ui| {
                ui::generic_device_panel(ui, &device, &mut host, &mut bypassed);
            });
            assert!(
                (used.width() - width.points()).abs() < 1.5,
                "panel is {} wide, declared {}",
                used.width(),
                width.points()
            );
            assert!(used.height() < 600.0);
        }
    }
}

#[test]
fn generic_panel_choice_is_one_complete_gesture() {
    let rows = rows();
    let ctx = themed();
    let device = GenericDevice {
        name: "R1 Test",
        rows: &rows,
        width: DevicePanelWidth::Medium,
        mix: Some(3),
        output: None,
        presets: None,
        input_peaks: None,
        output_peaks: None,
    };
    let mut host = Host::default();
    let mut bypassed = false;
    let panel = run(&ctx, vec![], |ui| {
        ui::generic_device_panel(ui, &device, &mut Host::default(), &mut false);
    });
    'scan: for y in (panel.top() as i32..panel.bottom() as i32).step_by(8) {
        for x in (panel.left() as i32..panel.left() as i32 + 400).step_by(12) {
            click(pos2(x as f32, y as f32), 1, &mut |events| {
                run(&ctx, events, |ui| {
                    ui::generic_device_panel(ui, &device, &mut host, &mut bypassed);
                });
            });
            if host.log.iter().any(|entry| entry == "set 2") {
                break 'scan;
            }
        }
    }
    let at = host
        .log
        .iter()
        .position(|entry| entry == "set 2")
        .expect("the choice was set");
    assert_eq!(host.log[at - 1], "begin 2");
    assert_eq!(host.log[at + 1], "end 2");
}

#[test]
fn wheel_clamps_and_springs_back() {
    let ctx = themed();
    let mut pitch = 0.0f32;
    let size = Vec2::new(34.0, 120.0);
    let frame = |ctx: &Context, pitch: &mut f32, events| {
        run(ctx, events, |ui| {
            ui::wheel(ui, pitch, true, "Pitch", size);
        })
    };
    let used = frame(&ctx, &mut pitch, vec![]);
    let start = used.center();
    frame(&ctx, &mut pitch, vec![Event::PointerMoved(start)]);
    frame(&ctx, &mut pitch, button(start, true));
    frame(
        &ctx,
        &mut pitch,
        vec![Event::PointerMoved(start - vec2(0.0, 10.0))],
    );
    frame(
        &ctx,
        &mut pitch,
        vec![Event::PointerMoved(start - vec2(0.0, 40.0))],
    );
    assert!(pitch > 0.0 && pitch <= 1.0, "dragging up bends up: {pitch}");
    frame(&ctx, &mut pitch, button(start, false));
    frame(&ctx, &mut pitch, vec![]);
    assert_eq!(pitch, 0.0, "the pitch wheel returns to center");
}

#[test]
fn every_visualization_renders_without_overflow() {
    let ctx = themed();
    let size = vec2(233.0, 120.0);
    let spectrum = vec![-40.0; 513];
    let curve = vec![0.0; 64];
    let peaks = vec![(-0.5, 0.5); 64];
    let used = run(&ctx, vec![], |ui| {
        let spec = ui::Spectrum {
            bins_db: &spectrum,
            sample_rate: 48_000.0,
        };
        ui::frequency_plot(
            ui,
            &ui::FrequencyPlot {
                curves: &[ui::FrequencyCurve {
                    db: &curve,
                    emphasis: true,
                }],
                spectrum: Some(spec),
                nodes: &[],
                range_db: 12.0,
                size,
            },
        );
        ui::spectrum(ui, spec, size);
        ui::transfer_plot(
            ui,
            &ui::TransferPlot {
                curve: &curve,
                x_range: (-60.0, 0.0),
                y_range: (-60.0, 0.0),
                input: Some(-12.0),
                size,
            },
        );
        ui::gain_reduction_history(ui, &[(0.5, 3.0); 32], size, 24.0);
        ui::reduction_meter(ui, 3.0, 6.0, 24.0, vec2(13.0, 120.0));
        ui::scope(ui, &[0.0, 0.5, -0.5, 0.2], true, size);
        ui::wavetable_view(
            ui,
            &[&[0.0, 1.0, 0.0][..], &[0.0, -1.0, 0.0][..]],
            0.5,
            size,
        );
        ui::lfo_view(ui, &[0.0, 1.0, 0.0, -1.0], Some(0.3), size);
        ui::sample_view(
            ui,
            &ui::SampleViewData {
                peaks: &peaks,
                start: 0.0,
                end: 1.0,
                loop_range: None,
                playheads: &[0.5],
            },
            size,
        );
        ui::delay_view(
            ui,
            &ui::DelayViewData {
                taps: &[(0.2, 0.8, 0.5)],
                decay_seconds: 1.0,
                pre_delay_seconds: 0.0,
                span_seconds: 1.0,
            },
            size,
        );
        ui::harmonics_view(ui, &[-6.0; 10], size);
        // Degenerate input must not panic.
        ui::scope(ui, &[], true, size);
        ui::transfer_plot(
            ui,
            &ui::TransferPlot {
                curve: &[],
                x_range: (-1.0, 1.0),
                y_range: (-1.0, 1.0),
                input: Some(f32::NAN),
                size,
            },
        );
    });
    assert!(
        used.width() <= size.x + 1.0,
        "each plot keeps its declared width"
    );
}

#[test]
fn frequency_node_nudges_with_arrow_keys() {
    let ctx = themed();
    let curve = vec![0.0; 64];
    let nodes = [ui::FrequencyNode {
        frequency: 1000.0,
        gain_db: 0.0,
        selected: true,
        color: None,
        gain_editable: true,
    }];
    let size = vec2(610.0, 170.0);
    let mut drags = Vec::new();
    let frame = |events: Vec<Event>, drags: &mut Vec<ui::NodeDrag>| {
        run(&ctx, events, |ui| {
            *drags = ui::frequency_plot(
                ui,
                &ui::FrequencyPlot {
                    curves: &[ui::FrequencyCurve {
                        db: &curve,
                        emphasis: true,
                    }],
                    spectrum: None,
                    nodes: &nodes,
                    range_db: 18.0,
                    size,
                },
            )
            .drags;
        })
    };
    let used = frame(vec![], &mut drags);
    let fraction = (1000f32 / 20.0).ln() / 1000f32.ln();
    let at = pos2(
        used.left() + 1.0 + (used.width() - 2.0) * fraction,
        used.center().y,
    );
    click(at, 1, &mut |events| {
        frame(events, &mut Vec::new());
    });
    frame(
        vec![Event::Key {
            key: egui::Key::ArrowUp,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        &mut drags,
    );
    assert_eq!(drags.len(), 1);
    assert!(drags[0].gain_db > 0.0);
}

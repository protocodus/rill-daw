use daw_ui::{self as ui, EnvelopeShape, KnobSize, ModSlot, PadView, Taper, tokens};

pub struct State {
    cutoff: f64,
    resonance: f64,
    pan: f64,
    drive: f64,
    mode: usize,
    wave: usize,
    on: bool,
    xy: (f64, f64),
    envelope: EnvelopeShape,
    selected_pad: usize,
    slot: ModSlot,
    pitch: f32,
    modulation: f32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            cutoff: 1200.0,
            resonance: 0.3,
            pan: 0.0,
            drive: 0.5,
            mode: 1,
            wave: 3,
            on: true,
            xy: (0.4, 0.6),
            envelope: EnvelopeShape::default(),
            selected_pad: 2,
            slot: ModSlot {
                source: Some(1),
                destination: Some(2),
                amount: 0.4,
            },
            pitch: 0.0,
            modulation: 0.3,
        }
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut State) {
    ui::text::note(
        ui,
        "Controls shared by every device panel. Knobs offer Reset, typing and modulation from the context menu.",
    );
    ui.add_space(tokens::SPACE_MD);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_LG;
        let sources = [(1, "LFO 1"), (2, "Env 2")];
        ui::labeled_knob("Cutoff", &mut state.cutoff, 20.0..=20_000.0)
            .size(KnobSize::Hero)
            .taper(Taper::Log)
            .default_value(1000.0)
            .modulation(600.0..=4000.0)
            .mod_sources(&sources)
            .format(|hz| {
                if hz >= 1000.0 {
                    format!("{:.1} kHz", hz / 1000.0)
                } else {
                    format!("{hz:.0} Hz")
                }
            })
            .show(ui);
        ui::labeled_knob("Resonance", &mut state.resonance, 0.0..=1.0)
            .default_value(0.0)
            .unit("")
            .show(ui);
        ui::labeled_knob("Pan", &mut state.pan, -1.0..=1.0)
            .bipolar()
            .default_value(0.0)
            .show(ui);
        ui::labeled_knob("Drive", &mut state.drive, 0.0..=1.0)
            .size(KnobSize::Dense)
            .automation(Some(ui::AutomationState::Read))
            .show(ui);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = tokens::SPACE_MD;
            ui::segmented(ui, &mut state.mode, &["Digital", "Tube", "Opto", "FET"]);
            ui::stepped_selector(
                ui,
                &mut state.wave,
                &[
                    "Sine",
                    "Triangle",
                    "Saw",
                    "Square",
                    "Wavetable A",
                    "Wavetable B",
                ],
            );
            ui::toggle_led(ui, &mut state.on, "Echo");
        });
        ui::wheel(ui, &mut state.pitch, true, "Pitch", egui::vec2(34.0, 120.0));
        ui::wheel(
            ui,
            &mut state.modulation,
            false,
            "Mod",
            egui::vec2(34.0, 120.0),
        );
    });
    ui.add_space(tokens::SPACE_LG);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_LG;
        let (mut x, mut y) = state.xy;
        ui::xy_pad(
            ui,
            &mut x,
            &mut y,
            egui::vec2(180.0, 144.0),
            ["Cutoff", "Resonance"],
        );
        state.xy = (x, y);
        let output =
            ui::envelope_editor(ui, &state.envelope, egui::vec2(377.0, 144.0), &[(0.5, 0.7)]);
        for edit in output.edits {
            state.envelope.apply(edit);
        }
        let pads: [PadView; 16] = std::array::from_fn(|index| PadView {
            name: ["Kick", "Snare", "Hat", "Clap"][index % 4],
            loaded: index < 9,
            activity: if index == 1 { 0.6 } else { 0.0 },
        });
        if let Some(selected) =
            ui::pad_grid(ui, &pads, state.selected_pad, egui::vec2(233.0, 144.0)).selected
        {
            state.selected_pad = selected;
        }
    });
    ui.add_space(tokens::SPACE_LG);
    ui::keyboard_strip(ui, 48, 3, &[60, 64], Some(60), egui::vec2(610.0, 55.0));
    ui.add_space(tokens::SPACE_LG);
    ui.horizontal(|ui| {
        ui::mod_source_chip(ui, 1, "LFO 1");
        ui::mod_source_chip(ui, 2, "Env 2");
    });
    ui.add_space(tokens::SPACE_SM);
    ui::mod_slot_row(
        ui,
        &mut state.slot,
        &[(1, "LFO 1"), (2, "Env 2")],
        &[(1, "Pitch"), (2, "Cutoff")],
    );
}

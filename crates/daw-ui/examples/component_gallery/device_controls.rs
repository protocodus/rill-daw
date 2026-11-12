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
    /// Frequency (Hz), gain (dB) and Q of three table columns.
    bands: [[f64; 3]; 3],
    band_types: [ui::Glyph; 3],
    slope: u32,
    shape: ui::Glyph,
    selected_band: usize,
    division: usize,
    section_on: bool,
    model: usize,
    seed: i32,
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
            bands: [[100.0, 0.0, 0.71], [1200.0, 3.5, 1.4], [8000.0, -6.0, 0.71]],
            band_types: [ui::Glyph::LowShelf, ui::Glyph::Bell, ui::Glyph::HighShelf],
            slope: 24,
            shape: ui::Glyph::Sine,
            selected_band: 1,
            division: 3,
            section_on: true,
            model: 0,
            seed: 1,
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
            ui.horizontal(|ui| {
                ui::value_link(ui, "Kick +1", false, 89.0);
                ui::value_link(ui, "No source", true, 89.0);
                // The value under a detented knob: a click lists the options.
                let options = ["1/16", "1/8 T", "1/8", "1/8 D", "1/4", "1/2"];
                if let (_, Some(picked)) =
                    ui::value_menu(ui, options[state.division], &options, state.division, 89.0)
                {
                    state.division = picked;
                }
            });
            // One of a numbered set of variations, under its number.
            ui.vertical(|ui| {
                ui::variation_stepper(ui, &mut state.seed, 1..=32, "seed");
                ui::text::note(ui, format!("#{}", state.seed));
            });
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
    ui.add_space(tokens::SPACE_LG);
    ui::text::note(
        ui,
        "inline_knob · glyph_toggle · column_highlight: a table of like values with a selected column",
    );
    ui.add_space(tokens::SPACE_SM);
    band_table(ui, state);
    ui.add_space(tokens::SPACE_LG);
    ui::text::note(
        ui,
        "grid_row_cells · device_section_header_choice: a bare picture cell beside a card with an enable lamp and its model",
    );
    ui.add_space(tokens::SPACE_SM);
    ui.scope(|ui| {
        ui.set_max_width(610.0);
        picture_row(ui, state);
    });
    ui.add_space(tokens::SPACE_MD);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_LG;
        let text = |label| ui::ToggleFace::Text(label);
        ui::glyph_toggle(
            ui,
            &mut state.slope,
            &[
                (12, text("12"), "12 dB/oct"),
                (24, text("24"), "24 dB/oct"),
                (48, text("48"), "48 dB/oct"),
            ],
            28.0,
        );
        let wave = |glyph, name| (glyph, ui::ToggleFace::Glyph(glyph), name);
        ui::glyph_toggle(
            ui,
            &mut state.shape,
            &[
                wave(ui::Glyph::Sine, "Sine"),
                wave(ui::Glyph::Triangle, "Triangle"),
                wave(ui::Glyph::Saw, "Saw"),
                wave(ui::Glyph::Square, "Square"),
                wave(ui::Glyph::SampleHold, "Sample and hold"),
                wave(ui::Glyph::Wander, "Random, smooth"),
            ],
            28.0,
        );
        let mut filter = ui::Glyph::LowPass;
        ui::glyph_toggle(
            ui,
            &mut filter,
            &[
                wave(ui::Glyph::LowPass, "Low-pass"),
                wave(ui::Glyph::BandPass, "Band-pass"),
                wave(ui::Glyph::HighPass, "High-pass"),
            ],
            28.0,
        );
        ui.add_enabled_ui(false, |ui| {
            let mut level = 0.3;
            ui.add(ui::inline_knob("Level", &mut level, 0.0..=1.0, "30 %"));
        });
        let mut send = 0.6;
        ui.add(
            ui::inline_knob("Send", &mut send, 0.0..=1.0, "60 %")
                .automation(Some(ui::AutomationState::Read)),
        );
    });
}

/// A picture as a bare cell beside a card whose title carries the section's
/// enable lamp and its model: `grid_row_cells` and
/// `device_section_header_choice`.
pub fn picture_row(ui: &mut egui::Ui, state: &mut State) {
    let cells = [
        ui::GridCell {
            span: 5,
            framed: false,
        },
        ui::GridCell {
            span: 7,
            framed: true,
        },
    ];
    ui::grid_row_cells(ui, &cells, |ui, index| {
        if index == 0 {
            // As tall as the row: the card decides the height.
            let size = ui.available_size();
            ui::scope(ui, &[0.0, 0.6, 0.0, -0.6, 0.0], false, size);
            return;
        }
        // The section's model on its title line.
        ui::device_section_header_choice(
            ui,
            "Echo",
            Some(&mut state.section_on),
            &mut state.model,
            &["Digital", "BBD", "Tape"],
            [None, None],
        );
        ui.add_space(tokens::SPACE_XS);
        ui.add_enabled_ui(state.section_on, |ui| {
            ui.horizontal(|ui| {
                ui::labeled_knob("Feedback", &mut state.resonance, 0.0..=1.0).show(ui);
                ui::labeled_knob("Level", &mut state.drive, 0.0..=1.0).show(ui);
            });
        });
    });
}

/// Three columns of frequency, gain and Q under a type selector, the middle
/// one selected: how a device lays out a table of bands.
fn band_table(ui: &mut egui::Ui, state: &mut State) {
    const COLUMN: f32 = 89.0;
    const CAPTION: f32 = 48.0;
    let origin = ui.cursor().min;
    let height = 4.0 * tokens::CONTROL_HEIGHT + 3.0 * ui.spacing().item_spacing.y;
    ui::column_highlight(
        ui,
        egui::Rect::from_min_size(
            origin + egui::vec2(CAPTION + COLUMN * state.selected_band as f32, 0.0),
            egui::vec2(COLUMN, height),
        )
        .expand2(egui::vec2(0.0, tokens::SPACE_XS)),
    );
    let line = |ui: &mut egui::Ui, caption: &str, cell: &mut dyn FnMut(&mut egui::Ui, usize)| {
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(CAPTION + COLUMN * 3.0, tokens::CONTROL_HEIGHT),
            egui::Sense::hover(),
        );
        ui.painter().text(
            row.left_center(),
            egui::Align2::LEFT_CENTER,
            caption,
            ui::control_caption_font(),
            ui::theme::palette(ui.ctx()).text,
        );
        for band in 0..3 {
            let center = egui::pos2(
                row.left() + CAPTION + COLUMN * (band as f32 + 0.5),
                row.center().y,
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt((caption, band))
                    .max_rect(egui::Rect::from_center_size(
                        center,
                        egui::vec2(COLUMN, ui::MINI_KNOB_DIAMETER),
                    ))
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            cell(&mut child, band);
        }
    };
    let types = [
        (ui::Glyph::Bell, "Bell"),
        (ui::Glyph::LowShelf, "Low shelf"),
        (ui::Glyph::HighShelf, "High shelf"),
        (ui::Glyph::Notch, "Notch"),
    ]
    .map(|(glyph, name)| (glyph, ui::ToggleFace::Glyph(glyph), name));
    let mut selected = state.selected_band;
    line(ui, "", &mut |ui, band| {
        if ui::glyph_toggle(ui, &mut state.band_types[band], &types, 21.0).clicked() {
            selected = band;
        }
    });
    let width = COLUMN - tokens::SPACE_SM;
    line(ui, "Freq", &mut |ui, band| {
        let hz = state.bands[band][0];
        let text = if hz >= 1000.0 {
            format!("{:.2} kHz", hz / 1000.0)
        } else {
            format!("{hz:.0} Hz")
        };
        let knob = ui::inline_knob(
            "Frequency",
            &mut state.bands[band][0],
            20.0..=20_000.0,
            text,
        )
        .taper(Taper::Log)
        .default_value(1000.0)
        .width(width);
        if ui.add(knob).clicked() {
            selected = band;
        }
    });
    line(ui, "Gain", &mut |ui, band| {
        let text = format!("{:+.1} dB", state.bands[band][1]);
        let knob = ui::inline_knob("Gain", &mut state.bands[band][1], -24.0..=24.0, text)
            .bipolar()
            .default_value(0.0)
            .width(width);
        if ui.add(knob).clicked() {
            selected = band;
        }
    });
    line(ui, "Q", &mut |ui, band| {
        let text = format!("{:.2}", state.bands[band][2]);
        let knob = ui::inline_knob("Q", &mut state.bands[band][2], 0.1..=18.0, text)
            .taper(Taper::Log)
            .default_value(1.0)
            .width(width);
        if ui.add(knob).clicked() {
            selected = band;
        }
    });
    state.selected_band = selected;
}

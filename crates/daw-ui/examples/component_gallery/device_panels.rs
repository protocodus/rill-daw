use daw_ui::{
    self as ui, DevicePanelWidth, GenericDevice, ParamKind, ParamRow, ParameterEdits, PresetEntry,
    PresetInfo, tokens,
};

pub struct State {
    values: Vec<f64>,
    bypassed: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            values: vec![0.5, 0.3, 0.8, 0.0, 0.25, 0.6, 0.4, 1.0, 0.5, 0.5, 0.7],
            bypassed: false,
        }
    }
}

impl ParameterEdits for State {
    fn value(&self, id: u32) -> f64 {
        self.values[id as usize]
    }
    fn begin_edit(&mut self, _: u32) {}
    fn set_value(&mut self, id: u32, v: f64) {
        self.values[id as usize] = v;
    }
    fn end_edit(&mut self, _: u32) {}
}

pub fn show(ui: &mut egui::Ui, state: &mut State) {
    ui::text::note(
        ui,
        "A panel generated from plain parameter rows: header, groups in signal order, no scrolling.",
    );
    ui.add_space(tokens::SPACE_MD);
    let hz = |n: f64| {
        let hz = 20.0 * 1000f64.powf(n);
        if hz >= 1000.0 {
            format!("{:.1} kHz", hz / 1000.0)
        } else {
            format!("{hz:.0} Hz")
        }
    };
    let db = |n: f64| format!("{:+.1} dB", (n - 0.5) * 48.0);
    let rows = [
        ParamRow {
            id: 0,
            name: "Cutoff",
            group: "Filter",
            unit: "Hz",
            kind: ParamKind::Continuous {
                default: 0.5,
                bipolar: false,
            },
            format: Some(&hz),
            parse: None,
        },
        ParamRow {
            id: 1,
            name: "Resonance",
            group: "Filter",
            unit: "",
            kind: ParamKind::Continuous {
                default: 0.0,
                bipolar: false,
            },
            format: None,
            parse: None,
        },
        ParamRow {
            id: 2,
            name: "Drive",
            group: "Filter",
            unit: "dB",
            kind: ParamKind::Continuous {
                default: 0.5,
                bipolar: true,
            },
            format: Some(&db),
            parse: None,
        },
        ParamRow {
            id: 3,
            name: "Mode",
            group: "Filter",
            unit: "",
            kind: ParamKind::Choice(&["Low", "Band", "High", "Notch"]),
            format: None,
            parse: None,
        },
        ParamRow {
            id: 4,
            name: "Rate",
            group: "Motion",
            unit: "",
            kind: ParamKind::Continuous {
                default: 0.25,
                bipolar: false,
            },
            format: None,
            parse: None,
        },
        ParamRow {
            id: 5,
            name: "Depth",
            group: "Motion",
            unit: "",
            kind: ParamKind::Continuous {
                default: 0.5,
                bipolar: false,
            },
            format: None,
            parse: None,
        },
        ParamRow {
            id: 6,
            name: "Sync",
            group: "Motion",
            unit: "",
            kind: ParamKind::Toggle,
            format: None,
            parse: None,
        },
        ParamRow {
            id: 7,
            name: "Shape",
            group: "Motion",
            unit: "",
            kind: ParamKind::Choice(&["Sine", "Tri", "Saw", "Square", "Random", "Smooth"]),
            format: None,
            parse: None,
        },
        ParamRow {
            id: 9,
            name: "Mix",
            group: "Output",
            unit: "",
            kind: ParamKind::Continuous {
                default: 1.0,
                bipolar: false,
            },
            format: None,
            parse: None,
        },
        ParamRow {
            id: 10,
            name: "Output",
            group: "Output",
            unit: "",
            kind: ParamKind::Continuous {
                default: 0.5,
                bipolar: false,
            },
            format: Some(&db),
            parse: None,
        },
    ];
    let entries = [
        PresetEntry {
            name: "Init",
            category: "Basic",
            tags: &[],
            description: "",
            factory: true,
            favorite: false,
        },
        PresetEntry {
            name: "Warm Sweep",
            category: "Movement",
            tags: &["warm", "slow"],
            description: "Slow low-pass motion.",
            factory: true,
            favorite: true,
        },
        PresetEntry {
            name: "My Wobble",
            category: "Movement",
            tags: &["bass"],
            description: "",
            factory: false,
            favorite: false,
        },
    ];
    let device = GenericDevice {
        name: "R1 Filter",
        rows: &rows,
        width: DevicePanelWidth::Medium,
        mix: Some(9),
        output: Some(10),
        presets: Some(PresetInfo {
            name: "Warm Sweep",
            modified: true,
            entries: &entries,
            current: Some(1),
        }),
        input_peaks: Some([0.4, 0.35]),
        output_peaks: Some([0.5, 0.45]),
    };
    let mut bypassed = state.bypassed;
    // The host owns the values; the panel edits them through the trait.
    let mut host = State {
        values: state.values.clone(),
        bypassed,
    };
    ui::generic_device_panel(ui, &device, &mut host, &mut bypassed);
    state.values = host.values;
    state.bypassed = bypassed;
}

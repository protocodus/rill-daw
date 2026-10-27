//! A non-destructive, seeded note draft. Only Add notes touches the project.
use daw_core::{
    Accompaniment, ArpeggioPattern, Chord, GenerateHarmony, GenerateKind, GenerateSettings,
    GeneratedNote, MelodyAlgorithm, Scale, complete_visible_bars, generate_notes,
};
use daw_ui::TimelineViewport;
use eframe::egui::{self, Rect};
use serde_json::{Value, json};

/// Session-only choices for one clip. Preview viewport and generated notes are
/// deliberately rebuilt against the current clip when the dialog reopens.
#[derive(Clone)]
pub(crate) struct GenerateMemory {
    settings: GenerateSettings,
    preview: bool,
    start_bar: u32,
    length_bars: u32,
    chord: Chord,
    scale: Scale,
}

pub(crate) struct GenerateDialog {
    pub settings: GenerateSettings,
    pub notes: Vec<GeneratedNote>,
    pub preview: bool,
    pub original_view: TimelineViewport,
    pub clip_window: (f64, f64, f64, f64),
    start_bar: u32,
    length_bars: u32,
    cached: Option<GenerateSettings>,
    error: Option<String>,
    controls: Value,
    chord: Chord,
    scale: Scale,
    added_notes: usize,
    applied: Option<GenerateSettings>,
}

#[derive(Default)]
pub(crate) struct GenerateAction {
    pub apply: bool,
    pub cancel: bool,
    pub fit: bool,
    pub grid_scale: bool,
}

impl GenerateDialog {
    pub fn new(
        settings: GenerateSettings,
        view: TimelineViewport,
        visible_end: f64,
        beats_per_bar: u8,
        clip_window: (f64, f64, f64, f64),
        chord: Chord,
        scale: Scale,
    ) -> Self {
        let meter = f64::from(beats_per_bar);
        let (start, length) = complete_visible_bars(view.start_beat, visible_end, beats_per_bar)
            .unwrap_or(((view.start_beat.max(0.0) / meter).ceil() * meter, 0.0));
        let mut dialog = Self {
            settings,
            notes: Vec::new(),
            preview: true,
            original_view: view,
            clip_window,
            start_bar: (start / meter).round() as u32 + 1,
            length_bars: (length / meter).round() as u32,
            cached: None,
            error: None,
            controls: Value::Null,
            chord,
            scale,
            added_notes: 0,
            applied: None,
        };
        dialog.refresh(beats_per_bar);
        dialog
    }

    pub fn memory(&self) -> GenerateMemory {
        GenerateMemory {
            settings: self.settings.clone(),
            preview: self.preview,
            start_bar: self.start_bar,
            length_bars: self.length_bars,
            chord: self.chord,
            scale: self.scale,
        }
    }

    pub fn restore(&mut self, memory: &GenerateMemory, meter: u8) {
        self.settings = memory.settings.clone();
        self.preview = memory.preview;
        self.start_bar = memory.start_bar;
        self.length_bars = memory.length_bars;
        self.chord = memory.chord;
        self.scale = memory.scale;
        self.cached = None;
        self.refresh(meter);
    }

    pub fn committed(&mut self, clip_window: (f64, f64, f64, f64), count: usize) {
        self.clip_window = clip_window;
        self.added_notes += count;
        self.applied = Some(self.settings.clone());
    }

    fn already_added(&self) -> bool {
        self.applied.as_ref() == Some(&self.settings)
    }

    pub fn preview_visible(&self) -> bool {
        self.preview && !self.already_added()
    }

    fn next_start_bar(&self, meter: u8) -> Option<u32> {
        if self.length_bars == 0 {
            return None;
        }
        let start_bar = self.start_bar.checked_add(self.length_bars)?;
        if start_bar > 1_000_000 {
            return None;
        }
        let end = f64::from(start_bar - 1 + self.length_bars) * f64::from(meter);
        let (clip_start, offset, clip_length, rate) = self.clip_window;
        let end = end.max(clip_length);
        (clip_start + end <= daw_core::MAX_BEAT && (offset + end) * rate <= daw_core::MAX_BEAT)
            .then_some(start_bar)
    }

    fn refresh(&mut self, meter: u8) {
        match self.settings.harmony {
            GenerateHarmony::Chord(chord) => self.chord = chord,
            GenerateHarmony::Scale(scale) => self.scale = scale,
        }
        self.settings.start = f64::from(self.start_bar.saturating_sub(1)) * f64::from(meter);
        self.settings.length = f64::from(self.length_bars) * f64::from(meter);
        if self.cached.as_ref() == Some(&self.settings) {
            return;
        }
        self.cached = Some(self.settings.clone());
        if self.length_bars == 0 {
            self.notes.clear();
            self.error = Some("Choose a length of at least one bar.".into());
            return;
        }
        match generate_notes(&self.settings) {
            Ok(notes) => {
                let (clip_start, offset, clip_length, rate) = self.clip_window;
                let end = (self.settings.start + self.settings.length).max(clip_length);
                if clip_start + end > daw_core::MAX_BEAT
                    || (offset + end) * rate > daw_core::MAX_BEAT
                {
                    self.notes.clear();
                    self.error =
                        Some("This range exceeds the clip's supported source length.".into());
                } else {
                    self.notes = notes;
                    self.error = None;
                }
            }
            Err(error) => {
                self.notes.clear();
                self.error = Some(error.to_string());
            }
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, clip_name: &str, meter: u8) -> GenerateAction {
        let mut action = GenerateAction::default();
        let mut open = true;
        self.controls = json!({});
        let bounds = ctx.available_rect();
        daw_ui::popup(format!("Generate notes · {clip_name}"), ctx)
            .id(egui::Id::new("midi_generate_dialog"))
            .pivot(egui::Align2::RIGHT_TOP)
            .default_pos(bounds.right_top() - egui::vec2(12.0, -8.0))
            .default_width(360.0)
            .vscroll(false)
            .movable(true)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for kind in GenerateKind::ALL {
                        daw_ui::selectable_value(ui, &mut self.settings.kind, kind, kind.name());
                    }
                });
                ui.add_space(daw_ui::tokens::SPACE_SM);
                let range =
                    daw_ui::generation_bar_range(ui, &mut self.start_bar, &mut self.length_bars);
                self.controls["start_bar"] = rect_json(range.start.rect);
                self.controls["length_bars"] = rect_json(range.length.rect);
                ui.separator();
                // Scroll only the parameters; the range, preview and commit
                // actions remain reachable on small displays.
                let form_height = (ctx.content_rect().height()
                    - daw_ui::tokens::POPUP_TOP
                    - daw_ui::tokens::POPUP_MARGIN
                    - 440.0)
                    .max(100.0);
                let form = egui::ScrollArea::vertical()
                    .id_salt("generate_options")
                    .max_height(form_height)
                    .show(ui, |ui| {
                        daw_ui::form_grid(ui, "generate_fields", |ui| {
                            daw_ui::text::label(ui, "Harmony");
                            let mut chord =
                                matches!(self.settings.harmony, GenerateHarmony::Chord(_));
                            let response = daw_ui::select(
                                "generate_harmony",
                                if chord { "Chord" } else { "Scale" },
                            )
                            .show_ui(ui, |ui| {
                                daw_ui::selectable_value(ui, &mut chord, true, "Chord");
                                daw_ui::selectable_value(ui, &mut chord, false, "Scale");
                            })
                            .response;
                            if chord != matches!(self.settings.harmony, GenerateHarmony::Chord(_)) {
                                self.settings.harmony = if chord {
                                    GenerateHarmony::Chord(self.chord)
                                } else {
                                    GenerateHarmony::Scale(self.scale)
                                };
                            }
                            self.controls["harmony"] = rect_json(response.rect);
                            ui.end_row();
                            let root_label = daw_ui::text::label(ui, "Root");
                            let response =
                                daw_ui::generation_root_selector(ui, &mut self.settings.root)
                                    .labelled_by(root_label.id);
                            self.controls["root"] = rect_json(response.rect);
                            ui.end_row();
                            match &mut self.settings.harmony {
                                GenerateHarmony::Chord(chord) => {
                                    choice(
                                        ui,
                                        &mut self.controls,
                                        "chord",
                                        "Chord",
                                        chord,
                                        &Chord::ALL,
                                        |v| v.name(),
                                    );
                                }
                                GenerateHarmony::Scale(scale) => {
                                    choice(
                                        ui,
                                        &mut self.controls,
                                        "scale",
                                        "Scale",
                                        scale,
                                        &Scale::ALL,
                                        |v| v.name(),
                                    );
                                }
                            }
                            number(
                                ui,
                                &mut self.controls,
                                "octaves",
                                "Octaves",
                                &mut self.settings.octaves,
                                1..=8,
                            );
                            number(
                                ui,
                                &mut self.controls,
                                "start_octave",
                                "Start octave",
                                &mut self.settings.start_octave,
                                -1..=9,
                            );
                            match self.settings.kind {
                                GenerateKind::Chord => {
                                    number(
                                        ui,
                                        &mut self.controls,
                                        "shift",
                                        "Shift",
                                        &mut self.settings.shift,
                                        -24..=24,
                                    )
                                    .on_hover_text(
                                        "Invert the voicing by moving tones up or down an octave",
                                    );
                                }
                                GenerateKind::Arpeggio => {
                                    choice(
                                        ui,
                                        &mut self.controls,
                                        "pattern",
                                        "Pattern",
                                        &mut self.settings.arp,
                                        &ArpeggioPattern::ALL,
                                        |v| v.name(),
                                    );
                                    speed(
                                        ui,
                                        &mut self.controls,
                                        &mut self.settings.speed_multiplier,
                                    );
                                }
                                GenerateKind::Melody => {
                                    choice(
                                        ui,
                                        &mut self.controls,
                                        "algorithm",
                                        "Algorithm",
                                        &mut self.settings.melody,
                                        &MelodyAlgorithm::ALL,
                                        |v| v.name(),
                                    );
                                    speed(
                                        ui,
                                        &mut self.controls,
                                        &mut self.settings.speed_multiplier,
                                    );
                                    match self.settings.melody {
                                        MelodyAlgorithm::RandomWalk => {
                                            number(
                                                ui,
                                                &mut self.controls,
                                                "step_size",
                                                "Max step",
                                                &mut self.settings.step_size,
                                                1..=12,
                                            );
                                        }
                                        MelodyAlgorithm::Motif => {
                                            number(
                                                ui,
                                                &mut self.controls,
                                                "motif_steps",
                                                "Motif steps",
                                                &mut self.settings.motif_steps,
                                                2..=32,
                                            );
                                            number(
                                                ui,
                                                &mut self.controls,
                                                "step_size",
                                                "Variation",
                                                &mut self.settings.step_size,
                                                1..=12,
                                            );
                                        }
                                        MelodyAlgorithm::Euclidean => {
                                            number(
                                                ui,
                                                &mut self.controls,
                                                "motif_steps",
                                                "Rhythm steps",
                                                &mut self.settings.motif_steps,
                                                2..=32,
                                            );
                                        }
                                    }
                                    percentage(
                                        ui,
                                        &mut self.controls,
                                        "density",
                                        "Density",
                                        &mut self.settings.density,
                                        5.0..=100.0,
                                    );
                                    choice(
                                        ui,
                                        &mut self.controls,
                                        "accompaniment",
                                        "Voices",
                                        &mut self.settings.accompaniment,
                                        &Accompaniment::ALL,
                                        |v| v.name(),
                                    );
                                    if self.settings.accompaniment != Accompaniment::None {
                                        number(
                                            ui,
                                            &mut self.controls,
                                            "additional_voices",
                                            "Additional voices",
                                            &mut self.settings.additional_voices,
                                            1..=3,
                                        );
                                    }
                                }
                            }
                            if self.settings.kind != GenerateKind::Chord {
                                percentage(
                                    ui,
                                    &mut self.controls,
                                    "gate",
                                    "Note length",
                                    &mut self.settings.gate,
                                    10.0..=100.0,
                                );
                            }
                            number(
                                ui,
                                &mut self.controls,
                                "velocity",
                                "Velocity",
                                &mut self.settings.velocity,
                                1..=127,
                            );
                        });
                        if self.settings.kind == GenerateKind::Melody {
                            match self.settings.accompaniment {
                                Accompaniment::Counterpoint => {
                                    daw_ui::text::note(
                                        ui,
                                        "Consonant voices favoring contrary motion.",
                                    );
                                }
                                Accompaniment::Tintinnabuli => {
                                    daw_ui::text::note(
                                        ui,
                                        "Nearest tonic-triad tones around the melody.",
                                    );
                                }
                                Accompaniment::None => {}
                            }
                        }
                    });
                self.controls["options"] = rect_json(form.inner_rect);
                if (self.settings.kind == GenerateKind::Melody
                    || self.settings.kind == GenerateKind::Arpeggio
                        && self.settings.arp == ArpeggioPattern::Random)
                    && daw_ui::button(ui, "New variation").clicked()
                {
                    self.settings.seed = self.settings.seed.wrapping_add(1);
                }
                self.refresh(meter);
                if matches!(self.settings.harmony, GenerateHarmony::Scale(_)) {
                    action.grid_scale = daw_ui::button(ui, "Use scale for grid")
                        .on_hover_text("Use this root and scale for the editor's visual guide")
                        .clicked();
                }
                ui.separator();
                // View-only controls are grouped apart from generation actions.
                ui.horizontal(|ui| {
                    daw_ui::checkbox(ui, &mut self.preview, "Preview");
                    action.fit = daw_ui::button(ui, "Fit preview").clicked();
                    daw_ui::text::note(ui, format!("{} notes", self.notes.len()));
                });
                if let Some(error) = &self.error {
                    daw_ui::text::note(ui, error);
                }
                if self.added_notes > 0 {
                    daw_ui::text::note(
                        ui,
                        format!(
                            "Added {} notes · choose Next range to continue",
                            self.added_notes
                        ),
                    );
                }
                ui.add_space(daw_ui::tokens::SPACE_SM);
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(
                        self.error.is_none() && !self.notes.is_empty() && !self.already_added(),
                        |ui| {
                            action.apply = daw_ui::primary_button(ui, "Add notes").clicked();
                        },
                    );
                    let next_start = self.next_start_bar(meter);
                    if ui
                        .add_enabled(next_start.is_some(), daw_ui::button_widget("Next range"))
                        .on_hover_text("Move forward by this range's length, keeping your settings")
                        .clicked()
                    {
                        self.start_bar = next_start.unwrap();
                        self.refresh(meter);
                    }
                    action.cancel = daw_ui::button(
                        ui,
                        if self.added_notes > 0 {
                            "Done"
                        } else {
                            "Cancel"
                        },
                    )
                    .clicked();
                });
            });
        action.cancel |= !open;
        action
    }

    pub fn diagnostics(&self, range: Option<Rect>, painted_notes: Vec<Value>) -> Value {
        json!({
            "kind":self.settings.kind.name(), "start":self.settings.start, "length":self.settings.length,
            "start_bar":self.start_bar,"length_bars":self.length_bars,"seed":self.settings.seed,
            "settings":{
                "root":self.settings.root,
                "harmony":match self.settings.harmony { GenerateHarmony::Chord(_) => "Chord", GenerateHarmony::Scale(_) => "Scale" },
                "chord":self.chord.name(),"scale":self.scale.name(),
                "octaves":self.settings.octaves,"start_octave":self.settings.start_octave,"shift":self.settings.shift,
                "pattern":self.settings.arp.name(),"speed":self.settings.speed_multiplier,
                "algorithm":self.settings.melody.name(),"voices":self.settings.accompaniment.name(),
                "additional_voices":self.settings.additional_voices,"gate":self.settings.gate,
                "density":self.settings.density,"step_size":self.settings.step_size,
                "motif_steps":self.settings.motif_steps,"velocity":self.settings.velocity,
            },
            "notes":self.notes.iter().map(|note| json!({"pitch":note.pitch,"start":note.start,"length":note.length,"velocity":note.velocity,"voice":note.voice})).collect::<Vec<_>>(),
            "preview":self.preview,"preview_visible":self.preview_visible(),"range_rect":range.map(rect_json),"painted_notes":painted_notes,
            "controls":self.controls,"error":self.error,"added_notes":self.added_notes,
        })
    }
}

fn rect_json(rect: Rect) -> Value {
    json!({"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]})
}

fn choice<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    controls: &mut Value,
    id: &str,
    label: &str,
    value: &mut T,
    values: &[T],
    name: impl Fn(T) -> &'static str,
) {
    let label_response = daw_ui::text::label(ui, label);
    let response = daw_ui::select(("generate", id), name(*value))
        .show_ui(ui, |ui| {
            for &item in values {
                daw_ui::selectable_value(ui, value, item, name(item));
            }
        })
        .response
        .labelled_by(label_response.id);
    controls[id] = rect_json(response.rect);
    ui.end_row();
}

fn number<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    controls: &mut Value,
    id: &str,
    label: &str,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
) -> egui::Response {
    let label_response = daw_ui::text::label(ui, label);
    let response = ui
        .push_id(("generate", id), |ui| {
            ui.add(daw_ui::number(value).range(range).speed(0.1))
        })
        .inner
        .labelled_by(label_response.id);
    controls[id] = rect_json(response.rect);
    ui.end_row();
    response
}

fn speed(ui: &mut egui::Ui, controls: &mut Value, value: &mut f64) {
    let label_response = daw_ui::text::label(ui, "Speed");
    let response = daw_ui::select("generate_speed", format!("{value}×"))
        .show_ui(ui, |ui| {
            for speed in [0.25, 0.5, 1.0, 2.0, 4.0, 8.0] {
                daw_ui::selectable_value(ui, value, speed, format!("{speed}×"));
            }
        })
        .response
        .labelled_by(label_response.id)
        .on_hover_text("1× = eighth notes; 2× = sixteenth notes");
    controls["speed_multiplier"] = rect_json(response.rect);
    ui.end_row();
}

fn percentage(
    ui: &mut egui::Ui,
    controls: &mut Value,
    id: &str,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
) {
    let mut percent = *value * 100.0;
    let label_response = daw_ui::text::label(ui, label);
    let response = ui
        .push_id(("generate", id), |ui| {
            ui.add(
                daw_ui::number(&mut percent)
                    .range(range)
                    .speed(0.5)
                    .suffix("%"),
            )
        })
        .inner
        .labelled_by(label_response.id);
    controls[id] = rect_json(response.rect);
    ui.end_row();
    *value = percent / 100.0;
}

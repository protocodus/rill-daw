//! Piano roll, velocity lane, and shared timeline gestures.
use crate::{
    app::{DawApp, Tool},
    theme::*,
};
use daw_core::{
    Chord, ClipContent, GenerateHarmony, GenerateSettings, Note, NoteStack, QuantizeSettings,
    Scale, copy_midi_notes, legato_notes, move_midi_notes, paste_midi_notes, quantize_notes,
};
use daw_ui::{PianoRows, TimelineBounds, TimelinePane, TimelineViewport};
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::collections::{HashMap, HashSet};

const KEYS: f32 = 60.0;
const RULER: f32 = daw_ui::tokens::TIMELINE_RULER_HEIGHT;
const VELOCITY: f32 = 96.0;
const MIN_NOTE: f64 = 1.0 / 64.0;

#[derive(Clone)]
enum Gesture {
    Notes {
        originals: Vec<Note>,
        origin: Pos2,
        edge: i8,
        duplicate: bool,
    },
    Draw {
        originals: Vec<Note>,
        origin: Pos2,
    },
    Marquee {
        origin: Pos2,
        selected: HashSet<u64>,
    },
    Velocity {
        originals: Vec<Note>,
        origin: Pos2,
        previous: Pos2,
        paint: bool,
    },
}

struct QuantizePreview {
    source: Vec<Note>,
    selected: HashSet<u64>,
    settings: QuantizeSettings,
    origin: f64,
    clip_window: (f64, f64, f64, f64),
}

pub(crate) struct MidiEditorState {
    pub viewport: TimelineViewport,
    clip: Option<u64>,
    pub chord: Chord,
    pub stack: NoteStack,
    highlight_scale: bool,
    highlight_chord: bool,
    only_scale_keys: bool,
    hidden_selection: HashSet<u64>,
    last_length: f64,
    velocity: u8,
    gesture: Option<Gesture>,
    audition: Option<(u64, u8, u8)>,
    audition_from_keys: bool,
    last_click: Option<(Option<u64>, Pos2, f64)>,
    last_created: Option<(Vec<u64>, f64)>,
    clipboard: Vec<Note>,
    quantize: Option<QuantizePreview>,
    quantize_settings: QuantizeSettings,
    generate: Option<crate::midi_generate_ui::GenerateDialog>,
}

impl Default for MidiEditorState {
    fn default() -> Self {
        Self {
            viewport: TimelineViewport {
                start_beat: 0.0,
                pixels_per_beat: 78.0,
                row_offset: 0.0,
                row_height: 18.0,
            },
            clip: None,
            chord: Chord::Major,
            stack: NoteStack::Chord,
            highlight_scale: true,
            highlight_chord: true,
            only_scale_keys: false,
            hidden_selection: HashSet::new(),
            last_length: 0.25,
            velocity: 100,
            gesture: None,
            audition: None,
            audition_from_keys: false,
            last_click: None,
            last_created: None,
            clipboard: Vec::new(),
            quantize: None,
            quantize_settings: QuantizeSettings::default(),
            generate: None,
        }
    }
}

impl MidiEditorState {
    pub(crate) fn clear_hidden_selection(&mut self) {
        self.hidden_selection.clear();
    }

    pub(crate) fn visible_note_ids(&self, notes: &[Note], root: u8, scale: Scale) -> HashSet<u64> {
        notes
            .iter()
            .filter(|note| !self.only_scale_keys || scale.contains(root, note.pitch))
            .map(|note| note.id)
            .collect()
    }

    pub(crate) fn quantize_active(&self) -> bool {
        self.quantize.is_some()
    }
    pub(crate) fn cancel_quantize(&mut self) -> bool {
        self.quantize.take().is_some()
    }
    pub(crate) fn generate_active(&self) -> bool {
        self.generate.is_some()
    }
    pub(crate) fn cancel_generate(&mut self) -> bool {
        if let Some(generate) = self.generate.take() {
            self.viewport = generate.original_view;
            true
        } else {
            false
        }
    }
}

enum NoteEdit {
    Add(Vec<Note>),
    Generate(Vec<Note>),
    Move(Vec<Note>, f64, i16),
    Resize(Vec<Note>, f64, bool),
    Delete(u64),
    Cut(u64, f64),
    Duplicate(u64),
    Glue(Vec<u64>),
    Velocity(Vec<(u64, u8)>),
    Replace(Vec<Note>, HashSet<u64>, &'static str),
}

/// Editor gestures and the clipboard use audible beats. Persisted notes keep
/// source beats so differently stretched ghosts can share the same source.
fn scale_note_times(notes: &mut [Note], factor: f64) {
    for note in notes {
        note.start *= factor;
        note.length *= factor;
    }
}

fn local_note_times(notes: &mut [Note], rate: f64) {
    for note in notes {
        note.start /= rate;
        note.length /= rate;
    }
}

impl NoteEdit {
    fn into_source_time(mut self, rate: f64) -> Self {
        match &mut self {
            Self::Add(notes) | Self::Generate(notes) | Self::Replace(notes, _, _) => {
                scale_note_times(notes, rate)
            }
            Self::Move(notes, delta, _) | Self::Resize(notes, delta, _) => {
                scale_note_times(notes, rate);
                *delta *= rate;
            }
            Self::Cut(_, at) => *at *= rate,
            Self::Delete(_) | Self::Duplicate(_) | Self::Glue(_) | Self::Velocity(_) => {}
        }
        self
    }
}

impl DawApp {
    /// The application routes this held note through the assigned VST instrument.
    /// It must call this each frame, including when the editor is no longer shown.
    pub(crate) fn midi_audition(&self, ctx: &egui::Context) -> Option<(u64, u8, u8)> {
        if self.view != crate::app::WorkspaceView::Clip
            || self.midi_editor.generate_active()
            || self.playing() && !self.midi_editor.audition_from_keys
            || !ctx.input(|input| input.focused && input.pointer.primary_down())
            || !matches!(
                self.selected().map(|clip| &clip.content),
                Some(ClipContent::Midi { .. })
            )
        {
            return None;
        }
        self.midi_editor
            .audition
            .filter(|(track, _, _)| Some(*track) == self.selected_track)
    }

    pub(crate) fn piano_roll(&mut self, ui: &mut egui::Ui) {
        let Some(clip) = self.selected() else { return };
        let ClipContent::Midi { notes } = &clip.content else {
            return;
        };
        let (clip_id, clip_offset, clip_length, clip_start, source_rate) = (
            clip.id,
            clip.offset / clip.source_rate,
            clip.length,
            clip.start,
            clip.source_rate,
        );
        let clip_name = clip.name.clone();
        // Everything below, including quantize/clipboard/velocity geometry,
        // uses one common beat scale. Convert only at the model boundary.
        let mut source_notes = notes.clone();
        local_note_times(&mut source_notes, source_rate);
        let mut notes = source_notes.clone();
        let mut state = std::mem::take(&mut self.midi_editor);
        let previous_rows = PianoRows::new(|pitch| {
            state.generate.is_some()
                || !state.only_scale_keys
                || self.scale.contains(self.root, pitch)
        });
        if state.clip != Some(clip_id)
            || state.generate.as_ref().is_some_and(|draft| {
                draft.clip_window != (clip_start, clip_offset, clip_length, source_rate)
            })
        {
            state.cancel_generate();
        }
        let view_before_generate = state.viewport;
        if state.clip != Some(clip_id)
            || state.quantize.as_ref().is_some_and(|preview| {
                preview.source != source_notes
                    || preview.clip_window != (clip_start, clip_offset, clip_length, source_rate)
            })
        {
            state.quantize = None;
        }
        let mut actions = Vec::new();
        let mut fit = false;
        let mut open_generate = false;
        let supported = [
            Tool::Select,
            Tool::Pencil,
            Tool::Cut,
            Tool::Duplicate,
            Tool::Glue,
            Tool::Erase,
            Tool::MultiDraw,
            Tool::Cursor,
        ];
        if !supported.contains(&self.tool) {
            self.tool = Tool::Select;
        }
        if state.quantize.is_none() && state.generate.is_none() {
            daw_ui::editor_tool_shortcuts(ui, &mut self.tool, &supported);
        }
        daw_ui::toolbar(ui, |ui| {
            if state.generate.is_some() {
                ui.disable();
            }
            daw_ui::editor_tools(ui, &mut self.tool, &supported);
            open_generate = ui
                .add_enabled(state.quantize.is_none(), daw_ui::button_widget("Generate…"))
                .on_hover_text("Create chords, arpeggios or melodies with a live preview")
                .clicked();
            daw_ui::action_menu(ui, "Notes", |ui| {
                ui.add_enabled_ui(
                    !self.selected_notes.is_empty() && state.quantize.is_none(),
                    |ui| {
                        if daw_ui::button(ui, "Quantize...").clicked() {
                            state.quantize = Some(QuantizePreview {
                                source: source_notes.clone(),
                                selected: self.selected_notes.clone(),
                                settings: state.quantize_settings,
                                origin: clip_start - clip_offset,
                                clip_window: (clip_start, clip_offset, clip_length, source_rate),
                            });
                            state.gesture = None;
                            ui.close();
                        }
                        if daw_ui::button(ui, "Legato").clicked() {
                            match legato_notes(&source_notes, &self.selected_notes) {
                                Ok(edited) => actions.push(NoteEdit::Replace(
                                    edited,
                                    self.selected_notes.clone(),
                                    "Legato notes",
                                )),
                                Err(error) => self.message(Err(error), ""),
                            }
                            ui.close();
                        }
                        for (label, semitones) in [
                            ("+1 semitone", 1),
                            ("-1 semitone", -1),
                            ("+1 octave", 12),
                            ("-1 octave", -12),
                        ] {
                            if daw_ui::button(ui, label).clicked() {
                                match move_midi_notes(
                                    &source_notes,
                                    &self.selected_notes,
                                    0.0,
                                    semitones,
                                ) {
                                    Ok(edited) => actions.push(NoteEdit::Replace(
                                        edited,
                                        self.selected_notes.clone(),
                                        "Transpose notes",
                                    )),
                                    Err(error) => self.message(Err(error), ""),
                                }
                                ui.close();
                            }
                        }
                        if daw_ui::button(ui, "Copy notes").clicked() {
                            state.clipboard = copy_midi_notes(&source_notes, &self.selected_notes)
                                .unwrap_or_default();
                            ui.close();
                        }
                    },
                );
                if ui
                    .add_enabled(
                        !state.clipboard.is_empty() && state.quantize.is_none(),
                        daw_ui::button_widget("Paste notes at cursor"),
                    )
                    .clicked()
                {
                    match paste_midi_notes(
                        &source_notes,
                        &state.clipboard,
                        (self.cursor - clip_start).max(0.0) + clip_offset,
                    ) {
                        Ok((edited, selected)) => {
                            actions.push(NoteEdit::Replace(edited, selected, "Paste notes"))
                        }
                        Err(error) => self.message(Err(error), ""),
                    }
                    ui.close();
                }
                if self.selected_notes.len() > 1
                    && ui
                        .add_enabled(
                            state.quantize.is_none(),
                            daw_ui::button_widget("Glue selected"),
                        )
                        .clicked()
                {
                    actions.push(NoteEdit::Glue(
                        self.selected_notes.iter().copied().collect(),
                    ));
                    ui.close();
                }
                if !self.selected_notes.is_empty()
                    || matches!(self.tool, Tool::Pencil | Tool::MultiDraw)
                {
                    ui.separator();
                    let mut velocity = notes
                        .iter()
                        .find(|n| self.selected_notes.contains(&n.id))
                        .map_or(state.velocity, |n| n.velocity);
                    if ui
                        .add_enabled(
                            state.quantize.is_none(),
                            daw_ui::slider(&mut velocity, 1..=127).text("Velocity"),
                        )
                        .changed()
                    {
                        state.velocity = velocity;
                        actions.push(NoteEdit::Velocity(
                            notes
                                .iter()
                                .filter(|n| self.selected_notes.contains(&n.id))
                                .map(|n| (n.id, velocity))
                                .collect(),
                        ));
                    }
                }
            });
            ui.separator();
            daw_ui::choice_menu(
                ui,
                format!("{} · {}", pitch_class(self.root), self.scale.name()),
                |ui| {
                    daw_ui::choice_menu(ui, pitch_class(self.root), |ui| {
                        for root in 0..12 {
                            if daw_ui::selectable_value(ui, &mut self.root, root, pitch_class(root))
                                .changed()
                            {
                                ui.close();
                            }
                        }
                    })
                    .response
                    .on_hover_text("Scale and chord root");
                    daw_ui::choice_menu(ui, self.scale.name(), |ui| {
                        for scale in Scale::ALL {
                            if daw_ui::selectable_value(ui, &mut self.scale, scale, scale.name())
                                .changed()
                            {
                                ui.close();
                            }
                        }
                    })
                    .response
                    .on_hover_text("Blue scale highlights");
                    daw_ui::choice_menu(ui, state.chord.name(), |ui| {
                        for chord in Chord::ALL {
                            if daw_ui::selectable_value(ui, &mut state.chord, chord, chord.name())
                                .changed()
                            {
                                ui.close();
                            }
                        }
                    })
                    .response
                    .on_hover_text("Bright blue chord highlights");
                },
            )
            .response
            .on_hover_text(format!(
                "{} {} · {}",
                pitch_class(self.root),
                self.scale.name(),
                state.chord.name()
            ));
            daw_ui::view_menu(ui, "View", |ui| {
                daw_ui::checkbox(ui, &mut state.highlight_scale, "Highlight scale");
                daw_ui::checkbox(ui, &mut state.highlight_chord, "Highlight chord");
                daw_ui::checkbox(ui, &mut state.only_scale_keys, "Only scale keys")
                    .on_hover_text("Hide other pitches without changing your notes");
            });
            fit = daw_ui::timeline_fit_button(ui).clicked();
            let hidden_count = source_notes
                .iter()
                .filter(|note| state.only_scale_keys && !self.scale.contains(self.root, note.pitch))
                .count();
            if hidden_count > 0
                && daw_ui::button(
                    ui,
                    format!(
                        "{hidden_count} {} hidden · Show all",
                        if hidden_count == 1 { "note" } else { "notes" }
                    ),
                )
                .clicked()
            {
                state.only_scale_keys = false;
            }
            if self.tool == Tool::MultiDraw {
                // Keep the active tool's options with its toolbar. At narrower
                // widths they share the wrapped row instead of adding a third
                // padded bar above the note canvas.
                daw_ui::toolbar_group_sized(
                    ui,
                    2.0 * daw_ui::tokens::SELECT_WIDTH + daw_ui::tokens::SPACE_SM,
                    |ui| {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            daw_ui::select("midi_stack", state.stack.name())
                                .show_ui(ui, |ui| {
                                    for stack in [NoteStack::Chord, NoteStack::Scale] {
                                        daw_ui::selectable_value(
                                            ui,
                                            &mut state.stack,
                                            stack,
                                            stack.name(),
                                        );
                                    }
                                })
                                .response
                                .on_hover_text("Multi-draw fills the clicked octave");
                            match state.stack {
                                NoteStack::Single => {}
                                NoteStack::Chord => {
                                    daw_ui::select("active_midi_chord", state.chord.name())
                                        .show_ui(ui, |ui| {
                                            for chord in Chord::ALL {
                                                daw_ui::selectable_value(
                                                    ui,
                                                    &mut state.chord,
                                                    chord,
                                                    chord.name(),
                                                );
                                            }
                                        });
                                }
                                NoteStack::Scale => {
                                    daw_ui::select("active_midi_scale", self.scale.name()).show_ui(
                                        ui,
                                        |ui| {
                                            for scale in Scale::ALL {
                                                daw_ui::selectable_value(
                                                    ui,
                                                    &mut self.scale,
                                                    scale,
                                                    scale.name(),
                                                );
                                            }
                                        },
                                    );
                                }
                            }
                        });
                    },
                );
            }
        });
        // Preview always exposes its pitches; closing restores the original
        // scale-only view and viewport without changing that view preference.
        let mut rows = PianoRows::new(|pitch| {
            open_generate
                || state.generate.is_some()
                || !state.only_scale_keys
                || self.scale.contains(self.root, pitch)
        });
        if state.clip != Some(clip_id) {
            state.hidden_selection.clear();
        }
        if !state.hidden_selection.is_empty() {
            let existing: HashSet<_> = notes.iter().map(|note| note.id).collect();
            state.hidden_selection.retain(|id| existing.contains(id));
        }
        // Preview unfolds drawing only. Hidden selections must stay hidden so
        // dismissing the dialog cannot expose them to Delete or Duplicate.
        let selection_rows =
            PianoRows::new(|pitch| !state.only_scale_keys || self.scale.contains(self.root, pitch));
        for note in &notes {
            if selection_rows.row(note.pitch).is_some() {
                if state.hidden_selection.remove(&note.id) {
                    self.selected_notes.insert(note.id);
                }
            } else if self.selected_notes.remove(&note.id) {
                state.hidden_selection.insert(note.id);
            }
        }
        if rows != previous_rows {
            state.gesture = None;
            state.last_click = None;
            state.last_created = None;
            state.quantize = None;
        }
        let mut quantize_controls = serde_json::Value::Null;
        if let Some(preview) = &mut state.quantize {
            let mut apply = false;
            let mut cancel = false;
            daw_ui::card(ui.ctx()).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Quantize preview");
                    daw_ui::select("quantize_grid", format!("{} beat", preview.settings.grid)).width(90.0).show_ui(ui, |ui| {
                        for grid in [0.0625, 0.125, 0.25, 0.5, 1.0, 2.0] { daw_ui::selectable_value(ui, &mut preview.settings.grid, grid, format!("{grid} beat")); }
                    });
                    let mut strength = preview.settings.strength * 100.0;
                    let strength_response = ui.add(daw_ui::slider(&mut strength, 0.0..=100.0).text("Strength").suffix("%"));
                    preview.settings.strength = strength / 100.0;
                    let mut swing = preview.settings.swing * 100.0;
                    let swing_response = ui.add(daw_ui::slider(&mut swing, 50.0..=75.0).text("Swing").suffix("%"));
                    preview.settings.swing = swing / 100.0;
                    daw_ui::checkbox(ui, &mut preview.settings.ends, "Note ends");
                    apply = daw_ui::primary_button(ui, "Apply quantize").clicked();
                    cancel = daw_ui::button(ui, "Cancel quantize").clicked();
                    let slider = |rect: Rect| rect_json(Rect::from_min_size(rect.min, Vec2::new(ui.spacing().slider_width, rect.height())));
                    quantize_controls = serde_json::json!({"strength_rect":slider(strength_response.rect),"swing_rect":slider(swing_response.rect),"strength":preview.settings.strength,"swing":preview.settings.swing,"ends":preview.settings.ends,"grid":preview.settings.grid});
                });
            });
            match quantize_notes(
                &preview.source,
                &preview.selected,
                preview.origin,
                preview.settings,
            ) {
                Ok(edited) => {
                    notes = edited;
                    if apply {
                        actions.push(NoteEdit::Replace(
                            notes.clone(),
                            preview.selected.clone(),
                            "Quantize notes",
                        ));
                    }
                }
                Err(error) => {
                    self.message(Err(error), "");
                    cancel = true;
                }
            }
            state.quantize_settings = preview.settings;
            if apply || cancel {
                state.quantize = None;
                if cancel {
                    notes = source_notes.clone();
                }
            }
        }
        if ui.is_enabled()
            && state.quantize.is_none()
            && state.generate.is_none()
            && !open_generate
            && !ui.ctx().wants_keyboard_input()
            && !egui::Popup::is_any_open(ui.ctx())
        {
            let modifiers = ui.input(|input| input.modifiers);
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::C)) {
                state.clipboard =
                    copy_midi_notes(&source_notes, &self.selected_notes).unwrap_or_default();
            }
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::V))
                && !state.clipboard.is_empty()
            {
                match paste_midi_notes(
                    &source_notes,
                    &state.clipboard,
                    (self.cursor - clip_start).max(0.0) + clip_offset,
                ) {
                    Ok((edited, selected)) => {
                        actions.push(NoteEdit::Replace(edited, selected, "Paste notes"))
                    }
                    Err(error) => self.message(Err(error), ""),
                }
            }
            if !modifiers.command
                && !modifiers.ctrl
                && !modifiers.alt
                && !self.selected_notes.is_empty()
            {
                for (key, time, pitch) in [
                    (egui::Key::ArrowLeft, -1.0, 0),
                    (egui::Key::ArrowRight, 1.0, 0),
                    (egui::Key::ArrowUp, 0.0, 1),
                    (egui::Key::ArrowDown, 0.0, -1),
                ] {
                    let shift = ui.input_mut(|input| {
                        if input.consume_key(egui::Modifiers::SHIFT, key) {
                            Some(true)
                        } else if input.consume_key(egui::Modifiers::NONE, key) {
                            Some(false)
                        } else {
                            None
                        }
                    });
                    if let Some(shift) = shift {
                        let beats = time
                            * if shift {
                                self.project.beats_per_bar as f64
                            } else {
                                self.snap.max(0.0625)
                            };
                        match move_midi_notes(
                            &source_notes,
                            &self.selected_notes,
                            beats,
                            if state.only_scale_keys && !shift {
                                0
                            } else {
                                pitch * if shift { 12 } else { 1 }
                            },
                        ) {
                            Ok(mut edited) => {
                                if state.only_scale_keys && !shift && pitch != 0 {
                                    let delta = rows.clamp_move(
                                        edited
                                            .iter()
                                            .filter(|note| self.selected_notes.contains(&note.id))
                                            .map(|note| note.pitch),
                                        -pitch,
                                    );
                                    for note in edited
                                        .iter_mut()
                                        .filter(|note| self.selected_notes.contains(&note.id))
                                    {
                                        if let Some(row) = rows.row(note.pitch) {
                                            note.pitch =
                                                rows.pitch_at(row as f32 + f32::from(delta));
                                        }
                                    }
                                }
                                actions.push(NoteEdit::Replace(
                                    edited,
                                    self.selected_notes.clone(),
                                    if pitch == 0 {
                                        "Nudge notes"
                                    } else {
                                        "Transpose notes"
                                    },
                                ));
                            }
                            Err(error) => self.message(Err(error), ""),
                        }
                    }
                }
            }
        }
        let available = Vec2::new(
            ui.available_width().max(100.0),
            ui.available_height().max(200.0),
        );
        let (canvas, response) = ui.allocate_exact_size(available, Sense::click_and_drag());
        let plot = Rect::from_min_max(
            canvas.min + Vec2::new(KEYS, RULER),
            Pos2::new(canvas.right(), canvas.bottom() - VELOCITY),
        );
        let ruler = Rect::from_min_max(
            Pos2::new(plot.left(), canvas.top()),
            Pos2::new(plot.right(), plot.top()),
        );
        let velocity_rect = Rect::from_min_max(
            Pos2::new(plot.left(), plot.bottom() + 24.0),
            canvas.max - Vec2::new(0.0, 6.0),
        );
        let keys = Rect::from_min_max(
            Pos2::new(canvas.left(), plot.top()),
            Pos2::new(plot.left() - 4.0, plot.bottom()),
        );
        if state.clip != Some(clip_id) {
            let center = if notes.is_empty() {
                64.0
            } else {
                notes.iter().map(|n| n.pitch as f32).sum::<f32>() / notes.len() as f32
            };
            state.viewport = TimelineViewport {
                start_beat: 0.0,
                pixels_per_beat: self.note_zoom,
                row_height: 18.0,
                row_offset: (rows.nearest_row(center.round() as u8) as f32 - plot.height() / 36.0)
                    .max(0.0),
            };
            state.clip = Some(clip_id);
            state.gesture = None;
        } else if rows != previous_rows {
            let center = previous_rows.pitch_at(
                state.viewport.row_offset + plot.height() / state.viewport.row_height * 0.5,
            );
            state.viewport.row_offset = (rows.nearest_row(center) as f32 + 0.5
                - plot.height() / state.viewport.row_height * 0.5)
                .max(0.0);
        }
        self.last_editor_clip = Some(clip_id);
        let visible: Vec<_> = notes
            .iter()
            .filter(|n| {
                rows.row(n.pitch).is_some()
                    && n.start + n.length > clip_offset
                    && n.start < clip_offset + clip_length
            })
            .collect();
        let bounds = if visible.is_empty() {
            TimelineBounds {
                start_beat: 0.0,
                end_beat: clip_length.max(1.0),
                start_row: (rows.nearest_row(72) as f32 - 1.0).max(0.0),
                end_row: (rows.nearest_row(48) + 2).min(rows.pitches().len()) as f32,
            }
        } else {
            TimelineBounds {
                start_beat: visible
                    .iter()
                    .map(|n| (n.start - clip_offset).max(0.0))
                    .fold(f64::INFINITY, f64::min),
                end_beat: visible
                    .iter()
                    .map(|n| (n.start + n.length - clip_offset).min(clip_length))
                    .fold(0.0, f64::max),
                start_row: visible
                    .iter()
                    .map(|n| rows.row(n.pitch).unwrap() as f32)
                    .fold(f32::INFINITY, f32::min),
                end_row: visible
                    .iter()
                    .map(|n| (rows.row(n.pitch).unwrap() + 1) as f32)
                    .fold(0.0, f32::max),
            }
        };
        if fit {
            state.viewport.fit_bounds(plot, bounds);
        }
        let navigation = ui
            .add_enabled_ui(state.generate.is_none(), |ui| {
                TimelinePane::new("midi_timeline", plot).navigate(ui, &mut state.viewport, bounds)
            })
            .inner;
        state.viewport.row_offset = state.viewport.row_offset.clamp(
            0.0,
            (rows.pitches().len() as f32 - plot.height() / state.viewport.row_height).max(0.0),
        );
        self.note_zoom = state.viewport.pixels_per_beat;
        if open_generate {
            state.gesture = None;
            state.audition = None;
            let mut draft = crate::midi_generate_ui::GenerateDialog::new(
                GenerateSettings {
                    root: self.root,
                    harmony: if state.stack == NoteStack::Scale {
                        GenerateHarmony::Scale(self.scale)
                    } else {
                        GenerateHarmony::Chord(state.chord)
                    },
                    velocity: state.velocity,
                    ..Default::default()
                },
                state.viewport,
                state.viewport.start_beat
                    + f64::from(plot.width()) / f64::from(state.viewport.pixels_per_beat),
                self.project.beats_per_bar,
                (clip_start, clip_offset, clip_length, source_rate),
                state.chord,
                self.scale,
            );
            draft.original_view = view_before_generate;
            state.generate = Some(draft);
        }
        if let Some(draft) = &mut state.generate {
            let action = draft.show(ui.ctx(), &clip_name, self.project.beats_per_bar);
            if action.fit && !draft.notes.is_empty() {
                state.viewport.fit_bounds(
                    plot,
                    TimelineBounds {
                        start_beat: draft.settings.start,
                        end_beat: draft.settings.start + draft.settings.length,
                        start_row: draft
                            .notes
                            .iter()
                            .filter_map(|note| rows.row(note.pitch))
                            .min()
                            .unwrap_or(0) as f32,
                        end_row: (draft
                            .notes
                            .iter()
                            .filter_map(|note| rows.row(note.pitch))
                            .max()
                            .unwrap_or(127)
                            + 1) as f32,
                    },
                );
            }
            if action.apply {
                actions.push(NoteEdit::Generate(
                    draft
                        .notes
                        .iter()
                        .map(|note| {
                            Note::new(
                                note.pitch,
                                note.start + clip_offset,
                                note.length,
                                note.velocity,
                            )
                        })
                        .collect(),
                ));
            }
            if action.apply || action.cancel {
                state.cancel_generate();
                rows = PianoRows::new(|pitch| {
                    !state.only_scale_keys || self.scale.contains(self.root, pitch)
                });
            }
        }
        self.note_zoom = state.viewport.pixels_per_beat;
        let view = state.viewport;
        let painter = ui.painter_at(canvas);
        painter.rect_filled(canvas, 2.0, palette(ui.ctx()).bg);
        let row_painter = painter.with_clip_rect(plot);
        let key_painter = painter.with_clip_rect(keys);
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let pressed = ui.input(|i| i.pointer.primary_pressed());
        let down = ui.input(|i| i.pointer.primary_down());
        let released = ui.input(|i| i.pointer.primary_released());
        let owns_pointer = ui.is_enabled()
            && state.quantize.is_none()
            && state.generate.is_none()
            && !open_generate
            && ui.rect_contains_pointer(canvas)
            && !egui::Popup::is_any_open(ui.ctx());
        let mut double = false;
        let modifiers = ui.input(|i| i.modifiers);
        let pitch_at = |pos: Pos2| rows.pitch_at(view.row_at(plot, pos.y));
        let snap = self.snap;
        let snap_beat = |beat: f64| {
            if snap > 0.0 {
                (beat / snap).round() * snap
            } else {
                beat
            }
        };
        let beat_at = |pos: Pos2| snap_beat(view.beat_at(plot, pos.x)).max(0.0);
        let mut key_diagnostics = Vec::new();
        for (row_index, &pitch) in rows.pitches().iter().enumerate() {
            let top = view.y_at(plot, row_index as f32);
            let row = Rect::from_min_size(
                Pos2::new(plot.left(), top),
                Vec2::new(plot.width(), view.row_height),
            );
            if !row.intersects(plot) {
                continue;
            }
            let chord = state.chord.contains(self.root, pitch);
            let scale = self.scale.contains(self.root, pitch);
            let black = matches!(pitch % 12, 1 | 3 | 6 | 8 | 10);
            let row_color = palette(ui.ctx()).piano_row(
                black,
                if chord && state.highlight_chord {
                    2
                } else if scale && state.highlight_scale {
                    1
                } else {
                    0
                },
            );
            row_painter.rect_filled(row, 0.0, row_color);
            row_painter.line_segment(
                [row.left_bottom(), row.right_bottom()],
                Stroke::new(0.5_f32, palette(ui.ctx()).line),
            );
            let key = Rect::from_min_size(
                Pos2::new(keys.left(), top),
                Vec2::new(keys.width(), view.row_height - 1.0),
            );
            let active = down
                && !navigation.navigating
                && pointer.is_some_and(|p| keys.contains(p) && key.contains(p));
            let label = if state.only_scale_keys || pitch % 12 == 0 || pitch % 12 == self.root {
                daw_core::note_name(pitch)
            } else {
                String::new()
            };
            daw_ui::piano_key(
                &key_painter,
                key,
                &label,
                black,
                if chord && state.highlight_chord {
                    2
                } else if scale && state.highlight_scale {
                    1
                } else {
                    0
                },
                active,
            );
            key_diagnostics.push(serde_json::json!({"pitch":pitch,"row":row_index,"rect":rect_json(key),"scale":scale,"chord":chord,"black":black,"row_color":row_color.to_array(),"outside_color":row_color.blend(palette(ui.ctx()).inactive_clip_tint()).to_array()}));
        }
        let grid = if snap > 0.0 { snap.max(0.0625) } else { 1.0 };
        let grid = grid.max((12.0 / view.pixels_per_beat) as f64);
        let first = (view.start_beat / grid).floor() as usize;
        let last = ((view.start_beat + plot.width() as f64 / view.pixels_per_beat as f64) / grid)
            .ceil() as usize;
        for step in first..=last.min(first.saturating_add(2000)) {
            let beat = step as f64 * grid;
            let x = view.x_at(plot, beat);
            let bar = (beat % self.project.beats_per_bar as f64).abs() < 0.001;
            row_painter.line_segment(
                [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                Stroke::new(
                    1.0_f32,
                    if bar {
                        palette(ui.ctx()).grid_bar
                    } else {
                        palette(ui.ctx()).grid_minor
                    },
                ),
            );
        }
        daw_ui::paint_timeline_ruler(
            &painter,
            ruler,
            view,
            self.project.tempo,
            self.project.beats_per_bar,
        );

        let end_x = view.x_at(plot, clip_length);
        let outside_clip = daw_ui::paint_clip_tail(&row_painter, plot, end_x);
        painter.line_segment(
            [
                Pos2::new(canvas.left(), plot.bottom()),
                Pos2::new(canvas.right(), plot.bottom()),
            ],
            Stroke::new(1.0_f32, palette(ui.ctx()).line),
        );
        painter.text(
            Pos2::new(canvas.left() + 8.0, velocity_rect.top() + 8.0),
            Align2::LEFT_TOP,
            "Velocity",
            FontId::proportional(daw_ui::tokens::FONT_CANVAS),
            palette(ui.ctx()).muted,
        );
        let lane_painter = painter.with_clip_rect(velocity_rect);
        let mut note_diagnostics = Vec::new();
        let mut note_hit = None;
        let mut velocity_hit = None;
        for note in &notes {
            let Some(row) = rows.row(note.pitch) else {
                continue;
            };
            let left = (note.start - clip_offset).max(0.0);
            let right = (note.start + note.length - clip_offset).min(clip_length);
            if right <= left {
                continue;
            }
            let rect = Rect::from_min_max(
                Pos2::new(view.x_at(plot, left), view.y_at(plot, row as f32) + 1.0),
                Pos2::new(
                    (view.x_at(plot, right) - 1.0).max(view.x_at(plot, left) + 3.0),
                    view.y_at(plot, row as f32 + 1.0) - 1.0,
                ),
            );
            let selected = self.selected_notes.contains(&note.id);
            if rect.intersects(plot) {
                let fill = palette(ui.ctx()).note_fill(selected, note.velocity);
                row_painter.rect_filled(rect, 3.0, fill);
                if selected {
                    row_painter.rect_stroke(
                        rect,
                        3.0,
                        Stroke::new(1.0_f32, palette(ui.ctx()).text),
                        StrokeKind::Inside,
                    );
                }
                if rect.width() > 36.0 && rect.height() >= 14.0 {
                    row_painter.with_clip_rect(rect.intersect(plot)).text(
                        rect.left_center() + Vec2::new(5.0, 0.0),
                        Align2::LEFT_CENTER,
                        daw_core::note_name(note.pitch),
                        FontId::monospace(12.0),
                        contrast_text(fill),
                    );
                }
                if pointer.is_some_and(|p| plot.contains(p) && rect.contains(p)) {
                    note_hit = Some((note.clone(), rect));
                }
            }
            let marker = Rect::from_min_max(
                Pos2::new(view.x_at(plot, left) - 5.0, velocity_rect.top()),
                Pos2::new(view.x_at(plot, left) + 5.0, velocity_rect.bottom()),
            );
            if marker.intersects(velocity_rect) {
                daw_ui::velocity_marker(
                    &lane_painter,
                    marker,
                    f32::from(note.velocity) / 127.0,
                    selected,
                );
                if pointer.is_some_and(|p| velocity_rect.contains(p) && marker.contains(p)) {
                    velocity_hit = Some(note.clone());
                }
            }
            note_diagnostics.push(serde_json::json!({"id":note.id,"pitch":note.pitch,"rect":rect_json(rect),"velocity_rect":rect_json(marker),"selected":selected}));
        }
        if let Some((_, rect)) = &note_hit {
            ui.ctx().set_cursor_icon(
                if pointer.is_some_and(|p| daw_ui::piano_note_edge(*rect, p.x) != 0) {
                    egui::CursorIcon::ResizeHorizontal
                } else {
                    egui::CursorIcon::Grab
                },
            );
        }
        if response.clicked()
            && owns_pointer
            && let Some(pos) = pointer
            && plot.contains(pos)
        {
            let identity = note_hit.as_ref().map(|(note, _)| note.id);
            let now = ui.input(|input| input.time);
            double = state.last_click.is_some_and(|(last, position, time)| {
                last == identity && position.distance(pos) < 8.0 && now - time < 0.4
            });
            state.last_click = if double {
                None
            } else {
                Some((identity, pos, now))
            };
        }
        state.audition = None;
        state.audition_from_keys = false;
        if down
            && owns_pointer
            && !navigation.navigating
            && let Some(pos) = pointer
            // During playback, edits sound through the running timeline. An
            // extra preview voice would double/retrigger an already held note.
            && (keys.contains(pos)
                || !self.playing() && plot.contains(pos) && self.tool != Tool::Cursor)
        {
            let (pitch, velocity) = note_hit
                .as_ref()
                .map_or((pitch_at(pos), state.velocity), |(note, _)| {
                    (note.pitch, note.velocity)
                });
            state.audition = self.selected_track.map(|track| (track, pitch, velocity));
            state.audition_from_keys = keys.contains(pos);
        }
        if !navigation.navigating
            && owns_pointer
            && let Some(pos) = pointer
        {
            if pressed && velocity_rect.contains(pos) {
                self.begin_edit();
                let paint =
                    matches!(self.tool, Tool::Pencil | Tool::MultiDraw) || velocity_hit.is_none();
                let originals = if !paint {
                    let hit = velocity_hit.as_ref().unwrap();
                    if !self.selected_notes.contains(&hit.id) {
                        state.clear_hidden_selection();
                        self.selected_notes = HashSet::from([hit.id]);
                    }
                    notes
                        .iter()
                        .filter(|n| self.selected_notes.contains(&n.id))
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                state.gesture = Some(Gesture::Velocity {
                    originals,
                    origin: pos,
                    previous: pos,
                    paint,
                });
                if paint {
                    let value = velocity_at(velocity_rect, pos.y);
                    let values = notes
                        .iter()
                        .filter(|n| {
                            rows.row(n.pitch).is_some()
                                && (view.x_at(plot, n.start - clip_offset) - pos.x).abs() < 7.0
                        })
                        .map(|n| (n.id, value))
                        .collect();
                    actions.push(NoteEdit::Velocity(values));
                }
            } else if pressed && plot.contains(pos) {
                if let Some((note, rect)) = &note_hit {
                    match self.tool {
                        Tool::Erase => actions.push(NoteEdit::Delete(note.id)),
                        Tool::Cut => {
                            actions.push(NoteEdit::Cut(note.id, beat_at(pos) + clip_offset))
                        }
                        Tool::Duplicate => actions.push(NoteEdit::Duplicate(note.id)),
                        Tool::Glue => {
                            self.selected_notes.insert(note.id);
                            if self.selected_notes.len() > 1 {
                                actions.push(NoteEdit::Glue(
                                    self.selected_notes.iter().copied().collect(),
                                ));
                            }
                        }
                        Tool::Cursor => self.seek(clip_start + beat_at(pos)),
                        _ => {
                            if !self.selected_notes.contains(&note.id) {
                                if !modifiers.shift {
                                    state.clear_hidden_selection();
                                    self.selected_notes.clear();
                                }
                                self.selected_notes.insert(note.id);
                            }
                            let edge = daw_ui::piano_note_edge(*rect, pos.x);
                            state.last_length = note.length;
                            state.gesture = Some(Gesture::Notes {
                                originals: notes
                                    .iter()
                                    .filter(|n| self.selected_notes.contains(&n.id))
                                    .cloned()
                                    .collect(),
                                origin: pos,
                                edge,
                                duplicate: modifiers.shift && edge == 0,
                            });
                        }
                    }
                } else if matches!(self.tool, Tool::Pencil | Tool::MultiDraw) {
                    let beat = beat_at(pos);
                    if beat < clip_length {
                        let pitches = if self.tool == Tool::MultiDraw {
                            state
                                .stack
                                .pitches(pitch_at(pos), self.root, state.chord, self.scale)
                        } else {
                            vec![pitch_at(pos)]
                        };
                        let drawn: Vec<_> = pitches
                            .into_iter()
                            .filter(|pitch| {
                                !notes.iter().any(|note| {
                                    note.pitch == *pitch
                                        && (note.start - clip_offset - beat).abs() < 1e-8
                                })
                            })
                            .map(|pitch| {
                                Note::new(
                                    pitch,
                                    clip_offset + beat,
                                    state.last_length.max(MIN_NOTE).min(clip_length - beat),
                                    state.velocity,
                                )
                            })
                            .collect();
                        if !drawn.is_empty() {
                            state.last_created = Some((
                                drawn.iter().map(|note| note.id).collect(),
                                ui.input(|input| input.time),
                            ));
                            actions.push(NoteEdit::Add(drawn.clone()));
                            state.gesture = Some(Gesture::Draw {
                                originals: drawn,
                                origin: pos,
                            });
                        }
                    }
                } else if self.tool == Tool::Cursor {
                    self.seek(clip_start + beat_at(pos));
                } else if self.tool == Tool::Select {
                    let selected = if modifiers.shift {
                        self.selected_notes.clone()
                    } else {
                        state.clear_hidden_selection();
                        HashSet::new()
                    };
                    self.selected_notes = selected.clone();
                    state.gesture = Some(Gesture::Marquee {
                        origin: pos,
                        selected,
                    });
                }
            } else if pressed && ruler.contains(pos) {
                self.seek(clip_start + beat_at(pos));
            }
            if double && plot.contains(pos) {
                if let Some((note, _)) = &note_hit {
                    // A double background gesture with Pencil has already drawn
                    // its note on the first press; keep that new note.
                    let just_created = state.last_created.as_ref().is_some_and(|(ids, time)| {
                        ids.contains(&note.id) && ui.input(|input| input.time) - time < 0.4
                    });
                    if !just_created
                        && matches!(self.tool, Tool::Select | Tool::Pencil | Tool::MultiDraw)
                    {
                        actions.push(NoteEdit::Delete(note.id));
                    }
                } else if self.tool == Tool::Select && beat_at(pos) < clip_length {
                    actions.push(NoteEdit::Add(vec![Note::new(
                        pitch_at(pos),
                        clip_offset + beat_at(pos),
                        state
                            .last_length
                            .min(clip_length - beat_at(pos))
                            .max(MIN_NOTE),
                        state.velocity,
                    )]));
                }
                state.gesture = None;
            }
            if down && let Some(gesture) = state.gesture.clone() {
                match gesture {
                    Gesture::Notes {
                        originals,
                        origin,
                        edge,
                        duplicate,
                    } if pos.distance(origin) > 2.0 => {
                        let raw_delta = (pos.x - origin.x) as f64 / view.pixels_per_beat as f64;
                        let delta = if edge != 0 && modifiers.shift {
                            raw_delta
                        } else {
                            snap_beat(raw_delta)
                        };
                        let originals = if duplicate {
                            let copied: Vec<_> = originals
                                .into_iter()
                                .map(|mut note| {
                                    note.id = daw_core::new_id();
                                    note
                                })
                                .collect();
                            actions.push(NoteEdit::Add(copied.clone()));
                            state.gesture = Some(Gesture::Notes {
                                originals: copied.clone(),
                                origin,
                                edge,
                                duplicate: false,
                            });
                            copied
                        } else {
                            originals
                        };
                        if edge == 0 {
                            let row_delta = rows.clamp_move(
                                originals.iter().map(|note| note.pitch),
                                ((pos.y - origin.y) / view.row_height).round() as i16,
                            );
                            let mut moved = originals;
                            for note in &mut moved {
                                if let Some(row) = rows.row(note.pitch) {
                                    note.pitch = rows.pitch_at(row as f32 + f32::from(row_delta));
                                }
                            }
                            actions.push(NoteEdit::Move(moved, delta, 0));
                        } else {
                            actions.push(NoteEdit::Resize(originals, delta, edge < 0));
                        }
                    }
                    Gesture::Draw { originals, origin } if (pos.x - origin.x).abs() > 4.0 => {
                        let length = (beat_at(pos) + clip_offset - originals[0].start)
                            .max(snap.max(MIN_NOTE));
                        state.last_length = length;
                        actions.push(NoteEdit::Resize(
                            originals.clone(),
                            length - originals[0].length,
                            false,
                        ));
                    }
                    Gesture::Marquee { origin, selected } => {
                        let selection = Rect::from_two_pos(origin, pos);
                        row_painter.rect_filled(
                            selection,
                            0.0,
                            palette(ui.ctx()).note.gamma_multiply(0.15),
                        );
                        row_painter.rect_stroke(
                            selection,
                            0.0,
                            Stroke::new(1.0_f32, palette(ui.ctx()).note_selected),
                            StrokeKind::Inside,
                        );
                        self.selected_notes = selected;
                        for n in &notes {
                            let Some(row) = rows.row(n.pitch) else {
                                continue;
                            };
                            let rect = Rect::from_min_max(
                                Pos2::new(
                                    view.x_at(plot, n.start - clip_offset),
                                    view.y_at(plot, row as f32),
                                ),
                                Pos2::new(
                                    view.x_at(plot, n.start + n.length - clip_offset),
                                    view.y_at(plot, row as f32 + 1.0),
                                ),
                            );
                            if selection.intersects(rect) {
                                self.selected_notes.insert(n.id);
                            }
                        }
                    }
                    Gesture::Velocity {
                        originals,
                        origin,
                        previous,
                        paint,
                    } => {
                        let values = if paint {
                            let left = previous.x.min(pos.x) - 5.0;
                            let right = previous.x.max(pos.x) + 5.0;
                            notes
                                .iter()
                                .filter_map(|n| {
                                    rows.row(n.pitch)?;
                                    let x = view.x_at(plot, n.start - clip_offset);
                                    if x < left || x > right {
                                        return None;
                                    }
                                    let t = if (pos.x - previous.x).abs() < 1.0 {
                                        1.0
                                    } else {
                                        ((x - previous.x) / (pos.x - previous.x)).clamp(0.0, 1.0)
                                    };
                                    Some((
                                        n.id,
                                        velocity_at(
                                            velocity_rect,
                                            previous.y + (pos.y - previous.y) * t,
                                        ),
                                    ))
                                })
                                .collect()
                        } else {
                            let delta = ((origin.y - pos.y) / velocity_rect.height() * 127.0)
                                .round() as i16;
                            originals
                                .iter()
                                .map(|n| {
                                    (n.id, (i16::from(n.velocity) + delta).clamp(1, 127) as u8)
                                })
                                .collect()
                        };
                        actions.push(NoteEdit::Velocity(values));
                        state.gesture = Some(Gesture::Velocity {
                            originals,
                            origin,
                            previous: pos,
                            paint,
                        });
                    }
                    _ => {}
                }
            }
            if self.tool == Tool::Erase
                && down
                && let Some((note, _)) = note_hit
            {
                actions.push(NoteEdit::Delete(note.id));
            }
        }
        if released
            && let Some(Gesture::Velocity {
                originals,
                origin,
                paint: false,
                ..
            }) = &state.gesture
            && pointer.is_some_and(|pos| pos.distance(*origin) < 2.0)
        {
            actions.push(NoteEdit::Velocity(
                originals
                    .iter()
                    .map(|note| (note.id, velocity_at(velocity_rect, origin.y)))
                    .collect(),
            ));
        }
        if released || !down {
            state.gesture = None;
            state.audition = None;
        }
        let generate_diagnostics = if let Some(draft) = &state.generate {
            daw_ui::paint_generation_backdrop(&painter, canvas);
            let range = daw_ui::paint_generation_range(
                &painter,
                plot,
                ruler,
                view,
                draft.settings.start,
                draft.settings.start + draft.settings.length,
            );
            let mut painted = Vec::new();
            if draft.preview {
                for note in &draft.notes {
                    if let Some(row) = rows.row(note.pitch) {
                        let rect = Rect::from_min_max(
                            Pos2::new(
                                view.x_at(plot, note.start),
                                view.y_at(plot, row as f32) + 1.0,
                            ),
                            Pos2::new(
                                view.x_at(plot, note.start + note.length),
                                view.y_at(plot, (row + 1) as f32) - 1.0,
                            ),
                        );
                        if let Some(rect) =
                            daw_ui::paint_generation_note(&row_painter, rect, note.voice)
                        {
                            painted.push(serde_json::json!({"pitch":note.pitch,"voice":note.voice,"rect":rect_json(rect)}));
                        }
                    }
                }
            }
            draft.diagnostics(range, painted)
        } else {
            serde_json::Value::Null
        };
        let cursor = view.x_at(plot, self.cursor - clip_start);
        if (plot.left()..=plot.right()).contains(&cursor) {
            row_painter.line_segment(
                [
                    Pos2::new(cursor, plot.top()),
                    Pos2::new(cursor, plot.bottom()),
                ],
                Stroke::new(1.5_f32, palette(ui.ctx()).orange),
            );
        }
        ui.ctx().data_mut(|data|data.insert_temp(egui::Id::new("midi_editor_diagnostics"),serde_json::json!({
            "plot":rect_json(plot),"velocity":rect_json(velocity_rect),"keys":key_diagnostics,"notes":note_diagnostics,
            "outside_clip":outside_clip.map(rect_json),
            "start_beat":view.start_beat,"pixels_per_beat":view.pixels_per_beat,"row_offset":view.row_offset,"row_height":view.row_height,
            "visible_pitches":rows.pitches(),"highlight_scale":state.highlight_scale,"highlight_chord":state.highlight_chord,"only_scale_keys":state.only_scale_keys,
            "hidden_notes":notes.iter().filter(|note| rows.row(note.pitch).is_none()).count(),
            "tool":format!("{:?}",self.tool),"cursor":self.cursor,"chord":state.chord.name(),"stack":state.stack.name(),"audition":state.audition,
            "quantize":quantize_controls,"preview_notes":if state.quantize.is_some(){Some(&notes)}else{None},"clipboard_count":state.clipboard.len(),
            "generate":generate_diagnostics,
        })));
        self.midi_editor = state;
        let edited = !actions.is_empty();
        for action in actions {
            self.apply_note_edit(action);
        }
        if edited
            && self.midi_editor.only_scale_keys
            && let Some(ClipContent::Midi { notes }) = self.selected().map(|clip| &clip.content)
        {
            let visible = self
                .midi_editor
                .visible_note_ids(notes, self.root, self.scale);
            self.midi_editor
                .hidden_selection
                .extend(self.selected_notes.difference(&visible).copied());
            self.selected_notes.retain(|id| visible.contains(id));
        }
    }

    fn apply_note_edit(&mut self, action: NoteEdit) {
        let (Some(track), Some(clip)) = (self.selected_track, self.selected_clip) else {
            return;
        };
        let Some(original) = self.project.clip(track, clip).cloned() else {
            return;
        };
        // Source edits establish a new editing selection; only an untouched
        // fold/unfold round trip restores the selection of hidden pitches.
        self.midi_editor.clear_hidden_selection();
        let minimum_note = MIN_NOTE * original.source_rate;
        let action = action.into_source_time(original.source_rate);
        let selection = self.selected_notes.clone();
        self.begin_edit();
        let mut edit_label = None;
        let extend = matches!(
            action,
            NoteEdit::Add(_)
                | NoteEdit::Generate(_)
                | NoteEdit::Move(..)
                | NoteEdit::Resize(..)
                | NoteEdit::Duplicate(_)
                | NoteEdit::Replace(..)
        );
        let result = match action {
            NoteEdit::Replace(mut edited, selected, label) => {
                // Preserve unselected source values bit-for-bit through the
                // editor/source conversion, including arbitrary stretch ratios.
                if original.source_rate != 1.0
                    && let ClipContent::Midi { notes } = &original.content
                {
                    let unchanged: HashMap<_, _> =
                        notes.iter().map(|note| (note.id, note)).collect();
                    for note in &mut edited {
                        if !selected.contains(&note.id)
                            && let Some(source) = unchanged.get(&note.id)
                        {
                            *note = (*source).clone();
                        }
                    }
                }
                if let Some(notes) = self.project.midi_notes_mut(track, clip) {
                    *notes = edited;
                    self.selected_notes = selected;
                }
                edit_label = Some(label);
                Ok(())
            }
            NoteEdit::Generate(drawn) => {
                if let Some(notes) = self.project.midi_notes_mut(track, clip) {
                    self.selected_notes = drawn.iter().map(|note| note.id).collect();
                    notes.extend(drawn);
                }
                edit_label = Some("Generate notes");
                Ok(())
            }
            NoteEdit::Add(drawn) => {
                if let Some(notes) = self.project.midi_notes_mut(track, clip) {
                    self.selected_notes = drawn.iter().map(|n| n.id).collect();
                    notes.extend(drawn);
                }
                Ok(())
            }
            NoteEdit::Delete(id) => {
                if self
                    .project
                    .midi_notes_mut(track, clip)
                    .is_some_and(|notes| notes.iter().any(|n| n.id == id))
                {
                    self.project.delete_note(track, clip, id).map(|_| {
                        self.selected_notes.remove(&id);
                    })
                } else {
                    Ok(())
                }
            }
            NoteEdit::Cut(id, at) => self
                .project
                .split_note(track, clip, id, at)
                .map(|id| self.selected_notes = HashSet::from([id])),
            NoteEdit::Duplicate(id) => self
                .project
                .duplicate_note(track, clip, id)
                .map(|id| self.selected_notes = HashSet::from([id])),
            NoteEdit::Glue(ids) => self
                .project
                .glue_notes(track, clip, &ids)
                .map(|id| self.selected_notes = HashSet::from([id])),
            NoteEdit::Move(originals, delta, pitch_delta) => {
                let earliest = originals
                    .iter()
                    .map(|n| n.start)
                    .fold(f64::INFINITY, f64::min);
                let min = originals.iter().map(|n| n.pitch).min().unwrap_or(0) as i16;
                let max = originals.iter().map(|n| n.pitch).max().unwrap_or(127) as i16;
                let delta = delta.max(-earliest);
                let pitch_delta = pitch_delta.clamp(-min, 127 - max);
                if let Some(notes) = self.project.midi_notes_mut(track, clip) {
                    for original in originals {
                        if let Some(n) = notes.iter_mut().find(|n| n.id == original.id) {
                            n.start = original.start + delta;
                            n.pitch = (i16::from(original.pitch) + pitch_delta) as u8;
                        }
                    }
                }
                Ok(())
            }
            NoteEdit::Resize(originals, delta, start_edge) => {
                let delta = if start_edge {
                    delta
                        .max(
                            -originals
                                .iter()
                                .map(|n| n.start)
                                .fold(f64::INFINITY, f64::min),
                        )
                        .min(
                            originals
                                .iter()
                                .map(|n| n.length - minimum_note)
                                .fold(f64::INFINITY, f64::min),
                        )
                } else {
                    delta.max(
                        originals
                            .iter()
                            .map(|n| minimum_note - n.length)
                            .fold(f64::NEG_INFINITY, f64::max),
                    )
                };
                if let Some(notes) = self.project.midi_notes_mut(track, clip) {
                    for original in originals {
                        if let Some(n) = notes.iter_mut().find(|n| n.id == original.id) {
                            if start_edge {
                                n.start = original.start + delta;
                                n.length = original.length - delta;
                            } else {
                                n.length = original.length + delta;
                            }
                        }
                    }
                }
                Ok(())
            }
            NoteEdit::Velocity(values) => {
                if let Some(notes) = self.project.midi_notes_mut(track, clip) {
                    for (id, value) in values {
                        if let Some(note) = notes.iter_mut().find(|n| n.id == id) {
                            note.velocity = value.clamp(1, 127);
                        }
                    }
                }
                Ok(())
            }
        };
        self.finish_midi_edit(track, original, selection, result, extend, edit_label);
    }
}

fn velocity_at(rect: Rect, y: f32) -> u8 {
    ((rect.bottom() - y) / rect.height() * 127.0)
        .round()
        .clamp(1.0, 127.0) as u8
}
fn rect_json(rect: Rect) -> serde_json::Value {
    serde_json::json!({"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]})
}
fn pitch_class(pitch: u8) -> &'static str {
    [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ][(pitch % 12) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretched_ghost_edits_use_audible_beats_and_extend_only_that_instance() {
        let context = egui::Context::default();
        let mut project = daw_core::Project::new();
        let track = project.add_midi_track("Pattern");
        let note = Note::new(60, 1.0, 1.0, 100);
        let clip = daw_core::Clip::midi("Source", 0.0, 4.0, vec![note.clone()]);
        let original = clip.id;
        project.track_mut(track).unwrap().clips.push(clip);
        let ghost = project.clone_clip(track, original).unwrap();
        project.clip_mut(track, ghost).unwrap().source_rate = 0.5;
        project.clip_mut(track, ghost).unwrap().length = 8.0;
        let before = project.clone();
        let mut app = DawApp::with_context(&context, project, None);
        app.selected_track = Some(track);
        app.selected_clip = Some(ghost);
        let mut displayed = vec![note];
        local_note_times(&mut displayed, 0.5);
        app.apply_note_edit(NoteEdit::Move(displayed, 1.0, 0));
        app.apply_note_edit(NoteEdit::Add(vec![Note::new(64, 6.5, 2.0, 80)]));
        assert!(!app.error, "{}", app.status);
        for id in [original, ghost] {
            let ClipContent::Midi { notes } = &app.project.clip(track, id).unwrap().content else {
                panic!("expected MIDI");
            };
            assert_eq!(notes[0].start, 1.5);
            assert_eq!((notes[1].start, notes[1].length), (3.25, 1.0));
        }
        assert_eq!(app.project.clip(track, original).unwrap().length, 4.0);
        assert_eq!(app.project.clip(track, ghost).unwrap().length, 8.5);
        app.project.validate().unwrap();
        app.flush_pending_edit();
        assert_eq!(app.history.undo_labels().count(), 1);
        app.undo_redo(false);
        assert_eq!(app.project, before);
    }

    #[test]
    fn copying_between_stretch_rates_preserves_audible_note_spacing_and_duration() {
        let first = Note::new(60, 1.0, 0.5, 100);
        let second = Note::new(64, 2.0, 1.0, 80);
        let mut displayed = vec![first.clone(), second.clone()];
        local_note_times(&mut displayed, 0.5);
        let clipboard = copy_midi_notes(&displayed, &HashSet::from([first.id, second.id])).unwrap();
        let (pasted, selected) = paste_midi_notes(&[], &clipboard, 1.0).unwrap();
        let mut project = daw_core::Project::new();
        let track = project.add_midi_track("Destination");
        let mut clip = daw_core::Clip::midi("Faster", 0.0, 8.0, vec![]);
        clip.source_rate = 2.0;
        let id = clip.id;
        project.track_mut(track).unwrap().clips.push(clip);
        let mut app = DawApp::with_context(&egui::Context::default(), project, None);
        app.selected_track = Some(track);
        app.selected_clip = Some(id);
        app.apply_note_edit(NoteEdit::Replace(pasted, selected, "Paste notes"));
        let clip = app.selected().unwrap();
        let ClipContent::Midi { notes } = &clip.content else {
            panic!("expected MIDI")
        };
        assert_eq!(
            (
                clip.source_to_local(notes[0].start),
                notes[0].length / clip.source_rate
            ),
            (1.0, 1.0)
        );
        assert_eq!(
            (
                clip.source_to_local(notes[1].start),
                notes[1].length / clip.source_rate
            ),
            (3.0, 2.0)
        );
        assert_ne!(notes[0].id, first.id);
        assert_ne!(notes[1].id, second.id);
        app.project.validate().unwrap();
    }

    #[test]
    fn quantize_in_a_stretched_trimmed_clip_aligns_to_arrangement_grid() {
        let mut project = daw_core::Project::new();
        let track = project.add_midi_track("Quantize");
        let note = Note::new(60, 1.1, 0.4, 100);
        let mut clip = daw_core::Clip::midi("Offset", 0.125, 4.0, vec![note.clone()]);
        clip.offset = 0.5;
        clip.source_rate = 0.5;
        let id = clip.id;
        let mut displayed = vec![note.clone()];
        local_note_times(&mut displayed, clip.source_rate);
        let selection = HashSet::from([note.id]);
        let edited = quantize_notes(
            &displayed,
            &selection,
            clip.start - clip.offset / clip.source_rate,
            QuantizeSettings::default(),
        )
        .unwrap();
        project.track_mut(track).unwrap().clips.push(clip);
        let mut app = DawApp::with_context(&egui::Context::default(), project, None);
        app.selected_track = Some(track);
        app.selected_clip = Some(id);
        app.apply_note_edit(NoteEdit::Replace(edited, selection, "Quantize notes"));
        let clip = app.selected().unwrap();
        let ClipContent::Midi { notes } = &clip.content else {
            panic!("expected MIDI")
        };
        assert!((clip.start + clip.source_to_local(notes[0].start) - 1.25).abs() < 1e-10);
        assert!((notes[0].length - 0.4).abs() < 1e-10);
        app.project.validate().unwrap();
    }

    #[test]
    fn transforming_selected_notes_preserves_other_source_notes_exactly() {
        let mut project = daw_core::Project::new();
        let track = project.add_midi_track("Exact source");
        let untouched = Note::new(60, 0.7, 0.3, 100);
        let chosen = Note::new(64, 1.1, 0.4, 90);
        let mut clip =
            daw_core::Clip::midi("Ratio", 0.0, 4.0, vec![untouched.clone(), chosen.clone()]);
        clip.source_rate = 1.3;
        let id = clip.id;
        let mut edited = vec![untouched.clone(), chosen.clone()];
        local_note_times(&mut edited, clip.source_rate);
        edited[1].start += 0.25;
        project.track_mut(track).unwrap().clips.push(clip);
        let mut app = DawApp::with_context(&egui::Context::default(), project, None);
        app.selected_track = Some(track);
        app.selected_clip = Some(id);
        app.apply_note_edit(NoteEdit::Replace(
            edited,
            HashSet::from([chosen.id]),
            "Nudge notes",
        ));
        let ClipContent::Midi { notes } = &app.selected().unwrap().content else {
            panic!("expected MIDI")
        };
        assert_eq!(notes[0], untouched);
        assert!((notes[1].start - (chosen.start + 0.25 * 1.3)).abs() < 1e-12);
        app.project.validate().unwrap();
    }

    fn linked_clip_at_limit(remaining: f64) -> (DawApp, u64, Note) {
        let mut project = daw_core::Project::new();
        let track = project.add_midi_track("Linked patterns");
        let note = Note::new(60, 0.0, 1.0, 100);
        let clip = daw_core::Clip::midi("Last beat", 0.0, 1.0, vec![note.clone()]);
        let clip_id = clip.id;
        project.track_mut(track).unwrap().clips.push(clip);
        let clone = project.clone_clip(track, clip_id).unwrap();
        project.clone_clip(track, clone).unwrap();
        project.clip_mut(track, clip_id).unwrap().start = daw_core::MAX_BEAT - remaining;
        project.validate().unwrap();
        let mut app = DawApp::with_context(&egui::Context::default(), project, None);
        app.selected_track = Some(track);
        app.selected_clip = Some(clip_id);
        app.selected_notes = HashSet::from([note.id]);
        (app, track, note)
    }

    #[test]
    fn note_add_move_resize_and_duplicate_reject_bounds_atomically_across_links() {
        let (mut app, _, note) = linked_clip_at_limit(1.0);
        let before = app.project.clone();
        for action in [
            NoteEdit::Add(vec![Note::new(64, 1.0, 1.0, 100)]),
            NoteEdit::Move(vec![note.clone()], 1.0, 0),
            NoteEdit::Resize(vec![note.clone()], 1.0, false),
            NoteEdit::Duplicate(note.id),
        ] {
            app.apply_note_edit(action);
            assert!(app.error);
            assert_eq!(app.project, before, "no peer retains a rejected source");
            assert_eq!(app.selected_notes, HashSet::from([note.id]));
            app.project.validate().unwrap();
            app.flush_pending_edit();
            assert!(!app.history.can_undo());
        }
    }

    #[test]
    fn invalid_note_drag_frame_keeps_last_valid_source_and_original_undo_checkpoint() {
        let (mut app, track, note) = linked_clip_at_limit(3.0);
        let before = app.project.clone();
        app.apply_note_edit(NoteEdit::Move(vec![note.clone()], 0.5, 0));
        let last_valid = app.project.clone();
        app.apply_note_edit(NoteEdit::Move(vec![note.clone()], 3.0, 0));
        assert_eq!(app.project, last_valid);
        assert_eq!(app.selected_notes, HashSet::from([note.id]));
        app.apply_note_edit(NoteEdit::Move(vec![note], 1.0, 0));
        for clip in &app.project.track(track).unwrap().clips {
            let ClipContent::Midi { notes } = &clip.content else {
                unreachable!()
            };
            assert_eq!(notes[0].start, 1.0);
        }
        app.project.validate().unwrap();
        app.flush_pending_edit();
        assert_eq!(app.history.undo_labels().count(), 1);
        app.undo_redo(false);
        assert_eq!(app.project, before);
    }

    #[test]
    fn note_transform_rolls_back_when_clip_extension_exceeds_project_bounds() {
        let mut project = daw_core::Project::new();
        let track = project.add_midi_track("At timeline end");
        let note = Note::new(60, 0.0, 1.0, 100);
        let clip = daw_core::Clip::midi(
            "Last beat",
            daw_core::MAX_BEAT - 1.0,
            1.0,
            vec![note.clone()],
        );
        let clip_id = clip.id;
        project.track_mut(track).unwrap().clips.push(clip);
        project.validate().unwrap();
        let mut app = DawApp::with_context(&egui::Context::default(), project.clone(), None);
        app.selected_track = Some(track);
        app.selected_clip = Some(clip_id);
        app.selected_notes = HashSet::from([note.id]);
        let original_notes = std::slice::from_ref(&note);
        let (edited, pasted) = paste_midi_notes(original_notes, original_notes, 1.0).unwrap();
        app.apply_note_edit(NoteEdit::Replace(edited, pasted, "Paste notes"));
        assert_eq!(app.project, project);
        assert_eq!(app.selected_notes, HashSet::from([note.id]));
        assert!(app.error);
        assert!(app.edit_label.is_none());
    }
}

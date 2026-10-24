use crate::{
    app::{DawApp, Drag, PopupKind, Tool},
    arrangement::paint_waveform,
    theme::*,
};
use daw_core::*;
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

const KEY_WIDTH: f32 = 60.0;
const NOTE_HEIGHT: f32 = 18.0;
enum NoteEdit {
    Add(Note),
    Move(Vec<Note>, f64, i16),
    Resize(Vec<Note>, f64),
    Delete(u64),
    Cut(u64, f64),
    Duplicate(u64),
    Glue(Vec<u64>),
    Velocity(u8),
}

impl DawApp {
    pub fn editor(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            if let Some(clip) = self.selected() {
                ui.label(egui::RichText::new(&clip.name).color(ACCENT));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if daw_ui::button(ui, "Properties").clicked() {
                    self.open_popup(PopupKind::Inspector);
                }
            });
        });
        ui.separator();
        let Some(clip) = self.selected() else {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("Select a clip in Arrangement").color(MUTED));
            });
            return;
        };
        if matches!(clip.content, ClipContent::Audio { .. }) {
            self.audio_editor(ui);
            return;
        }
        self.piano_roll(ui);
    }
    fn audio_editor(&mut self, ui: &mut egui::Ui) {
        let Some(clip) = self.selected() else {
            return;
        };
        let ClipContent::Audio {
            path,
            fade_in,
            fade_out,
            ..
        } = &clip.content
        else {
            return;
        };
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), ui.available_height().max(80.0) - 8.0),
            Sense::click(),
        );
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 3.0, BG);
        if let Some(audio) = self.waveforms.get(path) {
            paint_waveform(
                &p,
                rect.shrink(16.0),
                audio,
                clip.offset * 60.0 / self.project.tempo,
                clip.length * 60.0 / self.project.tempo,
                ACCENT,
            );
        }
        let fade_in = *fade_in / clip.length * rect.width() as f64;
        let fade_out = *fade_out / clip.length * rect.width() as f64;
        p.line_segment(
            [
                rect.left_bottom(),
                Pos2::new(rect.left() + fade_in as f32, rect.top()),
            ],
            Stroke::new(2.0_f32, ORANGE),
        );
        p.line_segment(
            [
                Pos2::new(rect.right() - fade_out as f32, rect.top()),
                rect.right_bottom(),
            ],
            Stroke::new(2.0_f32, ORANGE),
        );
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let beat =
                clip.start + (pos.x - rect.left()) as f64 / rect.width() as f64 * clip.length;
            self.seek(beat);
        }
    }
    pub(crate) fn mixer(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let mut popup = None;
        let names: std::collections::HashMap<_, _> = self
            .project
            .tracks
            .iter()
            .map(|t| (t.id, t.name.clone()))
            .collect();
        let meter = self
            .engine
            .as_ref()
            .map(|e| e.snapshot())
            .unwrap_or_default();
        egui::ScrollArea::horizontal()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for track in
                        self.project.tracks.iter_mut().filter(|t| {
                            !matches!(t.kind, TrackKind::Folder | TrackKind::Automation)
                        })
                    {
                        daw_ui::card().inner_margin(12).show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.set_width(148.0);
                                ui.spacing_mut().slider_width = 116.0;
                                ui.label(
                                    egui::RichText::new(&track.name)
                                        .color(color(track.color))
                                        .strong(),
                                );
                                ui.label(
                                    egui::RichText::new(format!(
                                        "To {}",
                                        track
                                            .output_bus
                                            .and_then(|id| names.get(&id))
                                            .map(String::as_str)
                                            .unwrap_or("Master")
                                    ))
                                    .small()
                                    .color(MUTED),
                                );
                                ui.horizontal(|ui| {
                                    changed |= ui
                                        .add(
                                            daw_ui::slider(&mut track.gain, 0.0..=1.5)
                                                .vertical()
                                                .show_value(false),
                                        )
                                        .changed();
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{:.1} dB",
                                                20.0 * track.gain.max(0.00001).log10()
                                            ))
                                            .monospace(),
                                        );
                                        changed |=
                                            daw_ui::toggle(ui, &mut track.mute, "Mute").changed();
                                        changed |=
                                            daw_ui::toggle(ui, &mut track.solo, "Solo").changed();
                                    });
                                });
                                changed |= ui
                                    .add(
                                        daw_ui::slider(&mut track.pan, -1.0..=1.0)
                                            .show_value(false)
                                            .text("Pan"),
                                    )
                                    .changed();
                                for (name, kind, visible) in [
                                    (
                                        "Instrument",
                                        PopupKind::Instrument,
                                        track.kind == TrackKind::Midi,
                                    ),
                                    ("Effects", PopupKind::Effects, true),
                                    ("Routing", PopupKind::Routing, true),
                                ] {
                                    if visible && daw_ui::button(ui, name).clicked() {
                                        self.selected_track = Some(track.id);
                                        popup = Some(kind);
                                    }
                                }
                            });
                        });
                    }
                    daw_ui::card().fill(PANEL).inner_margin(12).show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(110.0);
                            ui.label(egui::RichText::new("Master").strong().color(ACCENT));
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::new(78.0, 116.0), Sense::hover());
                            let p = ui.painter_at(rect);
                            for (index, peak) in
                                [meter.peak_left, meter.peak_right].iter().enumerate()
                            {
                                let bar = Rect::from_min_size(
                                    rect.min + Vec2::new(index as f32 * 28.0 + 8.0, 0.0),
                                    Vec2::new(18.0, rect.height()),
                                );
                                p.rect_filled(bar, 2.0, BG);
                                let level = ((20.0 * peak.max(0.001).log10() + 60.0) / 60.0)
                                    .clamp(0.0, 1.0);
                                p.rect_filled(
                                    Rect::from_min_max(
                                        Pos2::new(bar.left(), bar.bottom() - bar.height() * level),
                                        bar.max,
                                    ),
                                    2.0,
                                    if *peak >= 1.0 { ORANGE } else { ACCENT },
                                );
                            }
                            ui.label(egui::RichText::new("L      R").monospace().color(MUTED));
                            ui.label(
                                egui::RichText::new(format!("{} Hz", self.settings.sample_rate))
                                    .small()
                                    .color(MUTED),
                            );
                        });
                    });
                });
            });
        if changed {
            self.changed();
        }
        if let Some(popup) = popup {
            self.open_popup(popup);
        }
    }

    pub fn instrument_editor(&mut self, ctx: &egui::Context) {
        if !self.instrument_open {
            return;
        }
        let Some(track) = self
            .selected_track
            .and_then(|id| self.project.track(id))
            .filter(|track| track.kind == TrackKind::Midi)
        else {
            self.instrument_open = false;
            return;
        };
        let id = track.id;
        let name = track.name.clone();
        let mut instrument = track.instrument.clone();
        let mut open = true;
        daw_ui::popup(format!("Instrument · {name}"))
            .id(egui::Id::new("instrument_editor"))
            .open(&mut open)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (name, value) in [
                        ("Sine", Instrument::Sine),
                        ("Triangle", Instrument::Triangle),
                        ("Saw", Instrument::Saw),
                    ] {
                        daw_ui::selectable_value(ui, &mut instrument, value, name);
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    daw_ui::text_field(
                        ui,
                        &mut self.plugin_filter,
                        "Search VST3 instruments",
                        280.0,
                    );
                    if ui
                        .add_enabled(self.vst_scan.is_none(), daw_ui::button_widget("Scan"))
                        .clicked()
                    {
                        self.scan_plugins();
                    }
                    if self.vst_scan.is_some() {
                        ui.spinner();
                    }
                });
                let filter = self.plugin_filter.to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for path in &self.vst_paths {
                            let name = path.file_stem().unwrap_or_default().to_string_lossy();
                            if name.to_lowercase().contains(&filter) {
                                daw_ui::selectable_value(
                                    ui,
                                    &mut instrument,
                                    Instrument::Vst3 {
                                        path: path.to_string_lossy().into_owned(),
                                    },
                                    name.as_ref(),
                                )
                                .on_hover_text(path.display().to_string());
                            }
                        }
                        if self.vst_paths.is_empty() && self.vst_scan.is_none() {
                            ui.label(egui::RichText::new("No VST3 instruments found").color(MUTED));
                        }
                    });
            });
        self.instrument_open = open;
        if self
            .project
            .track(id)
            .is_some_and(|track| track.instrument != instrument)
        {
            self.begin_edit();
            let result = self.project.set_instrument(id, instrument);
            if result.is_ok() {
                self.changed();
            }
            self.message(result, "Instrument updated");
        }
    }
    fn piano_roll(&mut self, ui: &mut egui::Ui) {
        let mut edit = None;
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            for (tool, name, hint) in [
                (
                    Tool::Select,
                    "↖ Select",
                    "Select and drag notes; drag background to box-select; Shift adds to selection",
                ),
                (
                    Tool::Pencil,
                    "Pencil",
                    "Click to draw a note; drag its right edge to resize",
                ),
                (
                    Tool::Duplicate,
                    "Duplicate",
                    "Click a note to duplicate it after itself",
                ),
                (
                    Tool::Cut,
                    "✂ Cut",
                    "Click inside a note to split at the snapped position",
                ),
                (
                    Tool::Glue,
                    "⊞ Glue",
                    "Click two notes of the same pitch to join; or select a group and press Glue selected",
                ),
            ] {
                if daw_ui::selectable(ui, self.tool == tool, name)
                    .on_hover_text(hint)
                    .clicked()
                {
                    self.tool = tool;
                }
            }
            ui.separator();
            if self.selected_notes.len() > 1 && daw_ui::button(ui, "Glue selected").clicked() {
                edit = Some(NoteEdit::Glue(self.selected_notes.iter().copied().collect()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                daw_ui::select("scale", self.scale.name())
                    .show_ui(ui, |ui| {
                        for scale in Scale::ALL {
                            daw_ui::selectable_value(ui, &mut self.scale, scale, scale.name());
                        }
                    })
                    .response
                    .on_hover_text("Highlight notes in this scale");
                daw_ui::select("root", pitch_name(self.root))
                    .width(42.0)
                    .show_ui(ui, |ui| {
                        for root in 0..12 {
                            daw_ui::selectable_value(ui, &mut self.root, root, pitch_name(root));
                        }
                    })
                    .response
                    .on_hover_text("Scale root");
            });
        });
        let selected = self.selected_notes.clone();
        let mut velocity = self
            .selected()
            .and_then(|clip| {
                if let ClipContent::Midi { notes } = &clip.content {
                    notes
                        .iter()
                        .find(|n| selected.contains(&n.id))
                        .map(|n| n.velocity)
                } else {
                    None
                }
            })
            .unwrap_or(100);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!("{} selected", self.selected_notes.len()))
                    .small()
                    .color(MUTED),
            );
            if ui
                .add_enabled(
                    !self.selected_notes.is_empty(),
                    daw_ui::slider(&mut velocity, 1..=127).text("Velocity"),
                )
                .changed()
            {
                edit = Some(NoteEdit::Velocity(velocity));
            }
            ui.add(
                daw_ui::slider(&mut self.note_zoom, 36.0..=180.0)
                    .show_value(false)
                    .text("Zoom"),
            );
        });
        let Some(clip) = self.selected() else {
            return;
        };
        let (clip_id, clip_start, clip_length, clip_offset) =
            (clip.id, clip.start, clip.length, clip.offset);
        let ClipContent::Midi { notes } = &clip.content else {
            return;
        };
        let center_pitch = if notes.is_empty() {
            64.0
        } else {
            notes.iter().map(|n| n.pitch as f32).sum::<f32>() / notes.len() as f32
        };
        let initial = self.last_editor_clip != Some(clip_id);
        self.last_editor_clip = Some(clip_id);
        let mut scroll = egui::ScrollArea::both()
            .scroll_source(egui::scroll_area::ScrollSource {
                drag: false,
                ..Default::default()
            })
            .id_salt("piano_scroll")
            .auto_shrink([false, false]);
        if initial {
            scroll = scroll.vertical_scroll_offset(
                ((127.0 - center_pitch) * NOTE_HEIGHT - ui.available_height() / 2.0).max(0.0),
            );
        }
        let mut actions = Vec::new();
        if let Some(edit) = edit {
            actions.push(edit);
        }
        scroll.show(ui, |ui| {
            let width = (KEY_WIDTH + (clip_length.max(16.0)) as f32 * self.note_zoom)
                .max(ui.available_width() - 12.0);
            let (canvas, response) = ui.allocate_exact_size(
                Vec2::new(width, 128.0 * NOTE_HEIGHT + 24.0),
                Sense::click_and_drag(),
            );
            let p = ui.painter_at(canvas);
            let origin = canvas.left() + KEY_WIDTH;
            let note_area = Rect::from_min_max(Pos2::new(origin, canvas.top() + 24.0), canvas.max);
            p.rect_filled(canvas, 0.0, BG);
            for pitch in 0..128u8 {
                let y = canvas.top() + 24.0 + (127 - pitch) as f32 * NOTE_HEIGHT;
                let row = Rect::from_min_size(
                    Pos2::new(origin, y),
                    Vec2::new(width - KEY_WIDTH, NOTE_HEIGHT),
                );
                if !ui.is_rect_visible(row) {
                    continue;
                }
                let is_root = pitch % 12 == self.root;
                let in_scale = self.scale.contains(self.root, pitch);
                p.rect_filled(
                    row,
                    0.0,
                    if is_root {
                        SCALE_ROOT
                    } else if in_scale {
                        SCALE_MEMBER
                    } else {
                        BG
                    },
                );
                p.line_segment(
                    [row.left_bottom(), row.right_bottom()],
                    Stroke::new(0.5_f32, LINE.gamma_multiply(0.7)),
                );
                let black = matches!(pitch % 12, 1 | 3 | 6 | 8 | 10);
                let key = Rect::from_min_size(
                    Pos2::new(canvas.left(), y),
                    Vec2::new(KEY_WIDTH - 4.0, NOTE_HEIGHT - 1.0),
                );
                p.rect_filled(key, 1.0, if black { PIANO_BLACK } else { PIANO_WHITE });
                if is_root {
                    p.rect_filled(
                        Rect::from_min_size(
                            Pos2::new(key.right() - 4.0, key.top()),
                            Vec2::new(4.0, key.height()),
                        ),
                        0.0,
                        ACCENT,
                    );
                }
                if pitch % 12 == 0 || is_root {
                    p.text(
                        key.right_center() - Vec2::new(9.0, 0.0),
                        Align2::RIGHT_CENTER,
                        format!("{}{}", pitch_name(pitch % 12), pitch as i16 / 12 - 1),
                        FontId::monospace(10.0),
                        if black { TEXT } else { BG },
                    );
                }
            }
            let grid = if self.snap > 0.0 {
                self.snap.max(0.25)
            } else {
                1.0
            };
            let steps = (clip_length.max(16.0) / grid).ceil() as usize;
            let first_step = (((ui.clip_rect().left() - origin) / self.note_zoom) as f64 / grid)
                .floor()
                .max(0.0) as usize;
            let last_step = ((((ui.clip_rect().right() - origin) / self.note_zoom) as f64 / grid)
                .ceil()
                .max(0.0) as usize
                + 1)
            .min(steps);
            for step in first_step..=last_step {
                let beat = step as f64 * grid;
                let x = origin + beat as f32 * self.note_zoom;
                let bar = (beat % self.project.beats_per_bar as f64).abs() < 0.001;
                p.line_segment(
                    [Pos2::new(x, canvas.top()), Pos2::new(x, canvas.bottom())],
                    Stroke::new(
                        1.0_f32,
                        if bar {
                            GRID_BAR
                        } else {
                            LINE.gamma_multiply(0.6)
                        },
                    ),
                );
                if bar {
                    p.text(
                        Pos2::new(x + 4.0, canvas.top() + 12.0),
                        Align2::LEFT_CENTER,
                        format!(
                            "{}",
                            (beat / self.project.beats_per_bar as f64) as usize + 1
                        ),
                        FontId::monospace(10.0),
                        MUTED,
                    );
                }
            }
            let clip_end_x = origin + clip_length as f32 * self.note_zoom;
            if clip_end_x < canvas.right() {
                p.rect_filled(
                    Rect::from_min_max(Pos2::new(clip_end_x, canvas.top()), canvas.max),
                    0.0,
                    egui::Color32::from_black_alpha(70),
                );
            }
            let mut note_hit = false;
            let notes = self
                .selected()
                .and_then(|c| {
                    if let ClipContent::Midi { notes } = &c.content {
                        Some(notes.as_slice())
                    } else {
                        None
                    }
                })
                .unwrap_or(&[]);
            // Render actions are applied after painting so the persistent model is never
            // mutably borrowed while widgets read it.
            let mut selection_next = None;
            let mut drag_next = None;
            for note in notes {
                let start = (note.start - clip_offset).max(0.0);
                let end = (note.start + note.length - clip_offset).min(clip_length);
                if end <= start {
                    continue;
                }
                let rect = Rect::from_min_size(
                    Pos2::new(
                        origin + start as f32 * self.note_zoom,
                        canvas.top() + 24.0 + (127 - note.pitch) as f32 * NOTE_HEIGHT + 1.0,
                    ),
                    Vec2::new(
                        ((end - start) as f32 * self.note_zoom - 1.0).max(3.0),
                        NOTE_HEIGHT - 2.0,
                    ),
                );
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let selected = self.selected_notes.contains(&note.id);
                let col = if selected { ACCENT } else { NOTE_MUTED };
                p.rect_filled(
                    rect,
                    3.0,
                    col.gamma_multiply(0.45 + 0.55 * note.velocity as f32 / 127.0),
                );
                if selected {
                    p.rect_stroke(
                        rect,
                        3.0,
                        Stroke::new(1.2_f32, egui::Color32::WHITE),
                        StrokeKind::Inside,
                    );
                }
                if rect.width() > 32.0 {
                    p.with_clip_rect(rect).text(
                        rect.left_center() + Vec2::new(5.0, 0.0),
                        Align2::LEFT_CENTER,
                        format!(
                            "{}{}",
                            pitch_name(note.pitch % 12),
                            note.pitch as i16 / 12 - 1
                        ),
                        FontId::monospace(9.0),
                        BG,
                    );
                }
                let r = ui.interact(rect, ui.id().with(note.id), Sense::click_and_drag());
                if r.contains_pointer() {
                    note_hit = true;
                }
                if r.hovered() {
                    ui.ctx().set_cursor_icon(
                        if r.hover_pos().is_some_and(|pos| pos.x > rect.right() - 7.0) {
                            egui::CursorIcon::ResizeHorizontal
                        } else {
                            egui::CursorIcon::Grab
                        },
                    );
                }
                if r.secondary_clicked() {
                    actions.push(NoteEdit::Delete(note.id));
                }
                if r.clicked() {
                    match self.tool {
                        Tool::Duplicate => actions.push(NoteEdit::Duplicate(note.id)),
                        Tool::Cut => {
                            if let Some(pos) = r.interact_pointer_pos() {
                                let at = self
                                    .snapped((pos.x - origin) as f64 / self.note_zoom as f64)
                                    + clip_offset;
                                actions.push(NoteEdit::Cut(note.id, at));
                            }
                        }
                        Tool::Glue => {
                            let mut ids = self.selected_notes.clone();
                            ids.insert(note.id);
                            if ids.len() >= 2 {
                                actions.push(NoteEdit::Glue(ids.iter().copied().collect()));
                            } else {
                                selection_next = Some(ids);
                            }
                        }
                        _ => {
                            let mut ids = if ui.input(|i| i.modifiers.shift) {
                                self.selected_notes.clone()
                            } else {
                                HashSet::new()
                            };
                            if !ids.insert(note.id) {
                                ids.remove(&note.id);
                            }
                            selection_next = Some(ids);
                        }
                    }
                }
                if r.drag_started() {
                    let resize = r
                        .interact_pointer_pos()
                        .is_some_and(|pos| pos.x > rect.right() - 8.0);
                    let mut ids = self.selected_notes.clone();
                    if !ids.contains(&note.id) {
                        if !ui.input(|i| i.modifiers.shift) {
                            ids.clear();
                        }
                        ids.insert(note.id);
                    }
                    let originals = notes
                        .iter()
                        .filter(|n| ids.contains(&n.id))
                        .cloned()
                        .collect();
                    selection_next = Some(ids);
                    drag_next = Some(Drag::Notes {
                        notes: originals,
                        resize,
                    });
                }
                if r.dragged()
                    && let Some(Drag::Notes { notes, resize }) = &self.drag
                {
                    let delta =
                        r.total_drag_delta().unwrap_or_default().x as f64 / self.note_zoom as f64;
                    let delta = if self.snap > 0.0 {
                        (delta / self.snap).round() * self.snap
                    } else {
                        delta
                    };
                    if *resize {
                        actions.push(NoteEdit::Resize(notes.clone(), delta));
                    } else {
                        actions.push(NoteEdit::Move(
                            notes.clone(),
                            delta,
                            (-r.total_drag_delta().unwrap_or_default().y / NOTE_HEIGHT).round()
                                as i16,
                        ));
                    }
                }
            }
            if let Some(ids) = selection_next {
                self.selected_notes = ids;
            }
            if let Some(drag) = drag_next {
                self.drag = Some(drag);
            }
            if response.clicked()
                && !note_hit
                && let Some(pos) = response.interact_pointer_pos()
                && note_area.contains(pos)
            {
                let beat = self
                    .snapped((pos.x - origin) as f64 / self.note_zoom as f64)
                    .max(0.0);
                let pitch =
                    (127 - ((pos.y - note_area.top()) / NOTE_HEIGHT) as i16).clamp(0, 127) as u8;
                if self.tool == Tool::Pencil && beat < clip_length {
                    actions.push(NoteEdit::Add(Note::new(
                        pitch,
                        clip_offset + beat,
                        self.snap.max(0.25).min(clip_length - beat),
                        100,
                    )));
                } else {
                    self.selected_notes.clear();
                }
            }
            if response.drag_started()
                && !note_hit
                && self.tool == Tool::Select
                && let Some(pos) = response.interact_pointer_pos()
            {
                self.drag = Some(Drag::Marquee {
                    origin: pos - response.total_drag_delta().unwrap_or_default(),
                });
                if !ui.input(|i| i.modifiers.shift) {
                    self.selected_notes.clear();
                }
            }
            if response.dragged()
                && let Some(Drag::Marquee { origin: start }) = self.drag
                && let Some(pos) = response.interact_pointer_pos()
            {
                let selection = Rect::from_two_pos(start, pos);
                p.rect_filled(selection, 0.0, ACCENT.gamma_multiply(0.12));
                p.rect_stroke(
                    selection,
                    0.0,
                    Stroke::new(1.0_f32, ACCENT),
                    StrokeKind::Inside,
                );
                let mut ids = if ui.input(|i| i.modifiers.shift) {
                    self.selected_notes.clone()
                } else {
                    HashSet::new()
                };
                if let Some(Clip {
                    content: ClipContent::Midi { notes },
                    ..
                }) = self.selected()
                {
                    for n in notes {
                        let rect = Rect::from_min_size(
                            Pos2::new(
                                origin + (n.start - clip_offset) as f32 * self.note_zoom,
                                canvas.top() + 24.0 + (127 - n.pitch) as f32 * NOTE_HEIGHT,
                            ),
                            Vec2::new(n.length as f32 * self.note_zoom, NOTE_HEIGHT),
                        );
                        if selection.intersects(rect) {
                            ids.insert(n.id);
                        }
                    }
                }
                self.selected_notes = ids;
            }
            let play_x = origin + (self.cursor - clip_start) as f32 * self.note_zoom;
            if play_x >= origin && play_x <= clip_end_x {
                p.line_segment(
                    [
                        Pos2::new(play_x, canvas.top()),
                        Pos2::new(play_x, canvas.bottom()),
                    ],
                    Stroke::new(1.5_f32, ORANGE),
                );
            }
        });
        for action in actions {
            self.apply_note_edit(action);
        }
    }
    fn apply_note_edit(&mut self, action: NoteEdit) {
        let (Some(t), Some(c)) = (self.selected_track, self.selected_clip) else {
            return;
        };
        self.begin_edit();
        let extend = matches!(
            &action,
            NoteEdit::Add(_) | NoteEdit::Move(..) | NoteEdit::Resize(..) | NoteEdit::Duplicate(_)
        );
        let result = match action {
            NoteEdit::Cut(id, at) => self.project.split_note(t, c, id, at).map(|id| {
                self.selected_notes = HashSet::from([id]);
            }),
            NoteEdit::Duplicate(id) => self.project.duplicate_note(t, c, id).map(|id| {
                self.selected_notes = HashSet::from([id]);
            }),
            NoteEdit::Glue(ids) => self.project.glue_notes(t, c, &ids).map(|id| {
                self.selected_notes = HashSet::from([id]);
            }),
            NoteEdit::Delete(id) => self.project.delete_note(t, c, id).map(|_| {
                self.selected_notes.remove(&id);
            }),
            NoteEdit::Add(note) => {
                let id = note.id;
                if let Some(notes) = self.project.midi_notes_mut(t, c) {
                    notes.push(note);
                    self.selected_notes = HashSet::from([id]);
                }
                Ok(())
            }
            NoteEdit::Move(originals, delta, pitch_delta) => {
                let min_start = originals
                    .iter()
                    .map(|n| n.start)
                    .fold(f64::INFINITY, f64::min);
                let min_pitch = originals.iter().map(|n| n.pitch).min().unwrap_or(0) as i16;
                let max_pitch = originals.iter().map(|n| n.pitch).max().unwrap_or(127) as i16;
                let delta = delta.max(-min_start);
                let pitch_delta = pitch_delta.clamp(-min_pitch, 127 - max_pitch);
                if let Some(notes) = self.project.midi_notes_mut(t, c) {
                    for original in originals {
                        if let Some(n) = notes.iter_mut().find(|n| n.id == original.id) {
                            n.start = original.start + delta;
                            n.pitch = (original.pitch as i16 + pitch_delta) as u8;
                        }
                    }
                }
                Ok(())
            }
            NoteEdit::Resize(originals, delta) => {
                if let Some(notes) = self.project.midi_notes_mut(t, c) {
                    for original in originals {
                        if let Some(n) = notes.iter_mut().find(|n| n.id == original.id) {
                            n.length = (original.length + delta).max(0.0625);
                        }
                    }
                }
                Ok(())
            }
            NoteEdit::Velocity(velocity) => {
                let selected = self.selected_notes.clone();
                if let Some(notes) = self.project.midi_notes_mut(t, c) {
                    for n in notes {
                        if selected.contains(&n.id) {
                            n.velocity = velocity;
                        }
                    }
                }
                Ok(())
            }
        };
        if result.is_ok() {
            if extend {
                self.extend_selected_clip();
            }
            self.changed();
        } else {
            self.message(result, "");
        }
    }
}

use std::collections::HashSet;
fn pitch_name(pitch: u8) -> &'static str {
    [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ][(pitch % 12) as usize]
}

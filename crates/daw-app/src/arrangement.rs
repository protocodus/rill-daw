use crate::{
    app::{DawApp, Drag, PopupKind, WorkspaceView},
    theme::*,
};
use daw_core::*;
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

const HEADER: f32 = 176.0;
const ROW: f32 = 84.0;

enum Edit {
    Move(u64, u64, f64),
    Trim(u64, u64, f64, f64),
    Split(u64, u64, f64),
    Duplicate(u64, u64),
    Delete(u64, u64),
    New(u64, f64),
    Mute(u64),
    Solo(u64),
    Collapse(u64),
    DeleteTrack(u64),
    Transfer(u64, u64, u64),
}

impl DawApp {
    pub fn arrangement(&mut self, ui: &mut egui::Ui) {
        self.drop_target = None;
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    daw_ui::slider(&mut self.zoom, 16.0..=128.0)
                        .show_value(false)
                        .text("Zoom"),
                );
                daw_ui::select(
                    "snap",
                    if self.snap == 0.0 {
                        "Snap off".into()
                    } else {
                        format!("Snap 1/{}", (4.0 / self.snap) as u32)
                    },
                )
                .show_ui(ui, |ui| {
                    for (name, value) in [
                        ("Off", 0.0),
                        ("1/4", 1.0),
                        ("1/8", 0.5),
                        ("1/16", 0.25),
                        ("1/32", 0.125),
                    ] {
                        daw_ui::selectable_value(ui, &mut self.snap, value, name);
                    }
                });
            });
        });
        ui.add_space(8.0);
        let end = (self.project.end_beat() + 16.0).max(48.0);
        let content_width = (HEADER + end as f32 * self.zoom).max(ui.available_width() - 12.0);
        let mut edits = Vec::new();
        let mut new_cursor = None;
        let mut popup = None;
        let mut open_editor = false;
        egui::ScrollArea::both()
            .id_salt("arrangement_scroll")
            // Pointer drags belong to clips and track headers. Background drag
            // scrolling can move a hit target between press and release.
            .scroll_source(egui::scroll_area::ScrollSource {
                drag: false,
                ..Default::default()
            })
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (ruler, ruler_response) =
                    ui.allocate_exact_size(Vec2::new(content_width, 32.0), Sense::click());
                let p = ui.painter_at(ruler);
                p.rect_filled(ruler, 0.0, PANEL);
                p.text(
                    ruler.left_top() + Vec2::new(16.0, 16.0),
                    Align2::LEFT_CENTER,
                    "TRACKS",
                    FontId::proportional(10.0),
                    MUTED,
                );
                let origin = ruler.left() + HEADER;
                let first_beat = (((ui.clip_rect().left() - origin) / self.zoom)
                    .floor()
                    .max(0.0)) as usize;
                let last_beat = (((ui.clip_rect().right() - origin) / self.zoom)
                    .ceil()
                    .max(0.0) as usize
                    + 1)
                .min(end.ceil() as usize);
                for beat in first_beat..last_beat {
                    let x = origin + beat as f32 * self.zoom;
                    if beat % self.project.beats_per_bar as usize == 0 {
                        p.text(
                            Pos2::new(x + 5.0, ruler.center().y),
                            Align2::LEFT_CENTER,
                            format!("{}", beat / self.project.beats_per_bar as usize + 1),
                            FontId::monospace(11.0),
                            MUTED,
                        );
                    }
                }
                if self.project.loop_enabled {
                    let rr = Rect::from_min_max(
                        Pos2::new(
                            origin + self.project.loop_start as f32 * self.zoom,
                            ruler.bottom() - 5.0,
                        ),
                        Pos2::new(
                            origin + self.project.loop_end as f32 * self.zoom,
                            ruler.bottom(),
                        ),
                    );
                    p.rect_filled(rr, 0.0, ACCENT.gamma_multiply(0.6));
                }
                if ruler_response.clicked()
                    && let Some(pos) = ruler_response.interact_pointer_pos()
                    && pos.x >= origin
                {
                    new_cursor = Some(self.snapped(((pos.x - origin) / self.zoom) as f64).max(0.0));
                }
                for (index, depth) in crate::routing::visible_tracks(&self.project) {
                    let track = &self.project.tracks[index];
                    let compact = track.collapsed && track.kind != TrackKind::Folder;
                    let row_height = if compact { 44.0 } else { ROW };
                    let (row, row_response) = ui
                        .allocate_exact_size(Vec2::new(content_width, row_height), Sense::click());
                    if !ui.is_rect_visible(row) {
                        continue;
                    }
                    let p = ui.painter_at(row);
                    let selected = self.selected_track == Some(track.id);
                    p.rect_filled(
                        row,
                        0.0,
                        if selected {
                            TRACK_SELECTED
                        } else if index % 2 == 0 {
                            BG
                        } else {
                            TRACK_ALTERNATE
                        },
                    );
                    p.line_segment(
                        [row.left_bottom(), row.right_bottom()],
                        Stroke::new(1.0_f32, LINE),
                    );
                    let header = Rect::from_min_size(row.min, Vec2::new(HEADER - 1.0, row_height));
                    p.rect_filled(header, 0.0, if selected { RAISED } else { PANEL });
                    p.rect_filled(
                        Rect::from_min_size(row.min, Vec2::new(3.0, row_height)),
                        0.0,
                        color(track.color),
                    );
                    p.text(
                        row.min + Vec2::new(14.0 + depth as f32 * 12.0, 16.0),
                        Align2::LEFT_CENTER,
                        format!(
                            "{} {}",
                            if track.kind == TrackKind::Folder {
                                if track.collapsed { "▸" } else { "▾" }
                            } else {
                                ""
                            },
                            track.name
                        ),
                        FontId::proportional(12.0),
                        TEXT,
                    );
                    p.text(
                        row.min + Vec2::new(14.0, 36.0),
                        Align2::LEFT_CENTER,
                        match track.kind {
                            TrackKind::Audio => "AUDIO",
                            TrackKind::Midi => "MIDI / INSTRUMENT",
                            TrackKind::Bus => "BUS / RETURN",
                            TrackKind::Folder => "FOLDER",
                            TrackKind::Automation => "AUTOMATION",
                        },
                        FontId::proportional(9.0),
                        MUTED,
                    );
                    if !compact && track.kind != TrackKind::Automation {
                        for (i, name, active) in [(0, "M", track.mute), (1, "S", track.solo)] {
                            let rect = Rect::from_min_size(
                                row.min + Vec2::new(14.0 + i as f32 * 30.0, 51.0),
                                Vec2::new(24.0, 21.0),
                            );
                            if daw_ui::selectable_at(ui, rect, (track.id, name), active, name)
                                .on_hover_text(if i == 0 { "Mute track" } else { "Solo track" })
                                .clicked()
                            {
                                edits.push(if i == 0 {
                                    Edit::Mute(track.id)
                                } else {
                                    Edit::Solo(track.id)
                                });
                            }
                        }
                        p.text(
                            row.min + Vec2::new(84.0, 62.0),
                            Align2::LEFT_CENTER,
                            format!("{:.1} dB", 20.0 * track.gain.max(0.0001).log10()),
                            FontId::monospace(10.0),
                            MUTED,
                        );
                    }
                    for beat in first_beat..last_beat {
                        let x = origin + beat as f32 * self.zoom;
                        let bar = beat % self.project.beats_per_bar as usize == 0;
                        p.line_segment(
                            [Pos2::new(x, row.top()), Pos2::new(x, row.bottom())],
                            Stroke::new(1.0_f32, if bar { LINE } else { GRID_MINOR }),
                        );
                    }
                    if row_response.clicked() {
                        self.selected_track = Some(track.id);
                        self.selected_clip = None;
                        self.selected_notes.clear();
                        if track.kind == TrackKind::Folder
                            && row_response
                                .interact_pointer_pos()
                                .is_some_and(|p| p.x < origin)
                        {
                            edits.push(Edit::Collapse(track.id));
                        }
                    }
                    row_response.context_menu(|ui| {
                        if track.kind == TrackKind::Automation
                            && daw_ui::button(ui, "Edit automation").clicked()
                        {
                            self.selected_track = Some(track.id);
                            popup = Some(PopupKind::Automation);
                            ui.close();
                        }
                        if daw_ui::button(ui, "Properties").clicked() {
                            self.selected_track = Some(track.id);
                            popup = Some(PopupKind::Inspector);
                            ui.close();
                        }
                        if daw_ui::button(
                            ui,
                            if track.collapsed {
                                "Expand"
                            } else {
                                "Collapse"
                            },
                        )
                        .clicked()
                        {
                            edits.push(Edit::Collapse(track.id));
                            ui.close();
                        }
                        if daw_ui::button(ui, "Delete track").clicked() {
                            edits.push(Edit::DeleteTrack(track.id));
                            ui.close();
                        }
                    });
                    if let Some(lane) = &track.automation {
                        let left = origin.max(ui.clip_rect().left());
                        let right = row.right().min(ui.clip_rect().right());
                        if right > left {
                            let curve_rect = Rect::from_min_max(
                                Pos2::new(left, row.top() + 10.0),
                                Pos2::new(right, row.bottom() - 10.0),
                            );
                            let start = ((left - origin) / self.zoom) as f64;
                            let end = ((right - origin) / self.zoom) as f64;
                            daw_ui::paint_curve(
                                &p,
                                curve_rect,
                                start,
                                end,
                                color(track.color),
                                |beat| lane.value_at(beat).unwrap_or(0.5),
                            );
                            for point in &lane.points {
                                let x = origin + point.beat as f32 * self.zoom;
                                if x >= left && x <= right {
                                    p.circle_filled(
                                        Pos2::new(
                                            x,
                                            curve_rect.bottom() - point.value * curve_rect.height(),
                                        ),
                                        3.0,
                                        color(track.color),
                                    );
                                }
                            }
                        }
                        if row_response.double_clicked() {
                            self.selected_track = Some(track.id);
                            self.selected_clip = None;
                            popup = Some(PopupKind::Automation);
                        }
                    }
                    if row_response.double_clicked()
                        && track.kind == TrackKind::Midi
                        && let Some(pos) = row_response.interact_pointer_pos()
                        && pos.x >= origin
                    {
                        edits.push(Edit::New(
                            track.id,
                            self.snapped(((pos.x - origin) / self.zoom) as f64).max(0.0),
                        ));
                    }
                    for clip in &track.clips {
                        let rect = Rect::from_min_size(
                            Pos2::new(origin + clip.start as f32 * self.zoom, row.top() + 8.0),
                            Vec2::new((clip.length as f32 * self.zoom).max(4.0), row_height - 16.0),
                        );
                        if !ui.is_rect_visible(rect) {
                            continue;
                        }
                        let col = color(track.color);
                        let is_selected = self.selected_clip == Some(clip.id);
                        p.rect_filled(
                            rect,
                            4.0,
                            col.gamma_multiply(if is_selected { 0.32 } else { 0.18 }),
                        );
                        p.rect_filled(
                            Rect::from_min_max(
                                rect.min,
                                Pos2::new(rect.right(), rect.top() + 20.0),
                            ),
                            4.0,
                            col.gamma_multiply(0.35),
                        );
                        p.rect_stroke(
                            rect,
                            4.0,
                            Stroke::new(
                                if is_selected { 1.8_f32 } else { 0.8_f32 },
                                col.gamma_multiply(if is_selected { 1.0 } else { 0.65 }),
                            ),
                            StrokeKind::Inside,
                        );
                        let cp = p.with_clip_rect(rect.shrink(3.0));
                        cp.text(
                            rect.min + Vec2::new(8.0, 10.0),
                            Align2::LEFT_CENTER,
                            &clip.name,
                            FontId::proportional(11.0),
                            col,
                        );
                        if !compact {
                            match &clip.content {
                                ClipContent::Midi { notes } => {
                                    for n in notes {
                                        let start = (n.start - clip.offset).max(0.0);
                                        let stop =
                                            (n.start + n.length - clip.offset).min(clip.length);
                                        if stop <= start {
                                            continue;
                                        }
                                        let x = rect.left() + start as f32 * self.zoom;
                                        let y = rect.bottom()
                                            - 7.0
                                            - (n.pitch as f32 - 36.0).clamp(0.0, 60.0) / 60.0
                                                * 32.0;
                                        cp.rect_filled(
                                            Rect::from_min_size(
                                                Pos2::new(x, y),
                                                Vec2::new(
                                                    ((stop - start) as f32 * self.zoom - 1.0)
                                                        .max(2.0),
                                                    2.5,
                                                ),
                                            ),
                                            1.0,
                                            col.gamma_multiply(0.8),
                                        );
                                    }
                                }
                                ClipContent::Audio { path, .. } => {
                                    if let Some(audio) = self.waveforms.get(path) {
                                        paint_waveform(
                                            &cp,
                                            Rect::from_min_max(
                                                Pos2::new(rect.left(), rect.top() + 23.0),
                                                rect.max,
                                            ),
                                            audio,
                                            clip.offset * 60.0 / self.project.tempo,
                                            clip.length * 60.0 / self.project.tempo,
                                            col,
                                        );
                                    }
                                }
                            }
                        }
                        let response =
                            ui.interact(rect, ui.id().with(clip.id), Sense::click_and_drag());
                        if response.clicked() || response.drag_started() {
                            self.selected_track = Some(track.id);
                            self.selected_clip = Some(clip.id);
                            self.selected_notes.clear();
                        }
                        if response.double_clicked() {
                            self.selected_track = Some(track.id);
                            self.selected_clip = Some(clip.id);
                            open_editor = true;
                        }
                        if response.hovered()
                            && let Some(pos) = response.hover_pos()
                        {
                            if pos.x > rect.right() - 8.0 {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                            } else {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                            }
                        }
                        if response.drag_started() {
                            let resize = response
                                .interact_pointer_pos()
                                .is_some_and(|pos| pos.x > rect.right() - 10.0);
                            self.drag = Some(Drag::Clip {
                                track: track.id,
                                clip: clip.id,
                                start: clip.start,
                                length: clip.length,
                                resize,
                            });
                        }
                        if response.dragged()
                            && let Some(Drag::Clip {
                                track: t,
                                clip: c,
                                start,
                                length,
                                resize,
                            }) = &self.drag
                            && *c == clip.id
                        {
                            let delta = response.total_drag_delta().unwrap_or_default().x as f64
                                / self.zoom as f64;
                            if *resize {
                                edits.push(Edit::Trim(
                                    *t,
                                    *c,
                                    *start,
                                    self.snapped(*length + delta).max(self.snap.max(0.0625)),
                                ));
                            } else {
                                edits.push(Edit::Move(
                                    *t,
                                    *c,
                                    self.snapped(*start + delta).max(0.0),
                                ));
                            }
                        }
                        response.context_menu(|ui| {
                            if daw_ui::button(ui, "Duplicate").clicked() {
                                edits.push(Edit::Duplicate(track.id, clip.id));
                                ui.close();
                            }
                            if daw_ui::button(ui, "Split at cursor").clicked() {
                                edits.push(Edit::Split(track.id, clip.id, self.cursor));
                                ui.close();
                            }
                            if daw_ui::button(ui, "Delete").clicked() {
                                edits.push(Edit::Delete(track.id, clip.id));
                                ui.close();
                            }
                        });
                    }
                    if let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos())
                        && row.contains(pos)
                        && pos.x >= origin
                    {
                        let beat = self.snapped(((pos.x - origin) / self.zoom) as f64).max(0.0);
                        self.drop_target = Some((Some(track.id), beat));
                        if !ui.ctx().input(|i| i.raw.hovered_files.is_empty()) {
                            let x = origin + beat as f32 * self.zoom;
                            p.rect_filled(row, 0.0, ACCENT.gamma_multiply(0.08));
                            p.line_segment(
                                [Pos2::new(x, row.top()), Pos2::new(x, row.bottom())],
                                Stroke::new(3.0_f32, ACCENT),
                            );
                        }
                    }
                    let x = origin + self.cursor as f32 * self.zoom;
                    if x >= origin && x <= row.right() {
                        p.line_segment(
                            [Pos2::new(x, row.top()), Pos2::new(x, row.bottom())],
                            Stroke::new(1.5_f32, ACCENT),
                        );
                    }
                }
                let empty_height = ui.available_height().max(95.0);
                let (empty, response) =
                    ui.allocate_exact_size(Vec2::new(content_width, empty_height), Sense::click());
                let p = ui.painter_at(empty);
                p.text(
                    Pos2::new(origin + 24.0, empty.top() + 34.0),
                    Align2::LEFT_CENTER,
                    "+   Drop audio here to create a track",
                    FontId::proportional(12.0),
                    MUTED,
                );
                if let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos())
                    && empty.contains(pos)
                {
                    self.drop_target = Some((
                        None,
                        self.snapped(((pos.x - origin) / self.zoom) as f64).max(0.0),
                    ));
                }
                if response.double_clicked() {
                    edits.push(Edit::New(0, 0.0));
                }
                if !ui.ctx().input(|i| i.raw.hovered_files.is_empty())
                    && let Some((target, beat)) = self.drop_target
                {
                    let y = empty.top();
                    if target.is_some() {
                        return;
                    }
                    let x = origin + beat as f32 * self.zoom;
                    p.line_segment(
                        [Pos2::new(x, y), Pos2::new(x, y + ROW)],
                        Stroke::new(3.0_f32, ACCENT),
                    );
                }
            });
        if let Some(beat) = new_cursor {
            self.seek(beat);
        }
        if let Some(popup) = popup {
            self.open_popup(popup);
        }
        if open_editor {
            self.open_workspace(WorkspaceView::Clip);
        }
        if ui.input(|i| i.pointer.any_released())
            && let Some(Drag::Clip {
                track,
                clip,
                resize: false,
                ..
            }) = self.drag
            && let Some((Some(target), _)) = self.drop_target
            && target != track
            && self
                .project
                .track(target)
                .zip(self.project.track(track))
                .is_some_and(|(a, b)| a.kind == b.kind)
        {
            edits.push(Edit::Transfer(track, target, clip));
        }
        for edit in edits {
            self.begin_edit();
            let result = match edit {
                Edit::Move(t, c, b) => self.project.move_clip(t, c, b),
                Edit::Trim(t, c, s, l) => self.project.trim_clip(t, c, s, l),
                Edit::Split(t, c, b) => self.project.split_clip(t, c, b).map(|_| ()),
                Edit::Duplicate(t, c) => self.project.duplicate_clip(t, c).map(|_| ()),
                Edit::Delete(t, c) => self.project.delete_clip(t, c).map(|_| ()),
                Edit::DeleteTrack(id) => self.project.delete_track(id).map(|_| {
                    if self.selected_track == Some(id) {
                        self.selected_track = None;
                        self.selected_clip = None;
                        self.selected_notes.clear();
                    }
                }),
                Edit::Transfer(source, target, clip) => {
                    self.project.delete_clip(source, clip).map(|clip| {
                        self.project.track_mut(target).unwrap().clips.push(clip);
                        self.selected_track = Some(target);
                    })
                }
                Edit::Mute(id) => {
                    if let Some(t) = self.project.track_mut(id) {
                        t.mute = !t.mute;
                    }
                    Ok(())
                }
                Edit::Solo(id) => {
                    if let Some(t) = self.project.track_mut(id) {
                        t.solo = !t.solo;
                    }
                    Ok(())
                }
                Edit::Collapse(id) => {
                    if let Some(t) = self.project.track_mut(id) {
                        t.collapsed = !t.collapsed;
                    }
                    Ok(())
                }
                Edit::New(id, start) => {
                    let id = if id == 0 {
                        self.project.add_midi_track("New instrument")
                    } else {
                        id
                    };
                    let clip = Clip::midi("New pattern", start, 4.0, Vec::new());
                    self.selected_clip = Some(clip.id);
                    self.selected_track = Some(id);
                    self.project.track_mut(id).unwrap().clips.push(clip);
                    Ok(())
                }
            };
            if result.is_ok() {
                self.changed();
            } else {
                self.message(result, "");
            }
        }
    }
    pub fn inspector(&mut self, ctx: &egui::Context) {
        let mut open = self.inspector_open;
        if !open {
            return;
        }
        daw_ui::popup("Clip / track")
            .id(egui::Id::new("inspector"))
            .default_width(300.0)
            .open(&mut open)
            .show(ctx, |ui| {
                let Some(id) = self.selected_track else {
                    ui.label(egui::RichText::new("Select a track or clip").color(MUTED));
                    return;
                };
                let folders: Vec<_> = self
                    .project
                    .tracks
                    .iter()
                    .filter(|track| track.kind == TrackKind::Folder && track.id != id)
                    .map(|track| (track.id, track.name.clone()))
                    .collect();
                let mut changed = false;
                let mut parent_change = None;
                if let Some(track) = self.project.track_mut(id) {
                    daw_ui::text_field(ui, &mut track.name, "Track name", 165.0)
                        .changed()
                        .then(|| changed = true);
                    ui.horizontal(|ui| {
                        changed |= daw_ui::color_edit(ui, &mut track.color).changed();
                        changed |= daw_ui::checkbox(
                            ui,
                            &mut track.collapsed,
                            if track.kind == TrackKind::Folder {
                                "Collapse folder"
                            } else {
                                "Compact track"
                            },
                        )
                        .changed();
                    });
                    let mut parent = track.parent_folder;
                    daw_ui::select_labeled(
                        "Folder",
                        parent
                            .and_then(|id| folders.iter().find(|(folder, _)| *folder == id))
                            .map(|(_, name)| name.as_str())
                            .unwrap_or("Top level"),
                    )
                    .show_ui(ui, |ui| {
                        daw_ui::selectable_value(ui, &mut parent, None, "Top level");
                        for (id, name) in &folders {
                            daw_ui::selectable_value(ui, &mut parent, Some(*id), name);
                        }
                    });
                    if parent != track.parent_folder {
                        parent_change = Some(parent);
                    }
                    if track.kind == TrackKind::Automation {
                        changed |=
                            daw_ui::toggle(ui, &mut track.mute, "Bypass automation").changed();
                    } else {
                        ui.add_space(14.0);
                        label(ui, "LEVEL");
                        changed |= ui
                            .add(daw_ui::slider(&mut track.gain, 0.0..=1.5).show_value(false))
                            .changed();
                        ui.label(
                            egui::RichText::new(format!(
                                "{:.1} dB",
                                20.0 * track.gain.max(0.00001).log10()
                            ))
                            .monospace()
                            .color(ACCENT),
                        );
                        label(ui, "PAN");
                        changed |= ui
                            .add(daw_ui::slider(&mut track.pan, -1.0..=1.0).show_value(false))
                            .changed();
                        ui.horizontal(|ui| {
                            changed |= daw_ui::toggle(ui, &mut track.mute, "Mute").changed();
                            changed |= daw_ui::toggle(ui, &mut track.solo, "Solo").changed();
                        });
                        ui.add_space(14.0);
                        label(ui, "PATH LATENCY");
                        changed |= ui
                            .add(
                                daw_ui::number(&mut track.latency_samples)
                                    .range(0..=192_000)
                                    .suffix(" samples"),
                            )
                            .changed();
                    }
                }
                if changed {
                    self.changed();
                }
                if let Some(parent) = parent_change {
                    match self.project.set_parent_folder(id, parent) {
                        Ok(()) => self.changed(),
                        Err(error) => self.message(Err(error), ""),
                    }
                }
                ui.add_space(14.0);
                ui.separator();
                let mut changed = false;
                if let Some(clip) = self.selected_mut() {
                    label(ui, "CLIP");
                    changed |= daw_ui::text_field(ui, &mut clip.name, "Clip name", 165.0).changed();
                    ui.label(
                        egui::RichText::new(format!(
                            "Start  {:.2} beats\nLength  {:.2} beats\nSource offset  {:.2}",
                            clip.start, clip.length, clip.offset
                        ))
                        .small()
                        .color(MUTED),
                    );
                    if let ClipContent::Audio {
                        gain,
                        fade_in,
                        fade_out,
                        ..
                    } = &mut clip.content
                    {
                        label(ui, "CLIP GAIN");
                        changed |= ui
                            .add(daw_ui::slider(gain, 0.0..=2.0).show_value(false))
                            .changed();
                        label(ui, "FADES / BEATS");
                        changed |= ui
                            .add(
                                daw_ui::number(fade_in)
                                    .range(0.0..=clip.length)
                                    .speed(0.05)
                                    .prefix("In  "),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                daw_ui::number(fade_out)
                                    .range(0.0..=clip.length)
                                    .speed(0.05)
                                    .prefix("Out  "),
                            )
                            .changed();
                    }
                }
                if changed {
                    self.changed();
                }
                ui.add_space(14.0);
                label(ui, "LOOP RANGE");
                let mut start = self.project.loop_start;
                let mut end = self.project.loop_end;
                if ui
                    .add(
                        daw_ui::number(&mut start)
                            .speed(0.25)
                            .range(0.0..=(end - 0.25))
                            .prefix("From  "),
                    )
                    .changed()
                {
                    self.begin_edit();
                    self.project.loop_start = start;
                    self.changed();
                }
                if ui
                    .add(
                        daw_ui::number(&mut end)
                            .speed(0.25)
                            .range((start + 0.25)..=10_000.0)
                            .prefix("To  "),
                    )
                    .changed()
                {
                    self.begin_edit();
                    self.project.loop_end = end;
                    self.changed();
                }
            });
        self.inspector_open = open;
    }
}

pub fn paint_waveform(
    p: &egui::Painter,
    rect: Rect,
    audio: &AudioData,
    offset_seconds: f64,
    length_seconds: f64,
    col: egui::Color32,
) {
    let frames = audio.samples.len() / audio.channels;
    let columns = rect.width().min(2000.0) as usize;
    if columns == 0 || frames == 0 {
        return;
    }
    let center = rect.center().y;
    for x in 0..columns {
        let start = ((offset_seconds + length_seconds * x as f64 / columns as f64)
            * audio.sample_rate as f64)
            .max(0.0) as usize;
        let end = ((offset_seconds + length_seconds * (x + 1) as f64 / columns as f64)
            * audio.sample_rate as f64)
            .max(0.0) as usize;
        let mut peak = 0.0_f32;
        let step = ((end.saturating_sub(start)) / 32).max(1);
        for frame in (start..end.min(frames)).step_by(step) {
            peak = peak.max(audio.samples[frame * audio.channels].abs());
        }
        let height = peak.min(1.0) * (rect.height() / 2.0 - 2.0);
        let px = rect.left() + x as f32 / columns as f32 * rect.width();
        p.line_segment(
            [
                Pos2::new(px, center - height),
                Pos2::new(px, center + height),
            ],
            Stroke::new(1.0_f32, col.gamma_multiply(0.8)),
        );
    }
}

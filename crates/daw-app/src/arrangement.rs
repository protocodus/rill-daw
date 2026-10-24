use crate::{
    app::{ChannelDestination, DawApp, Drag, InspectorPage, PopupKind, WorkspaceView},
    theme::*,
};
use daw_core::*;
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

const HEADER: f32 = 248.0;
const ROW: f32 = 96.0;
const COMPACT_ROW: f32 = 72.0;

enum Edit {
    Move(u64, u64, f64),
    Trim(u64, u64, f64, f64),
    Split(u64, u64, f64),
    Duplicate(u64, u64),
    Clone(u64, u64),
    Unlink(u64, u64),
    Delete(u64, u64),
    New(u64, f64),
    Mute(u64),
    Solo(u64),
    Gain(u64, f32),
    Pan(u64, f32),
    Color(u64, [u8; 3]),
    Output(u64, Option<u64>),
    Collapse(u64),
    DeleteTrack(u64),
    Transfer(u64, u64, u64),
}

impl DawApp {
    pub fn arrangement(&mut self, ui: &mut egui::Ui) {
        self.drop_target = None;
        let viewport_id = egui::Id::new("arrangement_timeline_viewport");
        let mut viewport = ui
            .ctx()
            .data_mut(|data| data.get_temp::<daw_ui::TimelineViewport>(viewport_id))
            .unwrap_or(daw_ui::TimelineViewport {
                pixels_per_beat: self.zoom,
                row_height: ROW,
                ..Default::default()
            });
        let supported = [
            daw_ui::EditorTool::Select,
            daw_ui::EditorTool::Pencil,
            daw_ui::EditorTool::Cut,
            daw_ui::EditorTool::Duplicate,
            daw_ui::EditorTool::Erase,
            daw_ui::EditorTool::Cursor,
        ];
        daw_ui::editor_tool_shortcuts(ui, &mut self.tool, &supported);
        let mut fit = false;
        daw_ui::toolbar(ui, |ui| {
            daw_ui::editor_tools(ui, &mut self.tool, &supported);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                fit = daw_ui::timeline_fit_button(ui).clicked();
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
        let end = (self.project.end_beat() + 16.0).max(48.0);
        let visible_tracks = crate::routing::visible_tracks(&self.project);
        let row_units: f32 = visible_tracks
            .iter()
            .map(|(index, _)| {
                let track = &self.project.tracks[*index];
                if track.collapsed && track.kind != TrackKind::Folder {
                    COMPACT_ROW / ROW
                } else {
                    1.0
                }
            })
            .sum();
        let earliest = self
            .project
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .map(|clip| clip.start)
            .reduce(f64::min)
            .unwrap_or(0.0);
        let bounds = daw_ui::TimelineBounds {
            start_beat: earliest,
            end_beat: self.project.end_beat().max(earliest + 0.25),
            start_row: 0.0,
            end_row: row_units.max(1.0),
        };
        let canvas = Rect::from_min_max(
            ui.available_rect_before_wrap().min
                + Vec2::new(
                    HEADER,
                    daw_ui::tokens::TIMELINE_RULER_HEIGHT + ui.spacing().item_spacing.y,
                ),
            ui.available_rect_before_wrap().max - Vec2::new(12.0, 0.0),
        );
        if fit {
            viewport.fit_bounds(canvas, bounds);
        }
        let navigation = daw_ui::TimelinePane::new("arrangement_pane", canvas).navigate(
            ui,
            &mut viewport,
            bounds,
        );
        self.zoom = viewport.pixels_per_beat;
        let content_width = (ui.available_width() - 12.0).max(HEADER + 1.0);
        let mut edits = Vec::new();
        let mut new_cursor = None;
        let mut popup = None;
        let mut channel_target = None;
        let mut channel_controls = Vec::new();
        let mut track_colors = Vec::new();
        let mut instrument_target = None;
        let mut open_editor = false;
        let mut clip_diagnostics = Vec::new();
        let channels: std::collections::HashMap<_, _> = self
            .project
            .tracks
            .iter()
            .filter(|track| {
                matches!(
                    track.kind,
                    TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                )
            })
            .enumerate()
            .map(|(index, track)| (track.id, index + 1))
            .collect();
        let (ruler, ruler_response) = ui.allocate_exact_size(
            Vec2::new(content_width, daw_ui::tokens::TIMELINE_RULER_HEIGHT),
            Sense::click(),
        );
        let p = ui.painter_at(ruler);
        p.rect_filled(ruler, 0.0, palette(ui.ctx()).panel);
        p.text(
            ruler.left_top() + Vec2::new(16.0, 16.0),
            Align2::LEFT_CENTER,
            "TRACKS",
            FontId::proportional(daw_ui::tokens::FONT_CANVAS),
            palette(ui.ctx()).muted,
        );
        let grid_left = ruler.left() + HEADER;
        let origin = grid_left - (viewport.start_beat * self.zoom as f64) as f32;
        let first_beat = (((grid_left - origin) / self.zoom).floor().max(0.0)) as usize;
        let last_beat = (((ui.clip_rect().right() - origin) / self.zoom)
            .ceil()
            .max(0.0) as usize
            + 1)
        .min(end.ceil() as usize);
        daw_ui::paint_timeline_ruler(
            &p,
            Rect::from_min_max(Pos2::new(grid_left, ruler.top()), ruler.max),
            viewport,
            self.project.tempo,
            self.project.beats_per_bar,
        );
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
            p.rect_filled(rr, 0.0, palette(ui.ctx()).accent.gamma_multiply(0.6));
        }
        if ruler_response.clicked()
            && let Some(pos) = ruler_response.interact_pointer_pos()
            && pos.x >= grid_left
        {
            new_cursor = Some(self.snapped(((pos.x - origin) / self.zoom) as f64).max(0.0));
        }
        let scroll = egui::ScrollArea::vertical().id_salt("arrangement_scroll")
            .scroll_source(egui::scroll_area::ScrollSource::SCROLL_BAR)
            .vertical_scroll_offset(viewport.row_offset * viewport.row_height)
            .auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (index, depth) in visible_tracks {
                    let track = &self.project.tracks[index];
                    let compact = track.collapsed && track.kind != TrackKind::Folder;
                    let row_height = viewport.row_height * if compact { COMPACT_ROW / ROW } else { 1.0 };
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
                            palette(ui.ctx()).track_selected
                        } else if index % 2 == 0 {
                            palette(ui.ctx()).bg
                        } else {
                            palette(ui.ctx()).track_alternate
                        },
                    );
                    p.line_segment(
                        [row.left_bottom(), row.right_bottom()],
                        Stroke::new(1.0_f32, palette(ui.ctx()).line),
                    );
                    let header = Rect::from_min_size(row.min, Vec2::new(HEADER - 1.0, row_height));
                    p.rect_filled(header, 0.0, if selected { palette(ui.ctx()).raised } else { palette(ui.ctx()).panel });
                    let mut track_color = track.color;
                    let color_strip = daw_ui::color_strip_at(ui,
                        Rect::from_min_size(row.min, Vec2::new(14.0, row_height)),
                        (track.id, "color"), &format!("Color of {}", track.name), &mut track_color);
                    if color_strip.response.changed() {
                        edits.push(Edit::Color(track.id, track_color));
                    }
                    let rect_json = |rect: Rect| serde_json::json!({"min": [rect.min.x,rect.min.y], "max": [rect.max.x,rect.max.y]});
                    track_colors.push(serde_json::json!({"track":track.id,
                        "rect":rect_json(color_strip.response.rect), "color":track_color,
                        "picker":color_strip.picker.map(rect_json)}));
                    let header_painter = ui.painter_at(header.shrink2(Vec2::new(4.0, 0.0)));
                    let track_label = format!("{} {}", if track.kind == TrackKind::Folder {
                        if track.collapsed { "▸" } else { "▾" }
                    } else { "" }, track.name);
                    daw_ui::text::paint_single_line(&header_painter,
                        Rect::from_min_max(row.min + Vec2::new(14.0 + depth as f32 * 12.0, 0.0),
                            Pos2::new(header.right() - 4.0, row.top() + row_height.min(32.0))),
                        FontId::proportional(daw_ui::tokens::FONT_TRACK), palette(ui.ctx()).text, &track_label);
                    row_response.clone().on_hover_text(&track.name);
                    if row_height >= 44.0 {
                    if let Some(channel) = channels.get(&track.id) {
                        let destination = track.output_bus.and_then(|id| self.project.track(id))
                            .map(|bus| bus.name.as_str()).unwrap_or("Master");
                        let route = format!("Ch {channel} > {destination}");
                        let midi = track.kind == TrackKind::Midi;
                        daw_ui::picker_at(
                            ui,
                            Rect::from_min_size(row.min + Vec2::new(14.0, 25.0),
                                Vec2::new(if midi { 110.0 } else { 226.0 }, 18.0)),
                            (track.id, "output"),
                            egui::RichText::new(&route).size(daw_ui::tokens::FONT_CANVAS).color(palette(ui.ctx()).muted),
                            |ui| {
                                ui.set_min_width(160.0);
                                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                                    if daw_ui::selectable(ui, track.output_bus.is_none(), "Master").clicked() {
                                        if track.output_bus.is_some() {
                                            edits.push(Edit::Output(track.id, None));
                                        }
                                        ui.close();
                                    }
                                    if let Ok(buses) = self.project.eligible_output_buses(track.id) {
                                        for bus in buses {
                                            ui.push_id(bus.id, |ui| {
                                                if daw_ui::selectable(ui, track.output_bus == Some(bus.id), &bus.name).clicked() {
                                                    if track.output_bus != Some(bus.id) {
                                                        edits.push(Edit::Output(track.id, Some(bus.id)));
                                                    }
                                                    ui.close();
                                                }
                                            });
                                        }
                                    }
                                });
                            },
                        ).0.on_hover_text(format!("{route}\nChange output bus"));
                        if midi {
                            let instrument = match &track.instrument {
                                Instrument::None => "Choose instrument…".to_owned(),
                                Instrument::Vst3 { path, class_id } => self.vst_catalog.iter()
                                    .find(|entry| entry.plugin.path == std::path::Path::new(path)
                                        && entry.plugin.class_id == *class_id)
                                    .map(|entry| entry.plugin.name.clone())
                                    .unwrap_or_else(|| std::path::Path::new(path).file_stem()
                                        .unwrap_or_default().to_string_lossy().into_owned()),
                            };
                            if daw_ui::button_at(ui,
                                Rect::from_min_size(row.min + Vec2::new(128.0, 25.0), Vec2::new(112.0, 18.0)),
                                (track.id, "instrument"),
                                egui::RichText::new(&instrument).size(daw_ui::tokens::FONT_CANVAS).color(palette(ui.ctx()).muted),
                            ).on_hover_text(format!("{instrument}\nChoose VST instrument")).clicked() {
                                instrument_target = Some(track.id);
                            }
                        }
                    } else {
                        header_painter.text(
                            row.min + Vec2::new(14.0, 36.0),
                            Align2::LEFT_CENTER,
                            if track.kind == TrackKind::Folder { "FOLDER" } else { "AUTOMATION" },
                            FontId::proportional(daw_ui::tokens::FONT_CANVAS),
                            palette(ui.ctx()).muted,
                        );
                    }
                    }
                    if row_height >= 68.0 && track.kind != TrackKind::Automation {
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
                        if channels.contains_key(&track.id) {
                            let compact_controls = compact || row_height < 92.0;
                            let controls_rect = Rect::from_min_size(
                                row.min + Vec2::new(78.0, 44.0),
                                Vec2::new(162.0, if compact_controls { 24.0 } else { 44.0 }),
                            );
                            let mut gain = track.gain;
                            let mut pan = track.pan;
                            let peaks = self.engine.as_ref()
                                .map(|engine| engine.channel_peaks(track.id)).unwrap_or([0.0; 2]);
                            let controls = daw_ui::channel_controls_at(
                                ui, controls_rect, (track.id, "channel_controls"),
                                &mut gain, &mut pan, peaks, compact_controls,
                            );
                            if controls.gain.changed() {
                                edits.push(Edit::Gain(track.id, gain));
                            }
                            if controls.pan.changed() {
                                edits.push(Edit::Pan(track.id, pan));
                            }
                            channel_controls.push(serde_json::json!({
                                "track": track.id,
                                "gain": {"min": [controls.gain_slider.min.x, controls.gain_slider.min.y], "max": [controls.gain_slider.max.x, controls.gain_slider.max.y]},
                                "pan": {"min": [controls.pan_slider.min.x, controls.pan_slider.min.y], "max": [controls.pan_slider.max.x, controls.pan_slider.max.y]},
                                "meter": {"min": [controls.meter.rect.min.x, controls.meter.rect.min.y], "max": [controls.meter.rect.max.x, controls.meter.rect.max.y]},
                                "peaks": peaks,
                                "compact": compact_controls,
                            }));
                        }
                    }
                    let grid_step = (8.0 / self.zoom).ceil().max(1.0) as usize;
                    for beat in ((first_beat / grid_step) * grid_step..last_beat).step_by(grid_step) {
                        let x = origin + beat as f32 * self.zoom;
                        if x < grid_left { continue; }
                        let bar = beat % self.project.beats_per_bar as usize == 0;
                        p.line_segment(
                            [Pos2::new(x, row.top()), Pos2::new(x, row.bottom())],
                            Stroke::new(1.0_f32, if bar { palette(ui.ctx()).line } else { palette(ui.ctx()).grid_minor }),
                        );
                    }
                    if row_response.clicked() && !navigation.navigating {
                        self.selected_track = Some(track.id);
                        self.selected_clip = None;
                        self.selected_notes.clear();
                        if self.tool == daw_ui::EditorTool::Cursor && let Some(pos) = row_response.interact_pointer_pos()
                            && pos.x >= grid_left {
                            new_cursor = Some(self.snapped(((pos.x - origin) / self.zoom) as f64).max(0.0));
                        }
                        if track.kind == TrackKind::Folder
                            && row_response
                                .interact_pointer_pos()
                                .is_some_and(|p| p.x < grid_left)
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
                            self.selected_clip = None;
                            popup = Some(PopupKind::Inspector);
                            ui.close();
                        }
                        if matches!(track.kind, TrackKind::Audio | TrackKind::Midi | TrackKind::Bus)
                            && daw_ui::button(ui, "Channel").clicked()
                        {
                            channel_target = Some(track.id);
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
                        let left = grid_left.max(ui.clip_rect().left());
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
                                color(track_color),
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
                                        color(track_color),
                                    );
                                }
                            }
                        }
                        if !navigation.navigating && row_response.double_clicked() {
                            self.selected_track = Some(track.id);
                            self.selected_clip = None;
                            popup = Some(PopupKind::Automation);
                        }
                    }
                    if !navigation.navigating && (row_response.double_clicked()
                        || (row_response.clicked() && self.tool == daw_ui::EditorTool::Pencil))
                        && track.kind == TrackKind::Midi
                        && let Some(pos) = row_response.interact_pointer_pos()
                        && pos.x >= grid_left
                    {
                        edits.push(Edit::New(
                            track.id,
                            self.snapped(((pos.x - origin) / self.zoom) as f64).max(0.0),
                        ));
                    }
                    let mut clips: Vec<_> = track.clips.iter().collect();
                    if track.kind == TrackKind::Audio {
                        // Stable sort preserves storage order for equal-start takes.
                        clips.sort_by(|a, b| a.start.total_cmp(&b.start));
                    }
                    for clip in clips {
                        let rect = Rect::from_min_size(
                            Pos2::new(origin + clip.start as f32 * self.zoom, row.top() + 8.0),
                            Vec2::new((clip.length as f32 * self.zoom).max(4.0), (row_height - 16.0).max(1.0)),
                        );
                        if !ui.is_rect_visible(rect) || rect.right() <= grid_left {
                            continue;
                        }
                        let p = p.with_clip_rect(Rect::from_min_max(Pos2::new(grid_left, row.top()), row.max));
                        let col = color(track_color);
                        let is_selected = self.selected_clip == Some(clip.id);
                        if track.kind == TrackKind::Audio {
                            p.rect_filled(rect, 4.0, palette(ui.ctx()).bg);
                        }
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
                        let cp = p.with_clip_rect(rect.shrink(3.0).intersect(Rect::from_min_max(
                            Pos2::new(grid_left, row.top()), row.max)));
                        let title = daw_ui::clip_title(&cp,
                            Rect::from_min_max(rect.min + Vec2::new(8.0, 0.0),
                                Pos2::new(rect.right() - 3.0, rect.top() + rect.height().min(20.0))),
                            FontId::proportional(daw_ui::tokens::FONT_CANVAS), palette(ui.ctx()).text, &clip.name, clip.link_group.is_some());
                        let rect_json = |rect: Rect| serde_json::json!({"min": [rect.min.x,rect.min.y], "max": [rect.max.x,rect.max.y]});
                        clip_diagnostics.push(serde_json::json!({"id":clip.id,"track":track.id,"rect":rect_json(rect),"linked":clip.link_group.is_some(),"link_icon":title.icon.map(rect_json)}));
                        if !compact && row_height >= 44.0 {
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
                                        crate::audio_editor::paint_audio_clip(
                                            &cp,
                                            Rect::from_min_max(
                                                Pos2::new(rect.left(), rect.top() + 23.0),
                                                rect.max,
                                            ),
                                            audio,
                                            clip,
                                            self.project.tempo,
                                            col,
                                        );
                                    }
                                }
                            }
                        }
                        let response =
                            ui.interact(rect.intersect(Rect::from_min_max(Pos2::new(grid_left, row.top()), row.max)),
                                ui.id().with(clip.id), if navigation.navigating { Sense::hover() } else { Sense::click_and_drag() })
                                .on_hover_text(if clip.link_group.is_some() { format!("{} · Linked clip\nEdits and name are shared. Position and trim are independent.", clip.name) } else { clip.name.clone() });
                        if response.clicked() || response.drag_started() {
                            self.selected_track = Some(track.id);
                            self.selected_clip = Some(clip.id);
                            self.selected_notes.clear();
                        }
                        if response.clicked() && !navigation.navigating {
                            let beat = response.interact_pointer_pos().map(|pos| self.snapped(((pos.x - origin) / self.zoom) as f64));
                            match self.tool {
                                daw_ui::EditorTool::Cut => { if let Some(beat) = beat { edits.push(Edit::Split(track.id, clip.id, beat)); } }
                                daw_ui::EditorTool::Duplicate => edits.push(Edit::Duplicate(track.id, clip.id)),
                                daw_ui::EditorTool::Erase => edits.push(Edit::Delete(track.id, clip.id)),
                                daw_ui::EditorTool::Cursor => new_cursor = beat,
                                _ => {}
                            }
                        }
                        if response.double_clicked() && self.tool == daw_ui::EditorTool::Select {
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
                        if response.drag_started() && self.tool == daw_ui::EditorTool::Select {
                            let resize = ui.input(|input| input.pointer.press_origin())
                                .is_some_and(|pos| pos.x > rect.right() - 10.0);
                            self.drag = Some(Drag::Clip {
                                track: track.id,
                                clip: clip.id,
                                start: clip.start,
                                length: clip.length,
                                resize,
                            });
                        }
                        if response.dragged() && self.tool == daw_ui::EditorTool::Select
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
                            if daw_ui::button(ui, "Create linked clone").clicked() {
                                edits.push(Edit::Clone(track.id, clip.id));
                                ui.close();
                            }
                            if clip.link_group.is_some() && daw_ui::button(ui, "Make independent").clicked() {
                                edits.push(Edit::Unlink(track.id, clip.id));
                                ui.close();
                            }
                            if daw_ui::button(ui, "Join clips…").clicked() {
                                self.selected_track = Some(track.id);
                                self.selected_clip = Some(clip.id);
                                popup = Some(PopupKind::Join);
                                ui.close();
                            }
                            if track.kind == TrackKind::Audio && daw_ui::button(ui, "Crossfade…").clicked() {
                                self.selected_track = Some(track.id);
                                self.selected_clip = Some(clip.id);
                                popup = Some(PopupKind::Crossfade);
                                ui.close();
                            }
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
                    if track.kind == TrackKind::Audio && let Ok(schedule) = daw_core::AudioSchedule::new(&track.clips) {
                        for fade in schedule.crossfades {
                            let rect = Rect::from_min_max(
                                Pos2::new(origin + fade.start as f32 * self.zoom, row.top() + 31.0),
                                Pos2::new(origin + fade.end as f32 * self.zoom, row.bottom() - 8.0),
                            );
                            let visible = rect.intersect(Rect::from_min_max(Pos2::new(grid_left, row.top()), row.max));
                            if visible.is_positive() {
                                daw_ui::paint_crossfade(&p.with_clip_rect(visible), rect, daw_ui::CrossfadeCurve {
                                    control1: fade.curve.control1, control2: fade.curve.control2,
                                });
                            }
                        }
                    }
                    if let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos())
                        && row.contains(pos)
                        && pos.x >= grid_left
                    {
                        let beat = self.snapped(((pos.x - origin) / self.zoom) as f64).max(0.0);
                        self.drop_target = Some((Some(track.id), beat));
                        if !ui.ctx().input(|i| i.raw.hovered_files.is_empty()) {
                            let x = origin + beat as f32 * self.zoom;
                            p.rect_filled(row, 0.0, palette(ui.ctx()).accent.gamma_multiply(0.08));
                            p.line_segment(
                                [Pos2::new(x, row.top()), Pos2::new(x, row.bottom())],
                                Stroke::new(3.0_f32, palette(ui.ctx()).accent),
                            );
                        }
                    }
                    let x = origin + self.cursor as f32 * self.zoom;
                    if x >= grid_left && x <= row.right() {
                        p.line_segment(
                            [Pos2::new(x, row.top()), Pos2::new(x, row.bottom())],
                            Stroke::new(1.5_f32, palette(ui.ctx()).accent),
                        );
                    }
                }
                let empty_height = ui.available_height().max(95.0);
                let (empty, response) =
                    ui.allocate_exact_size(Vec2::new(content_width, empty_height), Sense::click());
                let p = ui.painter_at(empty);
                p.text(
                    Pos2::new(grid_left + 24.0, empty.top() + 34.0),
                    Align2::LEFT_CENTER,
                    "+   Drop audio here to create a track",
                    FontId::proportional(12.0),
                    palette(ui.ctx()).muted,
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
                        Stroke::new(3.0_f32, palette(ui.ctx()).accent),
                    );
                }
            });
        viewport.row_offset = scroll.state.offset.y / viewport.row_height;
        ui.ctx().data_mut(|data| {
            data.insert_temp(viewport_id, viewport);
            data.insert_temp(egui::Id::new("timeline_diagnostics"), serde_json::json!({
                "start_beat": viewport.start_beat, "pixels_per_beat": viewport.pixels_per_beat,
                "row_offset": viewport.row_offset, "row_height": viewport.row_height,
                "canvas": {"min": [canvas.min.x, canvas.min.y], "max": [canvas.max.x, canvas.max.y]},
                "bounds": {"start_beat": bounds.start_beat, "end_beat": bounds.end_beat,
                    "start_row": bounds.start_row, "end_row": bounds.end_row},
                "tool": format!("{:?}", self.tool), "cursor": self.cursor, "clips": clip_diagnostics,
                "channels": channel_controls, "track_colors": track_colors,
            }));
        });
        if let Some(beat) = new_cursor {
            self.seek(beat);
        }
        if let Some(popup) = popup {
            self.open_popup(popup);
        }
        if let Some(track) = channel_target {
            self.open_channel(track);
        }
        if let Some(track) = instrument_target {
            self.selected_track = Some(track);
            self.selected_clip = None;
            self.selected_notes.clear();
            self.open_instrument_chooser(true);
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
                Edit::Clone(t, c) => self.project.clone_clip(t, c).map(|id| {
                    self.selected_track = Some(t);
                    self.selected_clip = Some(id);
                    self.selected_notes.clear();
                    self.edit_label = Some("Create linked clone".into());
                }),
                Edit::Unlink(t, c) => self.project.make_clip_independent(t, c).map(|()| {
                    self.selected_track = Some(t);
                    self.selected_clip = Some(c);
                    self.selected_notes.clear();
                    self.edit_label = Some("Make clip independent".into());
                }),
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
                Edit::Gain(id, gain) => {
                    if let Some(track) = self.project.track_mut(id) {
                        track.gain = gain;
                    }
                    Ok(())
                }
                Edit::Pan(id, pan) => {
                    if let Some(track) = self.project.track_mut(id) {
                        track.pan = pan;
                    }
                    Ok(())
                }
                Edit::Color(id, color) => {
                    if let Some(track) = self.project.track_mut(id) {
                        track.color = color;
                        self.edit_label = Some("Change track color".into());
                    }
                    Ok(())
                }
                Edit::Output(id, output) => self.project.set_output(id, output),
                Edit::Collapse(id) => {
                    if let Some(t) = self.project.track_mut(id) {
                        t.collapsed = !t.collapsed;
                    }
                    Ok(())
                }
                Edit::New(id, start) => {
                    let new_track = id == 0;
                    let id = if id == 0 {
                        self.project.add_midi_track("New instrument")
                    } else {
                        id
                    };
                    let clip = Clip::midi("New pattern", start, 4.0, Vec::new());
                    self.selected_clip = Some(clip.id);
                    self.selected_track = Some(id);
                    self.project.track_mut(id).unwrap().clips.push(clip);
                    if new_track {
                        self.open_instrument_chooser(true);
                    }
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
        let selection = (self.selected_track, self.selected().map(|clip| clip.id));
        if selection != self.inspector_selection {
            self.inspector_page = if selection.1.is_some() {
                InspectorPage::Clip
            } else {
                InspectorPage::Track
            };
            self.inspector_selection = selection;
        }
        let has_clip = self.selected().is_some();
        let has_channel = self
            .selected_track
            .and_then(|id| self.project.track(id))
            .is_some_and(|track| {
                matches!(
                    track.kind,
                    TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                )
            });
        if (self.inspector_page == InspectorPage::Clip && !has_clip)
            || (self.inspector_page == InspectorPage::Channel && !has_channel)
        {
            self.inspector_page = InspectorPage::Track;
        }
        let title = match self.inspector_page {
            InspectorPage::Clip => self
                .selected()
                .map(|clip| format!("Properties of {}", clip.name)),
            InspectorPage::Track => self
                .selected_track
                .and_then(|id| self.project.track(id))
                .map(|track| format!("Track · {}", track.name)),
            InspectorPage::Channel => self
                .selected_track
                .and_then(|id| self.project.track(id))
                .map(|track| format!("Channel · {}", track.name)),
        }
        .unwrap_or_else(|| "Properties".into());
        let mut navigated = false;
        daw_ui::popup(title, ctx)
            .id(egui::Id::new("inspector"))
            .default_width(540.0)
            .min_height(420.0)
            .vscroll(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let Some(id) = self.selected_track else {
                    daw_ui::text::note(ui, "Select a track or clip");
                    return;
                };
                ui.horizontal(|ui| {
                    if has_clip {
                        daw_ui::selectable_value(ui, &mut self.inspector_page, InspectorPage::Clip, "Clip");
                    }
                    daw_ui::selectable_value(ui, &mut self.inspector_page, InspectorPage::Track, "Track");
                    if has_channel {
                        daw_ui::selectable_value(ui, &mut self.inspector_page, InspectorPage::Channel, "Channel");
                    }
                });
                ui.separator();
                if self.inspector_page == InspectorPage::Channel {
                    if self.channel_navigation(ui, ChannelDestination::Channel) {
                        navigated = true;
                        return;
                    }
                    daw_ui::scroll_body(ui, "channel_properties_body", |ui| {
                        self.channel_properties(ui, id);
                    });
                    return;
                }
                daw_ui::scroll_body(ui, "properties_body", |ui| {
                    if self.inspector_page == InspectorPage::Clip {
                        let mut changed = false;
                        let linked = self.selected().and_then(|clip| clip.link_group);
                        let count = linked.map(|group| {
                            self.project
                                .tracks
                                .iter()
                                .flat_map(|track| &track.clips)
                                .filter(|clip| clip.link_group == Some(group))
                                .count()
                        });
                        let mut make_independent = false;
                        if let Some(clip) = self.selected_mut() {
                            if let Some(count) = count {
                                ui.horizontal_wrapped(|ui| {
                                    daw_ui::clip_label(ui, &format!("{count} linked instances"), true);
                                    make_independent = daw_ui::button(ui, "Make independent").clicked();
                                });
                                ui.add_space(daw_ui::tokens::SPACE_SM);
                            }
                            daw_ui::form_grid(ui, "clip_properties", |ui| {
                                let label = daw_ui::text::label(ui, "Name");
                                let name = daw_ui::text_field(ui, &mut clip.name, "Clip name", 260.0)
                                    .labelled_by(label.id)
                                    .on_hover_text(if linked.is_some() {
                                        "Shared name · renaming any instance renames every linked clip"
                                    } else {
                                        "Name of this independent clip"
                                    });
                                ctx.data_mut(|data| {
                                    data.insert_temp(egui::Id::new("clip_name_rect"), name.rect)
                                });
                                changed |= name.changed();
                                ui.end_row();
                                daw_ui::text::label(ui, "Start");
                                daw_ui::text::body(ui, format!("Beat {:.2}", clip.start + 1.0));
                                ui.end_row();
                                daw_ui::text::label(ui, "Duration");
                                daw_ui::text::body(ui, format!("{:.2} beats", clip.length));
                                ui.end_row();
                                daw_ui::text::label(ui, "Source offset");
                                daw_ui::text::body(ui, format!("{:.2} beats", clip.offset))
                                    .on_hover_text("Distance from the beginning of the source to this clip's first beat");
                                ui.end_row();
                                if let ClipContent::Audio {
                                    gain,
                                    fade_in,
                                    fade_out,
                                    envelope,
                                    ..
                                } = &mut clip.content
                                {
                                    let fade_length = envelope.map_or(clip.length, |region| region.length);
                                    let label = daw_ui::text::label(ui, "Clip gain");
                                    ui.horizontal(|ui| {
                                        changed |= ui
                                            .add(daw_ui::slider(gain, 0.0..=2.0)
                                                .clamping(egui::SliderClamping::Edits).show_value(false))
                                            .labelled_by(label.id)
                                            .changed();
                                        daw_ui::text::body(ui, if *gain <= 0.0 {
                                            "−∞ dB".to_string()
                                        } else {
                                            format!("{:.1} dB", 20.0 * gain.log10())
                                        });
                                    });
                                    ui.end_row();
                                    for (name, value) in [("Fade in", fade_in), ("Fade out", fade_out)] {
                                        let label = daw_ui::text::label(ui, name);
                                        changed |= ui
                                            .add(daw_ui::number(value)
                                                .range(0.0..=fade_length)
                                                .clamp_existing_to_range(false)
                                                .speed(0.05)
                                                .suffix(" beats"))
                                            .labelled_by(label.id)
                                            .changed();
                                        ui.end_row();
                                    }
                                }
                            });
                        }
                        if changed {
                            self.changed();
                        }
                        if make_independent {
                            self.make_clip_independent();
                        }
                        return;
                    }
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
                        daw_ui::form_grid(ui, "track_properties", |ui| {
                            let label = daw_ui::text::label(ui, "Name");
                            changed |= daw_ui::text_field(ui, &mut track.name, "Track name", 260.0)
                                .labelled_by(label.id).changed();
                            ui.end_row();
                            let label = daw_ui::text::label(ui, "Color");
                            changed |= daw_ui::color_edit(ui, &mut track.color).labelled_by(label.id).changed();
                            ui.end_row();
                            daw_ui::text::label(ui, "Display");
                            changed |= daw_ui::checkbox(ui, &mut track.collapsed, if track.kind == TrackKind::Folder {
                                "Collapse folder"
                            } else {
                                "Compact track"
                            }).changed();
                            ui.end_row();
                            let label = daw_ui::text::label(ui, "Folder");
                            let mut parent = track.parent_folder;
                            daw_ui::select("properties_folder", parent
                                .and_then(|id| folders.iter().find(|(folder, _)| *folder == id))
                                .map(|(_, name)| name.as_str())
                                .unwrap_or("Top level"))
                                .width(260.0).truncate().show_ui(ui, |ui| {
                                    daw_ui::selectable_value(ui, &mut parent, None, "Top level");
                                    for (id, name) in &folders {
                                        daw_ui::selectable_value(ui, &mut parent, Some(*id), name);
                                    }
                                }).response.labelled_by(label.id);
                            ui.end_row();
                            if parent != track.parent_folder {
                                parent_change = Some(parent);
                            }
                            if track.kind == TrackKind::Automation {
                                daw_ui::text::label(ui, "Playback");
                                changed |= daw_ui::toggle(ui, &mut track.mute, "Bypass automation").changed();
                                ui.end_row();
                            } else if track.kind == TrackKind::Folder {
                                daw_ui::text::label(ui, "Playback");
                                ui.horizontal(|ui| {
                                    changed |= daw_ui::toggle(ui, &mut track.mute, "Mute").changed();
                                    changed |= daw_ui::toggle(ui, &mut track.solo, "Solo").changed();
                                });
                                ui.end_row();
                            }
                        });
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
                });
            });
        if !navigated {
            self.inspector_open = open;
        }
    }

    fn channel_properties(&mut self, ui: &mut egui::Ui, id: u64) {
        let mut changed = false;
        if let Some(track) = self.project.track_mut(id) {
            daw_ui::form_grid(ui, "channel_properties", |ui| {
                let label = daw_ui::text::label(ui, "Gain");
                ui.horizontal(|ui| {
                    changed |= ui
                        .add(
                            daw_ui::slider(&mut track.gain, 0.0..=1.5)
                                .clamping(egui::SliderClamping::Edits)
                                .show_value(false),
                        )
                        .labelled_by(label.id)
                        .on_hover_text("Mixer channel gain")
                        .changed();
                    daw_ui::text::body(
                        ui,
                        if track.gain <= 0.0 {
                            "−∞ dB".to_string()
                        } else {
                            format!("{:.1} dB", 20.0 * track.gain.log10())
                        },
                    );
                });
                ui.end_row();
                let label = daw_ui::text::label(ui, "Pan");
                changed |= ui
                    .add(
                        daw_ui::slider(&mut track.pan, -1.0..=1.0)
                            .clamping(egui::SliderClamping::Edits)
                            .fixed_decimals(2),
                    )
                    .labelled_by(label.id)
                    .on_hover_text("Mixer channel pan · Left −1 · Center 0 · Right 1")
                    .changed();
                ui.end_row();
                daw_ui::text::label(ui, "Playback");
                ui.horizontal(|ui| {
                    changed |= daw_ui::toggle(ui, &mut track.mute, "Mute").changed();
                    changed |= daw_ui::toggle(ui, &mut track.solo, "Solo").changed();
                });
                ui.end_row();
                let label = daw_ui::text::label(ui, "Path latency");
                changed |= ui
                    .add(
                        daw_ui::number(&mut track.latency_samples)
                            .range(0..=192_000)
                            .clamp_existing_to_range(false)
                            .suffix(" samples"),
                    )
                    .labelled_by(label.id)
                    .changed();
                ui.end_row();
            });
        }
        if changed {
            self.changed();
        }
    }
}

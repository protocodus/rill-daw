use crate::{
    app::{
        ChannelDestination, ClipCommand, ClipDragMode, DawApp, Drag, InspectorPage, PopupKind,
        WorkspaceView,
    },
    theme::*,
};
use daw_core::*;
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};

const HEADER: f32 = daw_ui::TRACK_HEADER_WIDTH;
const ROW: f32 = daw_ui::TRACK_HEADER_HEIGHT;
const COMPACT_ROW: f32 = daw_ui::TRACK_HEADER_COMPACT_HEIGHT;

enum Edit {
    Select(u64, u64, bool, bool),
    Rename(u64, u64),
    AudioStart(u64, u64, crate::arranger_selection::ArrangeAudioMode, f64),
    AudioMotion(f64),
    GroupStart(u64, bool),
    GroupMove(f64),
    Bounds(u64, u64, std::sync::Arc<Clip>, f64, f64, bool),
    Command(u64, u64, ClipCommand),
    LoopRange(f64, f64),
    New(u64, f64),
    Mute(u64),
    Solo(u64),
    Gain(u64, f32),
    Pan(u64, f32),
    Color(u64, [u8; 3]),
    Output(u64, Option<u64>),
    Collapse(u64),
    DeleteTrack(u64),
    TransferGroup(u64),
    Folder(Option<u64>),
    Parent(u64, Option<u64>),
}

/// A folder cannot be moved into itself or one of its descendants. Filtering
/// choices keeps the chooser understandable; the core validates the edit too.
fn eligible_folders(project: &Project, track: u64) -> Vec<&Track> {
    project
        .tracks
        .iter()
        .filter(|folder| {
            if folder.kind != TrackKind::Folder || folder.id == track {
                return false;
            }
            let mut parent = folder.parent_folder;
            for _ in 0..project.tracks.len() {
                let Some(id) = parent else {
                    return true;
                };
                if id == track {
                    return false;
                }
                parent = project.track(id).and_then(|folder| folder.parent_folder);
            }
            parent.is_none()
        })
        .collect()
}

fn expand_folder_chain(project: &mut Project, mut parent: Option<u64>) {
    for _ in 0..project.tracks.len() {
        let Some(track) = parent.and_then(|id| project.track_mut(id)) else {
            break;
        };
        track.collapsed = false;
        parent = track.parent_folder;
    }
}

fn folder_lanes(project: &Project, visible: &[(usize, usize)]) -> Vec<daw_ui::FolderLane> {
    visible
        .iter()
        .map(|&(index, depth)| {
            let track = &project.tracks[index];
            daw_ui::FolderLane {
                id: track.id,
                depth,
                folder: track.kind == TrackKind::Folder,
                color: color(track.color),
                rect: Rect::NOTHING,
            }
        })
        .collect()
}

fn reveal_track(
    project: &Project,
    target: u64,
    height: f32,
    label_width: f32,
    view: &mut daw_ui::TimelineViewport,
) {
    let visible_rows = (height / view.row_height).max(0.01);
    let visible = crate::routing::visible_tracks(project);
    let lanes = folder_lanes(project, &visible);
    let (step, _) = daw_ui::folder_lane_padding(&lanes, label_width);
    let closures = daw_ui::folder_closures(&lanes);
    let mut top = 0.0;
    for (row_index, (index, _)) in visible.into_iter().enumerate() {
        let track = &project.tracks[index];
        let rows = if track.collapsed && track.kind != TrackKind::Folder {
            COMPACT_ROW / ROW
        } else {
            1.0
        };
        if track.id == target {
            if rows > visible_rows || top < view.row_offset {
                view.row_offset = top;
            } else if top + rows > view.row_offset + visible_rows {
                view.row_offset = (top + rows - visible_rows).max(0.0);
            }
            break;
        }
        top += rows
            + closures[row_index].len() as f32 * daw_ui::folder_footer_spacing(step)
                / view.row_height;
    }
}

impl DawApp {
    pub fn arrangement(&mut self, ui: &mut egui::Ui) {
        self.drop_target = None;
        self.sync_arranger_selection();
        let selected_loop_range = self.loop_selection_range();
        let can_change_loop = !self.recording_busy();
        let viewport_id = egui::Id::new("arrangement_timeline_viewport");
        let mut viewport = ui
            .ctx()
            .data_mut(|data| data.get_temp::<daw_ui::TimelineViewport>(viewport_id))
            .unwrap_or(daw_ui::TimelineViewport {
                pixels_per_beat: self.zoom,
                row_height: ROW,
                ..Default::default()
            });
        let view_before = viewport;
        let mut view_action = daw_ui::TimelineViewAction::None;
        let header_id = egui::Id::new("arrangement_header_width");
        let max_header = (ui.available_width() - 200.0).clamp(224.0, 600.0);
        let mut header_width = ui
            .ctx()
            .data(|data| data.get_temp::<f32>(header_id))
            .unwrap_or(HEADER)
            .clamp(224.0, max_header);
        let supported = [
            daw_ui::EditorTool::Select,
            daw_ui::EditorTool::Pencil,
            daw_ui::EditorTool::Cut,
            daw_ui::EditorTool::Duplicate,
            daw_ui::EditorTool::Glue,
            daw_ui::EditorTool::Ghost,
            daw_ui::EditorTool::Stretch,
            daw_ui::EditorTool::Slip,
            daw_ui::EditorTool::Erase,
            daw_ui::EditorTool::Cursor,
        ];
        if !supported.contains(&self.tool) {
            self.tool = daw_ui::EditorTool::Select;
        }
        daw_ui::editor_tool_shortcuts(ui, &mut self.tool, &supported);
        let mut fit = false;
        let mut nudge = 0;
        let mut add_pattern = false;
        let amplitude_id = egui::Id::new("arrangement_waveform_amplitude");
        let mut waveform_amplitude = ui.ctx().data_mut(|data| {
            data.get_temp::<daw_ui::WaveformAmplitudeScale>(amplitude_id)
                .unwrap_or_default()
        });
        let compact_toolbar = ui.available_width() < 1200.0;
        daw_ui::toolbar(ui, |ui| {
            if compact_toolbar {
                ui.spacing_mut().button_padding.x = daw_ui::tokens::SPACE_SM;
                ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_XS;
            }
            daw_ui::editor_tools_with_context(
                ui,
                &mut self.tool,
                &supported,
                daw_ui::ToolHintContext::Arrangement,
            );
            nudge = daw_ui::nudge_menu(ui, self.selected().is_some());
            if !compact_toolbar {
                ui.add_space(daw_ui::tokens::SPACE_SM);
            }
            add_pattern = daw_ui::quiet_button(ui, "Add pattern")
                .on_hover_text("Add a four-beat MIDI pattern at the cursor; use the selected MIDI track or create one")
                .clicked();
            daw_ui::toolbar_trailing(ui, if compact_toolbar { 384.0 } else { 430.0 }, |ui| {
                daw_ui::editor_tool_help(ui, self.tool, daw_ui::ToolHintContext::Arrangement);
                fit = daw_ui::timeline_fit_button(ui).clicked();
                daw_ui::quiet_view_menu(ui, "View", |ui| {
                    view_action = daw_ui::timeline_selection_view_actions(
                        ui,
                        "arrangement",
                        !self.arranger_selection_ids().is_empty(),
                    );
                    ui.separator();
                    daw_ui::waveform_amplitude_control(ui, &mut waveform_amplitude);
                    crate::spectrogram::view_options(ui);
                    daw_ui::track_dimensions(
                        ui,
                        &mut header_width,
                        &mut viewport.row_height,
                        max_header,
                    );
                });
                daw_ui::select(
                    "snap",
                    if self.snap == 0.0 {
                        "Snap off".into()
                    } else {
                        format!("Snap 1/{}", (4.0 / self.snap) as u32)
                    },
                )
                .width(if compact_toolbar {
                    120.0
                } else {
                    daw_ui::tokens::SELECT_WIDTH
                })
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
        ui.ctx()
            .data_mut(|data| data.insert_temp(amplitude_id, waveform_amplitude));
        if nudge != 0 {
            self.arranger_nudge(ui.ctx(), nudge);
        }
        let content_end = self.project.end_beat().max(if self.recording_busy() {
            self.cursor
        } else {
            0.0
        });
        let end = (content_end + 16.0).max(48.0);
        let visible_tracks = crate::routing::visible_tracks(&self.project);
        let mut folder_rows = folder_lanes(&self.project, &visible_tracks);
        let folder_closures = daw_ui::folder_closures(&folder_rows);
        let (folder_step, folder_padding) = daw_ui::folder_lane_padding(&folder_rows, header_width);
        let folder_footer = daw_ui::folder_footer_spacing(folder_step);
        let folder_footers =
            folder_rows.iter().filter(|row| row.folder).count() as f32 * folder_footer;
        let right_inset = if folder_padding > 0.0 {
            folder_padding + 4.0
        } else {
            0.0
        };
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
            .filter(|clip| !clip.is_take_source())
            .map(|clip| clip.start)
            .reduce(f64::min)
            .unwrap_or(0.0);
        let mut bounds = daw_ui::TimelineBounds {
            start_beat: earliest,
            end_beat: content_end.max(earliest + 0.25),
            start_row: 0.0,
            end_row: (row_units + folder_footers / viewport.row_height).max(1.0),
        };
        let canvas = Rect::from_min_max(
            ui.available_rect_before_wrap().min
                + Vec2::new(
                    header_width,
                    daw_ui::tokens::TIMELINE_LOOP_RULER_HEIGHT + ui.spacing().item_spacing.y,
                ),
            ui.available_rect_before_wrap().max - Vec2::new(12.0 + right_inset, 0.0),
        );
        if let Some((track, clip)) = self.arranger_selection.reveal.take() {
            if let Some(clip) = self.project.clip(track, clip) {
                let width = canvas.width() as f64 / viewport.pixels_per_beat as f64;
                if clip.start < viewport.start_beat {
                    viewport.start_beat = clip.start;
                } else if clip.end() > viewport.start_beat + width {
                    viewport.start_beat = (clip.end() - width).max(0.0);
                }
            }
            reveal_track(
                &self.project,
                track,
                canvas.height(),
                header_width,
                &mut viewport,
            );
        }
        let restored_previous = view_action == daw_ui::TimelineViewAction::Previous
            && daw_ui::restore_previous_timeline_view(ui.ctx(), "arrangement", &mut viewport);
        if view_action == daw_ui::TimelineViewAction::FitSelection {
            let selected = self.arranger_selection_ids();
            let selected_tracks = selected
                .iter()
                .map(|(track, _)| *track)
                .collect::<std::collections::HashSet<_>>();
            let span = self.arranger_phrase_span().or_else(|| {
                selected
                    .iter()
                    .filter_map(|(track, clip)| self.project.clip(*track, *clip))
                    .map(|clip| (clip.start, clip.end()))
                    .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
            });
            // Row heights scale, while recursive folder closing strips keep
            // their physical thickness. Solve those two terms separately.
            let mut row_units_before = 0.0_f32;
            let mut footers_before = 0_usize;
            let mut selected_rows = None::<(f32, f32, usize, usize)>;
            for (row_index, (index, _)) in visible_tracks.iter().enumerate() {
                let track = &self.project.tracks[*index];
                let rows = if track.collapsed && track.kind != TrackKind::Folder {
                    COMPACT_ROW / ROW
                } else {
                    1.0
                };
                if selected_tracks.contains(&track.id) {
                    selected_rows = Some(selected_rows.map_or(
                        (
                            row_units_before,
                            row_units_before + rows,
                            footers_before,
                            footers_before,
                        ),
                        |(first, _, first_footers, _)| {
                            (
                                first,
                                row_units_before + rows,
                                first_footers,
                                footers_before,
                            )
                        },
                    ));
                }
                row_units_before += rows;
                footers_before += folder_closures[row_index].len();
            }
            if let (Some((start, end)), Some((first, last, first_footers, last_footers))) =
                (span, selected_rows)
            {
                viewport.fit_bounds(
                    canvas,
                    daw_ui::TimelineBounds {
                        start_beat: start,
                        end_beat: end,
                        start_row: 0.0,
                        end_row: last - first,
                    },
                );
                let padding = 12.0_f32
                    .min(canvas.width() * 0.1)
                    .min(canvas.height() * 0.1);
                let fixed = (last_footers - first_footers) as f32 * folder_footer;
                viewport.row_height = ((canvas.height() - padding * 2.0 - fixed).max(1.0)
                    / (last - first).max(0.01))
                .max(0.01);
                viewport.row_offset = (first
                    + (first_footers as f32 * folder_footer - padding) / viewport.row_height)
                    .max(0.0);
            }
        }
        bounds.end_row = (row_units + folder_footers / viewport.row_height).max(1.0);
        if fit {
            viewport.fit_bounds(canvas, bounds);
        }
        let navigation = daw_ui::TimelinePane::new("arrangement_pane", canvas)
            .keyboard_active()
            .navigate(ui, &mut viewport, bounds);
        if (fit || navigation.fit) && folder_footers > 0.0 {
            viewport.row_height =
                ((canvas.height() - 24.0 - folder_footers).max(1.0) / row_units.max(1.0)).max(0.01);
            viewport.row_offset = 0.0;
        }
        if fit || navigation.fit {
            // Fit keeps sparse projects at an ordinary track scale; explicit
            // selection fitting and manual dimensions retain their own sizing.
            viewport.row_height = viewport.row_height.min(ROW);
            bounds.end_row = (row_units + folder_footers / viewport.row_height).max(1.0);
        }
        self.zoom = viewport.pixels_per_beat;
        let content_width = (ui.available_width() - 12.0).max(header_width + 1.0);
        let pattern_start = self.snapped(self.cursor).max(0.0);
        let mut edits = Vec::new();
        if add_pattern {
            let track = self
                .selected_track
                .filter(|id| {
                    self.project
                        .track(*id)
                        .is_some_and(|track| track.kind == TrackKind::Midi)
                })
                .unwrap_or(0);
            edits.push(Edit::New(track, pattern_start));
        }
        let mut new_cursor = None;
        let mut automation_navigation = None;
        let mut track_reorder = None;
        let mut track_reorder_preview = None;
        let automation: std::collections::HashMap<_, _> = self
            .project
            .tracks
            .iter()
            .filter_map(|track| track.automation.as_ref())
            .filter_map(|lane| {
                self.automation_control_state(lane.target)
                    .map(|state| (lane.target, state))
            })
            .collect();
        let mut popup = None;
        let mut channel_target = None;
        let mut channel_controls = Vec::new();
        let mut track_colors = Vec::new();
        let mut instrument_target = None;
        let mut recording_target = None;
        let mut recording_controls = Vec::new();
        let mut arm_toggles = Vec::new();
        let mut open_editor = false;
        let mut clip_diagnostics = Vec::new();
        let mut selection_items = Vec::new();
        let mut selection_rows = Vec::new();
        let mut clicked_clip = false;
        let mut clicked_background = false;
        let mut empty_action = None;
        let mut empty_action_rects = Vec::new();
        let mut empty_canvas = Rect::NOTHING;
        let mut track_diagnostics = Vec::new();
        let mut row_resize_diagnostics = Vec::new();
        let mut resized_rows = None;
        let channel_ids: std::collections::HashSet<_> = self
            .project
            .tracks
            .iter()
            .filter(|track| {
                matches!(
                    track.kind,
                    TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                )
            })
            .map(|track| track.id)
            .collect();
        let channels: std::collections::HashMap<_, _> = self
            .project
            .ordered_track_ids()
            .into_iter()
            .filter(|id| channel_ids.contains(id))
            .enumerate()
            .map(|(index, id)| (id, index + 1))
            .collect();
        let (ruler, ruler_response) = ui.allocate_exact_size(
            Vec2::new(content_width, daw_ui::tokens::TIMELINE_LOOP_RULER_HEIGHT),
            Sense::click(),
        );
        let p = ui.painter_at(ruler);
        p.rect_filled(ruler, 0.0, palette(ui.ctx()).panel);
        let selected_folder = self.selected_track.filter(|id| {
            self.project
                .track(*id)
                .is_some_and(|track| track.kind == TrackKind::Folder)
        });
        daw_ui::picker_at(
            ui,
            Rect::from_min_size(ruler.min + Vec2::new(14.0, 3.0), Vec2::new(128.0, 28.0)),
            "arrangement_tracks",
            "Tracks",
            |ui| {
                if daw_ui::button(ui, "Add folder").clicked() {
                    edits.push(Edit::Folder(None));
                    ui.close();
                }
                if ui
                    .add_enabled(
                        selected_folder.is_some(),
                        daw_ui::button_widget("Add subfolder"),
                    )
                    .clicked()
                {
                    edits.push(Edit::Folder(selected_folder));
                    ui.close();
                }
                if let Some(track) = self.selected_track {
                    daw_ui::choice_menu(ui, "Move to folder", |ui| {
                        if daw_ui::selectable(
                            ui,
                            self.project
                                .track(track)
                                .is_some_and(|track| track.parent_folder.is_none()),
                            "Top level",
                        )
                        .clicked()
                        {
                            edits.push(Edit::Parent(track, None));
                            ui.close();
                        }
                        for folder in eligible_folders(&self.project, track) {
                            if daw_ui::selectable(
                                ui,
                                self.project
                                    .track(track)
                                    .is_some_and(|track| track.parent_folder == Some(folder.id)),
                                &folder.name,
                            )
                            .clicked()
                            {
                                edits.push(Edit::Parent(track, Some(folder.id)));
                                ui.close();
                            }
                        }
                    });
                }
            },
        )
        .0
        .on_hover_text("Organize tracks and folders");
        let grid_left = ruler.left() + header_width;
        let origin = grid_left - (viewport.start_beat * self.zoom as f64) as f32;
        let first_beat = (((grid_left - origin) / self.zoom).floor().max(0.0)) as usize;
        let last_beat = (((canvas.right() - origin) / self.zoom).ceil().max(0.0) as usize + 1)
            .min(end.ceil() as usize);
        let label_ruler = Rect::from_min_max(
            Pos2::new(grid_left, ruler.top()),
            Pos2::new(
                canvas.right(),
                ruler.top() + daw_ui::tokens::TIMELINE_RULER_HEIGHT,
            ),
        );
        let loop_lane = Rect::from_min_max(
            Pos2::new(grid_left, label_ruler.bottom()),
            Pos2::new(canvas.right(), ruler.bottom()),
        );
        daw_ui::paint_timeline_ruler(
            &p,
            label_ruler,
            viewport,
            self.project.tempo,
            self.project.beats_per_bar,
        );
        let mut loop_indicator = None;
        if self.project.loop_enabled {
            let rr = Rect::from_min_max(
                Pos2::new(
                    origin + self.project.loop_start as f32 * self.zoom,
                    ruler.bottom() - daw_ui::tokens::TIMELINE_LOOP_STRIP_HEIGHT,
                ),
                Pos2::new(
                    origin + self.project.loop_end as f32 * self.zoom,
                    loop_lane.bottom(),
                ),
            )
            .intersect(loop_lane);
            if rr.is_positive() {
                p.rect_filled(rr, 2.0, palette(ui.ctx()).accent.gamma_multiply(0.6));
                loop_indicator = Some(rr);
            }
        }
        if ruler_response.clicked()
            && let Some(pos) = ruler_response.interact_pointer_pos()
            && pos.x >= grid_left
            && pos.x <= canvas.right()
        {
            new_cursor = Some(self.snapped(((pos.x - origin) / self.zoom) as f64).max(0.0));
        }
        // Folder "After" means after its complete subtree, even when the
        // pointer stays over the folder header. Keep that boundary independent
        // of clipping and of whether its descendants have been painted yet.
        let mut row_tops = Vec::with_capacity(visible_tracks.len());
        let mut folder_after_spans = vec![None; visible_tracks.len()];
        let mut row_top = 0.0;
        for (row_index, (index, _)) in visible_tracks.iter().enumerate() {
            row_tops.push(row_top);
            let track = &self.project.tracks[*index];
            let compact = track.collapsed && track.kind != TrackKind::Folder;
            let bottom =
                row_top + viewport.row_height * if compact { COMPACT_ROW / ROW } else { 1.0 };
            for (order, start) in folder_closures[row_index].iter().enumerate() {
                folder_after_spans[*start] =
                    Some(bottom + (order + 1) as f32 * folder_footer - row_tops[*start]);
            }
            row_top = bottom + folder_closures[row_index].len() as f32 * folder_footer;
        }
        let scroll = egui::ScrollArea::vertical().id_salt("arrangement_scroll")
            .scroll_source(egui::scroll_area::ScrollSource::SCROLL_BAR)
            .vertical_scroll_offset(viewport.row_offset * viewport.row_height)
            .auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (row_index, (index, depth)) in visible_tracks.into_iter().enumerate() {
                    let track = &self.project.tracks[index];
                    let compact = track.collapsed && track.kind != TrackKind::Folder;
                    let row_height = viewport.row_height * if compact { COMPACT_ROW / ROW } else { 1.0 };
                    let (row, mut row_response) = ui
                        .allocate_exact_size(Vec2::new(content_width, row_height), Sense::click());
                    folder_rows[row_index].rect = row;
                    if matches!(track.kind, TrackKind::Audio | TrackKind::Midi) {
                        selection_rows.push((track.id, row));
                    }
                    ui.add_space(folder_closures[row_index].len() as f32 * folder_footer);
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
                    let left_padding = daw_ui::folder_content_inset(depth, track.kind == TrackKind::Folder, folder_step);
                    let header = Rect::from_min_max(row.min + Vec2::new(left_padding, 0.0), Pos2::new(row.left() + header_width - 1.0, row.bottom()));
                    let header_fill = if track.kind == TrackKind::Bus {
                        let fill = daw_ui::bus_header_fill(ui.ctx(), color(track.color));
                        if selected { fill.lerp_to_gamma(palette(ui.ctx()).track_selected, 0.5) } else { fill }
                    } else if selected { palette(ui.ctx()).raised } else { palette(ui.ctx()).panel };
                    p.rect_filled(header, 0.0, header_fill);
                    if track.kind == TrackKind::Folder {
                        p.rect_filled(header.shrink2(Vec2::new(4.0, 4.0)), 3.0,
                            color(track.color).gamma_multiply(0.10));
                    }
                    let mut header_layout = daw_ui::TrackHeaderLayout::new(header);
                    if track.kind == TrackKind::Bus {
                        header_layout = header_layout.with_bus_badge();
                    } else if matches!(track.kind, TrackKind::Audio | TrackKind::Midi) {
                        header_layout = header_layout.with_record_arm();
                    } else if track.kind == TrackKind::Folder {
                        header_layout = header_layout.with_folder_disclosure();
                    }
                    if !channels.contains_key(&track.id) {
                        header_layout = header_layout.without_channel_sections();
                    }
                    header_layout = header_layout.with_depth(depth);
                    daw_ui::paint_track_header_sections(&p, &header_layout);
                    let mut track_color = track.color;
                    let color_strip = daw_ui::color_strip_at(ui, header_layout.color,
                        (track.id, "color"), &format!("Color of {}", track.name), &mut track_color);
                    if color_strip.response.changed() {
                        edits.push(Edit::Color(track.id, track_color));
                    }
                    folder_rows[row_index].color = color(track_color);
                    let rect_json = |rect: Rect| serde_json::json!({"min": [rect.min.x,rect.min.y], "max": [rect.max.x,rect.max.y]});
                    track_colors.push(serde_json::json!({"track":track.id,
                        "rect":rect_json(color_strip.response.rect), "color":track_color,
                        "picker":color_strip.picker.map(rect_json)}));
                    let header_painter = ui.painter_at(header);
                    let bus_badge = header_layout.bus_badge;
                    if let Some(badge) = bus_badge {
                        daw_ui::paint_bus_badge(&header_painter, badge, color(track.color));
                    }
                    let reorder_grip = header_layout.reorder;
                    daw_ui::track_reorder_grip_at(ui, reorder_grip,
                        ui.id().with((track.id, "reorder_track")), track.id);
                    let name_rect = daw_ui::paint_track_header_identity(&header_painter,
                        &header_layout, daw_ui::TrackHeaderIdentity {
                            name: &track.name,
                            icon: match track.kind {
                                TrackKind::Folder => daw_ui::Icon::Folder,
                                TrackKind::Audio => daw_ui::Icon::Waveform,
                                TrackKind::Midi => daw_ui::Icon::Instrument,
                                TrackKind::Bus => daw_ui::Icon::Routing,
                                TrackKind::Automation => daw_ui::Icon::Automation,
                            },
                            tint: color(track.color), folder: track.kind == TrackKind::Folder,
                            collapsed: track.collapsed,
                        });
                    let name_response = ui.interact(name_rect,
                        ui.id().with((track.id, "reorder_name")), Sense::click_and_drag());
                    let name_response = daw_ui::track_reorder_source(ui, &name_response, track.id);
                    // The title retains selection, folder disclosure and reorder;
                    // each section below it owns only its own controls.
                    row_response = row_response.union(name_response);
                    track_diagnostics.push(serde_json::json!({"track":track.id,"depth":depth,
                        "folder":track.kind == TrackKind::Folder,"bus":track.kind == TrackKind::Bus,
                        "bus_badge":bus_badge.map(rect_json),"header_fill":header_fill.to_array(),
                        "left_padding":left_padding,"header":rect_json(header),"name":rect_json(name_rect),
                        "reorder_grip":rect_json(reorder_grip),"reorder_target":rect_json(header),
                        "sections": {
                            "identity":rect_json(header_layout.identity),
                            "mix":header_layout.mix.map(rect_json),"meter":header_layout.meter.map(rect_json),
                            "routing":header_layout.routing.map(rect_json),"device":header_layout.device.map(rect_json),
                            "details":header_layout.details.map(rect_json),"record_arm":header_layout.record_arm.map(rect_json),
                        }}));
                    row_response.clone().on_hover_text(if depth == 0 { track.name.clone() }
                        else { format!("{}\nFolder depth {depth}", track.name) });
                    if let Some(rect) = header_layout.details {
                        let response = daw_ui::track_header_details_at(ui, rect, (track.id, "channel_details"));
                        if response.clicked() {
                            if channels.contains_key(&track.id) { channel_target = Some(track.id); }
                            else {
                                self.selected_track = Some(track.id);
                                self.selected_clip = None;
                                popup = Some(PopupKind::Inspector);
                            }
                        }
                    }
                    if let Some(arm_rect) = header_layout.record_arm {
                        recording_controls.push(serde_json::json!({"track":track.id,"arm":rect_json(arm_rect)}));
                        if daw_ui::record_arm_at(ui, arm_rect, egui::Id::new((track.id,"record_arm")),
                            self.recording.armed.contains(&track.id), !self.recording_busy()).clicked() {
                            arm_toggles.push(track.id);
                        }
                    }
                    if let Some(channel) = channels.get(&track.id) {
                        let destination = track.output_bus.and_then(|id| self.project.track(id))
                            .map(|bus| bus.name.as_str()).unwrap_or("Master");
                        if let Some(rect) = header_layout.routing {
                            let response = daw_ui::track_header_link_at(ui, rect, (track.id, "output"),
                                daw_ui::Icon::Routing, "Routing", destination)
                                .on_hover_text(format!("Channel {channel} → {destination}\nChange output bus"));
                            egui::Popup::menu(&response).show(|ui| {
                                ui.set_min_width(160.0);
                                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                                    if daw_ui::selectable(ui, track.output_bus.is_none(), "Master").clicked() {
                                        if track.output_bus.is_some() { edits.push(Edit::Output(track.id, None)); }
                                        ui.close();
                                    }
                                    if let Ok(buses) = self.project.eligible_output_buses(track.id) {
                                        for bus in buses {
                                            ui.push_id(bus.id, |ui| {
                                                if daw_ui::selectable(ui, track.output_bus == Some(bus.id), &bus.name).clicked() {
                                                    if track.output_bus != Some(bus.id) { edits.push(Edit::Output(track.id, Some(bus.id))); }
                                                    ui.close();
                                                }
                                            });
                                        }
                                    }
                                });
                            });
                        }
                        if let Some(rect) = header_layout.device {
                            match track.kind {
                                TrackKind::Midi => {
                                    let instrument = match &track.instrument {
                                        Instrument::None => "Choose…".to_owned(),
                                        Instrument::Vst3 { path, class_id } => self.vst_catalog.iter()
                                            .find(|entry| entry.plugin.path == std::path::Path::new(path)
                                                && entry.plugin.class_id == *class_id)
                                            .map(|entry| entry.plugin.name.clone())
                                            .unwrap_or_else(|| std::path::Path::new(path).file_stem()
                                                .unwrap_or_default().to_string_lossy().into_owned()),
                                    };
                                    if daw_ui::track_header_link_at(ui, rect, (track.id, "instrument"),
                                        daw_ui::Icon::Instrument, "Instrument", &instrument)
                                        .on_hover_text(format!("{instrument}\nChoose VST instrument")).clicked() {
                                        instrument_target = Some(track.id);
                                    }
                                }
                                TrackKind::Audio => {
                                    let input = match track.audio_input {
                                        Some(AudioInputChannels::Mono { channel }) => format!("Mono {}", channel + 1),
                                        Some(AudioInputChannels::Stereo { left, right }) => format!("Stereo {}–{}", left + 1, right + 1),
                                        None => "None".to_owned(),
                                    };
                                    if daw_ui::track_header_link_at(ui, rect, (track.id, "recording_input"),
                                        daw_ui::Icon::Record, "Input", &input)
                                        .on_hover_text("Choose recording inputs and monitoring").clicked() {
                                        recording_target = Some(track.id);
                                    }
                                }
                                TrackKind::Bus => {
                                    let response = daw_ui::track_header_link_at(ui, rect, (track.id, "channel"),
                                        daw_ui::Icon::Effect, "Channel", "Effects & sends");
                                    if response.clicked() {
                                        channel_target = Some(track.id);
                                    }
                                }
                                _ => {}
                            }
                        }
                    } else if let Some(rect) = header_layout.routing.or(header_layout.mix)
                        && let Some((lane_id, state, _)) = track.automation.as_ref().and_then(|lane| automation.get(&lane.target)).copied() {
                            let response = daw_ui::picker_at(ui, rect,
                                (track.id, "automation_state"),
                                state.rich_label(ui.ctx()).size(daw_ui::tokens::FONT_CAPTION),
                                |ui| { if let Some(action) = daw_ui::automation_menu_contents(ui, lane_id, state) { automation_navigation = Some(action); } }).0;
                            daw_ui::paint_automation_control(ui, &response, state);
                    }
                    if let Some(mix_rect) = header_layout.mix {
                        if channels.contains_key(&track.id) {
                            let gain_automation = automation.get(&AutomationTarget { track: track.id, parameter: AutomationParameter::TrackGain }).copied();
                            let pan_automation = automation.get(&AutomationTarget { track: track.id, parameter: AutomationParameter::TrackPan }).copied();
                            let mut gain = gain_automation.filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                                .map(|(_, _, value)| AutomationParameter::TrackGain.denormalize(value)).unwrap_or(track.gain);
                            let mut pan = pan_automation.filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                                .map(|(_, _, value)| AutomationParameter::TrackPan.denormalize(value)).unwrap_or(track.pan);
                            let controls = daw_ui::track_mix_controls_at(ui, mix_rect, (track.id, "channel_controls"),
                                &mut gain, &mut pan, track.mute, track.solo);
                            if controls.mute.clicked() { edits.push(Edit::Mute(track.id)); }
                            if controls.solo.clicked() { edits.push(Edit::Solo(track.id)); }
                            let peaks = self.engine.as_ref()
                                .map(|engine| engine.channel_peaks(track.id)).unwrap_or([0.0; 2]);
                            let meter_rect = header_layout.meter.expect("channel mix reserves its meter");
                            let mut meter_ui = ui.new_child(egui::UiBuilder::new()
                                .id_salt((track.id, "channel_meter")).max_rect(meter_rect));
                            meter_ui.set_clip_rect(ui.clip_rect().intersect(meter_rect));
                            let meter = daw_ui::stereo_meter(&mut meter_ui, peaks, meter_rect.size())
                                .on_hover_text("Channel output level · post-fader");
                            let mut exact_gain = false;
                            let mut exact_pan = false;
                            for (response, control, state, current, range, unit) in [
                                (&controls.gain, daw_ui::AutomationControl::Gain { track: track.id }, gain_automation,
                                    f64::from(daw_ui::channel_gain_to_db(gain)), -60.0..=f64::from(20.0 * 1.5_f32.log10()), "dB"),
                                (&controls.pan, daw_ui::AutomationControl::Pan { track: track.id }, pan_automation,
                                    f64::from(pan) * 100.0, -100.0..=100.0, "%"),
                            ] {
                                let (action, value) = daw_ui::automatable_parameter_value_context(ui, response, control,
                                    state.map(|(lane, state, _)| (lane, state)), current, range, unit);
                                if let Some(action) = action {
                                    automation_navigation = Some(action);
                                }
                                if let Some(value) = value {
                                    match control {
                                        daw_ui::AutomationControl::Gain { .. } => { gain = daw_ui::channel_gain_from_db(value as f32); exact_gain = true; }
                                        daw_ui::AutomationControl::Pan { .. } => { pan = (value / 100.0) as f32; exact_pan = true; }
                                        daw_ui::AutomationControl::Send { .. } => {}
                                    }
                                }
                            }
                            if controls.gain.changed() || exact_gain {
                                edits.push(Edit::Gain(track.id, gain));
                            }
                            if controls.pan.changed() || exact_pan {
                                edits.push(Edit::Pan(track.id, pan));
                            }
                            channel_controls.push(serde_json::json!({
                                "track": track.id,
                                "gain": {"min": [controls.gain_slider.min.x, controls.gain_slider.min.y], "max": [controls.gain_slider.max.x, controls.gain_slider.max.y]},
                                "pan": {"min": [controls.pan_slider.min.x, controls.pan_slider.min.y], "max": [controls.pan_slider.max.x, controls.pan_slider.max.y]},
                                "gain_scale": controls.gain_scale.as_ref().map(|scale| serde_json::json!({
                                    "min_position": [scale.minimum.x, scale.minimum.y], "max_position": [scale.maximum.x, scale.maximum.y],
                                    "handle_center": [scale.handle.x, scale.handle.y], "divisions": scale.divisions,
                                    "rail": {"min": [scale.rail.min.x, scale.rail.min.y], "max": [scale.rail.max.x, scale.rail.max.y]},
                                    "ticks": scale.ticks.iter().map(|tick| serde_json::json!({"fraction":tick.fraction,"center":[tick.center.x,tick.center.y],"major":tick.major})).collect::<Vec<_>>()
                                })),
                                "pan_scale": controls.pan_scale.as_ref().map(|scale| serde_json::json!({
                                    "min_position": [scale.minimum.x, scale.minimum.y], "max_position": [scale.maximum.x, scale.maximum.y],
                                    "handle_center": [scale.handle.x, scale.handle.y], "divisions": scale.divisions,
                                    "rail": {"min": [scale.rail.min.x, scale.rail.min.y], "max": [scale.rail.max.x, scale.rail.max.y]},
                                    "ticks": scale.ticks.iter().map(|tick| serde_json::json!({"fraction":tick.fraction,"center":[tick.center.x,tick.center.y],"major":tick.major})).collect::<Vec<_>>()
                                })),
                                "meter": {"min": [meter.rect.min.x, meter.rect.min.y], "max": [meter.rect.max.x, meter.rect.max.y]},
                                "peaks": peaks,
                                "gain_readout": rect_json(controls.gain_readout), "pan_readout": rect_json(controls.pan_readout),
                                "mute": rect_json(controls.mute.rect), "solo": rect_json(controls.solo.rect),
                                "compact": header_layout.compact,
                            }));
                        } else if track.kind == TrackKind::Folder {
                            for (index, name, active) in [(0, "M", track.mute), (1, "S", track.solo)] {
                                let rect = Rect::from_min_size(mix_rect.min + Vec2::new(index as f32 * 26.0, 0.0),
                                    Vec2::new(21.0, mix_rect.height().min(34.0)));
                                if daw_ui::selectable_at(ui, rect, (track.id, name), active, name)
                                    .on_hover_text(if index == 0 { "Mute folder" } else { "Solo folder" }).clicked() {
                                    edits.push(if index == 0 { Edit::Mute(track.id) } else { Edit::Solo(track.id) });
                                }
                            }
                        }
                    }
                    let grid_step = (8.0 / self.zoom).ceil().max(1.0) as usize;
                    for beat in ((first_beat / grid_step) * grid_step..last_beat).step_by(grid_step) {
                        let x = origin + beat as f32 * self.zoom;
                        if x < grid_left || x > canvas.right() { continue; }
                        let bar = beat % self.project.beats_per_bar as usize == 0;
                        p.line_segment(
                            [Pos2::new(x, row.top()), Pos2::new(x, row.bottom())],
                            Stroke::new(1.0_f32, if bar { palette(ui.ctx()).line } else { palette(ui.ctx()).grid_minor }),
                        );
                    }
                    let background_double = row_response.clicked() && !navigation.navigating
                        && row_response.interact_pointer_pos().is_some_and(|position| {
                            if position.x < grid_left || position.x > canvas.right() { return false; }
                            clicked_background = true;
                            self.arranger_selection.background_click_tracker.register(track.id, position,
                                ui.input(|input| input.time), self.tool == daw_ui::EditorTool::Select && !ui.input(|input| input.modifiers.shift))
                        });
                    if row_response.clicked() && !navigation.navigating {
                        self.selected_track = Some(track.id);
                        self.selected_clip = None;
                        self.arranger_selection.ids.clear();
                        self.arranger_selection.span = None;
                        self.selected_notes.clear();
                        if self.tool == daw_ui::EditorTool::Cursor && let Some(pos) = row_response.interact_pointer_pos()
                            && pos.x >= grid_left && pos.x <= canvas.right() {
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
                        if track.kind == TrackKind::Midi
                            && daw_ui::button(ui, "Add pattern").clicked()
                        {
                            edits.push(Edit::New(track.id, pattern_start));
                            ui.close();
                        }
                        if track.kind == TrackKind::Folder && daw_ui::button(ui, "Add subfolder").clicked() {
                            edits.push(Edit::Folder(Some(track.id)));
                            ui.close();
                        }
                        daw_ui::choice_menu(ui, "Move to folder", |ui| {
                            if daw_ui::selectable(ui, track.parent_folder.is_none(), "Top level").clicked() {
                                edits.push(Edit::Parent(track.id, None)); ui.close();
                            }
                            for folder in eligible_folders(&self.project, track.id) {
                                if daw_ui::selectable(ui, track.parent_folder == Some(folder.id), &folder.name).clicked() {
                                    edits.push(Edit::Parent(track.id, Some(folder.id))); ui.close();
                                }
                            }
                        });
                        ui.separator();
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
                        let right = canvas.right().min(ui.clip_rect().right());
                        if right > left {
                            let curve_rect = Rect::from_min_max(
                                Pos2::new(left, row.top() + 10.0),
                                Pos2::new(right, row.bottom() - 10.0),
                            );
                            let start = ((left - origin) / self.zoom) as f64;
                            let end = ((right - origin) / self.zoom) as f64;
                            let curve_color = palette(ui.ctx()).automation;
                            daw_ui::paint_curve(
                                &p,
                                curve_rect,
                                start,
                                end,
                                if automation.get(&lane.target).is_some_and(|(_, state, _)| *state != daw_ui::AutomationState::Read) { palette(ui.ctx()).muted } else { curve_color },
                                |beat| lane.value_at(beat).unwrap_or(0.5),
                            );
                            if let Some((_, daw_ui::AutomationState::Overridden, value)) = automation.get(&lane.target) {
                                daw_ui::paint_curve(&p, curve_rect, start, end, palette(ui.ctx()).automation_override, |_| *value);
                            }
                            for point in &lane.points {
                                let x = origin + point.beat as f32 * self.zoom;
                                if x >= left && x <= right {
                                    p.circle_filled(
                                        Pos2::new(
                                            x,
                                            curve_rect.bottom() - point.value * curve_rect.height(),
                                        ),
                                        3.0,
                                        curve_color,
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
                    if !navigation.navigating && (background_double
                        || (row_response.clicked() && self.tool == daw_ui::EditorTool::Pencil))
                        && track.kind == TrackKind::Midi
                        && let Some(pos) = row_response.interact_pointer_pos()
                        && pos.x >= grid_left && pos.x <= canvas.right()
                    {
                        edits.push(Edit::New(
                            track.id,
                            self.snapped(((pos.x - origin) / self.zoom) as f64).max(0.0),
                        ));
                    }
                    if track.kind == TrackKind::Midi && track.clips.is_empty() && row_height >= 32.0 {
                        p.text(
                            Pos2::new(grid_left + 24.0, row.center().y),
                            Align2::LEFT_CENTER,
                            "Double-click to add a pattern",
                            FontId::proportional(daw_ui::tokens::FONT_CANVAS),
                            palette(ui.ctx()).muted,
                        );
                    }
                    let mut clips: Vec<_> = track.clips.iter().filter(|clip| !clip.is_take_source()).collect();
                    if track.kind == TrackKind::Audio {
                        // Stable sort preserves storage order for equal-start takes.
                        clips.sort_by(|a, b| a.start.total_cmp(&b.start));
                    }
                    for clip in clips {
                        let rect = Rect::from_min_size(
                            Pos2::new(origin + clip.start as f32 * self.zoom, row.top() + 8.0),
                            Vec2::new((clip.length as f32 * self.zoom).max(4.0), (row_height - 16.0).max(1.0)),
                        );
                        selection_items.push((track.id, clip.id, rect));
                        if !ui.is_rect_visible(rect) || rect.right() <= grid_left {
                            continue;
                        }
                        let p = p.with_clip_rect(Rect::from_min_max(Pos2::new(grid_left, row.top()), Pos2::new(canvas.right(), row.bottom())));
                        let col = color(track_color);
                        let is_selected = self.arranger_selection.ids.contains(&clip.id);
                        let surface = daw_ui::clip_surface(&p, rect, col, is_selected);
                        let cp = p.with_clip_rect(rect.shrink(3.0).intersect(Rect::from_min_max(
                            Pos2::new(grid_left, row.top()), Pos2::new(canvas.right(), row.bottom()))));
                        let title = daw_ui::clip_title(&p, surface.title,
                            FontId::proportional(daw_ui::tokens::FONT_CANVAS), surface.title_color, &clip.name, clip.link_group.is_some());
                        folder_rows[row_index].color = color(track_color);
                    let rect_json = |rect: Rect| serde_json::json!({"min": [rect.min.x,rect.min.y], "max": [rect.max.x,rect.max.y]});
                        let visible_clip = rect.intersect(Rect::from_min_max(Pos2::new(grid_left, row.top()), Pos2::new(canvas.right(), row.bottom())));
                        clip_diagnostics.push(serde_json::json!({"id":clip.id,"track":track.id,"rect":rect_json(rect),"visible_rect":rect_json(visible_clip),"title_rect":rect_json(surface.title),"content_rect":rect_json(surface.content),"linked":clip.link_group.is_some(),"link_icon":title.icon.map(rect_json)}));
                        if !compact && row_height >= 44.0 {
                            match &clip.content {
                                ClipContent::Midi { notes } => {
                                    let pitches = notes.iter().filter(|note| {
                                        clip.source_to_local(note.start + note.length) > 0.0
                                            && clip.source_to_local(note.start) < clip.length
                                    }).fold(None, |range, note| {
                                        Some(range.map_or((note.pitch, note.pitch), |(low, high): (u8, u8)| {
                                            (low.min(note.pitch), high.max(note.pitch))
                                        }))
                                    }).unwrap_or((60, 72));
                                    for n in notes {
                                        let start = clip.source_to_local(n.start).max(0.0);
                                        let stop =
                                            clip.source_to_local(n.start + n.length).min(clip.length);
                                        if stop <= start {
                                            continue;
                                        }
                                        let left = rect.left() + start as f32 * self.zoom;
                                        let right = rect.left() + stop as f32 * self.zoom;
                                        if right <= cp.clip_rect().left() || left >= cp.clip_rect().right() {
                                            continue;
                                        }
                                        daw_ui::clip_note_preview(
                                            &cp, surface.content,
                                            left..right,
                                            n.pitch, pitches.0..=pitches.1, col,
                                        );
                                    }
                                }
                                ClipContent::Audio { path, .. } => {
                                    if let Some(audio) = self.waveforms.get(path) {
                                        crate::audio_editor::paint_audio_clip(
                                            &cp,
                                            Rect::from_min_max(Pos2::new(rect.left(), surface.content.top()), Pos2::new(rect.right(), surface.content.bottom())),
                                            audio,
                                            clip,
                                            self.project.tempo,
                                            col,
                                            waveform_amplitude,
                                        );
                                    }
                                }
                            }
                        }
                        let response =
                            ui.interact(rect.intersect(Rect::from_min_max(Pos2::new(grid_left, row.top()), Pos2::new(canvas.right(), row.bottom()))),
                                ui.id().with(clip.id), if navigation.navigating { Sense::hover() } else { Sense::click_and_drag() })
                                .on_hover_text(if clip.link_group.is_some() { format!("{} · Ghost clip\nEdits and name are shared. Position and trim are independent.", clip.name) } else { clip.name.clone() });
                        if ui.input(|input| input.pointer.primary_pressed())
                            && ui.input(|input| input.pointer.press_origin()).is_some_and(|position| visible_clip.contains(position)) {
                            self.arranger_selection.press_shift = Some((clip.id, ui.input(|input| input.modifiers.shift)));
                        }
                        let press_shift = self.arranger_selection.press_shift
                            .filter(|(id, _)| *id == clip.id).map_or_else(|| ui.input(|input| input.modifiers.shift), |(_, shift)| shift);
                        let mut editing_fade = false;
                        if is_selected && self.tool == daw_ui::EditorTool::Select && rect.width() >= 28.0
                            && let ClipContent::Audio { fade_in, fade_out, envelope, .. } = &clip.content
                        {
                            let offset = envelope.map_or(0.0, |envelope| envelope.offset);
                            let length = envelope.map_or(clip.source_length(), |envelope| envelope.length);
                            let fades = [
                                (true, ((*fade_in - offset) / clip.source_rate).clamp(0.0, clip.length)),
                                (false, ((length - *fade_out - offset) / clip.source_rate).clamp(0.0, clip.length)),
                            ];
                            for (incoming, local) in fades {
                                let center = Pos2::new((rect.left() + local as f32 * self.zoom).clamp(rect.left() + 7.0, rect.right() - 7.0),
                                    (rect.top() + 26.0).min(rect.bottom() - 7.0));
                                let handle = daw_ui::audio_fade_handle(ui, ui.id().with((clip.id, incoming, "fade")), center,
                                    visible_clip, col, incoming);
                                if let Some(diagnostic) = clip_diagnostics.last_mut() {
                                    diagnostic[if incoming { "fade_in_handle" } else { "fade_out_handle" }] = rect_json(handle.rect);
                                }
                                editing_fade |= handle.drag_started_by(egui::PointerButton::Primary) || handle.dragged_by(egui::PointerButton::Primary);
                                if !navigation.navigating && handle.drag_started_by(egui::PointerButton::Primary) {
                                    let at = ((center.x - origin) / self.zoom) as f64;
                                    edits.push(Edit::AudioStart(track.id, clip.id, crate::arranger_selection::ArrangeAudioMode::Fade(incoming), at));
                                }
                                if !navigation.navigating && handle.dragged_by(egui::PointerButton::Primary) && let Some(position) = handle.interact_pointer_pos() {
                                    let at = ((position.x - origin) / self.zoom) as f64;
                                    edits.push(Edit::AudioMotion(at));
                                }
                            }
                        }
                        if !navigation.navigating && self.tool == daw_ui::EditorTool::Slip && matches!(clip.content, ClipContent::Audio { .. }) {
                            if response.drag_started_by(egui::PointerButton::Primary) && let Some(source_beats) = self.audio_source_beats(clip) {
                                let pressed = ui.input(|input| input.pointer.press_origin()).unwrap_or(rect.center());
                                edits.push(Edit::AudioStart(track.id, clip.id, crate::arranger_selection::ArrangeAudioMode::Slip(source_beats), ((pressed.x - origin) / self.zoom) as f64));
                            }
                            if response.dragged_by(egui::PointerButton::Primary) && let Some(position) = response.interact_pointer_pos() {
                                let at = ((position.x - origin) / self.zoom) as f64;
                                let pressed = ui.input(|input| input.pointer.press_origin()).unwrap_or(position);
                                let pressed_beat = ((pressed.x - origin) / self.zoom) as f64;
                                let at = if ui.input(|input| input.modifiers.shift) { at } else { pressed_beat + self.snapped(at - pressed_beat) };
                                edits.push(Edit::AudioMotion(at));
                            }
                        }
                        if (response.clicked() || response.drag_started()) && self.tool != daw_ui::EditorTool::Cursor {
                            edits.push(Edit::Select(track.id, clip.id, press_shift, response.drag_started()));
                        }
                        if response.clicked() && !navigation.navigating {
                            let beat = response.interact_pointer_pos().map(|pos| self.snapped(((pos.x - origin) / self.zoom) as f64));
                            match self.tool {
                                daw_ui::EditorTool::Cut => { if let Some(beat) = beat { edits.push(Edit::Command(track.id, clip.id, ClipCommand::Split(beat))); } }
                                daw_ui::EditorTool::Duplicate => edits.push(Edit::Command(track.id, clip.id, ClipCommand::Duplicate)),
                                daw_ui::EditorTool::Ghost => edits.push(Edit::Command(track.id, clip.id, ClipCommand::Ghost)),
                                daw_ui::EditorTool::Glue => popup = Some(PopupKind::Join),
                                daw_ui::EditorTool::Erase => edits.push(Edit::Command(track.id, clip.id, ClipCommand::Delete)),
                                daw_ui::EditorTool::Cursor => new_cursor = beat,
                                _ => {}
                            }
                        }
                        clicked_clip |= response.clicked();
                        let double = response.clicked() && response.interact_pointer_pos().is_some_and(|position| {
                            self.arranger_selection.click_tracker.register(clip.id, position, ui.input(|input| input.time),
                                !press_shift && matches!(self.tool, daw_ui::EditorTool::Select | daw_ui::EditorTool::Pencil))
                        });
                        if double {
                            self.selected_track = Some(track.id);
                            self.selected_clip = Some(clip.id);
                            open_editor = true;
                        }
                        if response.hovered()
                            && let Some(pos) = response.hover_pos()
                        {
                            if self.tool == daw_ui::EditorTool::Stretch || pos.x > rect.right() - 8.0 || pos.x < rect.left() + 8.0 {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                            } else {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                            }
                        }
                        if response.drag_started() && !editing_fade && matches!(self.tool, daw_ui::EditorTool::Select | daw_ui::EditorTool::Stretch) {
                            let pressed = ui.input(|input| input.pointer.press_origin()).unwrap_or(rect.center());
                            let edge = 10.0_f32.min(rect.width() / 3.0);
                            let mode = if self.tool == daw_ui::EditorTool::Stretch {
                                ClipDragMode::Stretch
                            } else if pressed.x > rect.right() - edge {
                                ClipDragMode::TrimEnd
                            } else if pressed.x < rect.left() + edge {
                                ClipDragMode::TrimStart
                            } else { ClipDragMode::Move };
                            self.drag = Some(Drag::Clip {
                                track: track.id,
                                clip: clip.id,
                                handle: clip.id,
                                original: std::sync::Arc::new(clip.clone()),
                                mode,
                            });
                            if mode == ClipDragMode::Move {
                                edits.push(Edit::GroupStart(clip.id, press_shift));
                            }
                        }
                        if response.dragged() && !editing_fade && matches!(self.tool, daw_ui::EditorTool::Select | daw_ui::EditorTool::Stretch)
                            && let Some(Drag::Clip {
                                track: t,
                                clip: c,
                                handle,
                                original,
                                mode,
                            }) = &self.drag
                            && *handle == clip.id

                        {
                            let delta = response.total_drag_delta().unwrap_or_default().x as f64
                                / self.zoom as f64;
                            let snap = |beat| if ui.input(|i| i.modifiers.shift) { beat } else { self.snapped(beat) };
                            let minimum = original.length.min(0.0625);
                            match mode {
                                ClipDragMode::Move => edits.push(Edit::GroupMove(self.snapped(original.start + delta) - original.start)),
                                ClipDragMode::TrimStart => {
                                    let earliest = if matches!(original.content, ClipContent::Audio { reverse: true, .. }) {
                                        0.0
                                    } else { (original.start - original.offset / original.source_rate).max(0.0) };
                                    let start = snap(original.start + delta).clamp(earliest, (original.end() - minimum).max(earliest));
                                    let length = if start == original.start { original.length } else { original.end() - start };
                                    edits.push(Edit::Bounds(*t, *c, std::sync::Arc::clone(original), start, length, false));
                                }
                                ClipDragMode::TrimEnd | ClipDragMode::Stretch => {
                                    let mut length = (snap(original.end() + delta) - original.start).max(minimum);
                                    if *mode == ClipDragMode::Stretch {
                                        length = length.clamp(original.source_length() / 4.0, original.source_length() / 0.25);
                                    } else if matches!(original.content, ClipContent::Audio { reverse: true, .. }) {
                                        length = length.min(original.length + original.offset / original.source_rate);
                                    }
                                    edits.push(Edit::Bounds(*t, *c, std::sync::Arc::clone(original), original.start, length, *mode == ClipDragMode::Stretch));
                                }
                            }
                        }
                        response.context_menu(|ui| {
                            if daw_ui::button(ui, if matches!(clip.content, ClipContent::Midi { .. }) {
                                "Edit notes"
                            } else {
                                "Edit audio"
                            }).clicked() {
                                self.selected_track = Some(track.id);
                                self.selected_clip = Some(clip.id);
                                self.selected_notes.clear();
                                open_editor = true;
                                ui.close();
                            }
                            let member = self.arranger_selection.ids.contains(&clip.id);
                            if daw_ui::selection_membership_action(ui, member) {
                                edits.push(Edit::Select(track.id, clip.id, true, false));
                            }
                            if daw_ui::button(ui, "Select only this clip").clicked() {
                                self.selected_track = Some(track.id);
                                self.selected_clip = Some(clip.id);
                                self.arranger_selection.ids.clear();
                                self.arranger_selection.ids.insert(clip.id);
                                self.arranger_selection.span = None;
                                self.selected_notes.clear();
                                ui.close();
                            }
                            if ui.add_enabled(can_change_loop, daw_ui::button_widget(
                                if member { "Loop selection" } else { "Loop clip" },
                            ))
                                .on_hover_text(if member {
                                    "Repeat the selected clips or time range, including rests"
                                } else {
                                    "Repeat this clip without changing the current selection"
                                })
                                .on_disabled_hover_text("Finish recording before changing the loop range")
                                .clicked()
                            {
                                let range = if member { selected_loop_range }
                                    else { Some((clip.start, clip.end())) };
                                if let Some((start, end)) = range {
                                    edits.push(Edit::LoopRange(start, end));
                                }
                                ui.close();
                            }
                            if daw_ui::button(ui, "Rename").clicked() {
                                edits.push(Edit::Rename(track.id, clip.id));
                                ui.close();
                            }
                            if track.kind == TrackKind::Audio && daw_ui::button(ui, "Slip contents").clicked() {
                                self.selected_track = Some(track.id);
                                self.selected_clip = Some(clip.id);
                                self.tool = daw_ui::EditorTool::Slip;
                                ui.close();
                            }
                            ui.separator();
                            if daw_ui::button(ui, "Stretch").clicked() {
                                self.selected_track = Some(track.id);
                                self.selected_clip = Some(clip.id);
                                self.selected_notes.clear();
                                self.tool = daw_ui::EditorTool::Stretch;
                                ui.close();
                            }
                            if daw_ui::button(ui, "Ghost copy")
                                .on_hover_text("Create another clip sharing its name and edits")
                                .clicked() {
                                edits.push(Edit::Command(track.id, clip.id, ClipCommand::Ghost));
                                ui.close();
                            }
                            if clip.link_group.is_some() && daw_ui::button(ui, "Make independent")
                                .on_hover_text("Disconnect this ghost while keeping its current name and edits")
                                .clicked() {
                                edits.push(Edit::Command(track.id, clip.id, ClipCommand::MakeIndependent));
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
                                edits.push(Edit::Command(track.id, clip.id, ClipCommand::Duplicate));
                                ui.close();
                            }
                            if daw_ui::button(ui, "Split at cursor").clicked() {
                                edits.push(Edit::Command(track.id, clip.id, ClipCommand::Split(self.cursor)));
                                ui.close();
                            }
                            if daw_ui::button(ui, "Delete").clicked() {
                                edits.push(Edit::Command(track.id, clip.id, ClipCommand::Delete));
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
                            let visible = rect.intersect(Rect::from_min_max(Pos2::new(grid_left, row.top()), Pos2::new(canvas.right(), row.bottom())));
                            if visible.is_positive() {
                                daw_ui::paint_crossfade(&p.with_clip_rect(visible), rect, daw_ui::CrossfadeCurve {
                                    control1: fade.curve.control1, control2: fade.curve.control2,
                                });
                            }
                        }
                    }
                    if let Some((start, end)) = self.recording_region(track.id) {
                        let rect = Rect::from_min_max(Pos2::new((origin + start as f32 * self.zoom).max(grid_left), row.top()+8.0), Pos2::new((origin + end as f32 * self.zoom).min(canvas.right()), row.bottom()-8.0));
                        daw_ui::paint_recording_region(&p, rect);
                    }
                    if let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos())
                        && row.contains(pos)
                        && pos.x >= grid_left && pos.x <= canvas.right()
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
                    let resize_rect = Rect::from_min_max(
                        Pos2::new(header.left() + 14.0, header.bottom() - 5.0),
                        Pos2::new(header.right() - 5.0, header.bottom()));
                    let mut height = viewport.row_height;
                    let resize_id = ui.id().with((track.id, "all_row_height"));
                    if daw_ui::resize_value_at(ui, resize_id, resize_rect,
                        &mut height, 56.0..=1024.0, daw_ui::ResizeAxis::Vertical,
                        "Drag to resize all track rows").changed() {
                        resized_rows = Some(height);
                    }
                    row_resize_diagnostics.push(serde_json::json!({"track":track.id,"rect":rect_json(resize_rect)}));
                    if !navigation.navigating {
                        let target = daw_ui::track_reorder_target_with_after_edge(ui, header, track.id,
                            daw_ui::TrackReorderAxis::Vertical, track.kind == TrackKind::Folder,
                            folder_after_spans[row_index].map(|span| row.top() + span),
                            |intent| crate::track_order::track_drop_allowed(&self.project, intent));
                        if let Some(preview) = target.preview {
                            let placement = match preview.intent.placement {
                                daw_ui::TrackDropPlacement::Before => "Before",
                                daw_ui::TrackDropPlacement::After => "After",
                                daw_ui::TrackDropPlacement::IntoFolder => "Into",
                            };
                            daw_ui::track_reorder_preview_label(ui, &preview, &format!("{placement} {}", track.name));
                            track_reorder_preview = Some(preview);
                        }
                        if let Some(intent) = target.dropped { track_reorder = Some(intent); }
                    }
                }
                let empty_height = ui.available_height().max(95.0);
                let (empty, response) =
                    ui.allocate_exact_size(Vec2::new(content_width, empty_height), Sense::click());
                empty_canvas = empty.intersect(ui.clip_rect()).intersect(Rect::from_min_max(
                    Pos2::new(grid_left, empty.top()), Pos2::new(canvas.right(), empty.bottom()),
                ));
                let p = ui.painter_at(empty);
                if self.project.tracks.is_empty() {
                    let area = Rect::from_min_max(
                        Pos2::new(grid_left + daw_ui::tokens::SPACE_LG, empty.top() + daw_ui::tokens::SPACE_XL),
                        Pos2::new(canvas.right() - daw_ui::tokens::SPACE_LG, empty.bottom()),
                    );
                    let response = ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                        daw_ui::empty_workspace(ui, "Start a song", "Choose a sound, import a recording, or compose with AI.", &[
                            daw_ui::EmptyWorkspaceAction { id: "instrument", label: "Choose instrument…", description: "Browse installed instruments. Choosing a sound creates its MIDI track.", primary: true },
                            daw_ui::EmptyWorkspaceAction { id: "import", label: "Import audio or MIDI…", description: "Bring a WAV recording or MIDI file into the song. You can also drop files onto the arranger.", primary: false },
                            daw_ui::EmptyWorkspaceAction { id: "ai", label: "Compose with AI", description: "Explore editable starting points in the song assistant. Nothing runs until you choose Send.", primary: false },
                        ])
                    }).inner;
                    empty_action = response.action;
                    empty_action_rects.extend(response.actions.iter().map(|(_, response)| response.interact_rect));
                } else {
                    p.text(
                        Pos2::new(grid_left + 24.0, empty.top() + 34.0),
                        Align2::LEFT_CENTER,
                        "Drop audio here · Double-click for MIDI",
                        FontId::proportional(12.0),
                        palette(ui.ctx()).muted,
                    );
                }
                if let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos())
                    && empty.contains(pos)
                    && pos.x >= grid_left && pos.x <= canvas.right()
                {
                    self.drop_target = Some((
                        None,
                        self.snapped(((pos.x - origin) / self.zoom) as f64).max(0.0),
                    ));
                }
                let background_double = response.clicked() && !navigation.navigating
                    && response.interact_pointer_pos().is_some_and(|position| {
                        if position.x < grid_left || position.x > canvas.right() { return false; }
                        clicked_background = true;
                        self.arranger_selection.background_click_tracker.register(0, position,
                            ui.input(|input| input.time), self.tool == daw_ui::EditorTool::Select && !ui.input(|input| input.modifiers.shift))
                    });
                if background_double {
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
        let selection_canvas = Rect::from_min_max(
            Pos2::new(grid_left, scroll.inner_rect.top()),
            Pos2::new(canvas.right(), scroll.inner_rect.bottom()),
        );
        let pointer = ui.input(|input| {
            (
                input.pointer.primary_pressed(),
                input.pointer.primary_down(),
                input.pointer.primary_released(),
                input.pointer.press_origin(),
                input.pointer.interact_pos(),
                input.modifiers,
            )
        });
        if !navigation.navigating
            && pointer.0
            && let Some(origin) = pointer.3
            && selection_canvas.contains(origin)
            && !empty_action_rects.iter().any(|rect| rect.contains(origin))
            && ui.ctx().layer_id_at(origin) == Some(ui.layer_id())
            && !egui::Popup::is_any_open(ui.ctx())
            && matches!(
                self.tool,
                daw_ui::EditorTool::Select | daw_ui::EditorTool::Cursor
            )
            && (self.tool == daw_ui::EditorTool::Cursor
                || !selection_items
                    .iter()
                    .any(|(_, _, rect)| rect.contains(origin)))
        {
            self.arranger_selection.gesture = Some(crate::arranger_selection::SelectionGesture {
                origin,
                current: origin,
                phrase: self.tool == daw_ui::EditorTool::Cursor,
                additive: if pointer.5.shift && self.tool == daw_ui::EditorTool::Select {
                    self.arranger_selection.ids.clone()
                } else {
                    Default::default()
                },
            });
        }
        if let Some((start, end)) = self.arranger_phrase_span() {
            for (track, row) in &selection_rows {
                if selection_items.iter().any(|(owner, id, _)| {
                    owner == track && self.arranger_selection.ids.contains(id)
                }) {
                    let region = Rect::from_min_max(
                        Pos2::new(origin + start as f32 * self.zoom, row.top()),
                        Pos2::new(origin + end as f32 * self.zoom, row.bottom()),
                    );
                    daw_ui::paint_selection_region(
                        &ui.painter().with_clip_rect(selection_canvas),
                        region,
                    );
                }
            }
        }
        if let Some(mut gesture) = self.arranger_selection.gesture.take() {
            if let Some(position) = pointer.4 {
                gesture.current = position.clamp(selection_canvas.min, selection_canvas.max);
            }
            if gesture.origin.distance(gesture.current) > 3.0 {
                let mut region = Rect::from_two_pos(gesture.origin, gesture.current);
                let span = if gesture.phrase {
                    let a = self
                        .snapped(((region.left() - origin) / self.zoom) as f64)
                        .max(0.0);
                    let b = self
                        .snapped(((region.right() - origin) / self.zoom) as f64)
                        .max(a);
                    region.min.x = origin + a as f32 * self.zoom;
                    region.max.x = origin + b as f32 * self.zoom;
                    Some((a, b))
                } else {
                    None
                };
                let mut hit = gesture.additive.clone();
                hit.extend(
                    selection_items
                        .iter()
                        .filter(|(_, _, rect)| {
                            crate::arranger_selection::selection_hits(*rect, region)
                        })
                        .map(|(_, id, _)| *id),
                );
                let ids = self
                    .project
                    .tracks
                    .iter()
                    .flat_map(|track| {
                        track
                            .clips
                            .iter()
                            .filter(|clip| {
                                !clip.is_take_source()
                                    && hit.contains(&clip.id)
                                    && span.is_none_or(|(start, end)| {
                                        clip.start < end && clip.end() > start
                                    })
                            })
                            .map(|clip| (track.id, clip.id))
                    })
                    .collect::<Vec<_>>();
                self.arranger_select_clips(ids, span);
                daw_ui::paint_selection_region(
                    &ui.painter().with_clip_rect(selection_canvas),
                    region,
                );
            } else if pointer.2 {
                if gesture.phrase {
                    new_cursor = Some(
                        self.snapped(((gesture.current.x - origin) / self.zoom) as f64)
                            .max(0.0),
                    );
                } else if gesture.additive.is_empty() {
                    self.arranger_select_clips([], None);
                }
            }
            if pointer.1 && !pointer.2 {
                self.arranger_selection.gesture = Some(gesture);
            }
        }
        let folder_containers = daw_ui::paint_folder_containers(
            ui.painter(),
            &folder_rows,
            scroll.inner_rect,
            folder_step,
        );
        daw_ui::paint_timeline_cursor(
            ui.painter(),
            ruler.with_min_x(grid_left),
            selection_canvas,
            origin + self.cursor as f32 * self.zoom,
        );
        viewport.row_offset = scroll.state.offset.y / viewport.row_height;
        let reorder_scroll = daw_ui::track_reorder_scroll_delta(
            ui,
            scroll.inner_rect,
            daw_ui::TrackReorderAxis::Vertical,
        );
        if reorder_scroll.y != 0.0 {
            let maximum =
                (bounds.end_row - scroll.inner_rect.height() / viewport.row_height).max(0.0);
            viewport.row_offset =
                (viewport.row_offset - reorder_scroll.y / viewport.row_height).clamp(0.0, maximum);
        }
        if let Some(height) = resized_rows {
            viewport.row_height = height;
        }
        let header_gutter = Rect::from_min_max(
            Pos2::new(grid_left - 8.0, ruler.top()),
            Pos2::new(grid_left - 1.0, scroll.inner_rect.bottom()),
        );
        let header_resize_id = ui.id().with("track_label_width");
        daw_ui::resize_value_at(
            ui,
            header_resize_id,
            header_gutter,
            &mut header_width,
            224.0..=max_header,
            daw_ui::ResizeAxis::Horizontal,
            "Drag to resize track labels",
        );
        if !restored_previous {
            daw_ui::remember_timeline_view(
                ui.ctx(),
                "arrangement",
                view_before,
                viewport,
                fit || navigation.fit || view_action == daw_ui::TimelineViewAction::FitSelection,
            );
        }
        self.zoom = viewport.pixels_per_beat;
        ui.ctx().data_mut(|data| {
            data.insert_temp(viewport_id, viewport);
            data.insert_temp(header_id, header_width);
            let rect_json = |rect: Rect| serde_json::json!({"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]});
            let mut diagnostics = serde_json::json!({
                "start_beat": viewport.start_beat, "pixels_per_beat": viewport.pixels_per_beat,
                "row_offset": viewport.row_offset, "row_height": viewport.row_height,
                "canvas": {"min": [canvas.min.x, canvas.min.y], "max": [canvas.max.x, canvas.max.y]},
                "bounds": {"start_beat": bounds.start_beat, "end_beat": bounds.end_beat,
                    "start_row": bounds.start_row, "end_row": bounds.end_row},
                "tool": format!("{:?}", self.tool), "cursor": self.cursor, "clips": clip_diagnostics,
                "waveform_amplitude": format!("{waveform_amplitude:?}"),
                "channels": channel_controls, "track_colors": track_colors, "recording_controls": recording_controls,
            });
            diagnostics["header_width"] = serde_json::json!(header_width);
            diagnostics["empty_canvas"] = rect_json(empty_canvas);
            diagnostics["header_resize"] = rect_json(header_gutter);
            diagnostics["row_resize"] = serde_json::json!(row_resize_diagnostics);
            diagnostics["tracks"] = serde_json::json!(track_diagnostics);
            diagnostics["reorder_preview"] = serde_json::json!(track_reorder_preview.map(|preview| {
                serde_json::json!({"source":preview.intent.source,"target":preview.intent.target,
                    "placement":format!("{:?}",preview.intent.placement),"indicator":rect_json(preview.indicator)})
            }));
            diagnostics["ruler"] = rect_json(label_ruler);
            diagnostics["loop_lane"] = rect_json(loop_lane);
            diagnostics["loop_indicator"] = serde_json::json!(loop_indicator.map(rect_json));
            diagnostics["folder_padding"] = serde_json::json!(folder_padding);
            diagnostics["folder_containers"] = serde_json::json!(folder_containers.iter().map(|group| serde_json::json!({
                "folder":group.folder,"depth":group.depth,"rect":rect_json(group.rect),"visible":rect_json(group.visible),
                "color":group.color.to_array(),"borders":group.borders.map(|border| border.map(rect_json)),
            })).collect::<Vec<_>>());
            diagnostics["selected_clips"] = serde_json::json!(self.arranger_selection_ids().iter().map(|(_, clip)| *clip).collect::<Vec<_>>());
            diagnostics["selection_span"] = serde_json::json!(self.arranger_phrase_span());
            data.insert_temp(egui::Id::new("timeline_diagnostics"), diagnostics);
        });
        // Contextual utilities prepare from the clicked selection. Apply view
        // state before opening them, while musical edits remain transactional.
        edits.retain(|edit| {
            if let Edit::Select(track, clip, shift, dragging) = edit {
                self.arranger_click_clip(*track, *clip, *shift, *dragging);
                false
            } else {
                true
            }
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
        for track in arm_toggles {
            self.toggle_record_arm(track);
        }
        if let Some(track) = recording_target {
            self.open_recording(track);
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
        if ui.input(|i| i.pointer.primary_released())
            && self.arranger_selection.drag.is_some()
            && let Some((Some(target), _)) = self.drop_target
        {
            edits.push(Edit::TransferGroup(target));
        }
        for edit in edits {
            if !matches!(
                edit,
                Edit::Select(..)
                    | Edit::Rename(..)
                    | Edit::AudioStart(..)
                    | Edit::AudioMotion(..)
                    | Edit::GroupStart(..)
                    | Edit::GroupMove(..)
                    | Edit::TransferGroup(..)
                    | Edit::LoopRange(..)
            ) {
                self.begin_edit();
            }
            let result = match edit {
                Edit::Rename(track, clip) => {
                    self.arranger_select_clips([(track, clip)], None);
                    self.start_inline_rename(ui.ctx());
                    continue;
                }
                Edit::AudioStart(track, clip, mode, origin) => {
                    self.arranger_start_audio_drag(track, clip, mode, origin);
                    continue;
                }
                Edit::AudioMotion(beat) => {
                    self.arranger_edit_audio_drag(beat);
                    continue;
                }
                Edit::Select(track, clip, shift, dragging) => {
                    // Selection is view state and must never enter musical history.
                    self.arranger_click_clip(track, clip, shift, dragging);
                    continue;
                }
                Edit::GroupStart(handle, duplicate) => {
                    self.arranger_start_group_drag(handle, duplicate);
                    continue;
                }
                Edit::GroupMove(delta) => {
                    self.arranger_move_group(delta);
                    continue;
                }
                Edit::Command(t, c, command) => {
                    if !self.arranger_selection.ids.contains(&c) {
                        self.arranger_select_clips([(t, c)], None);
                    }
                    match command {
                        ClipCommand::Duplicate => {
                            self.arranger_duplicate_selection();
                        }
                        ClipCommand::Delete => {
                            self.arranger_delete_selection();
                        }
                        _ => self.apply_clip_command(t, c, command),
                    }
                    continue;
                }
                Edit::LoopRange(start, end) => {
                    self.loop_range(start, end);
                    continue;
                }
                Edit::Bounds(t, c, original, start, length, stretch) => {
                    if self.project.clip(t, c).is_some_and(|clip| {
                        clip.start == start
                            && clip.length == length
                            && (!stretch
                                || clip.source_rate
                                    == original.source_rate * original.length / length)
                    }) {
                        // Egui keeps a drag active while the pointer is still.
                        // Re-enqueuing the same edit would repeatedly cancel
                        // background stretching before it can finish.
                        continue;
                    }
                    let previous = self.project.clip(t, c).cloned();
                    if let Some(clip) = self.project.clip_mut(t, c) {
                        clip.clone_from(&original);
                    }
                    let result = if start == original.start && length == original.length {
                        Ok(())
                    } else if stretch {
                        self.project.stretch_clip(t, c, length)
                    } else {
                        self.project.trim_clip(t, c, start, length)
                    };
                    if result.is_err() {
                        if let (Some(clip), Some(previous)) =
                            (self.project.clip_mut(t, c), previous)
                        {
                            *clip = previous;
                        }
                    } else {
                        self.edit_label =
                            Some(if stretch { "Stretch clip" } else { "Trim clip" }.into());
                    }
                    result
                }
                Edit::DeleteTrack(id) => self.project.delete_track(id).map(|_| {
                    if self.selected_track == Some(id) {
                        self.selected_track = None;
                        self.selected_clip = None;
                        self.selected_notes.clear();
                    }
                }),
                Edit::TransferGroup(target) => {
                    self.arranger_transfer_group(target);
                    continue;
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
                    self.override_automated_parameter(
                        AutomationTarget {
                            track: id,
                            parameter: AutomationParameter::TrackGain,
                        },
                        AutomationParameter::TrackGain.normalize(gain),
                    );
                    Ok(())
                }
                Edit::Pan(id, pan) => {
                    if let Some(track) = self.project.track_mut(id) {
                        track.pan = pan;
                    }
                    self.override_automated_parameter(
                        AutomationTarget {
                            track: id,
                            parameter: AutomationParameter::TrackPan,
                        },
                        AutomationParameter::TrackPan.normalize(pan),
                    );
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
                Edit::Parent(id, parent) => self.project.set_parent_folder(id, parent).map(|_| {
                    expand_folder_chain(&mut self.project, parent);
                    reveal_track(
                        &self.project,
                        id,
                        canvas.height(),
                        header_width,
                        &mut viewport,
                    );
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(viewport_id, viewport));
                    self.selected_track = Some(id);
                    self.selected_clip = None;
                    self.selected_notes.clear();
                    self.edit_label = Some("Move track to folder".into());
                }),
                Edit::Folder(parent) => {
                    let id = self.project.add_folder(if parent.is_some() {
                        "New subfolder"
                    } else {
                        "New folder"
                    });
                    let result = self.project.set_parent_folder(id, parent);
                    if result.is_ok() {
                        expand_folder_chain(&mut self.project, parent);
                        reveal_track(
                            &self.project,
                            id,
                            canvas.height(),
                            header_width,
                            &mut viewport,
                        );
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(viewport_id, viewport));
                        self.selected_track = Some(id);
                        self.selected_clip = None;
                        self.selected_notes.clear();
                        self.edit_label = Some(
                            if parent.is_some() {
                                "Add subfolder"
                            } else {
                                "Add folder"
                            }
                            .into(),
                        );
                    } else {
                        let _ = self.project.delete_track(id);
                    }
                    result
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
                    self.edit_label = Some("Add pattern".into());
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
        self.apply_automation_navigation(automation_navigation);
        if let Some(intent) = track_reorder {
            self.apply_track_reorder(intent);
        }
        if ui.input(|input| input.pointer.primary_released()) {
            self.arranger_selection.drag = None;
            self.arranger_selection.audio_drag = None;
            self.arranger_selection.press_shift = None;
            if !clicked_clip {
                self.arranger_selection.click_tracker.clear();
            }
            if !clicked_background {
                self.arranger_selection.background_click_tracker.clear();
            }
        }
        match empty_action.as_deref() {
            Some("instrument") => self.open_instrument_chooser(false),
            Some("import") => self.import_dialog(),
            Some("ai") => self.open_assistant_examples(),
            _ => {}
        }
    }
    pub(crate) fn apply_automation_navigation(
        &mut self,
        action: Option<daw_ui::AutomationNavigation>,
    ) {
        match action {
            Some(daw_ui::AutomationNavigation::Create(control)) => {
                let (track, parameter) = match control {
                    daw_ui::AutomationControl::Gain { track } => {
                        (track, AutomationParameter::TrackGain)
                    }
                    daw_ui::AutomationControl::Pan { track } => {
                        (track, AutomationParameter::TrackPan)
                    }
                    daw_ui::AutomationControl::Send { track, bus } => {
                        (track, AutomationParameter::SendGain { bus })
                    }
                };
                let target = AutomationTarget { track, parameter };
                let existing = self
                    .project
                    .automation_for_target(target)
                    .map(|(lane, _)| lane);
                let result = if let Some(lane) = existing {
                    Ok(lane)
                } else {
                    let source = self
                        .project
                        .track(track)
                        .map(|track| track.name.as_str())
                        .unwrap_or("Channel");
                    let parameter_name = if let AutomationParameter::SendGain { bus } = parameter {
                        format!(
                            "Send · {}",
                            self.project
                                .track(bus)
                                .map(|bus| bus.name.as_str())
                                .unwrap_or("Bus")
                        )
                    } else {
                        parameter.name().to_owned()
                    };
                    let name = format!("{parameter_name} · {source}");
                    self.flush_pending_edit();
                    self.begin_edit();
                    self.project.add_automation_track(name, target)
                };
                match result {
                    Ok(lane) => {
                        self.open_popup(PopupKind::Automation);
                        self.selected_track = Some(lane);
                        self.selected_clip = None;
                        if existing.is_none() {
                            self.edit_label = Some("Create automation".into());
                            self.changed();
                        }
                    }
                    Err(error) => self.message(Err(error), ""),
                }
            }
            Some(daw_ui::AutomationNavigation::Edit(lane)) => {
                self.open_popup(PopupKind::Automation);
                self.selected_track = Some(lane);
                self.selected_clip = None;
            }
            Some(daw_ui::AutomationNavigation::Resume(lane)) => self.resume_automation(lane),
            Some(daw_ui::AutomationNavigation::Override(lane)) => {
                if let Some(value) = self
                    .project
                    .automation_lane(lane)
                    .and_then(|lane| lane.effective_value_at(self.cursor))
                {
                    self.begin_edit();
                    if self
                        .project
                        .set_automation_override(lane, Some(value))
                        .is_ok()
                    {
                        self.edit_label = Some("Override automation".into());
                        self.changed();
                    }
                }
            }
            None => {}
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
        let mut automation_navigation = None;
        daw_ui::popup(title, ctx)
            .id(egui::Id::new("inspector"))
            .default_width(540.0)
            .min_height(daw_ui::popup_body_size(ctx).y.min(420.0))
            .vscroll(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let Some(id) = self.selected_track else {
                    daw_ui::text::note(ui, "Select a track or clip");
                    return;
                };
                let ai_scope = match self.inspector_page {
                    InspectorPage::Clip => self.selected_clip_ai_scope(),
                    InspectorPage::Channel => Some(daw_tools::ToolScope::Channel { track_id: id }),
                    InspectorPage::Track => None,
                };
                let mut page = self.inspector_page;
                let open_ai = daw_ui::action_row(ui, |ui| {
                    ai_scope.is_some_and(|scope| daw_ui::contextual_ai_button(ui, &scope.label(&self.project)).clicked())
                }, |ui| {
                    if has_clip {
                        daw_ui::selectable_value(ui, &mut page, InspectorPage::Clip, "Clip");
                    }
                    daw_ui::selectable_value(ui, &mut page, InspectorPage::Track, "Track");
                    if has_channel {
                        daw_ui::selectable_value(ui, &mut page, InspectorPage::Channel, "Channel");
                    }
                }).inner.0;
                self.inspector_page = page;
                ui.separator();
                if open_ai && let Some(scope) = ai_scope {
                    self.open_context_assistant(scope);
                    navigated = true;
                    return;
                }
                if self.inspector_page == InspectorPage::Channel {
                    if self.channel_navigation(ui, ChannelDestination::Channel) {
                        navigated = true;
                        return;
                    }
                    daw_ui::scroll_body(ui, "channel_properties_body", |ui| {
                        automation_navigation = self.channel_properties(ui, id);
                    });
                    return;
                }
                daw_ui::scroll_body(ui, "properties_body", |ui| {
                    if self.inspector_page == InspectorPage::Clip {
                        let mut changed = false;
                        let mut timing_edit = daw_ui::ClipTimingEdit::default();
                        let maximum_source_offset = self.selected().map_or(MAX_BEAT, |clip| {
                            if let ClipContent::Audio { path, .. } = &clip.content {
                                self.waveforms.get(path).map_or(MAX_BEAT - clip.source_length(), |source| {
                                    (source.duration_seconds() * self.project.tempo / 60.0 - clip.source_length())
                                        .max(0.0).min(MAX_BEAT - clip.source_length())
                                })
                            } else {
                                MAX_BEAT - clip.source_length()
                            }
                        });
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
                                    let label = if count == 1 { "1 ghost".into() } else { format!("{count} ghosts") };
                                    daw_ui::clip_label(ui, &label, true);
                                    make_independent = daw_ui::button(ui, "Make independent")
                                        .on_hover_text("Disconnect this ghost while keeping its current name and edits")
                                        .clicked();
                                });
                                ui.add_space(daw_ui::tokens::SPACE_SM);
                            }
                            daw_ui::form_grid(ui, "clip_properties", |ui| {
                                let label = daw_ui::text::label(ui, "Name");
                                let name = daw_ui::text_field(ui, &mut clip.name, "Clip name", 260.0)
                                    .labelled_by(label.id)
                                    .on_hover_text(if linked.is_some() {
                                        "Shared name · renaming any ghost renames the whole group"
                                    } else {
                                        "Name of this independent clip"
                                    });
                                ctx.data_mut(|data| {
                                    data.insert_temp(egui::Id::new("clip_name_rect"), name.rect)
                                });
                                changed |= name.changed();
                                ui.end_row();
                                let duration_mode_id = egui::Id::new(("properties_duration_mode", clip.id));
                                let mut duration_mode = ctx.data(|data| data.get_temp::<daw_ui::DurationEditMode>(duration_mode_id)).unwrap_or_default();
                                timing_edit = daw_ui::clip_timing_fields(ui, daw_ui::ClipTimingValues {
                                    start: clip.start, duration: clip.length, source_offset: clip.offset,
                                }, &mut duration_mode, MAX_BEAT, maximum_source_offset);
                                ctx.data_mut(|data| data.insert_temp(duration_mode_id, duration_mode));
                                if clip.source_rate != 1.0 {
                                    daw_ui::text::label(ui, "Stretch");
                                    daw_ui::text::body(ui, format!("{:.1}%", 100.0 / clip.source_rate))
                                        .on_hover_text("Duration relative to the source; local to this occurrence");
                                    ui.end_row();
                                }
                                if let ClipContent::Audio {
                                    gain,
                                    processing,
                                    fade_in,
                                    fade_out,
                                    envelope,
                                    ..
                                } = &mut clip.content
                                {
                                    daw_ui::text::label(ui, "Pitch & time");
                                    changed |= crate::audio_editor::audio_processing_menu(ui, processing);
                                    ui.end_row();
                                    let fade_length = envelope.map_or(clip.length * clip.source_rate, |region| region.length) / clip.source_rate;
                                    let label = daw_ui::text::label(ui, "Clip gain");
                                    ui.horizontal(|ui| {
                                        changed |= ui
                                            .add(daw_ui::slider(gain, 0.0..=2.0)
                                                .zero_reference(1.0)
                                                .scale_labels(false)
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
                                        let mut audible = *value / clip.source_rate;
                                        if ui
                                            .add(daw_ui::number(&mut audible)
                                                .range(0.0..=fade_length)
                                                .clamp_existing_to_range(false)
                                                .speed(0.05)
                                                .suffix(" beats"))
                                            .labelled_by(label.id)
                                            .changed() {
                                                *value = audible * clip.source_rate;
                                                changed = true;
                                            }
                                        ui.end_row();
                                    }
                                }
                            });
                        }
                        if timing_edit.new_gesture {
                            self.history.seal_group();
                        }
                        if timing_edit.changed() {
                            let result = self.apply_precise_clip_timing(timing_edit);
                            match result {
                                Ok(()) => changed = true,
                                Err(error) => self.message(Err(error), ""),
                            }
                        }
                        if changed {
                            self.changed();
                        }
                        if make_independent {
                            self.make_clip_independent();
                        }
                        return;
                    }
                    let folders: Vec<_> = eligible_folders(&self.project, id)
                        .into_iter()
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
        self.apply_automation_navigation(automation_navigation);
    }

    fn channel_properties(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
    ) -> Option<daw_ui::AutomationNavigation> {
        let mut changed = false;
        let mut navigation = None;
        let gain_target = AutomationTarget {
            track: id,
            parameter: AutomationParameter::TrackGain,
        };
        let pan_target = AutomationTarget {
            track: id,
            parameter: AutomationParameter::TrackPan,
        };
        let gain_automation = self.automation_control_state(gain_target);
        let pan_automation = self.automation_control_state(pan_target);
        let mut overrides = Vec::new();
        if let Some(track) = self.project.track_mut(id) {
            daw_ui::form_grid(ui, "channel_properties", |ui| {
                let label = daw_ui::text::label(ui, "Gain");
                daw_ui::parameter_row(ui, |ui| {
                    let mut gain = gain_automation
                        .filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                        .map(|(_, _, value)| AutomationParameter::TrackGain.denormalize(value))
                        .unwrap_or(track.gain);
                    let response = ui
                        .add(
                            daw_ui::slider(&mut gain, 0.0..=1.5)
                                .zero_reference(1.0)
                                .clamping(egui::SliderClamping::Edits)
                                .endpoint_labels("−∞ dB", "+3.5 dB")
                                .show_value(false),
                        )
                        .labelled_by(label.id)
                        .on_hover_text("Mixer channel gain");
                    let mut db = daw_ui::channel_gain_to_db(gain);
                    let numeric = ui.add(daw_ui::channel_gain_number(&mut db));
                    if numeric.changed() {
                        gain = daw_ui::channel_gain_from_db(db);
                    }
                    let response = response.union(numeric);
                    let (action, exact) = daw_ui::automatable_parameter_value_context(
                        ui,
                        &response,
                        daw_ui::AutomationControl::Gain { track: id },
                        gain_automation.map(|(lane, state, _)| (lane, state)),
                        f64::from(db),
                        -60.0..=f64::from(20.0 * 1.5_f32.log10()),
                        "dB",
                    );
                    if let Some(action) = action {
                        navigation = Some(action);
                    }
                    if let Some(value) = exact {
                        gain = daw_ui::channel_gain_from_db(value as f32);
                    }
                    if response.changed() || exact.is_some() {
                        track.gain = gain;
                        changed = true;
                        overrides
                            .push((gain_target, AutomationParameter::TrackGain.normalize(gain)));
                    }
                });
                ui.end_row();
                {
                    daw_ui::text::label(ui, "Gain automation");
                    let state = gain_automation.map(|(lane, state, _)| (lane, state));
                    daw_ui::action_menu(
                        ui,
                        state
                            .map(|(_, state)| state.rich_label(ui.ctx()))
                            .unwrap_or_else(|| egui::RichText::new("Automation")),
                        |ui| {
                            if let Some(action) = daw_ui::automatable_menu_contents(
                                ui,
                                daw_ui::AutomationControl::Gain { track: id },
                                state,
                            ) {
                                navigation = Some(action);
                            }
                        },
                    );
                    ui.end_row();
                }
                let label = daw_ui::text::label(ui, "Pan");
                let mut pan = pan_automation
                    .filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                    .map(|(_, _, value)| AutomationParameter::TrackPan.denormalize(value))
                    .unwrap_or(track.pan);
                let mut pan_percent = pan * 100.0;
                let response = ui
                    .add(
                        daw_ui::slider(&mut pan_percent, -100.0..=100.0)
                            .clamping(egui::SliderClamping::Edits)
                            .endpoint_labels("100% L", "100% R")
                            .suffix("%")
                            .min_decimals(1)
                            .max_decimals(6),
                    )
                    .labelled_by(label.id)
                    .on_hover_text("Mixer channel pan · −100% left · 0% center · 100% right");
                if response.changed() {
                    pan = pan_percent / 100.0;
                }
                let (action, exact) = daw_ui::automatable_parameter_value_context(
                    ui,
                    &response,
                    daw_ui::AutomationControl::Pan { track: id },
                    pan_automation.map(|(lane, state, _)| (lane, state)),
                    f64::from(pan) * 100.0,
                    -100.0..=100.0,
                    "%",
                );
                if let Some(action) = action {
                    navigation = Some(action);
                }
                if let Some(value) = exact {
                    pan = (value / 100.0) as f32;
                }
                if response.changed() || exact.is_some() {
                    track.pan = pan;
                    changed = true;
                    overrides.push((pan_target, AutomationParameter::TrackPan.normalize(pan)));
                }
                ui.end_row();
                {
                    daw_ui::text::label(ui, "Pan automation");
                    let state = pan_automation.map(|(lane, state, _)| (lane, state));
                    daw_ui::action_menu(
                        ui,
                        state
                            .map(|(_, state)| state.rich_label(ui.ctx()))
                            .unwrap_or_else(|| egui::RichText::new("Automation")),
                        |ui| {
                            if let Some(action) = daw_ui::automatable_menu_contents(
                                ui,
                                daw_ui::AutomationControl::Pan { track: id },
                                state,
                            ) {
                                navigation = Some(action);
                            }
                        },
                    );
                    ui.end_row();
                }
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
        for (target, value) in overrides {
            self.override_automated_parameter(target, value);
        }
        if changed {
            self.changed();
        }
        navigation
    }
}

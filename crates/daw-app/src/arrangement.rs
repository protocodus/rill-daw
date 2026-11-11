use crate::{
    app::{ClipCommand, ClipDragMode, DawApp, Drag, InspectorPage, PopupKind, WorkspaceView},
    theme::*,
};
use daw_core::*;
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};

const HEADER: f32 = daw_ui::TRACK_HEADER_WIDTH;
const ROW: f32 = daw_ui::TRACK_HEADER_HEIGHT;
const COMPACT_ROW: f32 = daw_ui::TRACK_HEADER_COMPACT_HEIGHT;

/// Height of a visible row in units of the adjustable row height. A folder
/// shows only its identity line, so it keeps one physical height at any zoom.
fn track_row_units(track: &Track, row_height: f32) -> f32 {
    if track.kind == TrackKind::Folder {
        daw_ui::TRACK_HEADER_FOLDER_HEIGHT / row_height.max(1.0)
    } else if track.collapsed {
        // A global compact view should not make an individually collapsed
        // channel lose its basic mix controls. Fit can still go smaller when
        // the user deliberately asks to see an entire large arrangement.
        (row_height * COMPACT_ROW / ROW).max(56.0).min(row_height) / row_height.max(1.0)
    } else {
        1.0
    }
}

enum Edit {
    Select(u64, u64, bool, bool),
    SelectOnly(u64, u64),
    Rename(u64, u64),
    AudioStart(u64, u64, crate::arranger_selection::ArrangeAudioMode, f64),
    AudioMotion(f64),
    GroupStart(u64, bool),
    GroupMove(f64),
    Bounds(u64, u64, std::sync::Arc<Clip>, f64, f64, bool),
    Command(u64, u64, ClipCommand),
    Erase(u64, u64),
    LoopRange(f64, f64),
    Freeze(u64, Option<Vec<u64>>),
    Unfreeze(u64),
    MidiTransform(u64, u64, daw_ui::MidiClipAction),
    ClipMuted(u64, u64, bool),
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
    Fades(u64, u64, f64, f64, FadeCurve, FadeCurve),
    Crossfade(u64, u64, Option<AudioCrossfade>),
    Automation(u64, crate::arranger_automation::AutomationEdit),
    Folder(Option<u64>),
    Parent(u64, Option<u64>),
}

#[derive(Clone, Copy)]
struct ClipEdgeDrag {
    handle: egui::Id,
    clip: u64,
    pointer_start: f32,
    pixels_per_beat: f32,
    moved: bool,
    frame: u64,
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
    expanded: &std::collections::HashSet<u64>,
) {
    let visible_rows = (height / view.row_height).max(0.01);
    let visible = crate::track_order::visible_tracks_with_expanded(project, expanded);
    let lanes = folder_lanes(project, &visible);
    let (step, _) = daw_ui::folder_lane_padding(&lanes, label_width);
    let closures = daw_ui::folder_closures(&lanes);
    let mut top = 0.0;
    for (row_index, (index, _)) in visible.into_iter().enumerate() {
        let track = &project.tracks[index];
        let rows = track_row_units(track, view.row_height);
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
        let audio_file_drop = ui.input(|input| {
            input
                .raw
                .hovered_files
                .iter()
                .filter_map(|file| file.path.as_ref())
                .chain(
                    input
                        .raw
                        .dropped_files
                        .iter()
                        .filter_map(|file| file.path.as_ref()),
                )
                .any(|path| {
                    path.extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| {
                            ["wav", "mp3", "flac", "ogg"]
                                .iter()
                                .any(|supported| extension.eq_ignore_ascii_case(supported))
                        })
                })
        });
        self.sync_arranger_selection();
        // A popup can dismiss in the menu/toolbar code before clip responses
        // are read. Keep its entry/previous-frame state so that same pointer
        // gesture cannot also start a trim on the newly uncovered canvas.
        let edge_popup_id = egui::Id::new("arrangement_clip_edge_popup_open");
        let edge_popup_was_open = egui::Popup::is_any_open(ui.ctx())
            || ui
                .ctx()
                .data(|data| data.get_temp::<bool>(edge_popup_id))
                .unwrap_or(false);
        // Native event loops may deliver a complete click between two paints.
        // Egui has already cleared press_origin/button-down by that point, so
        // retain the actual press event and let its accepted response identify
        // the target. An initially unfocused window can still receive a valid
        // pointer click; only an actual focus-loss event cancels this gesture.
        let primary_press = ui.input(|input| {
            input.raw.events.iter().find_map(|event| match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers,
                } => Some((*pos, *modifiers)),
                _ => None,
            })
        });
        let cancel_plain_click = ui.input(|input| {
            input.modifiers != egui::Modifiers::NONE
                || input.pointer.secondary_down()
                || input.pointer.button_down(egui::PointerButton::Middle)
                || input.key_down(egui::Key::Escape)
                || input.raw.events.iter().any(|event| match event {
                    egui::Event::WindowFocused(false)
                    | egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        ..
                    } => true,
                    egui::Event::PointerButton {
                        button, modifiers, ..
                    } => {
                        *button != egui::PointerButton::Primary
                            || *modifiers != egui::Modifiers::NONE
                    }
                    _ => false,
                })
        });
        if self.tool != daw_ui::EditorTool::Select
            || cancel_plain_click
            || ui.input(|input| {
                input.pointer.primary_pressed()
                    || (!input.pointer.primary_down() && !input.pointer.primary_released())
            })
        {
            self.arranger_selection.plain_click_candidate = None;
        }
        let selected_loop_range = self.loop_selection_range();
        let can_change_loop = !self.recording_busy();
        let viewport_id = egui::Id::new("arrangement_timeline_viewport");
        let mut viewport = ui
            .ctx()
            .data_mut(|data| data.get_temp::<daw_ui::TimelineViewport>(viewport_id))
            .unwrap_or(daw_ui::TimelineViewport {
                pixels_per_beat: self.zoom,
                row_height: self.settings.track_density.row_height(),
                ..Default::default()
            });
        let view_before = viewport;
        let mut view_action = daw_ui::TimelineViewAction::None;
        let mut fit = false;
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
        let mut nudge = 0;
        let mut add_pattern = false;
        let mut density_choice = None;
        let amplitude_id = egui::Id::new("arrangement_waveform_amplitude");
        let mut waveform_amplitude = ui.ctx().data_mut(|data| {
            data.get_temp::<daw_ui::WaveformAmplitudeScale>(amplitude_id)
                .unwrap_or_default()
        });
        // Full padding needs about 1520 points for the tools, View and help on
        // one row; tighter padding keeps them together on laptop-width windows.
        let compact_toolbar = ui.available_width() < 1520.0;
        // Zoom lives beside the ruler, which is drawn after this frame's view
        // changes are applied, so its click arrives one frame later.
        let corner_zoom_id = egui::Id::new("arrangement_corner_zoom");
        if let Some(action) = ui
            .ctx()
            .data_mut(|data| data.remove_temp::<daw_ui::TimelineViewAction>(corner_zoom_id))
        {
            view_action = action;
            fit = action == daw_ui::TimelineViewAction::FitAll;
        }
        let trailing_id = egui::Id::new("arrangement_toolbar_trailing_width");
        let trailing_width = ui
            .ctx()
            .data(|data| data.get_temp::<f32>(trailing_id))
            .unwrap_or(320.0);
        // Workflow order: modes, then the precision that governs them, then
        // creation; the caption fills the gap before the view controls.
        daw_ui::toolbar(ui, |ui| {
            if compact_toolbar {
                ui.spacing_mut().button_padding.x = daw_ui::tokens::SPACE_SM;
                ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_XS;
            }
            let tools = daw_ui::editor_tools_with_context(
                ui,
                &mut self.tool,
                &supported,
                daw_ui::ToolHintContext::Arrangement,
            );
            let precision = daw_ui::measured_toolbar_group(
                ui,
                "arrangement_precision",
                daw_ui::tokens::SPACE_XS,
                |ui| {
                    daw_ui::snap_choice(ui, &mut self.snap, &self.project.meter_map(), None);
                    daw_ui::nudge_menu(ui, self.selected().is_some())
                },
            );
            nudge = precision.inner;
            if let Some(last) = tools.last() {
                daw_ui::toolbar_family_rule(ui, last.rect, precision.response.rect);
            }
            add_pattern = daw_ui::quiet_button(ui, "Add pattern")
                .on_hover_text("Add a four-beat MIDI pattern at the cursor; use the selected MIDI track or create one")
                .clicked();
            daw_ui::toolbar_trailing(ui, trailing_width, |ui| {
                let help =
                    daw_ui::editor_tool_help(ui, self.tool, daw_ui::ToolHintContext::Arrangement);
                let view = daw_ui::quiet_view_menu(ui, "View", |ui| {
                    let action = daw_ui::timeline_selection_view_actions(
                        ui,
                        "arrangement",
                        !self.arranger_selection_ids().is_empty(),
                    );
                    if action != daw_ui::TimelineViewAction::None {
                        view_action = action;
                    }
                    ui.separator();
                    density_choice = daw_ui::track_density_choices(ui, &mut viewport.row_height);
                    daw_ui::waveform_amplitude_control(ui, &mut waveform_amplitude);
                    crate::spectrogram::view_options(ui);
                    daw_ui::track_dimensions(
                        ui,
                        &mut header_width,
                        &mut viewport.row_height,
                        max_header,
                    );
                });
                let controls = help.rect.union(view.response.rect);
                ui.ctx()
                    .data_mut(|data| data.insert_temp(trailing_id, controls.width()));
            });
        });
        if let Some(density) = density_choice
            && self.settings.track_density != density
        {
            self.settings.track_density = density;
            if let Err(error) = self.save_independent_preferences() {
                self.message(Err(error), "");
            }
        }
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
        let visible_tracks = self.visible_arranger_tracks();
        let mut folder_rows = folder_lanes(&self.project, &visible_tracks);
        let folder_closures = daw_ui::folder_closures(&folder_rows);
        let (folder_step, folder_padding) = daw_ui::folder_lane_padding(&folder_rows, header_width);
        let folder_footer = daw_ui::folder_footer_spacing(folder_step);
        // Physical heights that do not scale with row zoom: each folder's
        // closing strip and its one-line row.
        let folder_footers = folder_rows.iter().filter(|row| row.folder).count() as f32
            * (folder_footer + daw_ui::TRACK_HEADER_FOLDER_HEIGHT);
        let right_inset = if folder_padding > 0.0 {
            folder_padding + 4.0
        } else {
            0.0
        };
        let row_units: f32 = visible_tracks
            .iter()
            .map(|(index, _)| &self.project.tracks[*index])
            .filter(|track| track.kind != TrackKind::Folder)
            .map(|track| track_row_units(track, viewport.row_height))
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
        let ruler_height = daw_ui::tokens::TIMELINE_RULER_HEIGHT
            + daw_ui::tokens::TIMELINE_LOOP_HEIGHT
            + daw_ui::tokens::TIMELINE_FLAG_HEIGHT;
        let canvas = Rect::from_min_max(
            ui.available_rect_before_wrap().min
                // A leading gutter keeps beat 0, the playhead and the first
                // clip edge off the header border.
                + Vec2::new(
                    header_width + daw_ui::tokens::SPACE_SM,
                    ruler_height + ui.spacing().item_spacing.y,
                ),
            ui.available_rect_before_wrap().max - Vec2::new(12.0 + right_inset, 0.0),
        );
        if let Some((track, clip)) = self.arranger_selection.reveal.take() {
            let bounds = self
                .project
                .clip(track, clip)
                .map(|clip| (clip.start, clip.end()))
                .or_else(|| {
                    self.project
                        .automation_clip(track, clip)
                        .map(|clip| (clip.start, clip.end()))
                });
            if let Some((start, end)) = bounds {
                let width = canvas.width() as f64 / viewport.pixels_per_beat as f64;
                if start < viewport.start_beat {
                    viewport.start_beat = start;
                } else if end > viewport.start_beat + width {
                    viewport.start_beat = (end - width).max(0.0);
                }
            }
            reveal_track(
                &self.project,
                track,
                canvas.height(),
                header_width,
                &mut viewport,
                &self.arranger_selection.revealed_folders,
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
            // Row heights scale, while folder rows and recursive folder closing
            // strips keep their physical thickness. Solve those two terms separately.
            let mut row_units_before = 0.0_f32;
            let mut fixed_before = 0.0_f32;
            let mut selected_rows = None::<(f32, f32, f32, f32)>;
            for (row_index, (index, _)) in visible_tracks.iter().enumerate() {
                let track = &self.project.tracks[*index];
                let (rows, fixed) = if track.kind == TrackKind::Folder {
                    (0.0, daw_ui::TRACK_HEADER_FOLDER_HEIGHT)
                } else {
                    (track_row_units(track, viewport.row_height), 0.0)
                };
                if selected_tracks.contains(&track.id) {
                    selected_rows = Some(selected_rows.map_or(
                        (
                            row_units_before,
                            row_units_before + rows,
                            fixed_before,
                            fixed_before + fixed,
                        ),
                        |(first, _, first_fixed, _)| {
                            (
                                first,
                                row_units_before + rows,
                                first_fixed,
                                fixed_before + fixed,
                            )
                        },
                    ));
                }
                row_units_before += rows;
                fixed_before += fixed + folder_closures[row_index].len() as f32 * folder_footer;
            }
            if let (Some((start, end)), Some((first, last, first_fixed, last_fixed))) =
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
                let fixed = last_fixed - first_fixed;
                viewport.row_height = ((canvas.height() - padding * 2.0 - fixed).max(1.0)
                    / (last - first).max(0.01))
                .max(0.01);
                viewport.row_offset =
                    (first + (first_fixed - padding) / viewport.row_height).max(0.0);
            }
        }
        bounds.end_row = (row_units + folder_footers / viewport.row_height).max(1.0);
        if fit {
            viewport.fit_bounds(canvas, bounds);
        }
        view_action.apply_zoom(&mut viewport, canvas);
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
        // Beat 0 is the earliest position; the canvas never shows negative time.
        viewport.start_beat = viewport.start_beat.max(0.0);
        self.zoom = viewport.pixels_per_beat;
        let content_width = (ui.available_width() - 12.0).max(header_width + 1.0);
        let pattern_start = self.snapped(self.cursor).max(0.0);
        let mut edits = Vec::new();
        let freeze_channels: std::collections::HashSet<_> = self
            .project
            .tracks
            .iter()
            .filter(|track| self.can_freeze(track.id, None))
            .map(|track| track.id)
            .collect();
        let unfreeze_channels: std::collections::HashSet<_> = self
            .project
            .tracks
            .iter()
            .filter(|track| self.can_unfreeze(track.id))
            .map(|track| track.id)
            .collect();
        let clip_edge_drag_id = egui::Id::new("arrangement_clip_edge_drag");
        let mut clip_edge_drag = ui
            .ctx()
            .data(|data| data.get_temp::<ClipEdgeDrag>(clip_edge_drag_id));
        let frame = ui.ctx().cumulative_frame_nr();
        let cancel_clip_edge_drag = self.tool != daw_ui::EditorTool::Select
            || !ui.is_enabled()
            || navigation.navigating
            || edge_popup_was_open
            || egui::Popup::is_any_open(ui.ctx())
            || ui.input(|input| {
                input.key_pressed(egui::Key::Escape)
                    || input.pointer.secondary_down()
                    || input.pointer.button_down(egui::PointerButton::Middle)
                    || input.raw.events.iter().any(|event| {
                        matches!(event, egui::Event::WindowFocused(false))
                    })
                    || (!input.pointer.primary_down() && !input.pointer.primary_released())
            })
            || clip_edge_drag.is_some_and(|gesture| {
                gesture.frame.saturating_add(1) < frame
                    || !matches!(&self.drag, Some(Drag::Clip { clip, mode: ClipDragMode::TrimStart | ClipDragMode::TrimEnd, .. }) if *clip == gesture.clip)
            });
        if cancel_clip_edge_drag && let Some(gesture) = clip_edge_drag.take() {
            if matches!(&self.drag, Some(Drag::Clip { clip, mode: ClipDragMode::TrimStart | ClipDragMode::TrimEnd, .. }) if *clip == gesture.clip)
            {
                self.drag = None;
            }
            self.arranger_selection.plain_click_candidate = None;
            self.arranger_selection.click_tracker.clear();
        }
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
        let mut reorder_menu = None;
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
        let mut mixer_target = None;
        let mut rename_track = None;
        let mut channel_controls = Vec::new();
        let mut track_colors = Vec::new();
        let mut instrument_target = None;
        let mut instrument_editor = None;
        let mut recording_target = None;
        let mut recording_controls = Vec::new();
        let mut arm_toggles = Vec::new();
        let mut open_editor = false;
        let mut clip_clipboard = None;
        let mut clip_diagnostics = Vec::new();
        let mut automation_diagnostics = Vec::new();
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
        let (ruler, ruler_response) =
            ui.allocate_exact_size(Vec2::new(content_width, ruler_height), Sense::click());
        let p = ui.painter_at(ruler);
        p.rect_filled(ruler, 0.0, palette(ui.ctx()).panel);
        let selected_folder = self.selected_track.filter(|id| {
            self.project
                .track(*id)
                .is_some_and(|track| track.kind == TrackKind::Folder)
        });
        let header_cell = Rect::from_min_size(
            ruler.min,
            Vec2::new(header_width, daw_ui::tokens::TIMELINE_RULER_HEIGHT),
        );
        // The timeline's zoom sits in the ruler corner, where the header column
        // is otherwise empty, instead of costing the toolbar a second row.
        let zoom_size = Vec2::new(
            daw_ui::tokens::CONTROL_HEIGHT * 4.0 + daw_ui::tokens::SPACE_XS * 3.0,
            daw_ui::tokens::CONTROL_HEIGHT,
        );
        let zoom_rect = Rect::from_min_size(
            egui::pos2(
                header_cell.right() - zoom_size.x - daw_ui::tokens::SPACE_SM,
                header_cell.center().y - zoom_size.y * 0.5,
            ),
            zoom_size,
        );
        // The gesture caption fills the otherwise empty strip beside the loop
        // band, so it stays visible without a toolbar row of its own.
        let tracks_left = daw_ui::TrackHeaderLayout::new(Rect::from_min_size(
            ruler.min,
            Vec2::new(header_width, ROW),
        ))
        .icon
        .left();
        daw_ui::editor_tool_caption_at(
            ui,
            Rect::from_min_max(
                egui::pos2(tracks_left, header_cell.bottom()),
                egui::pos2(
                    header_cell.right() - daw_ui::tokens::SPACE_SM,
                    ruler.bottom() - daw_ui::tokens::TIMELINE_FLAG_HEIGHT,
                ),
            ),
            self.tool,
            daw_ui::ToolHintContext::Arrangement,
        );
        let mut zoom_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("arrangement_zoom")
                .max_rect(zoom_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let zoom =
            daw_ui::timeline_zoom_controls(&mut zoom_ui, !self.arranger_selection_ids().is_empty());
        if zoom != daw_ui::TimelineViewAction::None {
            ui.ctx()
                .data_mut(|data| data.insert_temp(corner_zoom_id, zoom));
            ui.ctx().request_repaint();
        }
        daw_ui::track_list_menu_at(
            ui,
            header_cell,
            daw_ui::TrackHeaderLayout::new(Rect::from_min_size(
                ruler.min,
                Vec2::new(header_width, ROW),
            ))
            .icon
            .left(),
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
        let grid_left = ruler.left() + header_width + daw_ui::tokens::SPACE_SM;
        let view = viewport;
        let origin = grid_left - (view.start_beat * self.zoom as f64) as f32;
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
            Pos2::new(
                canvas.right(),
                label_ruler.bottom() + daw_ui::tokens::TIMELINE_LOOP_HEIGHT,
            ),
        );
        let flag_lane = Rect::from_min_max(
            Pos2::new(grid_left, loop_lane.bottom()),
            Pos2::new(canvas.right(), ruler.bottom()),
        );
        let meter = self.project.meter_map();
        daw_ui::paint_timeline_ruler(&p, label_ruler, view, self.project.tempo, &meter);
        let loop_response = self.timeline_loop_ui(
            ui,
            "arrangement_loop",
            loop_lane,
            view,
            can_change_loop && !navigation.navigating,
        );
        if ruler_response.clicked()
            && let Some(pos) = ruler_response.interact_pointer_pos()
            && pos.x >= grid_left
            && pos.x <= canvas.right()
            && !loop_lane.contains(pos)
            && !flag_lane.contains(pos)
        {
            new_cursor = Some(self.snapped(((pos.x - origin) / self.zoom) as f64).max(0.0));
        }
        let flag_diagnostics = self.timeline_flag_lane(ui, flag_lane, view, !navigation.navigating);
        let zoom = self.zoom;
        self.timeline_context_menu(&ruler_response, |x| f64::from((x - origin) / zoom));
        // Folder "After" means after its complete subtree, even when the
        // pointer stays over the folder header. Keep that boundary independent
        // of clipping and of whether its descendants have been painted yet.
        let mut row_tops = Vec::with_capacity(visible_tracks.len());
        let mut folder_after_spans = vec![None; visible_tracks.len()];
        let mut row_top = 0.0;
        for (row_index, (index, _)) in visible_tracks.iter().enumerate() {
            row_tops.push(row_top);
            let track = &self.project.tracks[*index];
            let bottom =
                row_top + viewport.row_height * track_row_units(track, viewport.row_height);
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
                    let row_height = viewport.row_height * track_row_units(track, viewport.row_height);
                    let compact = track.collapsed && track.kind != TrackKind::Folder;
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
                        } else if row_index % 2 == 0 {
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
                        header_layout = header_layout.with_folder_disclosure().with_state_toggles();
                    }
                    if !channels.contains_key(&track.id) {
                        header_layout = header_layout.without_channel_sections();
                    }
                    header_layout = header_layout.with_depth(depth);
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
                            collapsed: track.collapsed && !self.arranger_selection.revealed_folders.contains(&track.id),
                        });
                    if track.kind == TrackKind::Midi
                        && ui.interact(header_layout.icon, ui.id().with((track.id, "instrument_icon")), Sense::click())
                            .on_hover_text(if track.instrument == Instrument::None { "Choose an instrument" } else { "Edit instrument" })
                            .clicked()
                    {
                        if track.instrument == Instrument::None { instrument_target = Some(track.id); } else { instrument_editor = Some(track.id); }
                    }
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
                                        _ if !track.instrument_name.is_empty() => track.instrument_name.clone(),
                                        Instrument::Builtin { device } => daw_devices::descriptor(device)
                                            .map_or_else(|| device.clone(), |descriptor| descriptor.name.to_owned()),
                                        Instrument::Vst3 { path, class_id } => self.vst_catalog.iter()
                                            .find(|entry| entry.plugin.path == std::path::Path::new(path)
                                                && entry.plugin.class_id == *class_id)
                                            .map(|entry| entry.plugin.name.clone())
                                            .unwrap_or_else(|| std::path::Path::new(path).file_stem()
                                                .unwrap_or_default().to_string_lossy().into_owned()),
                                    };
                                    let assigned = track.instrument != Instrument::None;
                                    let response = daw_ui::track_header_link_at(ui, rect, (track.id, "instrument"),
                                        daw_ui::Icon::Instrument, "Instrument", &instrument)
                                        .on_hover_text(if assigned {
                                            format!("{instrument}\nEdit instrument · Right-click to change it")
                                        } else {
                                            "Choose an instrument".to_owned()
                                        });
                                    if response.clicked() {
                                        if assigned { instrument_editor = Some(track.id); } else { instrument_target = Some(track.id); }
                                    }
                                    response.context_menu(|ui| {
                                        if ui.add_enabled(assigned, daw_ui::button_widget("Edit instrument")).clicked() {
                                            instrument_editor = Some(track.id);
                                            ui.close();
                                        }
                                        if daw_ui::button(ui, "Change instrument…").clicked() {
                                            instrument_target = Some(track.id);
                                            ui.close();
                                        }
                                    });
                                }
                                TrackKind::Audio => {
                                    let input = match track.audio_input {
                                        Some(AudioInputChannels::Mono { channel }) => format!("Mono {}", channel + 1),
                                        Some(AudioInputChannels::Stereo { left, right }) => format!("Stereo {}–{}", left + 1, right + 1),
                                        None => "None".to_owned(),
                                    };
                                    if daw_ui::track_header_link_at(ui, rect, (track.id, "recording_input"),
                                        daw_ui::Icon::Input, "Input", &input)
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
                    if let Some(mix_rect) = header_layout.mix
                        && channels.contains_key(&track.id) {
                            let gain_automation = automation.get(&AutomationTarget { track: track.id, parameter: AutomationParameter::TrackGain }).copied();
                            let pan_automation = automation.get(&AutomationTarget { track: track.id, parameter: AutomationParameter::TrackPan }).copied();
                            let mut gain = gain_automation.filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                                .map(|(_, _, value)| AutomationParameter::TrackGain.denormalize(value)).unwrap_or(track.gain);
                            let mut pan = pan_automation.filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                                .map(|(_, _, value)| AutomationParameter::TrackPan.denormalize(value)).unwrap_or(track.pan);
                            let controls = daw_ui::track_mix_controls_at_named(ui, mix_rect, (track.id, "channel_controls"),
                                &mut gain, &mut pan, track.mute, track.solo, &track.name);
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
                            for (response, control, state, current, range, unit, name) in [
                                (&controls.gain, daw_ui::AutomationControl::Gain { track: track.id }, gain_automation,
                                    f64::from(daw_ui::channel_gain_to_db(gain)), -60.0..=f64::from(20.0 * 1.5_f32.log10()), "dB", "Volume"),
                                (&controls.pan, daw_ui::AutomationControl::Pan { track: track.id }, pan_automation,
                                    f64::from(pan) * 100.0, -100.0..=100.0, "%", "Pan"),
                            ] {
                                let (action, value) = daw_ui::automatable_parameter_value_context_named(ui, response, control,
                                    state.map(|(lane, state, _)| (lane, state)), current, range, unit, &format!("{} · {name}", track.name));
                                if let Some(action) = action {
                                    automation_navigation = Some(action);
                                }
                                if let Some(value) = value {
                                    match control {
                                        daw_ui::AutomationControl::Gain { .. } => { gain = daw_ui::channel_gain_from_db(value as f32); exact_gain = true; }
                                        daw_ui::AutomationControl::Pan { .. } => { pan = (value / 100.0) as f32; exact_pan = true; }
                                        daw_ui::AutomationControl::Send { .. } | daw_ui::AutomationControl::InputGain { .. } | daw_ui::AutomationControl::DeviceParameter { .. } => {}
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
                    }
                    if let Some(rect) = header_layout.state_toggles {
                        let [mute, solo] = daw_ui::track_state_toggles_at(ui, rect, (track.id, "folder_state"), track.mute, track.solo);
                        if mute.on_hover_text("Mute folder").clicked() { edits.push(Edit::Mute(track.id)); }
                        if solo.on_hover_text("Solo folder").clicked() { edits.push(Edit::Solo(track.id)); }
                    }
                    let grid_step = (8.0 / self.zoom).ceil().max(1.0) as usize;
                    for beat in ((first_beat / grid_step) * grid_step..last_beat).step_by(grid_step) {
                        let x = origin + beat as f32 * self.zoom;
                        if x < grid_left || x > canvas.right() { continue; }
                        let bar = meter.is_downbeat(beat as f64);
                        p.line_segment(
                            [Pos2::new(x, row.top()), Pos2::new(x, row.bottom())],
                            Stroke::new(1.0_f32, if bar { palette(ui.ctx()).line } else { palette(ui.ctx()).grid_minor }),
                        );
                    }
                    for marker in &self.project.markers {
                        let x = origin + marker.beat as f32 * self.zoom;
                        if x >= grid_left && x <= canvas.right() {
                            p.vline(x, row.y_range(), Stroke::new(1.0_f32, palette(ui.ctx()).guide.gamma_multiply(0.35)));
                        }
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
 daw_ui::command_menu_contents(ui, |ui| {
                        let (previous, next) = crate::track_order::track_shift_targets(&self.project, false)
                            .get(&track.id).copied().unwrap_or_default();
                        let actions = daw_ui::track_reorder_menu(ui, daw_ui::TrackReorderAxis::Vertical,
                            previous.is_some(), next.is_some());
                        reorder_menu = Some(serde_json::json!({"track":track.id,
                            "previous":{"rect":rect_json(actions.previous.rect),"enabled":actions.previous.enabled()},
                            "next":{"rect":rect_json(actions.next.rect),"enabled":actions.next.enabled()}}));
                        if let Some(direction) = actions.direction {
                            track_reorder = if direction { next } else { previous };
                            ui.close();
                        }
                        ui.separator();
                        if track.kind == TrackKind::Midi
                            && daw_ui::button(ui, "Add pattern").clicked()
                        {
                            edits.push(Edit::New(track.id, pattern_start));
                            ui.close();
                        }
                        if matches!(track.kind, TrackKind::Audio | TrackKind::Midi)
                            && ui.add_enabled(freeze_channels.contains(&track.id), daw_ui::button_widget("Freeze track"))
                                .on_hover_text("Print this channel's clips and insert processing to new audio tracks below; retain and mute the originals")
                                .clicked()
                            {
                                edits.push(Edit::Freeze(track.id, None));
                                ui.close();
                        }
                        if unfreeze_channels.contains(&track.id) && daw_ui::button(ui, "Return to originals").clicked() {
                            edits.push(Edit::Unfreeze(track.id)); ui.close();
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
                        if daw_ui::button(ui, "Rename…").clicked() {
                            rename_track = Some(track.id);
                            ui.close();
                        }
                        if daw_ui::button(ui, "Properties").clicked() {
                            self.selected_track = Some(track.id);
                            self.selected_clip = None;
                            popup = Some(PopupKind::Inspector);
                            ui.close();
                        }
                        if matches!(track.kind, TrackKind::Audio | TrackKind::Midi | TrackKind::Bus)
                            && daw_ui::button(ui, "Show in Mixer").clicked()
                        {
                            mixer_target = Some(track.id);
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
                            if track.collapsed && !self.arranger_selection.revealed_folders.contains(&track.id) {
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
});
                    if let Some(lane) = &track.automation {
                        let rect = Rect::from_min_max(Pos2::new(grid_left, row.top() + 8.0), Pos2::new(canvas.right(), row.bottom() - 8.0));
                        empty_action_rects.push(row);
                        let response = crate::arranger_automation::automation_lane(ui, crate::arranger_automation::LaneView {
                            track: track.id, rect, viewport: view, snap: self.snap, pointer_blocked: edge_popup_was_open, color: if automation.get(&lane.target).is_some_and(|(_, state, _)| *state != daw_ui::AutomationState::Read) { palette(ui.ctx()).muted } else { palette(ui.ctx()).automation }, navigating: navigation.navigating,
                        }, lane, &mut self.selected_clip);
                        if response.interacted {
                            self.selected_track = Some(track.id);
                            self.arranger_selection.ids.clear();
                            self.arranger_selection.span = None;
                        }
                        edits.extend(response.edits.into_iter().map(|edit| Edit::Automation(track.id, edit)));
                        automation_diagnostics.push(response.diagnostic);
                        if let Some((_, daw_ui::AutomationState::Overridden, value)) = automation.get(&lane.target) {
                            daw_ui::paint_curve(&p, rect, view.start_beat, view.beat_at(rect, rect.right()), palette(ui.ctx()).automation_override, |_| *value);
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
                            Pos2::new(grid_left + daw_ui::tokens::SPACE_LG, row.center().y),
                            Align2::LEFT_CENTER,
                            "Double-click to add a pattern",
                            daw_ui::text::TextRole::Note.font(),
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
                        let col = color(track_color).gamma_multiply(if clip.muted { 0.38 } else { 1.0 });
                        let is_selected = self.arranger_selection.ids.contains(&clip.id);
                        let surface = daw_ui::clip_surface_named(&p, rect, col, is_selected, &clip.name);
                        let cp = p.with_clip_rect(rect.shrink(3.0).intersect(Rect::from_min_max(
                            Pos2::new(grid_left, row.top()), Pos2::new(canvas.right(), row.bottom()))));
                        let title = daw_ui::clip_title(&p, surface.title,
                            FontId::proportional(daw_ui::tokens::FONT_CANVAS), surface.title_color, &clip.name, clip.link_group.is_some());
                        folder_rows[row_index].color = color(track_color);
                    let rect_json = |rect: Rect| serde_json::json!({"min": [rect.min.x,rect.min.y], "max": [rect.max.x,rect.max.y]});
                        let visible_clip = rect.intersect(Rect::from_min_max(Pos2::new(grid_left, row.top()), Pos2::new(canvas.right(), row.bottom())));
                        clip_diagnostics.push(serde_json::json!({"id":clip.id,"track":track.id,"rect":rect_json(rect),"visible_rect":rect_json(visible_clip),"title_rect":rect_json(surface.title),"content_rect":rect_json(surface.content),"name_hit_rect":rect_json(surface.name_hit),"named_header":!clip.name.trim().is_empty(),"linked":clip.link_group.is_some(),"muted":clip.muted,"link_icon":title.icon.map(rect_json)}));
                        let disk_streamed=self.clip_is_disk_streamed(track.id,clip);
                        if let Some(diagnostic)=clip_diagnostics.last_mut(){diagnostic["source_mode"]=serde_json::json!(if disk_streamed{"disk"}else{match &clip.content{ClipContent::Audio{path,..}=>self.audio_source_info(path).map_or("pending",|_|"memory"),_=>"memory"}});diagnostic["disk_badge"]=serde_json::Value::Null;}
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
                                    } else if let Some(file)=self.audio_files.get(path) {
                                        crate::audio_editor::paint_audio_file_clip(&cp,Rect::from_min_max(Pos2::new(rect.left(),surface.content.top()),Pos2::new(rect.right(),surface.content.bottom())),file,clip,self.project.tempo,col,"arranger",waveform_amplitude);
                                    }
                                }
                            }
                        }
                        let response = daw_ui::clip_occurrence_hover(
                            ui.interact(rect.intersect(Rect::from_min_max(Pos2::new(grid_left, row.top()), Pos2::new(canvas.right(), row.bottom()))),
                                ui.id().with(clip.id), if navigation.navigating { Sense::hover() } else { Sense::click_and_drag() }),
                            &clip.name, &track.name, clip.start..clip.end(),
                            self.project.tempo, &meter, clip.link_group.is_some(),
                        );
                        let response=if disk_streamed {daw_ui::hover_text(response,daw_ui::DISK_STREAMED_AUDIO)}else{response};
                        let edge_handles = [true, false].map(|start| {
                            (self.tool == daw_ui::EditorTool::Select && !navigation.navigating)
                                .then(|| daw_ui::clip_edge_handle(
                                    ui, ui.id().with((clip.id, start, "trim")), rect, visible_clip, start,
                                ))
                                .flatten()
                        });
                        let clip_view = daw_ui::TimelineViewport { start_beat: clip.start, ..viewport };
                        let mut fade_preview = None;
                        let fades = if matches!(clip.content, ClipContent::Audio { .. }) {
                            let fade_canvas = Rect::from_min_max(Pos2::new(rect.left(), surface.content.top()), Pos2::new(rect.right(), surface.content.bottom()));
                            fade_preview = daw_ui::paint_clip_fades(&p, fade_canvas, clip_view, clip.start, clip.end(), |beat| clip.audio_envelope_gain(beat - clip.start));
                            let mut fade_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(*ui.layout()));
                            fade_ui.set_clip_rect(p.clip_rect().intersect(rect));
                            Some(daw_ui::clip_fade_handles_with_hover_bounds(&mut fade_ui, ui.id().with(("clip_fades", clip.id)), fade_canvas, rect, clip_view,
                                crate::audio_editor::audio_fade_bounds(clip), !navigation.navigating))
                        } else { None };
                        if let Some((edge, beat)) = fades.as_ref().and_then(|fades| fades.edit) {
                            let beat = if beat <= clip.start {
                                clip.start
                            } else if beat >= clip.end() {
                                clip.end()
                            } else if ui.input(|input| input.modifiers.shift) {
                                beat
                            } else {
                                self.snapped(beat)
                            }
                            .clamp(clip.start, clip.end());
                            let (fi, fo, fic, foc) = crate::audio_editor::audio_fade_handle_edit(clip, edge, beat);
                            edits.push(Edit::SelectOnly(track.id, clip.id));
                            edits.push(Edit::Fades(track.id, clip.id, fi, fo, fic, foc));
                        }
                        let editing_fade = fades.as_ref().is_some_and(|fades| fades.interacting);
                        let pressed_fade = editing_fade && primary_press.is_some();
                        if let Some(diagnostic) = clip_diagnostics.last_mut() {
                            diagnostic["fade_preview"] = fade_preview.map(rect_json).into();
                            diagnostic["fade_handles_visible"] = serde_json::json!(fades.as_ref().map(|fades| fades.visible));
                            diagnostic["fade_handles"] = serde_json::json!(fades.as_ref().map(|fades| fades.handles.map(|handle| handle.map(rect_json))));
                            if let Some(fades) = &fades {
                                diagnostic["fade_in_handle"] = serde_json::json!(fades.handles[0].map(rect_json));
                                diagnostic["fade_out_handle"] = serde_json::json!(fades.handles[1].map(rect_json));
                            }
                        }
                        if disk_streamed {
                            let mut excluded=vec![surface.name_hit];
                            if let Some(fades)=&fades{excluded.extend(fades.handles.iter().flatten().copied());}
                            let badge=daw_ui::paint_audio_streaming_badge_avoiding(&p,rect,&excluded);
                            if let Some(diagnostic)=clip_diagnostics.last_mut(){diagnostic["disk_badge"]=badge.map(rect_json).into();}
                        }
                        if let Some((position, modifiers)) = primary_press
                            && visible_clip.contains(position) {
                            self.arranger_selection.press_shift = Some((clip.id, modifiers.shift));
                            if self.tool == daw_ui::EditorTool::Select && !navigation.navigating
                                && !egui::Popup::is_any_open(ui.ctx())
                                && !cancel_plain_click
                                && modifiers == egui::Modifiers::NONE
                                && (response.is_pointer_button_down_on()
                                    || response.clicked_by(egui::PointerButton::Primary)) {
                                self.arranger_selection.plain_click_candidate = Some(clip.id);
                            }
                        }
                        let press_shift = self.arranger_selection.press_shift
                            .filter(|(id, _)| *id == clip.id).map_or_else(|| ui.input(|input| input.modifiers.shift), |(_, shift)| shift);
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
                        // Drag-only edge targets capture immediately; retain the
                        // original bounds but make no edit until horizontal motion.
                        // Fade handles were registered last and keep precedence.
                        if !cancel_clip_edge_drag && !editing_fade && !pressed_fade {
                            for (start, handle) in [true, false].into_iter().zip(&edge_handles) {
                                if let Some(handle) = handle
                                    && (handle.drag_started_by(egui::PointerButton::Primary)
                                        // A complete short gesture may arrive in
                                        // one paint. Drag-only targets suppress
                                        // underlying clicks, but egui still marks
                                        // its accepted topmost drag target as
                                        // hovered. Require that hit ownership as
                                        // well as the retained press, never just
                                        // geometry through an overlaid widget.
                                        || handle.hovered()
                                            && ui.input(|input| input.pointer.primary_released())
                                            && primary_press.is_some_and(|(position, _)| handle.rect.contains(position)))
                                    && let Some(pressed) = primary_press.map(|(position, _)| position)
                                        .or_else(|| ui.input(|input| input.pointer.press_origin()))
                                {
                                    self.drag = Some(Drag::Clip {
                                        track: track.id,
                                        clip: clip.id,
                                        handle: clip.id,
                                        original: std::sync::Arc::new(clip.clone()),
                                        mode: if start { ClipDragMode::TrimStart } else { ClipDragMode::TrimEnd },
                                    });
                                    clip_edge_drag = Some(ClipEdgeDrag {
                                        handle: handle.id,
                                        clip: clip.id,
                                        pointer_start: pressed.x,
                                        pixels_per_beat: self.zoom,
                                        moved: false,
                                        frame,
                                    });
                                }
                            }
                        }
                        let mut edge_delta = None;
                        if let Some(gesture) = &mut clip_edge_drag
                            && gesture.clip == clip.id
                            && !editing_fade
                            && (ui.ctx().is_being_dragged(gesture.handle)
                                || ui.input(|input| input.pointer.primary_released()))
                            && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
                        {
                            gesture.frame = frame;
                            let delta = f64::from(pointer.x - gesture.pointer_start)
                                / f64::from(gesture.pixels_per_beat);
                            if delta != 0.0 && !gesture.moved {
                                gesture.moved = true;
                                self.arranger_selection.click_tracker.clear();
                                edits.push(Edit::Select(track.id, clip.id, press_shift, true));
                            }
                            if gesture.moved {
                                // Release can carry the final pointer position in
                                // the same frame, after egui stopped its drag.
                                edge_delta = Some(delta);
                            }
                        }
                        let edge_moved = clip_edge_drag.is_some_and(|gesture| gesture.clip == clip.id && gesture.moved);
                        if response.drag_started() || editing_fade || edge_moved {
                            self.arranger_selection.plain_click_candidate = None;
                        }
                        let plain_click = response.clicked_by(egui::PointerButton::Primary)
                            && !edge_moved
                            && self.tool == daw_ui::EditorTool::Select
                            && self.arranger_selection.plain_click_candidate == Some(clip.id);
                        // The drag-only trim edge swallows the body's click, so a
                        // press released there without motion selects like a click.
                        let edge_click = !edge_moved
                            && self.tool == daw_ui::EditorTool::Select
                            && clip_edge_drag.is_some_and(|gesture| gesture.clip == clip.id)
                            && ui.input(|input| input.pointer.primary_released());
                        if plain_click || edge_click {
                            edits.push(if edge_click && press_shift {
                                Edit::Select(track.id, clip.id, true, false)
                            } else {
                                Edit::SelectOnly(track.id, clip.id)
                            });
                        } else if (response.drag_started() || response.clicked()
                            && (self.tool != daw_ui::EditorTool::Select || press_shift))
                            && self.tool != daw_ui::EditorTool::Cursor && !edge_moved {
                            edits.push(Edit::Select(track.id, clip.id, press_shift, response.drag_started()));
                        }
                        if response.clicked() && !navigation.navigating && !edge_moved {
                            let beat = response.interact_pointer_pos().map(|pos| self.snapped(((pos.x - origin) / self.zoom) as f64));
                            match self.tool {
                                daw_ui::EditorTool::Cut => { if let Some(beat) = beat { edits.push(Edit::Command(track.id, clip.id, ClipCommand::Split(beat))); } }
                                daw_ui::EditorTool::Duplicate => edits.push(Edit::Command(track.id, clip.id, ClipCommand::Duplicate)),
                                daw_ui::EditorTool::Ghost => edits.push(Edit::Command(track.id, clip.id, ClipCommand::Ghost)),
                                daw_ui::EditorTool::Glue => popup = Some(PopupKind::Join),
                                daw_ui::EditorTool::Erase => edits.push(Edit::Erase(track.id, clip.id)),
                                daw_ui::EditorTool::Cursor => new_cursor = beat,
                                _ => {}
                            }
                        }
                        clicked_clip |= response.clicked() || edge_moved;
                        let double = response.clicked() && !edge_moved && response.interact_pointer_pos().is_some_and(|position| {
                            self.arranger_selection.click_tracker.register(clip.id, position, ui.input(|input| input.time),
                                plain_click || self.tool == daw_ui::EditorTool::Pencil && !press_shift
                                    && ui.input(|input| input.modifiers == egui::Modifiers::NONE))
                        });
                        if double {
                            self.selected_track = Some(track.id);
                            self.selected_clip = Some(clip.id);
                            let restore_name = clip.name.trim().is_empty()
                                && response.interact_pointer_pos().is_some_and(|pos| {
                                    surface.name_hit.contains(pos)
                                        && !fades.as_ref().is_some_and(|fades| fades.handles.iter().flatten().any(|handle| handle.contains(pos)))
                                });
                            if restore_name {
                                edits.push(Edit::Rename(track.id, clip.id));
                            } else {
                                open_editor = true;
                            }
                        }
                        if response.hovered()
                            && !editing_fade
                            && let Some(pos) = response.hover_pos()
                        {
                            let icon = match self.tool {
                                daw_ui::EditorTool::Select if clip.name.trim().is_empty()
                                    && surface.name_hit.contains(pos)
                                    && !fades.as_ref().is_some_and(|fades| fades.handles.iter().flatten().any(|handle| handle.contains(pos))) => egui::CursorIcon::Text,
                                daw_ui::EditorTool::Select
                                    if pos.x > rect.right() - 8.0 || pos.x < rect.left() + 8.0 =>
                                {
                                    egui::CursorIcon::ResizeHorizontal
                                }
                                daw_ui::EditorTool::Select | daw_ui::EditorTool::Slip => {
                                    if response.dragged() {
                                        egui::CursorIcon::Grabbing
                                    } else {
                                        egui::CursorIcon::Grab
                                    }
                                }
                                tool => tool.cursor_icon(),
                            };
                            ui.ctx().set_cursor_icon(icon);
                        }
                        if response.drag_started() && !editing_fade && !edge_moved && matches!(self.tool, daw_ui::EditorTool::Select | daw_ui::EditorTool::Stretch) {
                            let mode = if self.tool == daw_ui::EditorTool::Stretch {
                                ClipDragMode::Stretch
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
                        if (response.dragged() || edge_delta.is_some()) && !editing_fade && matches!(self.tool, daw_ui::EditorTool::Select | daw_ui::EditorTool::Stretch)
                            && let Some(Drag::Clip {
                                track: t,
                                clip: c,
                                handle,
                                original,
                                mode,
                            }) = &self.drag
                            && *handle == clip.id

                        {
                            let delta = edge_delta.unwrap_or_else(|| response.total_drag_delta().unwrap_or_default().x as f64
                                / self.zoom as f64);
                            let snap = |beat| if ui.input(|i| i.modifiers.shift) { beat } else { self.snapped(beat) };
                            let minimum = original.length.min(0.0625);
                            match mode {
                                ClipDragMode::Move => edits.push(Edit::GroupMove(self.snapped(original.start + delta) - original.start)),
                                ClipDragMode::TrimStart => {
                                    let earliest = if matches!(original.content, ClipContent::Audio { reverse: true, .. }) {
                                        0.0
                                    } else { (original.start - original.offset / original.source_rate).max(0.0) };
                                    let start = if delta == 0.0 { original.start } else {
                                        snap(original.start + delta).clamp(earliest, (original.end() - minimum).max(earliest))
                                    };
                                    let length = if start == original.start { original.length } else { original.end() - start };
                                    edits.push(Edit::Bounds(*t, *c, std::sync::Arc::clone(original), start, length, false));
                                }
                                ClipDragMode::TrimEnd | ClipDragMode::Stretch => {
                                    let mut length = if delta == 0.0 { original.length } else {
                                        (snap(original.end() + delta) - original.start).max(minimum)
                                    };
                                    if *mode == ClipDragMode::Stretch {
                                        length = length.clamp(original.source_length() / 4.0, original.source_length() / 0.25);
                                    } else if matches!(original.content, ClipContent::Audio { reverse: true, .. }) {
                                        length = length.min(original.length + original.offset / original.source_rate);
                                    }
                                    edits.push(Edit::Bounds(*t, *c, std::sync::Arc::clone(original), original.start, length, *mode == ClipDragMode::Stretch));
                                }
                            }
                        }
                        let has_clipboard = self.has_arranger_clipboard();
                        response.context_menu(|ui| {
 daw_ui::command_menu_contents(ui, |ui| {
                            let selected_count = self.arranger_selection.ids.len();
                            if selected_count > 1 && self.arranger_selection.ids.contains(&clip.id) {
                                daw_ui::text::note(ui, format!("{selected_count} clips selected"));
                            }
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
                            if daw_ui::button(ui, "Properties")
                                .on_hover_text("Edit this clip's exact timing and settings")
                                .clicked()
                            {
                                self.selected_track = Some(track.id);
                                self.selected_clip = Some(clip.id);
                                self.selected_notes.clear();
                                popup = Some(PopupKind::Inspector);
                                ui.close();
                            }
                            if daw_ui::button(ui, "Loop this clip")
                                .on_hover_text("Set the loop to this clip's exact bounds and enable looping")
                                .clicked()
                            {
                                edits.push(Edit::LoopRange(clip.start, clip.end()));
                                ui.close();
                            }
                            if ui.add_enabled(freeze_channels.contains(&track.id) && !clip.muted, daw_ui::button_widget("Freeze clip"))
                                .on_hover_text("Print this clip with its channel's insert processing to new audio below; retain and mute the original")
                                .clicked()
                            {
                                edits.push(Edit::Freeze(track.id, Some(vec![clip.id])));
                                ui.close();
                            }
                            if unfreeze_channels.contains(&track.id) && daw_ui::button(ui, "Return to originals").clicked() {
                                edits.push(Edit::Unfreeze(track.id)); ui.close();
                            }
                            if daw_ui::button(ui, if clip.muted { "Unmute clip" } else { "Mute clip" }).clicked() {
                                edits.push(Edit::ClipMuted(track.id, clip.id, !clip.muted));
                                ui.close();
                            }
                            if matches!(clip.content, ClipContent::Midi { .. }) {
                                daw_ui::choice_menu(ui, "MIDI timing and pitch", |ui| {
                                    if let Some(action) = daw_ui::midi_clip_action_menu_contents(ui,
                                        crate::clip_actions::midi_clip_action_availability(clip)) {
                                        edits.push(Edit::MidiTransform(track.id, clip.id, action));
                                    }
                                });
                            }
                            // Same order as the Edit menu: open, select, clipboard,
                            // name, structure, transitions, playback, delete.
                            ui.separator();
                            let member = self.arranger_selection.ids.contains(&clip.id);
                            if daw_ui::selection_membership_action(ui, member) {
                                edits.push(Edit::Select(track.id, clip.id, true, false));
                            }
                            if daw_ui::button(ui, "Select only this clip").clicked() {
                                edits.push(Edit::SelectOnly(track.id, clip.id));
                                ui.close();
                            }
                            ui.separator();
                            for (label, action) in [
                                ("Cut", crate::audio_editor::AudioClipboardAction::Cut),
                                ("Copy", crate::audio_editor::AudioClipboardAction::Copy),
                                ("Paste", crate::audio_editor::AudioClipboardAction::Paste),
                            ] {
                                let enabled = !matches!(action, crate::audio_editor::AudioClipboardAction::Paste)
                                    || has_clipboard;
                                if ui.add_enabled(enabled, daw_ui::button_widget(label)).clicked() {
                                    clip_clipboard = Some((track.id, clip.id, member, action));
                                    ui.close();
                                }
                            }
                            if daw_ui::button(ui, "Duplicate").clicked() {
                                edits.push(Edit::Command(track.id, clip.id, ClipCommand::Duplicate));
                                ui.close();
                            }
                            ui.separator();
                            if daw_ui::button(ui, "Rename…").clicked() {
                                edits.push(Edit::Rename(track.id, clip.id));
                                ui.close();
                            }
                            ui.separator();
                            if daw_ui::button(ui, "Split at cursor").clicked() {
                                edits.push(Edit::Command(track.id, clip.id, ClipCommand::Split(self.cursor)));
                                ui.close();
                            }
                            if daw_ui::button(ui, "Join clips…").clicked() {
                                self.selected_track = Some(track.id);
                                self.selected_clip = Some(clip.id);
                                popup = Some(PopupKind::Join);
                                ui.close();
                            }
                            if daw_ui::button(ui, "Stretch").clicked() {
                                self.selected_track = Some(track.id);
                                self.selected_clip = Some(clip.id);
                                self.selected_notes.clear();
                                self.tool = daw_ui::EditorTool::Stretch;
                                ui.ctx().set_cursor_icon(self.tool.cursor_icon());
                                ui.close();
                            }
                            if track.kind == TrackKind::Audio && daw_ui::button(ui, "Slip contents").clicked() {
                                self.selected_track = Some(track.id);
                                self.selected_clip = Some(clip.id);
                                self.tool = daw_ui::EditorTool::Slip;
                                ui.ctx().set_cursor_icon(self.tool.cursor_icon());
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
                            if track.kind == TrackKind::Audio {
                                ui.separator();
                                if daw_ui::button(ui, "Crossfade…").clicked() {
                                    self.selected_track = Some(track.id);
                                    self.selected_clip = Some(clip.id);
                                    popup = Some(PopupKind::Crossfade);
                                    ui.close();
                                }
                            }
                            ui.separator();
                            if ui.add_enabled(can_change_loop, daw_ui::button_widget(
                                if member { "Loop playback over selection" } else { "Loop playback over clip" },
                            ))
                                .on_hover_text(if member {
                                    "Enable playback looping over the selected clips or time range, including rests. To create copies, set a later loop end, then choose Edit → Repeat to loop end."
                                } else {
                                    "Enable playback looping over this clip, preserving the selection. To create copies, select this clip, set a later loop end, then choose Edit → Repeat to loop end."
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
                            ui.separator();
                            if daw_ui::button(ui, "Delete").clicked() {
                                edits.push(Edit::Command(track.id, clip.id, ClipCommand::Delete));
                                ui.close();
                            }
                        });
});
                    }
                    if track.kind == TrackKind::Audio && let Ok(schedule) = daw_core::AudioSchedule::new(&track.clips) {
                        for fade in &schedule.crossfades {
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
                        for overlap in &schedule.overlaps {
                            let incoming = &track.clips[overlap.incoming];
                            let ClipContent::Audio { crossfade, .. } = incoming.content else { continue; };
                            let lane = Rect::from_min_max(Pos2::new(grid_left, row.top() + 8.0), Pos2::new(row.right(), row.bottom() - 8.0));
                            let handle = daw_ui::crossfade_duration_handle(ui, ui.id().with(("overlap_handle", incoming.id)), lane, view,
                                daw_ui::CrossfadeBounds { start: overlap.start, end: overlap.end, duration: crossfade.map_or(0.0, |config| config.duration) }, !navigation.navigating);
                            if handle.open_curve {
                                self.selected_track = Some(track.id);
                                self.selected_clip = Some(incoming.id);
                                popup = Some(PopupKind::Crossfade);
                            }
                            if let Some(duration) = handle.duration {
                                let duration = if duration <= 0.0 {
                                    0.0
                                } else if duration >= overlap.end - overlap.start {
                                    overlap.end - overlap.start
                                } else {
                                    let end = if ui.input(|input| input.modifiers.shift) {
                                        overlap.start + duration
                                    } else {
                                        self.snapped(overlap.start + duration)
                                    };
                                    (end - overlap.start).clamp(0.0, overlap.end - overlap.start)
                                };
                                edits.push(Edit::Crossfade(track.id, incoming.id, (duration > 0.0).then_some(AudioCrossfade {
                                    duration, curve: crossfade.map_or(CrossfadeCurve::LINEAR, |config| config.curve),
                                })));
                            }
                            if let Some(diagnostic) = clip_diagnostics.iter_mut().find(|value| value["id"] == incoming.id) {
                                diagnostic["crossfade_handle"] = handle.handle.map(|rect| serde_json::json!({"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]})).into();
                                diagnostic["crossfade_handle_visible"] = handle.visible.into();
                            }
                        }
                    }
                    if let Some((start, end)) = self.recording_region(track.id) {
                        let rect = Rect::from_min_max(Pos2::new((origin + start as f32 * self.zoom).max(grid_left), row.top()+8.0), Pos2::new((origin + end as f32 * self.zoom).min(canvas.right()), row.bottom()-8.0));
                        daw_ui::paint_recording_region(&p, rect);
                    }
                    if let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos())
                        && row.contains(pos)
                        && (pos.x >= grid_left && pos.x <= canvas.right()
                            || audio_file_drop && header.contains(pos))
                    {
                        // External files can target the channel identity too.
                        // Internal clip drags keep their existing grid-only hit
                        // area; a header file drop uses the captured playhead.
                        let beat = if pos.x < grid_left { self.cursor }
                            else { self.snapped(((pos.x - origin) / self.zoom) as f64).max(0.0) };
                        self.drop_target = Some((Some(track.id), beat));
                        if ui.ctx().input(|i| !i.raw.hovered_files.is_empty() || !i.raw.dropped_files.is_empty()) {
                            let x = origin + beat as f32 * self.zoom;
                            p.rect_filled(row, 0.0, palette(ui.ctx()).accent.gamma_multiply(0.08));
                            if x >= grid_left && x <= canvas.right() { p.line_segment(
                                [Pos2::new(x, row.top()), Pos2::new(x, row.bottom())],
                                Stroke::new(3.0_f32, palette(ui.ctx()).accent),
                            ); }
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
                            daw_ui::EmptyWorkspaceAction { id: "import", label: "Import audio or MIDI…", description: "Import WAV, MP3, FLAC, Ogg or MIDI files. Drop audio files onto a track or the arranger.", primary: false },
                            daw_ui::EmptyWorkspaceAction { id: "ai", label: "Compose with AI", description: "Explore editable starting points in the song assistant. Nothing runs until you choose Send.", primary: false },
                        ])
                    }).inner;
                    empty_action = response.action;
                    empty_action_rects.extend(response.actions.iter().map(|(_, response)| response.interact_rect));
                } else {
                    // Bars continue below the last track so the timeline reads as one surface.
                    for (_, beat) in meter.bar_lines(first_beat as f64, last_beat as f64).take(4096) {
                        let x = origin + beat as f32 * self.zoom;
                        if x >= grid_left && x <= canvas.right() {
                            p.vline(x, empty.y_range(), Stroke::new(1.0_f32, palette(ui.ctx()).grid_minor));
                        }
                    }
                    p.text(
                        Pos2::new(
                            grid_left + daw_ui::tokens::SPACE_LG,
                            empty.top() + daw_ui::tokens::CONTROL_HEIGHT,
                        ),
                        Align2::LEFT_CENTER,
                        "Drop audio here · Double-click for MIDI",
                        daw_ui::text::TextRole::Note.font(),
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
                if background_double && let Some(pos) = response.interact_pointer_pos() {
                    edits.push(Edit::New(
                        0,
                        self.snapped(((pos.x - origin) / self.zoom) as f64).max(0.0),
                    ));
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
        if ui.is_enabled()
            && !navigation.navigating
            && !egui::Popup::is_any_open(ui.ctx())
            && ui.output(|output| output.cursor_icon == egui::CursorIcon::Default)
        {
            ui.ctx().set_cursor_icon(self.tool.cursor_icon());
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
            Pos2::new(grid_left - daw_ui::tokens::SPACE_SM - 8.0, ruler.top()),
            Pos2::new(
                grid_left - daw_ui::tokens::SPACE_SM - 1.0,
                scroll.inner_rect.bottom(),
            ),
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
                fit || navigation.fit
                    || view_action == daw_ui::TimelineViewAction::FitSelection
                    || view_action.is_zoom(),
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
                "density": daw_ui::TrackDensity::ALL.into_iter().find(|density| (density.row_height() - viewport.row_height).abs() < 0.5).map_or("Custom", daw_ui::TrackDensity::label),
                "preferred_density": self.settings.track_density.label(),
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
            diagnostics["reorder_menu"] = serde_json::json!(reorder_menu);
            diagnostics["ruler"] = rect_json(label_ruler);
            diagnostics["automation"] = serde_json::json!(automation_diagnostics);
            diagnostics["loop_lane"] = rect_json(loop_response.lane);
            diagnostics["timeline_flags"] = flag_diagnostics;
            diagnostics["loop_handles"] = serde_json::json!(loop_response.handles.map(|handle| handle.map(rect_json)));
            diagnostics["loop_indicator"] = serde_json::json!(loop_response.band.filter(|_| self.project.loop_enabled).map(rect_json));
            diagnostics["loop_start_handle"] = serde_json::json!(loop_response.handles[0].filter(|_| self.project.loop_enabled).map(rect_json));
            diagnostics["loop_end_handle"] = serde_json::json!(loop_response.handles[1].filter(|_| self.project.loop_enabled).map(rect_json));
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
            } else if let Edit::SelectOnly(track, clip) = edit {
                self.arranger_select_only_clip(*track, *clip);
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
        if let Some(track) = mixer_target {
            self.show_channel_in_mixer(ui.ctx(), track);
        }
        if let Some(track) = channel_target {
            self.open_channel(track);
        }
        if let Some(track) = rename_track {
            self.start_track_rename(ui.ctx(), track);
        }
        for track in arm_toggles {
            self.toggle_record_arm(track);
        }
        if let Some(track) = recording_target {
            self.open_recording(track);
        }
        if let Some(track) = instrument_editor {
            self.selected_track = Some(track);
            self.selected_clip = None;
            self.selected_notes.clear();
            self.open_popup(PopupKind::Instrument);
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
        if let Some((track, clip, member, action)) = clip_clipboard {
            // A clip outside the selection acts alone, as in the Edit menu
            // after clicking it.
            if !member {
                self.arranger_select_only_clip(track, clip);
            }
            self.arranger_clipboard_action(ui.ctx(), action);
        }
        if ui.input(|i| i.pointer.primary_released())
            && self.arranger_selection.drag.is_some()
            && let Some((Some(target), _)) = self.drop_target
        {
            edits.push(Edit::TransferGroup(target));
        }
        for edit in edits {
            // Closing a folder opened only for navigation restores its saved
            // collapsed view without making a document edit.
            if let Edit::Collapse(id) = &edit
                && self.project.track(*id).is_some_and(|track| track.collapsed)
                && self.arranger_selection.revealed_folders.remove(id)
            {
                continue;
            }
            if !matches!(
                edit,
                Edit::Select(..)
                    | Edit::SelectOnly(..)
                    | Edit::Rename(..)
                    | Edit::AudioStart(..)
                    | Edit::AudioMotion(..)
                    | Edit::GroupStart(..)
                    | Edit::GroupMove(..)
                    | Edit::TransferGroup(..)
                    | Edit::LoopRange(..)
                    | Edit::Freeze(..)
                    | Edit::Unfreeze(..)
                    | Edit::MidiTransform(..)
                    | Edit::Erase(..)
            ) {
                self.begin_edit();
            }
            let result = match edit {
                Edit::Automation(track, edit) => self.apply_arranger_automation(track, edit),
                Edit::Fades(t, c, fi, fo, fic, foc) => {
                    if self.project.clip(t, c).is_some_and(|clip| matches!(clip.content, ClipContent::Audio { fade_in, fade_out, fade_in_curve, fade_out_curve, .. }
                        if (fade_in, fade_out, fade_in_curve, fade_out_curve) == (fi, fo, fic, foc))) { continue; }
                    self.edit_label = Some("Edit clip fades".into());
                    self.project.set_audio_fades(t, c, fi, fo, fic, foc)
                }
                Edit::Crossfade(t, c, config) => {
                    if self.project.clip(t, c).is_some_and(|clip| matches!(clip.content, ClipContent::Audio { crossfade, .. } if crossfade == config)) { continue; }
                    self.selected_track = Some(t);
                    self.selected_clip = Some(c);
                    self.edit_label = Some("Edit crossfade".into());
                    self.project.set_audio_crossfade(t, c, config)
                }

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
                Edit::SelectOnly(track, clip) => {
                    self.arranger_select_only_clip(track, clip);
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
                Edit::Erase(track, clip) => {
                    // The Erase tool points at one clip. Explicit selection
                    // commands retain their separate bulk-delete behavior.
                    self.apply_clip_command(track, clip, ClipCommand::Delete);
                    continue;
                }
                Edit::LoopRange(start, end) => {
                    self.loop_range(start, end);
                    continue;
                }
                Edit::Freeze(track, clips) => {
                    self.request_freeze(track, clips);
                    continue;
                }
                Edit::Unfreeze(track) => {
                    self.unfreeze(track);
                    continue;
                }
                Edit::MidiTransform(track, clip, action) => {
                    self.apply_midi_clip_action(track, clip, action);
                    continue;
                }
                Edit::ClipMuted(track, clip, muted) => {
                    self.edit_label = Some(if muted { "Mute clip" } else { "Unmute clip" }.into());
                    self.project.set_clip_muted(track, clip, muted)
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
                    if let Some(t) = self.project.track_mut(id)
                        && (!self.arranger_selection.revealed_folders.remove(&id) || !t.collapsed)
                    {
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
                        &self.arranger_selection.revealed_folders,
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
                            &self.arranger_selection.revealed_folders,
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
                    // Keep the complete four-beat pattern inside the project range.
                    let start = start.clamp(0.0, MAX_BEAT - 4.0);
                    let new_track = id == 0;
                    let id = if id == 0 {
                        self.project.add_instrument_track("New instrument")
                    } else {
                        id
                    };
                    let clip = Clip::midi("New pattern", start, 4.0, Vec::new());
                    self.selected_clip = Some(clip.id);
                    self.selected_track = Some(id);
                    self.project.track_mut(id).unwrap().clips.push(clip);
                    self.edit_label = Some("Add pattern".into());
                    if new_track {
                        self.open_new_instrument_chooser();
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
        // Read the applied clip bounds rather than the pre-edit canvas data:
        // this feedback must agree with the current drag, including snap and
        // source limits. Painting creates no hit target or history entry.
        let trim_feedback = if ui.input(|input| input.pointer.primary_down())
            && clip_edge_drag.is_some_and(|gesture| gesture.moved)
            && let Some(Drag::Clip {
                track,
                clip,
                mode: ClipDragMode::TrimStart | ClipDragMode::TrimEnd,
                ..
            }) = &self.drag
            && let Some(clip_data) = self.project.clip(*track, *clip)
            && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
        {
            let end_seconds = clip_data.end() * 60.0 / self.project.tempo;
            let duration_seconds = clip_data.length * 60.0 / self.project.tempo;
            let text = format!(
                "End {end_seconds:.3} s · Beat {:.2}\nDuration {duration_seconds:.3} s · {:.2} beats",
                clip_data.end() + 1.0,
                clip_data.length,
            );
            let rect = daw_ui::pointer_text(
                ui.ctx(),
                ui.id().with("clip_trim_feedback"),
                pointer,
                text.clone(),
            );
            serde_json::json!({
                "track": track, "clip": clip, "text": text,
                "end_seconds": end_seconds, "duration_seconds": duration_seconds,
                "end_beat": clip_data.end() + 1.0, "duration_beats": clip_data.length,
                "rect": {"min": [rect.min.x, rect.min.y], "max": [rect.max.x, rect.max.y]},
            })
        } else {
            serde_json::Value::Null
        };
        ui.ctx().data_mut(|data| {
            data.get_temp_mut_or_default::<serde_json::Value>(egui::Id::new(
                "timeline_diagnostics",
            ))["trim_feedback"] = trim_feedback;
        });
        self.apply_automation_navigation(automation_navigation);
        if let Some(intent) = track_reorder {
            self.apply_track_reorder(intent);
        }
        if ui.input(|input| input.pointer.primary_released()) {
            clip_edge_drag = None;
            self.arranger_selection.drag = None;
            self.arranger_selection.audio_drag = None;
            self.arranger_selection.press_shift = None;
            self.arranger_selection.plain_click_candidate = None;
            if !clicked_clip {
                self.arranger_selection.click_tracker.clear();
            }
            if !clicked_background {
                self.arranger_selection.background_click_tracker.clear();
            }
        }
        ui.ctx().data_mut(|data| {
            if let Some(gesture) = clip_edge_drag {
                data.insert_temp(clip_edge_drag_id, gesture);
            } else {
                data.remove::<ClipEdgeDrag>(clip_edge_drag_id);
            }
        });
        let edge_popup_open = egui::Popup::is_any_open(ui.ctx());
        ui.ctx()
            .data_mut(|data| data.insert_temp(edge_popup_id, edge_popup_open));
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
                    daw_ui::AutomationControl::InputGain { track } => {
                        (track, AutomationParameter::TrackInputGain)
                    }
                    daw_ui::AutomationControl::Pan { track } => {
                        (track, AutomationParameter::TrackPan)
                    }
                    daw_ui::AutomationControl::Send { track, bus } => {
                        (track, AutomationParameter::SendGain { bus })
                    }
                    // Device panels resolve their own owner before this point.
                    daw_ui::AutomationControl::DeviceParameter { .. } => return,
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
                    Ok(lane) if existing.is_some() => {
                        self.show_automation(lane);
                    }
                    Ok(lane) => {
                        self.show_new_automation(lane);
                        self.edit_label = Some("Create automation".into());
                        self.changed();
                    }
                    Err(error) => self.message(Err(error), ""),
                }
            }
            Some(daw_ui::AutomationNavigation::Edit(lane)) => {
                self.show_automation(lane);
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
        // One title on every tab: the clip when one is selected, else the track.
        let title = self
            .selected()
            .map(|clip| clip.name.clone())
            .or_else(|| {
                self.selected_track
                    .and_then(|id| self.project.track(id))
                    .map(|track| track.name.clone())
            })
            .map(|subject| format!("Properties · {subject}"))
            .unwrap_or_else(|| "Properties".into());
        let mut navigated = false;
        let mut name_keyboard = None;
        let mut automation_navigation = None;
        daw_ui::popup(title, ctx)
            .id(egui::Id::new("inspector"))
            .default_width(daw_ui::tokens::POPUP_WIDTH_MD)
            .min_height(daw_ui::popup_body_size(ctx).y.min(if has_clip { 610.0 } else { 377.0 }))
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
                let mut pages = Vec::with_capacity(3);
                if has_clip {
                    pages.push((InspectorPage::Clip, "Clip"));
                }
                pages.push((InspectorPage::Track, "Track"));
                if has_channel {
                    pages.push((InspectorPage::Channel, "Channel"));
                }
                let open_ai = daw_ui::popup_tabs(ui, &mut page, &pages, |ui| {
                    ai_scope.is_some_and(|scope| daw_ui::contextual_ai_button(ui, &scope.label(&self.project)).clicked())
                });
                self.inspector_page = page;
                if open_ai && let Some(scope) = ai_scope {
                    self.open_context_assistant(scope);
                    navigated = true;
                    return;
                }
                if self.inspector_page == InspectorPage::Channel {
                    // The channel page keeps its mouse route to the related
                    // windows; the tab row above switches only this window's pages.
                    if self.channel_navigation(ui, crate::app::ChannelDestination::Channel) {
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
                                self.audio_source_info(path).map_or(MAX_BEAT - clip.source_length(), |source| {
                                    (source.frames as f64 / f64::from(source.sample_rate) * self.project.tempo / 60.0 - clip.source_length())
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
                        let timing_tempo = self.project.tempo;
                        let timing_meter = self.project.meter_map();
                        if self.selected().is_some_and(|clip|self.clip_is_disk_streamed(id,clip)){daw_ui::audio_streaming_status(ui);}
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
                                let label = daw_ui::form_label(ui, "Name");
                                let name = ui.horizontal(|ui| {
                                    let name = daw_ui::form_text_field(ui, &mut clip.name, "Clip name");
                                    if ui.add(daw_ui::quiet_button_widget("Keyboard…").small()).on_hover_text("Name this clip with the mouse").clicked() {
                                        name_keyboard = Some(true);
                                    }
                                    name
                                }).inner
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
                                }, &mut duration_mode, MAX_BEAT, maximum_source_offset,
                                clip.source_length() / MAX_SOURCE_RATE..=clip.source_length() / MIN_SOURCE_RATE,
                                &timing_meter);
                                ctx.data_mut(|data| data.insert_temp(duration_mode_id, duration_mode));
                                daw_ui::form_label(ui, "Stretch");
                                daw_ui::text::body(ui, format!("{:.1}%", 100.0 / clip.source_rate))
                                    .on_hover_text("Duration relative to the source; local to this occurrence");
                                ui.end_row();
                                if let ClipContent::Audio {
                                    gain,
                                    processing,
                                    fade_in,
                                    fade_out,
                                    envelope,
                                    ..
                                } = &mut clip.content
                                {
                                    daw_ui::form_label(ui, "Pitch & time");
                                    changed |= crate::audio_editor::audio_processing_menu(ui, processing);
                                    ui.end_row();
                                    let fade_length = envelope.map_or(clip.length * clip.source_rate, |region| region.length) / clip.source_rate;
                                    let label = daw_ui::form_label(ui, "Clip gain");
                                    ui.horizontal(|ui| {
                                        let slider = ui
                                            .add(daw_ui::slider(gain, 0.0..=2.0)
                                                .zero_reference(1.0)
                                                .scale_labels(false)
                                                .clamping(egui::SliderClamping::Edits).show_value(false))
                                            .labelled_by(label.id);
                                        changed |= slider.changed();
                                        let readout = daw_ui::text::body(ui, if *gain <= 0.0 {
                                            "−∞ dB".to_string()
                                        } else {
                                            format!("{:.1} dB", 20.0 * gain.log10())
                                        });
                                        let response = slider.union(readout);
                                        let mut exact = None;
                                        response.context_menu(|ui| {
 daw_ui::command_menu_contents(ui, |ui| {
                                            exact = daw_ui::value_entry_named(
                                                ui,
                                                ("inspector_clip_gain", id, clip.id),
                                                f64::from(daw_ui::channel_gain_to_db(*gain)),
                                                -60.0..=f64::from(20.0 * 2.0_f32.log10()),
                                                "dB",
                                                &format!("{} · Clip gain", clip.name),
                                            );
                                        });
});
                                        if let Some(db) = exact {
                                            let next = daw_ui::channel_gain_from_db(db as f32).clamp(0.0, 2.0);
                                            if *gain != next {
                                                *gain = next;
                                                changed = true;
                                            }
                                        }
                                    });
                                    ui.end_row();
                                    for (name, value) in [("Fade in", fade_in), ("Fade out", fade_out)] {
                                        let label = daw_ui::form_label(ui, name);
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
                                daw_ui::duration::timing_summary(
                                    ui, clip.start, clip.length, timing_tempo, &timing_meter,
                                );
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
                            let label = daw_ui::form_label(ui, "Name");
                            changed |= ui.horizontal(|ui| {
                                let name = daw_ui::form_text_field(ui, &mut track.name, "Track name").labelled_by(label.id);
                                if ui.add(daw_ui::quiet_button_widget("Keyboard…").small()).on_hover_text("Name this track with the mouse").clicked() {
                                    name_keyboard = Some(false);
                                }
                                name.changed()
                            }).inner;
                            ui.end_row();
                            let label = daw_ui::form_label(ui, "Color");
                            changed |= daw_ui::color_edit(ui, &mut track.color).labelled_by(label.id).changed();
                            ui.end_row();
                            daw_ui::form_label(ui, "Display");
                            changed |= daw_ui::checkbox(ui, &mut track.collapsed, if track.kind == TrackKind::Folder {
                                "Collapse folder"
                            } else {
                                "Compact track"
                            }).changed();
                            ui.end_row();
                            let label = daw_ui::form_label(ui, "Folder");
                            let mut parent = track.parent_folder;
                            daw_ui::select("properties_folder", parent
                                .and_then(|id| folders.iter().find(|(folder, _)| *folder == id))
                                .map(|(_, name)| name.as_str())
                                .unwrap_or("Top level"))
                                .width(daw_ui::tokens::FORM_FIELD_WIDTH).truncate().show_ui(ui, |ui| {
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
                                daw_ui::form_label(ui, "Playback");
                                changed |= daw_ui::toggle(ui, &mut track.mute, "Bypass automation").changed();
                                ui.end_row();
                            } else if track.kind == TrackKind::Folder {
                                daw_ui::form_label(ui, "Playback");
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
        if let Some(clip) = name_keyboard {
            self.start_item_name_keyboard(ctx, clip);
        }
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
        let input_target = AutomationTarget {
            track: id,
            parameter: AutomationParameter::TrackInputGain,
        };
        let input_automation = self.automation_control_state(input_target);
        let gain_automation = self.automation_control_state(gain_target);
        let pan_automation = self.automation_control_state(pan_target);
        let mut overrides = Vec::new();
        if let Some(track) = self.project.track_mut(id) {
            daw_ui::form_grid(ui, "channel_properties", |ui| {
                let label = daw_ui::form_label(ui, "Input gain");
                daw_ui::parameter_row(ui, |ui| {
                    let mut gain = input_automation
                        .filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                        .map(|(_, _, value)| AutomationParameter::TrackInputGain.denormalize(value))
                        .unwrap_or(track.input_gain);
                    let response = ui
                        .add(
                            daw_ui::slider(&mut gain, AutomationParameter::TrackInputGain.range())
                                .zero_reference(1.0)
                                .clamping(egui::SliderClamping::Edits)
                                .endpoint_labels("−∞ dB", "+24 dB")
                                .show_value(false),
                        )
                        .labelled_by(label.id)
                        .on_hover_text("Trim the source signal before the insert chain");
                    if let Some((_, state, _)) = input_automation {
                        daw_ui::paint_automation_control(ui, &response, state);
                    }
                    let numeric = daw_ui::gain_value_button(
                        ui,
                        "channel_input_gain",
                        &mut gain,
                        16.0,
                        "Gain",
                    );
                    let response = response.union(numeric);
                    let (action, exact) = daw_ui::automatable_parameter_value_context_named(
                        ui,
                        &response,
                        daw_ui::AutomationControl::InputGain { track: id },
                        input_automation.map(|(lane, state, _)| (lane, state)),
                        f64::from(daw_ui::channel_gain_to_db(gain)),
                        -60.0..=f64::from(20.0 * 16.0_f32.log10()),
                        "dB",
                        &format!("{} · Input gain", track.name),
                    );
                    if let Some(action) = action {
                        navigation = Some(action);
                    }
                    if let Some(value) = exact {
                        gain = daw_ui::channel_gain_from_db(value as f32);
                    }
                    if response.changed() || exact.is_some() {
                        track.input_gain = gain;
                        changed = true;
                        overrides.push((
                            input_target,
                            AutomationParameter::TrackInputGain.normalize(gain),
                        ));
                    }
                });
                ui.end_row();
                let label = daw_ui::form_label(ui, "Volume");
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
                    let (action, exact) = daw_ui::automatable_parameter_value_context_named(
                        ui,
                        &response,
                        daw_ui::AutomationControl::Gain { track: id },
                        gain_automation.map(|(lane, state, _)| (lane, state)),
                        f64::from(db),
                        -60.0..=f64::from(20.0 * 1.5_f32.log10()),
                        "dB",
                        &format!("{} · Volume", track.name),
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
                    daw_ui::form_label(ui, "Gain automation");
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
                let label = daw_ui::form_label(ui, "Pan");
                let mut pan = pan_automation
                    .filter(|(_, state, _)| *state != daw_ui::AutomationState::Bypassed)
                    .map(|(_, _, value)| AutomationParameter::TrackPan.denormalize(value))
                    .unwrap_or(track.pan);
                let mut pan_percent = pan * 100.0;
                let automated_pan = pan_automation
                    .is_some_and(|(_, state, _)| state != daw_ui::AutomationState::Bypassed);
                let response = ui
                    .add(
                        daw_ui::slider(&mut pan_percent, -100.0..=100.0)
                            .clamping(egui::SliderClamping::Edits)
                            .endpoint_labels("100% L", "100% R")
                            .suffix("%")
                            .min_decimals(1)
                            .max_decimals(6)
                            .display_round_trip(move |percent| {
                                let pan = percent as f32 / 100.0;
                                let pan = if automated_pan {
                                    AutomationParameter::TrackPan
                                        .denormalize(AutomationParameter::TrackPan.normalize(pan))
                                } else {
                                    pan
                                };
                                f64::from(pan * 100.0)
                            }),
                    )
                    .labelled_by(label.id)
                    .on_hover_text("Mixer channel pan · −100% left · 0% center · 100% right");
                if response.changed() {
                    pan = pan_percent / 100.0;
                }
                let (action, exact) = daw_ui::automatable_parameter_value_context_named(
                    ui,
                    &response,
                    daw_ui::AutomationControl::Pan { track: id },
                    pan_automation.map(|(lane, state, _)| (lane, state)),
                    f64::from(pan) * 100.0,
                    -100.0..=100.0,
                    "%",
                    &format!("{} · Pan", track.name),
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
                    daw_ui::form_label(ui, "Pan automation");
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
                daw_ui::form_label(ui, "Playback");
                ui.horizontal(|ui| {
                    changed |= daw_ui::toggle(ui, &mut track.mute, "Mute").changed();
                    if id != daw_core::MASTER_CHANNEL_ID {
                        changed |= daw_ui::toggle(ui, &mut track.solo, "Solo").changed();
                    }
                });
                ui.end_row();
                let label = daw_ui::form_label(ui, "Path latency");
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

impl DawApp {
    pub(crate) fn timeline_loop_ui(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        lane: Rect,
        view: daw_ui::TimelineViewport,
        enabled: bool,
    ) -> daw_ui::TimelineLoopResponse {
        let mut range = daw_ui::TimelineLoop {
            start: self.project.loop_start,
            end: self.project.loop_end,
            enabled: self.project.loop_enabled,
        };
        // Canvas overlays must not advance the parent's layout cursor.
        let mut controls = ui.new_child(egui::UiBuilder::new().max_rect(lane));
        if !enabled || self.recording_busy() {
            controls.disable();
        }
        let response = daw_ui::timeline_loop_editor(
            &mut controls,
            egui::Id::new(id),
            lane,
            view,
            &mut range,
            self.snap,
            MAX_BEAT,
            Some(daw_ui::TimelineLoopLabel {
                tempo: self.project.tempo,
                meter: &self.project.meter_map(),
                playhead: self.cursor,
            }),
        );
        if response.started {
            // Each drag is its own undo step, even right after another one.
            self.history.seal_group();
        }
        if response.changed {
            self.begin_edit();
            self.project.loop_start = range.start;
            self.project.loop_end = range.end;
            self.project.loop_enabled = range.enabled;
            self.edit_label = Some("Loop range".into());
            self.changed();
        }
        response
    }
}

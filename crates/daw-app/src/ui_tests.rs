//! Exercise egui's real pointer handling without a native window or audio device.

use crate::app::{DawApp, PopupKind, SetupPage, Tool, WorkspaceView};
use daw_core::{Clip, ClipContent, Note, Project, TrackKind};
use eframe::egui::{self, Event, FullOutput, PointerButton, Pos2, RawInput, Rect, Vec2};

struct Harness {
    ctx: egui::Context,
    app: DawApp,
    time: f64,
}

impl Harness {
    fn new(clip_start: f64) -> Self {
        let ctx = egui::Context::default();
        let mut project = Project::new();
        let track = project.add_midi_track("Test instrument");
        project.track_mut(track).unwrap().clips.push(Clip::midi(
            "Pattern",
            clip_start,
            8.0,
            vec![Note::new(60, 1.0, 1.0, 100)],
        ));
        let app = DawApp::with_context(&ctx, project, None);
        Self {
            ctx,
            app,
            time: 0.0,
        }
    }

    fn frame(&mut self, events: Vec<Event>, editor: bool) -> FullOutput {
        self.time += 1.0 / 60.0;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        self.ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if editor {
                    self.app.editor(ui);
                } else {
                    self.app.arrangement(ui);
                }
            });
        })
    }

    fn settle(&mut self, editor: bool) -> FullOutput {
        self.frame(vec![], editor);
        self.frame(vec![], editor);
        self.frame(vec![], editor)
    }

    fn full_frame(&mut self, mut input: RawInput) -> FullOutput {
        self.time += 1.0 / 60.0;
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 1000.0)));
        input.time = Some(self.time);
        self.ctx.run(input, |ctx| self.app.ui_frame(ctx))
    }

    fn settle_full(&mut self) -> FullOutput {
        self.full_frame(RawInput::default());
        self.full_frame(RawInput::default());
        self.full_frame(RawInput::default())
    }

    fn full_pointer(&mut self, pos: Pos2, button: PointerButton, pressed: bool) -> FullOutput {
        self.full_frame(RawInput {
            events: vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
            ..Default::default()
        })
    }

    fn full_click(&mut self, pos: Pos2) {
        self.full_pointer(pos, PointerButton::Primary, true);
        self.full_pointer(pos, PointerButton::Primary, false);
    }

    fn full_key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        for pressed in [true, false] {
            self.full_frame(RawInput {
                modifiers,
                events: vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            });
        }
    }

    fn menu(&mut self, menu: &str, item: &str) -> FullOutput {
        let output = self.settle_full();
        let menu = text_position(&output, menu, 0.0) + Vec2::new(8.0, 8.0);
        self.full_click(menu);
        let output = self.settle_full();
        // Menus are painted in front of the workspace, which may contain another
        // label with the same name as a menu entry (for example Properties).
        let item = output
            .shapes
            .iter()
            .rev()
            .find_map(|shape| {
                if let egui::epaint::Shape::Text(shape) = &shape.shape
                    && shape.galley.job.text == item
                {
                    Some(shape.pos + Vec2::new(8.0, 8.0))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("Could not find menu entry {item:?}"));
        self.full_click(item);
        self.settle_full()
    }

    fn drop_wav(&mut self, pos: Pos2, path: &std::path::Path) {
        self.full_frame(RawInput {
            events: vec![Event::PointerMoved(pos)],
            hovered_files: vec![egui::HoveredFile {
                path: Some(path.into()),
                ..Default::default()
            }],
            ..Default::default()
        });
        self.full_frame(RawInput {
            events: vec![Event::PointerMoved(pos)],
            dropped_files: vec![egui::DroppedFile {
                path: Some(path.into()),
                ..Default::default()
            }],
            ..Default::default()
        });
    }

    fn pointer(&mut self, pos: Pos2, pressed: bool, editor: bool) {
        self.frame(
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
            editor,
        );
    }

    fn notes(&self) -> &[Note] {
        match &self.app.selected().unwrap().content {
            ClipContent::Midi { notes } => notes,
            _ => panic!("expected MIDI clip"),
        }
    }
}

fn text_position(output: &FullOutput, text: &str, minimum_x: f32) -> Pos2 {
    output
        .shapes
        .iter()
        .find_map(|shape| {
            if let egui::epaint::Shape::Text(shape) = &shape.shape
                && shape.galley.job.text == text
                && shape.pos.x > minimum_x
            {
                Some(shape.pos)
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("Could not find rendered {text:?}"))
}

fn has_text(output: &FullOutput, text: &str) -> bool {
    output.shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::epaint::Shape::Text(shape) if shape.galley.job.text == text)
    })
}

fn note_position(output: &FullOutput, text: &str) -> Pos2 {
    output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::epaint::Shape::Text(shape) = &shape.shape
                && shape.galley.job.text == text
                && shape.galley.job.sections.first().is_some_and(|section| {
                    section.format.font_id.size == daw_ui::tokens::FONT_CANVAS
                })
            {
                Some(shape.pos)
            } else {
                None
            }
        })
        // Piano keys share the same readable font; notes are to their right.
        .max_by(|left, right| left.x.total_cmp(&right.x))
        .unwrap_or_else(|| panic!("Could not find rendered note {text:?}"))
}

/// A tenth of a second of mono PCM, sufficient to exercise decoding and waveform caching.
fn wav_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drop-fixture.wav");
    let frames = 4800_u32;
    let data_bytes = frames * 2;
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend((36 + data_bytes).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16_u32.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(48000_u32.to_le_bytes());
    bytes.extend(96000_u32.to_le_bytes());
    bytes.extend(2_u16.to_le_bytes());
    bytes.extend(16_u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(data_bytes.to_le_bytes());
    for frame in 0..frames {
        let sample = if frame % 100 < 50 { 1000_i16 } else { -1000 };
        bytes.extend(sample.to_le_bytes());
    }
    std::fs::write(&path, bytes).unwrap();
    (dir, path)
}

#[test]
fn piano_roll_drag_uses_total_displacement_across_multiple_frames() {
    let mut harness = Harness::new(0.0);
    let output = harness.settle(true);
    let start = note_position(&output, "C4") + Vec2::new(8.0, 5.0);
    let distance = harness.app.note_zoom;
    harness.pointer(start, true, true);
    harness.frame(
        vec![Event::PointerMoved(start + Vec2::new(distance * 0.5, 0.0))],
        true,
    );
    harness.frame(
        vec![Event::PointerMoved(start + Vec2::new(distance, 0.0))],
        true,
    );
    harness.pointer(start + Vec2::new(distance, 0.0), false, true);
    let note = &harness.notes()[0];
    assert_eq!(
        note.start, 2.0,
        "one beat of pointer movement must move the note one beat"
    );
    assert_eq!(note.pitch, 60);
    assert_eq!(note.length, 1.0);
}

#[test]
fn pencil_click_creates_a_snapped_note_at_the_selected_pitch() {
    let mut harness = Harness::new(0.0);
    let output = harness.settle(true);
    let reference = note_position(&output, "C4") + Vec2::new(8.0, 5.0);
    harness.app.tool = Tool::Pencil;
    let target = reference + Vec2::new(harness.app.note_zoom * 2.0, -36.0);
    harness.pointer(target, true, true);
    harness.pointer(target, false, true);
    assert_eq!(harness.notes().len(), 2);
    let note = harness
        .notes()
        .iter()
        .find(|note| note.pitch == 62)
        .expect("pencil creates D4");
    assert_eq!(note.start, 3.25);
    assert_eq!(note.length, 0.25);
    assert!(harness.app.selected_notes.contains(&note.id));
}

#[test]
fn arrangement_clip_drag_accumulates_movement_and_preserves_source_offset() {
    let mut harness = Harness::new(2.0);
    let output = harness.settle(false);
    let start = text_position(&output, "Pattern", 175.0) + Vec2::new(12.0, 22.0);
    let distance = harness.app.zoom * 2.0;
    harness.pointer(start, true, false);
    harness.frame(
        vec![Event::PointerMoved(start + Vec2::new(distance * 0.5, 0.0))],
        false,
    );
    harness.frame(
        vec![Event::PointerMoved(start + Vec2::new(distance, 0.0))],
        false,
    );
    harness.pointer(start + Vec2::new(distance, 0.0), false, false);
    let clip = harness.app.selected().unwrap();
    assert_eq!(clip.start, 4.0);
    assert_eq!(clip.length, 8.0);
    assert_eq!(clip.offset, 0.0);
}

#[test]
fn cut_and_glue_tools_split_then_rejoin_a_note_through_pointer_input() {
    let mut harness = Harness::new(0.0);
    let output = harness.settle(true);
    let label = note_position(&output, "C4");
    // The note label begins five points inside the note; half a beat lands in its middle.
    let cut = label + Vec2::new(harness.app.note_zoom * 0.5 - 5.0, 5.0);
    harness.app.tool = Tool::Cut;
    harness.pointer(cut, true, true);
    harness.pointer(cut, false, true);
    assert_eq!(harness.notes().len(), 2);
    assert!(harness.notes().iter().all(|note| note.length == 0.5));

    harness.app.tool = Tool::Glue;
    harness.settle(true);
    // Both halves have the same label, and Cut selects the second half. Clicking
    // the first adds it to Glue's selection; clicking the second again does not.
    let first_id = harness
        .notes()
        .iter()
        .find(|note| note.start == 1.0)
        .unwrap()
        .id;
    let diagnostics = harness.ctx.data(|data| {
        data.get_temp::<serde_json::Value>(egui::Id::new("midi_editor_diagnostics"))
            .unwrap()
    });
    let rect = &diagnostics["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == first_id)
        .unwrap()["rect"];
    let first_half = Pos2::new(
        ((rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5) as f32,
        ((rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5) as f32,
    );
    harness.pointer(first_half, true, true);
    harness.pointer(first_half, false, true);
    assert_eq!(harness.notes().len(), 1);
    assert_eq!(harness.notes()[0].start, 1.0);
    assert_eq!(harness.notes()[0].length, 1.0);
    assert_eq!(harness.notes()[0].pitch, 60);
}

#[test]
fn dropping_wav_on_an_audio_row_imports_at_the_pointer_beat_and_undoes_as_one_edit() {
    let (_dir, path) = wav_fixture();
    let mut harness = Harness::new(0.0);
    let target = harness.app.project.add_audio_track("Drop target");
    let before = harness.app.project.clone();
    let output = harness.settle_full();
    let label = text_position(&output, " Drop target", 0.0);
    // Track labels start 14 points inside a 248-point header.
    let pos = label + Vec2::new(234.0 + harness.app.zoom * 2.0, 30.0);
    harness.drop_wav(pos, &path);
    assert_eq!(harness.app.project.tracks.len(), 2);
    let clips = &harness.app.project.track(target).unwrap().clips;
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].start, 2.0);
    assert!((clips[0].length - 0.2).abs() < 1e-6);
    let canonical = path.canonicalize().unwrap().to_string_lossy().to_string();
    assert!(matches!(&clips[0].content, ClipContent::Audio { path, .. } if path == &canonical));
    assert!(harness.app.waveforms.contains_key(&canonical));
    assert_eq!(harness.app.selected_track, Some(target));
    assert!(harness.app.history.undo(&mut harness.app.project));
    assert_eq!(harness.app.project, before);
}

#[test]
fn dropping_wav_in_empty_arrangement_creates_an_audio_track_at_the_pointer_beat() {
    let (_dir, path) = wav_fixture();
    let mut harness = Harness::new(0.0);
    let output = harness.settle_full();
    let label = text_position(&output, "+   Drop audio here to create a track", 0.0);
    let pos = label + Vec2::new(harness.app.zoom * 3.0 - 24.0, 5.0);
    harness.drop_wav(pos, &path);
    assert_eq!(harness.app.project.tracks.len(), 2);
    let track = harness.app.project.tracks.last().unwrap();
    assert_eq!(track.kind, TrackKind::Audio);
    assert_eq!(track.name, "drop-fixture");
    assert_eq!(track.clips.len(), 1);
    assert_eq!(track.clips[0].start, 3.0);
    assert_eq!(harness.app.selected_track, Some(track.id));
    assert_eq!(harness.app.selected_clip, Some(track.clips[0].id));
    assert!(harness.app.history.undo(&mut harness.app.project));
    assert_eq!(harness.app.project.tracks.len(), 1);
    assert_eq!(harness.app.project.tracks[0].clips[0].name, "Pattern");
}

#[test]
fn folder_header_toggles_child_visibility_without_removing_clips() {
    let mut harness = Harness::new(0.0);
    let child = harness.app.project.tracks[0].id;
    let folder = harness.app.project.add_folder("Group");
    harness
        .app
        .project
        .set_parent_folder(child, Some(folder))
        .unwrap();
    harness.app.project.add_audio_track("Outside");
    let output = harness.settle_full();
    assert!(has_text(&output, " Test instrument"));
    let header = text_position(&output, "▾ Group", 0.0) + Vec2::new(20.0, 5.0);
    harness.full_pointer(header, PointerButton::Primary, true);
    harness.full_pointer(header, PointerButton::Primary, false);
    let output = harness.settle_full();
    assert!(harness.app.project.track(folder).unwrap().collapsed);
    assert!(!has_text(&output, " Test instrument"));
    assert!(has_text(&output, " Outside"));
    assert_eq!(harness.app.project.track(child).unwrap().clips.len(), 1);
    let header = text_position(&output, "▸ Group", 0.0) + Vec2::new(20.0, 5.0);
    harness.full_pointer(header, PointerButton::Primary, true);
    harness.full_pointer(header, PointerButton::Primary, false);
    let output = harness.settle_full();
    assert!(!harness.app.project.track(folder).unwrap().collapsed);
    assert!(has_text(&output, " Test instrument"));
}

#[test]
fn track_context_menu_compacts_its_row_while_keeping_the_clip_visible() {
    let mut harness = Harness::new(0.0);
    let first = harness.app.project.tracks[0].id;
    harness.app.project.add_audio_track("After");
    let output = harness.settle_full();
    let old_y = text_position(&output, " After", 0.0).y;
    let header = text_position(&output, " Test instrument", 0.0) + Vec2::new(20.0, 5.0);
    harness.full_pointer(header, PointerButton::Secondary, true);
    harness.full_pointer(header, PointerButton::Secondary, false);
    let output = harness.settle_full();
    let collapse = text_position(&output, "Collapse", 0.0) + Vec2::new(12.0, 5.0);
    harness.full_pointer(collapse, PointerButton::Primary, true);
    harness.full_pointer(collapse, PointerButton::Primary, false);
    let output = harness.settle_full();
    assert!(harness.app.project.track(first).unwrap().collapsed);
    assert_eq!(text_position(&output, " After", 0.0).y, old_y - 40.0);
    assert!(has_text(&output, "Pattern"));
}

#[test]
fn duplicating_the_final_note_extends_the_clip_and_renders_both_notes() {
    let mut harness = Harness::new(0.0);
    harness.app.project.tracks[0].clips[0].length = 2.0;
    let output = harness.settle(true);
    let note = note_position(&output, "C4") + Vec2::new(8.0, 5.0);
    harness.app.tool = Tool::Duplicate;
    harness.pointer(note, true, true);
    harness.pointer(note, false, true);
    assert_eq!(harness.notes().len(), 2);
    assert_eq!(harness.notes()[1].start, 2.0);
    assert_eq!(harness.app.selected().unwrap().length, 3.0);
    let output = harness.settle(true);
    let visible_notes = output.shapes.iter().filter(|shape| {
        matches!(&shape.shape, egui::epaint::Shape::Text(shape) if shape.galley.job.text == "C4" && shape.pos.x > 70.0)
    }).count();
    assert_eq!(visible_notes, 2);
}

#[test]
fn double_clicking_a_clip_opens_the_focused_editor_without_changing_the_project() {
    let mut harness = Harness::new(0.0);
    let before = harness.app.project.clone();
    let output = harness.settle_full();
    assert_eq!(harness.app.view, WorkspaceView::Arrangement);
    let clip = text_position(&output, "Pattern", 175.0) + Vec2::new(12.0, 22.0);
    for _ in 0..2 {
        harness.full_pointer(clip, PointerButton::Primary, true);
        harness.full_pointer(clip, PointerButton::Primary, false);
    }
    let output = harness.settle_full();
    assert_eq!(harness.app.view, WorkspaceView::Clip);
    assert!(has_text(&output, "C4"));
    assert!(has_text(&output, "Clip Editor · Pattern"));
    assert_eq!(harness.app.project, before);
}

#[test]
fn tool_popups_are_exclusive_and_escape_returns_to_the_arrangement() {
    let mut harness = Harness::new(0.0);
    let before = harness.app.project.clone();
    let output = harness.settle_full();
    assert!(!has_text(&output, "Properties"));
    let output = harness.menu("Window", "Properties");
    assert!(harness.app.inspector_open);
    assert!(has_text(&output, "Properties"));
    // Global edit shortcuts must never delete the selected clip behind a popup.
    harness.full_key(egui::Key::Delete, egui::Modifiers::NONE);
    assert_eq!(harness.app.project, before);

    harness.full_key(egui::Key::Escape, egui::Modifiers::NONE);
    harness.settle_full();
    assert!(!harness.app.inspector_open);
    let output = harness.menu("Window", "Mixer");
    assert_eq!(harness.app.view, WorkspaceView::Mixer);
    let channel = text_position(&output, "Channel", 0.0) + Vec2::new(12.0, 8.0);
    harness.full_click(channel);
    let output = harness.settle_full();
    let effects = text_position(&output, "Effects", 0.0) + Vec2::new(12.0, 8.0);
    harness.full_click(effects);
    let output = harness.settle_full();
    assert_eq!(harness.app.view, WorkspaceView::Arrangement);
    assert!(harness.app.effects_open);
    assert!(!harness.app.routing_open);
    assert!(!harness.app.instrument_open);
    assert!(!harness.app.inspector_open);
    assert!(!harness.app.library_open);
    assert!(!harness.app.setup_open);
    assert!(has_text(&output, "Effects"));

    harness.app.open_popup(PopupKind::Routing);
    let output = harness.settle_full();
    assert!(harness.app.routing_open);
    assert!(!harness.app.effects_open);
    assert!(!has_text(&output, "Effects"));
    assert!(has_text(&output, "Routing"));
    harness.full_key(egui::Key::Escape, egui::Modifiers::NONE);
    let output = harness.settle_full();
    assert_eq!(harness.app.view, WorkspaceView::Arrangement);
    assert!(!harness.app.routing_open);
    assert!(!has_text(&output, "Routing"));
    assert_eq!(harness.app.project, before);
}

#[test]
fn properties_follows_selection_and_never_leaves_an_unavailable_clip_page() {
    let mut harness = Harness::new(4.0);
    let clip_track = harness.app.selected_track;
    let clip = harness.app.selected_clip;
    let output = harness.menu("Window", "Properties");
    assert!(has_text(&output, "Beat 5.00"));
    assert!(has_text(&output, "Duration"));
    assert!(!has_text(&output, "Compact track"));

    let track_tab = output
        .shapes
        .iter()
        .rev()
        .find_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape
                && text.galley.job.text == "Track"
            {
                Some(text.pos + Vec2::new(8.0, 8.0))
            } else {
                None
            }
        })
        .unwrap();
    harness.full_click(track_tab);
    let output = harness.settle_full();
    assert!(has_text(&output, "Compact track"));
    assert!(!has_text(&output, "Duration"));

    let folder = harness.app.project.add_folder("Folder");
    harness.app.selected_track = Some(folder);
    // Keep a stale clip ID to exercise selection reconciliation while open.
    harness.app.selected_clip = clip;
    let output = harness.settle_full();
    assert!(has_text(&output, "Collapse folder"));
    assert!(has_text(&output, "Mute"));
    assert!(!has_text(&output, "Path latency"));
    assert!(!has_text(&output, "Duration"));

    harness.app.selected_track = clip_track;
    harness.app.selected_clip = clip;
    let output = harness.settle_full();
    assert!(has_text(&output, "Beat 5.00"));
    assert!(!has_text(&output, "Compact track"));
}

#[test]
fn canceling_setup_discards_a_buffer_dropdown_change_without_touching_active_settings() {
    let mut harness = Harness::new(0.0);
    let before = serde_json::to_value(&harness.app.settings).unwrap();
    // Open the configuration draft without querying a native audio device. Only
    // Cancel is exercised, so this test never calls Settings::save or writes config.
    harness.app.setup_draft = harness.app.settings.clone();
    harness.app.setup_page = SetupPage::Audio;
    harness.app.setup_open = true;
    let output = harness.settle_full();
    let current = format!("{} samples", harness.app.settings.buffer_frames);
    // This test exercises Cancel, not scrolling the buffer menu. Choose a
    // visible alternative: larger shared controls can scroll its last row.
    let chosen = if harness.app.settings.buffer_frames == 64 {
        128
    } else {
        64
    };
    let dropdown = text_position(&output, &current, 0.0) + Vec2::new(12.0, 8.0);
    harness.full_click(dropdown);
    let output = harness.settle_full();
    let choice = text_position(&output, &format!("{chosen} samples"), 0.0) + Vec2::new(12.0, 8.0);
    harness.full_click(choice);
    let output = harness.settle_full();
    assert_eq!(harness.app.setup_draft.buffer_frames, chosen);
    assert_eq!(serde_json::to_value(&harness.app.settings).unwrap(), before);
    let cancel = text_position(&output, "Cancel", 0.0) + Vec2::new(12.0, 8.0);
    harness.full_click(cancel);
    harness.settle_full();
    assert!(!harness.app.setup_open);
    assert_eq!(serde_json::to_value(&harness.app.settings).unwrap(), before);
    assert!(!harness.app.history.can_undo());
}

fn setup_with_two_rate_device() -> Harness {
    let mut harness = Harness::new(0.0);
    harness.app.settings.sample_rate = 48_000;
    harness.app.settings.device_name = None;
    harness.app.setup_draft = harness.app.settings.clone();
    harness.app.setup_page = SetupPage::Audio;
    harness.app.devices = vec![daw_engine::DeviceInfo {
        name: "Two-rate device fixture".into(),
        is_default: true,
        supported_sample_rates: vec![44_100, 48_000],
        channels: 2,
    }];
    harness
}

#[test]
fn native_setup_filters_sample_rates_to_the_selected_device_capabilities() {
    let mut harness = setup_with_two_rate_device();
    // Inject capabilities directly so no physical device is queried or opened.
    harness.app.setup_open = true;
    let output = harness.settle_full();
    let dropdown = text_position(&output, "48000 Hz", 0.0) + Vec2::new(12.0, 8.0);
    harness.full_click(dropdown);
    let output = harness.settle_full();
    assert!(has_text(&output, "44100 Hz"));
    assert!(!has_text(&output, "88200 Hz"));
    assert!(!has_text(&output, "96000 Hz"));
    let choice = text_position(&output, "44100 Hz", 0.0) + Vec2::new(12.0, 8.0);
    harness.full_click(choice);
    harness.settle_full();
    assert_eq!(harness.app.setup_draft.sample_rate, 44_100);
    assert_eq!(harness.app.settings.sample_rate, 48_000);
    assert!(harness.app.engine.is_none());
}

#[test]
fn offline_setup_discards_device_inventory_and_keeps_portable_export_rates() {
    let mut harness = setup_with_two_rate_device();
    harness.app.audio_device_enabled = false;
    harness.app.open_popup(PopupKind::Setup);
    assert!(harness.app.devices.is_empty());
    let output = harness.settle_full();
    let dropdown = text_position(&output, "48000 Hz", 0.0) + Vec2::new(12.0, 8.0);
    harness.full_click(dropdown);
    let output = harness.settle_full();
    for rate in ["44100 Hz", "48000 Hz", "88200 Hz", "96000 Hz"] {
        assert!(has_text(&output, rate), "Missing portable rate {rate}");
    }
    let choice = text_position(&output, "96000 Hz", 0.0) + Vec2::new(12.0, 8.0);
    harness.full_click(choice);
    let output = harness.settle_full();
    assert_eq!(harness.app.setup_draft.sample_rate, 96_000);
    let refresh = text_position(&output, "Refresh devices", 0.0) + Vec2::new(12.0, 8.0);
    harness.full_click(refresh);
    let output = harness.settle_full();
    assert!(harness.app.devices.is_empty());
    assert_eq!(harness.app.setup_draft.sample_rate, 96_000);
    let cancel = text_position(&output, "Cancel", 0.0) + Vec2::new(12.0, 8.0);
    harness.full_click(cancel);
    harness.settle_full();
    assert_eq!(harness.app.settings.sample_rate, 48_000);
    assert!(!harness.app.setup_open);
    assert!(harness.app.engine.is_none());
}

#[test]
fn leaving_the_clip_editor_clears_note_selection_so_keyboard_commands_target_the_clip() {
    let mut harness = Harness::new(0.0);
    let output = harness.menu("Window", "Clip Editor");
    assert_eq!(harness.app.view, WorkspaceView::Clip);
    let note = note_position(&output, "C4") + Vec2::new(8.0, 5.0);
    harness.full_click(note);
    harness.settle_full();
    assert_eq!(harness.app.selected_notes.len(), 1);
    harness.menu("Window", "Arrangement");
    assert_eq!(harness.app.view, WorkspaceView::Arrangement);
    assert!(harness.app.selected_notes.is_empty());

    harness.menu("Edit", "Duplicate");
    let clips = &harness.app.project.tracks[0].clips;
    assert_eq!(clips.len(), 2, "Duplicate must target the arrangement clip");
    assert!(
        clips.iter().all(|clip| {
            matches!(&clip.content, ClipContent::Midi { notes } if notes.len() == 1)
        })
    );
    harness.menu("Edit", "Delete");
    assert_eq!(harness.app.project.tracks[0].clips.len(), 1);
}

#[test]
fn escape_from_the_clip_editor_clears_note_selection_before_arrangement_edits() {
    let mut harness = Harness::new(0.0);
    harness.app.open_workspace(WorkspaceView::Clip);
    let output = harness.settle_full();
    let note = note_position(&output, "C4") + Vec2::new(8.0, 5.0);
    harness.full_click(note);
    assert_eq!(harness.app.selected_notes.len(), 1);
    harness.full_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(harness.app.view, WorkspaceView::Arrangement);
    assert!(harness.app.selected_notes.is_empty());
    harness.full_key(egui::Key::D, egui::Modifiers::COMMAND);
    assert_eq!(harness.app.project.tracks[0].clips.len(), 2);
}

#[test]
fn nested_utilities_restore_the_clip_selection_and_scrolled_piano_roll() {
    let mut harness = Harness::new(0.0);
    harness.app.open_workspace(WorkspaceView::Clip);
    let output = harness.settle_full();
    harness.full_click(note_position(&output, "C4") + Vec2::new(8.0, 5.0));
    let selected = harness.app.selected_notes.clone();
    let track = harness.app.selected_track;
    let clip = harness.app.selected_clip;
    harness.full_frame(RawInput {
        events: vec![
            Event::PointerMoved(Pos2::new(650.0, 500.0)),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, -72.0),
                modifiers: egui::Modifiers::NONE,
            },
        ],
        ..Default::default()
    });
    for _ in 0..30 {
        harness.full_frame(RawInput::default());
    }
    let scrolled = note_position(&harness.settle_full(), "C4");
    harness.menu("Window", "Properties");
    assert_eq!(harness.app.view, WorkspaceView::Arrangement);
    assert!(harness.app.inspector_open);
    harness.menu("Window", "Routing");
    assert!(harness.app.routing_open);
    assert!(!harness.app.inspector_open);
    harness.full_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(harness.app.inspector_open);
    assert!(!harness.app.routing_open);
    harness.full_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(harness.app.view, WorkspaceView::Clip);
    assert_eq!(harness.app.selected_track, track);
    assert_eq!(harness.app.selected_clip, clip);
    assert_eq!(harness.app.selected_notes, selected);
    let restored = note_position(&harness.settle_full(), "C4");
    assert!(
        restored.distance(scrolled) < 1.0,
        "Returning must retain piano-roll scroll"
    );

    // A utility's own close action follows the same return path as Escape.
    harness.menu("Window", "Setup");
    let output = harness.settle_full();
    harness.full_click(text_position(&output, "Cancel", 0.0) + Vec2::new(8.0, 6.0));
    assert_eq!(harness.app.view, WorkspaceView::Clip);
    assert_eq!(harness.app.selected_notes, selected);
}

#[test]
fn escape_dismisses_the_scale_dropdown_before_the_clip_editor() {
    let mut harness = Harness::new(0.0);
    harness.app.open_workspace(WorkspaceView::Clip);
    let output = harness.settle_full();
    harness.full_click(note_position(&output, "C4") + Vec2::new(8.0, 5.0));
    let selected = harness.app.selected_notes.clone();
    let output = harness.settle_full();
    harness.full_click(text_position(&output, "Pitch", 0.0) + Vec2::new(8.0, 6.0));
    let output = harness.settle_full();
    harness.full_frame(RawInput {
        events: vec![Event::PointerMoved(
            text_position(&output, "Natural minor", 0.0) + Vec2::new(8.0, 6.0),
        )],
        ..Default::default()
    });
    let output = harness.settle_full();
    assert!(has_text(&output, "Dorian"));
    harness.full_key(egui::Key::Escape, egui::Modifiers::NONE);
    if egui::Popup::is_any_open(&harness.ctx) {
        assert_eq!(harness.app.view, WorkspaceView::Clip);
        harness.full_key(egui::Key::Escape, egui::Modifiers::NONE);
    }
    assert!(!egui::Popup::is_any_open(&harness.ctx));
    assert_eq!(harness.app.view, WorkspaceView::Clip);
    assert_eq!(harness.app.selected_notes, selected);
    harness.full_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(harness.app.view, WorkspaceView::Arrangement);
    assert!(harness.app.selected_notes.is_empty());
}

#[test]
fn document_changes_and_explicit_arrangement_clear_utility_return_paths() {
    for reset in 0..3 {
        let mut harness = Harness::new(0.0);
        harness.app.open_workspace(WorkspaceView::Clip);
        harness.app.open_popup(PopupKind::Inspector);
        harness.app.open_popup(PopupKind::Routing);
        match reset {
            0 => harness.app.open_workspace(WorkspaceView::Arrangement),
            1 => harness.app.finish_new_session(),
            _ => harness.app.finish_open_project(
                Project::new().into(),
                std::path::PathBuf::from("another-session.rill"),
                false,
            ),
        }
        harness.app.close_popup();
        assert_eq!(harness.app.view, WorkspaceView::Arrangement);
        assert!(!harness.app.inspector_open);
        assert!(!harness.app.routing_open);
        assert!(harness.app.effect_editor_target.is_none());
    }
}

#[test]
fn returning_from_an_effect_utility_restores_its_slot_and_mixer_context() {
    let mut harness = Harness::new(0.0);
    let track = harness.app.selected_track.unwrap();
    let slot = daw_core::EffectSlot::new(
        "Test effect",
        daw_core::Effect::Vst3 {
            path: "unused-fixture.vst3".into(),
            class_id: None,
        },
    );
    let effect = slot.id;
    harness
        .app
        .project
        .track_mut(track)
        .unwrap()
        .inserts
        .push(slot);
    harness.app.vst_paths.push("unused-fixture.vst3".into());
    harness.app.open_workspace(WorkspaceView::Mixer);
    harness.app.open_popup(PopupKind::Instrument);
    harness.app.effect_editor_target = Some((track, effect));
    harness.app.open_popup(PopupKind::Inspector);
    harness.app.close_popup();
    assert!(harness.app.instrument_open);
    assert_eq!(harness.app.effect_editor_target, Some((track, effect)));
    harness.app.close_popup();
    assert_eq!(harness.app.view, WorkspaceView::Mixer);
    assert_eq!(harness.app.effect_editor_target, None);

    harness.app.open_popup(PopupKind::Instrument);
    harness.app.effect_editor_target = Some((track, effect));
    harness.app.open_popup(PopupKind::Inspector);
    harness
        .app
        .project
        .track_mut(track)
        .unwrap()
        .inserts
        .clear();
    harness.app.close_popup();
    assert_eq!(harness.app.view, WorkspaceView::Mixer);
    assert!(
        !harness.app.instrument_open,
        "Do not reopen a removed effect as an instrument"
    );
}

#[test]
fn unassigned_instrument_popup_opens_library_without_changing_the_session() {
    let mut harness = Harness::new(0.0);
    let before = harness.app.project.clone();
    harness.app.vst_paths.push("unused-fixture.vst3".into());
    harness.app.open_popup(PopupKind::Instrument);
    let output = harness.settle_full();
    assert!(has_text(&output, "Instrument"));
    assert!(has_text(&output, "Test instrument"));
    assert!(has_text(&output, "No instrument assigned"));
    let choose = text_position(&output, "Choose instrument", 0.0) + Vec2::new(8.0, 6.0);
    harness.full_click(choose);
    assert!(harness.app.library_open);
    assert!(!harness.app.instrument_open);
    assert_eq!(harness.app.project, before);
}

fn linked_midi_harness(note_start: f64) -> (Harness, u64, [u64; 3]) {
    let mut harness = Harness::new(0.0);
    let track = harness.app.selected_track.unwrap();
    let original = harness.app.selected_clip.unwrap();
    harness.app.project.midi_notes_mut(track, original).unwrap()[0].start = note_start;
    let second = harness.app.project.clone_clip(track, original).unwrap();
    let third = harness.app.project.clone_clip(track, second).unwrap();
    harness.app.selected_clip = Some(second);
    harness.app.open_workspace(WorkspaceView::Clip);
    (harness, track, [original, second, third])
}

fn assert_linked_note_start(harness: &Harness, track: u64, clips: [u64; 3], start: f64) {
    let selected = harness.app.selected().unwrap();
    for clip in clips {
        let instance = harness.app.project.clip(track, clip).unwrap();
        assert_eq!(instance.content, selected.content);
        let ClipContent::Midi { notes } = &instance.content else {
            panic!("expected MIDI instance");
        };
        assert!((notes[0].start - start).abs() < 1e-10);
    }
    harness.app.project.validate().unwrap();
}

#[test]
fn linked_midi_note_drag_updates_every_instance_on_each_frame_and_undoes_once() {
    let (mut harness, track, clips) = linked_midi_harness(1.0);
    let before = harness.app.project.clone();
    let output = harness.settle_full();
    let start = note_position(&output, "C4") + Vec2::new(8.0, 5.0);
    let distance = harness.app.note_zoom;
    harness.full_pointer(start, PointerButton::Primary, true);
    for (fraction, expected) in [(0.5, 1.5), (1.0, 2.0), (1.5, 2.5)] {
        harness.full_frame(RawInput {
            events: vec![Event::PointerMoved(
                start + Vec2::new(distance * fraction, 0.0),
            )],
            ..Default::default()
        });
        assert_linked_note_start(&harness, track, clips, expected);
        assert!(
            !harness.app.history.can_undo(),
            "held gesture is not yet committed"
        );
    }
    harness.full_pointer(
        start + Vec2::new(distance * 1.5, 0.0),
        PointerButton::Primary,
        false,
    );
    assert_eq!(harness.app.history.undo_labels().count(), 1);
    harness.full_key(egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(harness.app.project, before);
    assert!(!harness.app.history.can_undo());
    assert!(harness.app.history.can_redo());
}

#[test]
fn linked_midi_quantize_preview_and_cancel_are_local_but_apply_updates_every_instance() {
    let (mut harness, track, clips) = linked_midi_harness(1.13);
    let note = harness.notes()[0].id;
    harness.app.selected_notes.insert(note);
    let before = harness.app.project.clone();
    let output = harness.menu("Notes", "Quantize...");
    assert!(has_text(&output, "Quantize preview"));
    assert_eq!(harness.app.project, before);
    assert!(!harness.app.history.can_undo());
    let cancel = text_position(&output, "Cancel quantize", 0.0) + Vec2::new(8.0, 8.0);
    harness.full_click(cancel);
    assert!(!has_text(&harness.settle_full(), "Quantize preview"));
    assert_eq!(harness.app.project, before);
    assert!(!harness.app.history.can_undo());

    let output = harness.menu("Notes", "Quantize...");
    let apply = text_position(&output, "Apply quantize", 0.0) + Vec2::new(8.0, 8.0);
    harness.full_click(apply);
    assert!(!has_text(&harness.settle_full(), "Quantize preview"));
    assert_linked_note_start(&harness, track, clips, 1.25);
    assert_eq!(
        harness.app.history.undo_labels().collect::<Vec<_>>(),
        ["Quantize notes"]
    );
    harness.full_key(egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(harness.app.project, before);
}

#[test]
fn failed_midi_paste_rolls_back_every_linked_instance_and_creates_no_history() {
    let (mut harness, track, clips) = linked_midi_harness(0.0);
    harness.app.selected_clip = Some(clips[0]);
    let clip = harness.app.project.clip_mut(track, clips[0]).unwrap();
    clip.start = daw_core::MAX_BEAT - 1.0;
    clip.length = 1.0;
    harness.app.project.validate().unwrap();
    let selected_note = harness.notes()[0].id;
    harness.app.selected_notes.insert(selected_note);
    harness.app.cursor = daw_core::MAX_BEAT;
    let before = harness.app.project.clone();
    harness.settle_full();
    harness.menu("Notes", "Copy notes");
    harness.menu("Notes", "Paste notes at cursor");
    assert!(
        harness.app.error,
        "extending beyond the project bound must fail"
    );
    assert_eq!(
        harness.app.project, before,
        "propagated notes and clip bounds all roll back"
    );
    assert_eq!(
        harness.app.selected_notes,
        std::collections::HashSet::from([selected_note])
    );
    assert!(!harness.app.history.can_undo());
    harness.app.project.validate().unwrap();
}

#[test]
fn command_d_rejects_linked_note_duplication_past_timeline_end_without_changing_selection() {
    let (mut harness, track, clips) = linked_midi_harness(0.0);
    harness.app.selected_clip = Some(clips[0]);
    let clip = harness.app.project.clip_mut(track, clips[0]).unwrap();
    clip.start = daw_core::MAX_BEAT - 1.0;
    clip.length = 1.0;
    let selected = harness.notes()[0].id;
    harness.app.selected_notes.insert(selected);
    let before = harness.app.project.clone();
    harness.settle_full();
    harness.full_key(egui::Key::D, egui::Modifiers::COMMAND);
    assert!(harness.app.error);
    assert_eq!(harness.app.project, before);
    assert_eq!(
        harness.app.selected_notes,
        std::collections::HashSet::from([selected])
    );
    assert!(!harness.app.history.can_undo());
    harness.app.project.validate().unwrap();
}

#[test]
fn detaching_a_midi_instance_keeps_audio_equivalent_and_does_not_rebuild_graph() {
    let (mut harness, track, clips) = linked_midi_harness(1.0);
    let before = harness.app.project.clone();
    let revision = harness.app.revision;
    harness.menu("Edit", "Make independent");
    assert!(
        harness
            .app
            .project
            .clip(track, clips[1])
            .unwrap()
            .link_group
            .is_none()
    );
    assert_ne!(harness.app.project, before);
    assert!(crate::app::audio_equivalent(&before, &harness.app.project));
    assert_eq!(
        harness.app.revision, revision,
        "identity-only edit must not reinitialize plugins"
    );
    assert_eq!(harness.app.history.undo_labels().count(), 1);

    // Every musical field must still invalidate the compiled playback graph.
    for field in 0..4 {
        let mut changed = before.clone();
        let note = &mut changed.midi_notes_mut(track, clips[0]).unwrap()[0];
        match field {
            0 => note.pitch += 1,
            1 => note.start += 0.25,
            2 => note.length += 0.25,
            _ => note.velocity -= 1,
        }
        assert!(!crate::app::audio_equivalent(&before, &changed));
    }
}

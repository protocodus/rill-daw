use daw_dsp::sample_player::{PlayerKind, PlayerNode};
use daw_ui::{PlayerSampleView, SamplePlayerEditorOutput, SamplePlayerEditorState};
use egui::{Event, FullOutput, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};

struct Editor {
    ctx: egui::Context,
    root: PlayerNode,
    state: SamplePlayerEditorState,
    width: f32,
}

impl Editor {
    fn new(root: PlayerNode) -> Self {
        let ctx = egui::Context::default();
        daw_ui::theme::apply(&ctx);
        let mut editor = Self {
            ctx,
            root,
            state: Default::default(),
            width: 840.0,
        };
        editor.frame(vec![]);
        editor.frame(vec![]);
        editor
    }

    fn input(&mut self, input: RawInput) -> (FullOutput, SamplePlayerEditorOutput) {
        let mut result = None;
        let output = self.ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let samples = [PlayerSampleView {
                    slot: 0,
                    name: "Kick",
                    seconds: 1.0,
                    peaks: &[(-0.5, 0.5); 128],
                    audible_region: Some((0.2, 0.9)),
                }];
                let free = (0..128).find(|slot| !self.root.uses_sample(*slot));
                result = Some(daw_ui::sample_player_editor(
                    ui,
                    &mut self.root,
                    &mut self.state,
                    &samples,
                    free,
                ));
            });
        });
        (output, result.unwrap())
    }

    fn frame(&mut self, events: Vec<Event>) -> (FullOutput, SamplePlayerEditorOutput) {
        self.input(RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(self.width, 760.0))),
            time: Some(self.ctx.cumulative_frame_nr() as f64 * 0.1),
            events,
            ..Default::default()
        })
    }

    fn click(&mut self, point: Pos2) -> SamplePlayerEditorOutput {
        self.frame(vec![Event::PointerMoved(point)]);
        for pressed in [true, false] {
            let (_, result) = self.frame(vec![
                Event::PointerMoved(point),
                Event::PointerButton {
                    pos: point,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ]);
            if !pressed {
                return result;
            }
        }
        unreachable!()
    }

    fn button(&mut self, label: &str) -> SamplePlayerEditorOutput {
        let (output, _) = self.frame(vec![]);
        self.click(text_rect(&output, label).center())
    }
}

fn text_rect(output: &FullOutput, label: &str) -> Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing {label}"))
}

#[test]
fn wraps_adds_duplicates_and_reorders_nested_players_through_controls() {
    let mut editor = Editor::new(PlayerNode::sample(1, 0));
    editor.button("Wrap in");
    assert!(editor.button("Round robin").changed);
    let PlayerKind::RoundRobin { children } = &editor.root.kind else {
        panic!("wrapped root")
    };
    assert_eq!(children[0].id, 1);
    assert_eq!(editor.state.selected, Some(editor.root.id));
    assert!(editor.button("Add sample").changed);
    let PlayerKind::RoundRobin { children } = &editor.root.kind else {
        panic!("kept root")
    };
    assert_eq!(children.len(), 2);
    assert!(matches!(
        children[1].kind,
        PlayerKind::Sample { slot: 1, .. }
    ));
    assert!(editor.button("Move up").changed);
    let PlayerKind::RoundRobin { children } = &editor.root.kind else {
        unreachable!()
    };
    assert!(matches!(
        children[0].kind,
        PlayerKind::Sample { slot: 1, .. }
    ));
    assert!(editor.button("Duplicate").changed);
    let PlayerKind::RoundRobin { children } = &editor.root.kind else {
        unreachable!()
    };
    assert_eq!(
        children.len(),
        3,
        "duplicate is an adjacent alternative in the same player"
    );
    editor.root.validate(128).unwrap();
    assert!(editor.button("Remove").changed);
    editor.root.validate(128).unwrap();
}

#[test]
fn trim_and_reverse_preserve_a_user_crop_and_never_change_source_identity() {
    let mut root = PlayerNode::sample(1, 0);
    if let PlayerKind::Sample {
        start,
        end,
        gain_db,
        ..
    } = &mut root.kind
    {
        *start = 0.1;
        *end = 0.8;
        *gain_db = -4.0;
    }
    let mut editor = Editor::new(root);
    editor.button("Sample actions");
    assert!(editor.button("Trim silence").changed);
    assert!(matches!(
        editor.root.kind,
        PlayerKind::Sample {
            slot: 0,
            start: 0.2,
            end: 0.8,
            reverse: false,
            gain_db: -4.0
        }
    ));
    editor.button("Sample actions");
    assert!(editor.button("Reverse audible region").changed);
    assert!(matches!(
        editor.root.kind,
        PlayerKind::Sample {
            slot: 0,
            start: 0.2,
            end: 0.8,
            reverse: true,
            gain_db: -4.0
        }
    ));
    editor.root.validate(128).unwrap();
}

#[test]
fn native_files_drop_on_leaf_rows_and_waveforms_without_editing_the_tree() {
    let mut editor = Editor::new(PlayerNode::sample(1, 0));
    for at in [
        editor.state.rows[0].1.center(),
        editor.state.waveform.unwrap().center(),
    ] {
        let (_, result) = editor.input(RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(840.0, 760.0))),
            events: vec![Event::PointerMoved(at)],
            dropped_files: vec![egui::DroppedFile {
                path: Some("/samples/clap.wav".into()),
                ..Default::default()
            }],
            ..Default::default()
        });
        assert!(!result.changed);
        assert_eq!(result.file, Some((0, "/samples/clap.wav".into())));
    }
}

#[test]
fn sample_controls_and_waveform_fit_beside_a_browser_on_laptop_widths() {
    for width in [800.0, 840.0, 1140.0] {
        let mut editor = Editor::new(PlayerNode::sample(1, 0));
        editor.width = width;
        editor.frame(vec![]);
        editor.frame(vec![]);
        let bounds = Rect::from_min_size(Pos2::ZERO, vec2(width, 760.0));
        assert!(bounds.contains_rect(editor.state.waveform.unwrap()));
        for (name, rect) in &editor.state.controls {
            assert!(
                bounds.contains_rect(*rect),
                "{name}: {rect:?} outside {bounds:?}"
            );
        }
    }
}

#[test]
fn nested_players_open_with_their_top_level_structure_visible_and_remember_folds() {
    use daw_dsp::sample_player::VelocityChild;
    let mut next = 10;
    let children = (0..3)
        .map(|band| {
            let alternatives = (0..5)
                .map(|slot| {
                    next += 1;
                    PlayerNode::sample(next, slot)
                })
                .collect();
            let random = PlayerNode {
                id: 100 + band,
                name: format!("Variations {band}"),
                kind: PlayerKind::Random {
                    children: alternatives,
                },
            };
            VelocityChild {
                min: 1 + band as u8 * 42,
                max: if band == 2 {
                    127
                } else {
                    (band as u8 + 1) * 42
                },
                child: PlayerNode {
                    id: 200 + band,
                    name: format!("Band {band}"),
                    kind: PlayerKind::RoundRobin {
                        children: vec![random],
                    },
                },
            }
        })
        .collect();
    let mut editor = Editor::new(PlayerNode {
        id: 1,
        name: "Expressive kit".into(),
        kind: PlayerKind::Velocity { children },
    });
    assert_eq!(
        editor.state.rows.len(),
        7,
        "root, three bands and their variation groups"
    );
    assert!(editor.state.collapsed.contains(&100));
    editor.state.collapsed.remove(&100);
    editor.frame(vec![]);
    assert_eq!(
        editor.state.rows.len(),
        12,
        "user expansion is retained on later frames"
    );
}

#[test]
fn library_drops_preserve_alias_names_for_the_same_managed_recording() {
    let mut editor = Editor::new(PlayerNode::sample(1, 0));
    for (name, at) in [
        ("Soft kick", editor.state.rows[0].1.center()),
        ("Hard kick", editor.state.waveform.unwrap().center()),
    ] {
        editor.frame(vec![
            Event::PointerMoved(at),
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ]);
        let source = daw_ui::SampleDrop {
            path: "/project/audio/sha256.wav".into(),
            name: Some(name.into()),
        };
        egui::DragAndDrop::set_payload(&editor.ctx, source.clone());
        let (_, output) = editor.frame(vec![
            Event::PointerMoved(at),
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ]);
        assert_eq!(output.source, Some((0, source)));
        assert!(
            output.file.is_none(),
            "library metadata must survive instead of becoming a raw file path"
        );
    }
}

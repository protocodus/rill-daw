use daw_ui::{TrackDropIntent, TrackDropPlacement, TrackDropResponse, TrackReorderAxis};
use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

struct Board {
    ctx: Context,
    axis: TrackReorderAxis,
    folder: bool,
    enabled: bool,
    valid: bool,
    after_edge: Option<f32>,
    clicked: usize,
    result: TrackDropResponse,
    commits: Vec<TrackDropIntent>,
    scroll: Vec2,
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl Board {
    fn new(axis: TrackReorderAxis) -> Self {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        let mut board = Self {
            ctx,
            axis,
            folder: false,
            enabled: true,
            valid: true,
            after_edge: None,
            clicked: 0,
            result: Default::default(),
            commits: Vec::new(),
            scroll: Vec2::ZERO,
            shapes: Vec::new(),
        };
        board.frame(Vec::new(), Modifiers::NONE);
        board
    }

    fn source(&self) -> Rect {
        Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(100.0, 34.0))
    }

    fn target(&self) -> Rect {
        match self.axis {
            TrackReorderAxis::Vertical => {
                Rect::from_min_size(Pos2::new(20.0, 110.0), Vec2::new(200.0, 90.0))
            }
            TrackReorderAxis::Horizontal => {
                Rect::from_min_size(Pos2::new(200.0, 20.0), Vec2::new(120.0, 200.0))
            }
        }
    }

    fn at(&self, fraction: f32) -> Pos2 {
        let target = self.target();
        match self.axis {
            TrackReorderAxis::Vertical => {
                Pos2::new(target.center().x, egui::lerp(target.y_range(), fraction))
            }
            TrackReorderAxis::Horizontal => {
                Pos2::new(egui::lerp(target.x_range(), fraction), target.center().y)
            }
        }
    }

    fn frame(&mut self, events: Vec<Event>, modifiers: Modifiers) {
        let source = self.source();
        let target = self.target();
        let output = self.ctx.run(RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 280.0))),
            events, modifiers, ..Default::default()
        }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if !self.enabled { ui.disable(); }
                let source_response = ui.put(source, egui::Button::new("Channel"));
                let source_response = daw_ui::track_reorder_source(ui, &source_response, 1);
                if source_response.clicked() { self.clicked += 1; }
                self.result = daw_ui::track_reorder_target_with_after_edge(
                    ui, target, 2, self.axis, self.folder, self.after_edge, |_| self.valid,
                );
                if let Some(preview) = self.result.preview {
                    daw_ui::track_reorder_preview_label(ui, &preview,
                        "After a very long channel title that identifies its destination · An equally long folder name remains bounded");
                }
                if let Some(intent) = self.result.dropped { self.commits.push(intent); }
                self.scroll = daw_ui::track_reorder_scroll_delta(ui, ui.clip_rect(), self.axis);
            });
        });
        self.shapes = output.shapes;
    }

    fn button(
        &mut self,
        position: Pos2,
        button: PointerButton,
        pressed: bool,
        modifiers: Modifiers,
    ) {
        self.frame(
            vec![
                Event::PointerMoved(position),
                Event::PointerButton {
                    pos: position,
                    button,
                    pressed,
                    modifiers,
                },
            ],
            modifiers,
        );
    }

    fn start(&mut self, button: PointerButton, modifiers: Modifiers) {
        self.button(self.source().center(), button, true, modifiers);
        self.frame(
            vec![Event::PointerMoved(
                self.source().center() + Vec2::new(12.0, 0.0),
            )],
            modifiers,
        );
    }

    fn move_to(&mut self, position: Pos2) {
        self.frame(vec![Event::PointerMoved(position)], Modifiers::NONE);
    }
}

#[test]
fn native_title_drag_previews_before_after_and_commits_only_on_primary_release() {
    for axis in [TrackReorderAxis::Vertical, TrackReorderAxis::Horizontal] {
        for (fraction, placement) in [
            (0.15, TrackDropPlacement::Before),
            (0.85, TrackDropPlacement::After),
        ] {
            let mut board = Board::new(axis);
            // A normal click remains available for channel selection/opening.
            board.button(
                board.source().center(),
                PointerButton::Primary,
                true,
                Modifiers::NONE,
            );
            board.button(
                board.source().center(),
                PointerButton::Primary,
                false,
                Modifiers::NONE,
            );
            assert_eq!(board.clicked, 1);
            board.start(PointerButton::Primary, Modifiers::NONE);
            board.move_to(board.at(fraction));
            assert_eq!(board.result.preview.unwrap().intent.placement, placement);
            assert!(board.commits.is_empty());
            board.button(
                board.at(fraction),
                PointerButton::Primary,
                false,
                Modifiers::NONE,
            );
            assert_eq!(
                board.commits,
                [TrackDropIntent {
                    source: 1,
                    target: 2,
                    placement
                }]
            );
            assert_eq!(
                board.clicked, 1,
                "Dragging must not activate the channel title"
            );
            board.frame(Vec::new(), Modifiers::NONE);
            assert_eq!(board.commits.len(), 1, "Drop is emitted once");
        }
    }
}

#[test]
fn folders_offer_explicit_middle_drop_and_after_uses_the_full_subtree_boundary() {
    let mut board = Board::new(TrackReorderAxis::Vertical);
    board.folder = true;
    board.after_edge = Some(330.0);
    board.start(PointerButton::Primary, Modifiers::NONE);
    board.move_to(board.at(0.5));
    assert_eq!(
        board.result.preview.unwrap().intent.placement,
        TrackDropPlacement::IntoFolder
    );
    board.move_to(board.at(0.85));
    assert_eq!(board.result.preview.unwrap().indicator.center().y, 330.0);
    // Long destination names wrap/elide inside the viewport; the edge cue for
    // the offscreen subtree boundary remains visible there as well.
    for clipped in &board.shapes {
        if let egui::Shape::Text(text) = &clipped.shape
            && text.galley.text().starts_with("After a very long")
        {
            let bounds = text.visual_bounding_rect();
            assert!(
                clipped.clip_rect.contains_rect(bounds),
                "{bounds:?} beyond {:?}",
                clipped.clip_rect
            );
            assert!(bounds.width() <= 360.0);
        }
    }
    board.button(
        board.at(0.85),
        PointerButton::Primary,
        false,
        Modifiers::NONE,
    );
    assert_eq!(board.commits[0].placement, TrackDropPlacement::After);
}

#[test]
fn escape_invalid_disabled_and_navigation_gestures_never_commit() {
    let mut board = Board::new(TrackReorderAxis::Vertical);
    board.start(PointerButton::Primary, Modifiers::NONE);
    board.move_to(board.at(0.2));
    board.frame(
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    assert!(board.result.preview.is_none());
    board.button(
        board.at(0.2),
        PointerButton::Primary,
        false,
        Modifiers::NONE,
    );
    assert!(board.commits.is_empty());
    for button in [PointerButton::Secondary, PointerButton::Middle] {
        let mut board = Board::new(TrackReorderAxis::Vertical);
        board.start(button, Modifiers::NONE);
        board.move_to(board.at(0.2));
        board.button(board.at(0.2), button, false, Modifiers::NONE);
        assert!(board.commits.is_empty());
    }
    for modifiers in [Modifiers::ALT, Modifiers::CTRL, Modifiers::COMMAND] {
        let mut board = Board::new(TrackReorderAxis::Vertical);
        board.start(PointerButton::Primary, modifiers);
        board.move_to(board.at(0.2));
        board.button(
            board.at(0.2),
            PointerButton::Primary,
            false,
            Modifiers::NONE,
        );
        assert!(board.commits.is_empty());
    }
    for disabled in [false, true] {
        let mut board = Board::new(TrackReorderAxis::Vertical);
        board.enabled = !disabled;
        board.valid = false;
        board.frame(Vec::new(), Modifiers::NONE);
        board.start(PointerButton::Primary, Modifiers::NONE);
        board.move_to(board.at(0.2));
        assert!(board.result.preview.is_none());
        board.button(
            board.at(0.2),
            PointerButton::Primary,
            false,
            Modifiers::NONE,
        );
        assert!(board.commits.is_empty());
    }
}

#[test]
fn edge_scrolling_requires_a_primary_track_drag_and_stops_on_release() {
    for axis in [TrackReorderAxis::Horizontal, TrackReorderAxis::Vertical] {
        let mut board = Board::new(axis);
        let edge = match axis {
            TrackReorderAxis::Horizontal => Pos2::new(390.0, 130.0),
            TrackReorderAxis::Vertical => Pos2::new(170.0, 270.0),
        };
        board.move_to(edge);
        assert_eq!(board.scroll, Vec2::ZERO);
        board.start(PointerButton::Primary, Modifiers::NONE);
        board.move_to(edge);
        let amount = if axis == TrackReorderAxis::Horizontal {
            board.scroll.x
        } else {
            board.scroll.y
        };
        assert!(
            (-30.0..0.0).contains(&amount),
            "bounded native content delta: {amount}"
        );
        board.button(edge, PointerButton::Primary, false, Modifiers::NONE);
        assert_eq!(board.scroll, Vec2::ZERO);
    }
}

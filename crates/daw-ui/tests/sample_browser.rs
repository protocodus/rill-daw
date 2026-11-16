use daw_ui::{SampleDetails, SampleDrop, SampleFavorite, SampleTagIcon};
use egui::{Context, Event, FullOutput, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};

fn context() -> Context {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    ctx
}

fn frame<T>(
    ctx: &Context,
    width: f32,
    events: Vec<Event>,
    mut render: impl FnMut(&mut egui::Ui) -> T,
) -> (FullOutput, T) {
    let mut result = None;
    daw_ui::reset_icon_only_labels(ctx);
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 420.0))),
            time: Some(ctx.cumulative_frame_nr() as f64 * 0.05),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| result = Some(render(ui)));
        },
    );
    (output, result.unwrap())
}

fn pointer(at: Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(at),
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn click<T>(ctx: &Context, width: f32, at: Pos2, mut render: impl FnMut(&mut egui::Ui) -> T) -> T {
    frame(ctx, width, vec![Event::PointerMoved(at)], &mut render);
    frame(ctx, width, pointer(at, true), &mut render);
    frame(ctx, width, pointer(at, false), render).1
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
fn favorite_editor_has_named_icon_choices_clear_color_and_bounded_tag_input() {
    let ctx = context();
    let mut favorite = SampleFavorite {
        color: Some([240, 80, 40]),
        ..Default::default()
    };
    frame(&ctx, 260.0, vec![], |ui| {
        daw_ui::sample_favorite_editor(ui, &mut favorite)
    });
    let (output, _) = frame(&ctx, 260.0, vec![], |ui| {
        daw_ui::sample_favorite_editor(ui, &mut favorite)
    });
    assert!(click(
        &ctx,
        260.0,
        text_rect(&output, "Clear color").center(),
        |ui| daw_ui::sample_favorite_editor(ui, &mut favorite)
    ));
    assert_eq!(favorite.color, None);
    let (output, _) = frame(&ctx, 260.0, vec![], |ui| {
        daw_ui::sample_favorite_editor(ui, &mut favorite)
    });
    click(&ctx, 260.0, text_rect(&output, "Star").center(), |ui| {
        daw_ui::sample_favorite_editor(ui, &mut favorite)
    });
    let (output, _) = frame(&ctx, 260.0, vec![], |ui| {
        daw_ui::sample_favorite_editor(ui, &mut favorite)
    });
    assert!(click(
        &ctx,
        260.0,
        text_rect(&output, "Percussion").center(),
        |ui| daw_ui::sample_favorite_editor(ui, &mut favorite)
    ));
    assert_eq!(favorite.icon, SampleTagIcon::Percussion);
    let (output, _) = frame(&ctx, 260.0, vec![], |ui| {
        daw_ui::sample_favorite_editor(ui, &mut favorite)
    });
    click(
        &ctx,
        260.0,
        text_rect(&output, "Optional label").center(),
        |ui| daw_ui::sample_favorite_editor(ui, &mut favorite),
    );
    frame(&ctx, 260.0, vec![Event::Text("打".repeat(80))], |ui| {
        daw_ui::sample_favorite_editor(ui, &mut favorite)
    });
    assert_eq!(favorite.tag.chars().count(), 64);
}

#[test]
fn sample_details_show_source_metadata_and_do_not_invent_lossy_bit_depth() {
    let ctx = context();
    let details = SampleDetails {
        name: "Ocean texture",
        location: "/Samples/Environments/A very long folder/Ocean.flac",
        duration_seconds: Some(72.5),
        format: Some("FLAC"),
        codec: Some("FLAC"),
        sample_rate: Some(44_100),
        channels: Some(2),
        bit_depth: Some(24),
        size_bytes: Some(2 * 1024 * 1024),
        status: None,
    };
    let (output, rect) = frame(&ctx, 260.0, vec![], |ui| {
        daw_ui::sample_details(ui, &details).rect
    });
    assert!(rect.right() <= 252.1);
    text_rect(&output, "1:13 · FLAC");
    text_rect(&output, "44.1 kHz · Stereo · 24-bit · 2.0 MiB");
    let details = SampleDetails {
        format: Some("Ogg"),
        codec: Some("Vorbis"),
        bit_depth: None,
        ..details
    };
    let (output, _) = frame(&ctx, 260.0, vec![], |ui| {
        daw_ui::sample_details(ui, &details)
    });
    text_rect(&output, "1:13 · Ogg · Vorbis");
    text_rect(&output, "44.1 kHz · Stereo · 2.0 MiB");
    let details = SampleDetails {
        sample_rate: Some(11_025),
        ..details
    };
    let (output, _) = frame(&ctx, 260.0, vec![], |ui| {
        daw_ui::sample_details(ui, &details)
    });
    text_rect(&output, "11.025 kHz · Stereo · 2.0 MiB");
}

fn result(index: usize) -> daw_ui::SampleResult {
    daw_ui::SampleResult {
        id: format!("result-{index}"),
        name: format!("Analog kick {index:06}.wav"),
        provenance: "Drums / Analog / Kicks".into(),
        path: format!("/Samples/Drums/Analog/Kicks/{index}.wav"),
        source_name: Some(format!("Kick {index}")),
        ..Default::default()
    }
}
fn source(id: &str, name: &str, drop_target: bool) -> daw_ui::SampleSourceNode {
    daw_ui::SampleSourceNode {
        id: id.into(),
        name: name.into(),
        icon: daw_ui::Icon::Folder,
        detail: None,
        children: vec![],
        drop_target,
    }
}
fn key(key: egui::Key) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn flat_results_virtualize_a_hundred_thousand_and_bound_favorite_targets() {
    let ctx = context();
    let mut rows: Vec<_> = (0..100_000).map(result).collect();
    rows[0].favorite = Some(SampleFavorite {
        tag: "Warm kick".into(),
        color: Some([210, 70, 40]),
        icon: SampleTagIcon::Drum,
    });
    rows[0].audition = daw_ui::SampleAuditionState::Playing;
    for width in [260.0, 420.0] {
        let (paint, output) = frame(&ctx, width, vec![], |ui| {
            daw_ui::sample_results(ui, egui::Id::new("flat"), &rows, Some("result-0"), 240.0)
        });
        assert!(
            !output.rows.is_empty() && output.rows.len() < 10,
            "Only visible result widgets may exist"
        );
        let title = text_rect(&paint, &rows[0].name);
        assert!(output.rows[0].1.rect.contains_rect(title));
        for (label, rect) in daw_ui::icon_only_labels(&ctx) {
            assert!(label == "Add favorite" || label == "Remove favorite");
            assert!(rect.width() >= 33.9 && rect.height() >= 33.9);
            assert!(
                rect.right() <= width - 7.9,
                "{label} exceeds pane: {rect:?}"
            );
        }
        let has_playing = paint.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("Playing")));
        assert!(
            has_playing,
            "Playback needs a non-color cue independent of favorite tag"
        );
    }
}

#[test]
fn result_shortcuts_require_list_focus_and_reveal_keyboard_selection() {
    let ctx = context();
    let rows: Vec<_> = (0..80).map(result).collect();
    let id = egui::Id::new("keyboard-results");
    let draw = |ui: &mut egui::Ui| daw_ui::sample_results(ui, id, &rows, Some("result-0"), 140.0);
    frame(&ctx, 420.0, vec![], draw);
    let initial = frame(&ctx, 420.0, vec![], draw).1;
    let unowned = frame(
        &ctx,
        420.0,
        vec![key(egui::Key::ArrowDown), key(egui::Key::Space)],
        draw,
    )
    .1;
    assert!(unowned.selected.is_none() && !unowned.toggle_preview);
    click(&ctx, 420.0, initial.rows[0].1.rect.center(), draw);
    assert!(daw_ui::sample_results_owns_keys(&ctx));
    let down = frame(&ctx, 420.0, vec![key(egui::Key::ArrowDown)], draw).1;
    assert_eq!(down.selected.as_deref(), Some("result-1"));
    let space = frame(&ctx, 420.0, vec![key(egui::Key::Space)], draw).1;
    assert!(space.toggle_preview && space.preview.is_none());
    let enter = frame(&ctx, 420.0, vec![key(egui::Key::Enter)], draw).1;
    assert_eq!(enter.preview.as_deref(), Some("result-0"));
    let mut selected = "result-0".to_owned();
    for _ in 0..12 {
        let next = frame(&ctx, 420.0, vec![key(egui::Key::ArrowDown)], |ui| {
            daw_ui::sample_results(ui, id, &rows, Some(&selected), 140.0)
        })
        .1;
        selected = next.selected.unwrap();
    }
    let shown = frame(&ctx, 420.0, vec![], |ui| {
        daw_ui::sample_results(ui, id, &rows, Some(&selected), 140.0)
    })
    .1;
    assert!(
        shown.rows.iter().any(|(row, _)| row == &selected),
        "Keyboard selection must be revealed in virtual rows"
    );
    ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("search-field")));
    assert!(!daw_ui::sample_results_owns_keys(&ctx));
    let mut query = String::new();
    let no_preview = frame(&ctx, 420.0, vec![key(egui::Key::Space)], |ui| {
        ui.add(egui::TextEdit::singleline(&mut query).id(egui::Id::new("search-field")));
        daw_ui::sample_results(ui, id, &rows, Some(&selected), 140.0)
    })
    .1;
    assert!(!no_preview.toggle_preview && no_preview.preview.is_none());
}

#[test]
fn source_disclosure_does_not_select_and_project_drop_stays_visible() {
    let ctx = context();
    let collections = vec![
        source("all", "All samples", false),
        source("favorites", "Favorites", false),
        source("project", "Project samples", true),
    ];
    let mut folder = source("root", "Analog", false);
    folder.children = (0..100)
        .map(|index| {
            source(
                &format!("folder-{index}"),
                &format!("Folder {index}"),
                false,
            )
        })
        .collect();
    let folders = vec![folder];
    let draw = |ui: &mut egui::Ui| {
        daw_ui::sample_sources(
            ui,
            egui::Id::new("sources"),
            &collections,
            &folders,
            "favorites",
            320.0,
        )
    };
    frame(&ctx, 220.0, vec![], draw);
    frame(&ctx, 220.0, vec![], draw);
    let expand = daw_ui::icon_only_labels(&ctx)
        .iter()
        .find(|(label, _)| label == "Expand Analog")
        .unwrap()
        .1
        .center();
    let opened = click(&ctx, 220.0, expand, draw);
    assert!(opened.selected.is_none());
    let shown = frame(&ctx, 220.0, vec![], draw).1;
    assert!(shown.rows.iter().any(|(id, _)| id == "folder-0"));
    assert_eq!(shown.drop_targets.len(), 1);
    assert!(shown.drop_targets[0].1.rect.bottom() < 150.0);
    let root = shown
        .rows
        .iter()
        .find(|(id, _)| id == "root")
        .unwrap()
        .1
        .rect
        .center();
    assert_eq!(
        click(&ctx, 220.0, root, draw).selected.as_deref(),
        Some("root")
    );
}

#[test]
fn search_and_clear_fit_narrow_panes_without_widening() {
    for width in [260.0, 420.0] {
        let ctx = context();
        let mut query = "warm analog".to_owned();
        let (paint, rect) = frame(&ctx, width, vec![], |ui| {
            daw_ui::sample_search(ui, &mut query, "A very long named source collection", 12);
            ui.min_rect()
        });
        assert!(
            rect.right() <= width - 7.9,
            "Search expands parent: {rect:?}"
        );
        let labels = daw_ui::icon_only_labels(&ctx);
        assert!(labels.iter().all(|(label, _)| label != "Keyboard"));
        let (_, clear) = labels
            .iter()
            .find(|(label, _)| label == "Clear search")
            .unwrap();
        assert!(clear.width() >= 33.9 && clear.height() >= 33.9);
        assert!(
            clear.right() <= width - 7.9,
            "Clear search leaves pane: {clear:?}"
        );
        assert!(paint.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("12 samples"))));
    }
}

#[test]
fn inspector_headers_and_actions_stay_fixed_when_body_overflows() {
    let ctx = context();
    let mut bounds = Vec::new();
    let _ = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 720.0))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                daw_ui::sample_browser_workspace(
                    ui,
                    egui::Id::new("workspace"),
                    600.0,
                    |pane, ui| {
                        let pane_rect = ui.max_rect();
                        daw_ui::sample_regions(
                            ui,
                            egui::Id::new(pane),
                            ui.available_height(),
                            100.0,
                            100.0,
                            |region, ui| match region {
                                daw_ui::SampleRegion::Header => {
                                    bounds.push((pane_rect, daw_ui::button(ui, "Preview").rect));
                                }
                                daw_ui::SampleRegion::Body => {
                                    for _ in 0..100 {
                                        daw_ui::text::note(ui, "Long metadata or tag content");
                                    }
                                }
                                daw_ui::SampleRegion::Footer => {
                                    bounds.push((pane_rect, daw_ui::button(ui, "Use sample").rect));
                                }
                            },
                        );
                    },
                );
            });
        },
    );
    assert_eq!(bounds.len(), 6);
    for (pane, control) in bounds {
        assert!(pane.contains_rect(control));
        assert!(control.width() >= 34.0 && control.height() >= 34.0);
    }
}

#[test]
fn pcm_details_use_musical_encoding_labels_without_duplicate_raw_codec() {
    let ctx = context();
    let details = SampleDetails {
        name: "Kick",
        location: "Project samples",
        duration_seconds: Some(1.0),
        format: Some("WAV"),
        codec: Some("PCM_F32LE"),
        sample_rate: Some(48_000),
        channels: Some(2),
        bit_depth: Some(32),
        ..Default::default()
    };
    let (paint, _) = frame(&ctx, 280.0, vec![], |ui| {
        daw_ui::sample_details(ui, &details)
    });
    text_rect(&paint, "1.00 s · WAV");
    text_rect(&paint, "48 kHz · Stereo · 32-bit float");
    assert!(!paint.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("PCM_F32LE"))));
}

#[test]
fn flat_favorite_changes_only_membership_and_popup_blocks_result_shortcuts() {
    let ctx = context();
    let mut rows = vec![result(0), result(1)];
    rows[1].path = rows[0].path.clone();
    rows[1].favorite = Some(SampleFavorite {
        tag: "Soft kick".into(),
        icon: SampleTagIcon::Drum,
        ..Default::default()
    });
    let id = egui::Id::new("favorite-results");
    let draw = |ui: &mut egui::Ui| daw_ui::sample_results(ui, id, &rows, Some("result-0"), 240.0);
    frame(&ctx, 420.0, vec![], draw);
    let initial = frame(&ctx, 420.0, vec![], draw).1;
    click(&ctx, 420.0, initial.rows[0].1.rect.center(), draw);
    let favorite = daw_ui::icon_only_labels(&ctx)
        .into_iter()
        .find(|(label, _)| label == "Remove favorite")
        .unwrap()
        .1
        .center();
    let changed = click(&ctx, 420.0, favorite, draw);
    assert_eq!(changed.favorite_toggled.as_deref(), Some("result-1"));
    assert!(changed.selected.is_none() && changed.preview.is_none());
    assert!(!daw_ui::sample_results_owns_keys(&ctx));
    let space = frame(&ctx, 420.0, vec![key(egui::Key::Space)], draw).1;
    assert!(!space.toggle_preview && space.preview.is_none());

    click(&ctx, 420.0, initial.rows[0].1.rect.center(), draw);
    egui::Popup::open_id(&ctx, egui::Id::new("sample-use-menu"));
    assert!(!daw_ui::sample_results_owns_keys(&ctx));
    let blocked = frame(
        &ctx,
        420.0,
        vec![
            key(egui::Key::Space),
            key(egui::Key::Enter),
            key(egui::Key::ArrowDown),
        ],
        draw,
    )
    .1;
    assert!(!blocked.toggle_preview && blocked.preview.is_none() && blocked.selected.is_none());
}

#[test]
fn flat_result_drop_preserves_alias_name_into_visible_project_collection() {
    let ctx = context();
    let collections = vec![
        source("all", "All samples", false),
        source("project", "Project samples", true),
    ];
    let rows = vec![result(0)];
    let draw = |ui: &mut egui::Ui| {
        let sources = daw_ui::sample_sources(
            ui,
            egui::Id::new("drop-sources"),
            &collections,
            &[],
            "all",
            170.0,
        );
        let results = daw_ui::sample_results(ui, egui::Id::new("drop-results"), &rows, None, 150.0);
        (sources, results)
    };
    frame(&ctx, 420.0, vec![], draw);
    let (_, (sources, results)) = frame(&ctx, 420.0, vec![], draw);
    assert_eq!(sources.drop_targets.len(), 1);
    let origin = results.rows[0].1.rect.center();
    let target = sources.drop_targets[0].1.rect.center();
    frame(&ctx, 420.0, pointer(origin, true), draw);
    frame(
        &ctx,
        420.0,
        vec![Event::PointerMoved(origin + vec2(20.0, 0.0))],
        draw,
    );
    frame(&ctx, 420.0, vec![Event::PointerMoved(target)], draw);
    let (_, (sources, _)) = frame(&ctx, 420.0, pointer(target, false), draw);
    assert_eq!(
        sources.dropped,
        Some((
            "project".into(),
            SampleDrop {
                path: rows[0].path.clone().into(),
                name: rows[0].source_name.clone()
            }
        ))
    );
    assert!(
        sources.selected.is_none(),
        "Drop does not navigate away from the current scope"
    );
}

#[test]
fn double_click_requests_preview_without_any_load_or_favorite_action() {
    let ctx = context();
    let rows = vec![result(0), result(1)];
    let id = egui::Id::new("double-click-results");
    let draw = |ui: &mut egui::Ui| daw_ui::sample_results(ui, id, &rows, Some("result-0"), 240.0);
    frame(&ctx, 420.0, vec![], draw);
    let initial = frame(&ctx, 420.0, vec![], draw).1;
    let target = initial.rows[1].1.rect.center();
    let once = click(&ctx, 420.0, target, draw);
    assert_eq!(once.selected.as_deref(), Some("result-1"));
    assert!(once.preview.is_none());
    let twice = click(&ctx, 420.0, target, draw);
    assert_eq!(twice.preview.as_deref(), Some("result-1"));
    assert!(twice.favorite_toggled.is_none() && twice.dragged.is_none());
}

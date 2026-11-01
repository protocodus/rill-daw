use daw_ui::{
    ParameterBrowserResponse, ParameterBrowserState, ParameterItem, ParameterScope, theme,
};
use egui::{Context, Event, FullOutput, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};

fn items() -> Vec<ParameterItem<'static>> {
    vec![
        ParameterItem {
            id: 10,
            name: "Pre filter cutoff",
            unit: "Hz",
            value: 0.5,
            automated: false,
            last_touched: false,
        },
        ParameterItem {
            id: 11,
            name: "Cutoff frequency",
            unit: "Hz",
            value: 0.7,
            automated: true,
            last_touched: false,
        },
        ParameterItem {
            id: 12,
            name: "Resonance",
            unit: "",
            value: 0.2,
            automated: false,
            last_touched: true,
        },
        ParameterItem {
            id: 13,
            name: "明るさ 🎛",
            unit: "%",
            value: 0.3,
            automated: true,
            last_touched: false,
        },
    ]
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn pointer(point: Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(point),
        Event::PointerButton {
            pos: point,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn frame(
    ctx: &Context,
    state: &mut ParameterBrowserState,
    items: &[ParameterItem<'_>],
    events: Vec<Event>,
) -> (FullOutput, ParameterBrowserResponse<Option<u32>>) {
    let mut result = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(760.0, 480.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = Some(daw_ui::parameter_browser(
                    ui,
                    "parameters",
                    state,
                    items,
                    440.0,
                    |ui, item| {
                        daw_ui::button(ui, "Create lane");
                        item.map(|item| item.id)
                    },
                ));
            });
        },
    );
    (output, result.unwrap())
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
fn focused_search_ranks_tokens_and_keyboard_activates_only_visible_selection() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let items = items();
    let mut state = ParameterBrowserState {
        focus_search: true,
        ..Default::default()
    };
    for _ in 0..3 {
        frame(&ctx, &mut state, &items, vec![]);
    }
    let (output, result) = frame(
        &ctx,
        &mut state,
        &items,
        vec![Event::Text("CUTOFF Hz".into())],
    );
    assert_eq!(state.query, "CUTOFF Hz");
    assert_eq!(
        result.inner,
        Some(10),
        "token search matches names and units"
    );
    text_rect(&output, "50.0% · Hz");
    let (_, result) = frame(
        &ctx,
        &mut state,
        &items,
        vec![key(Key::ArrowDown, Modifiers::NONE)],
    );
    assert_eq!(result.inner, Some(11));
    let (_, result) = frame(
        &ctx,
        &mut state,
        &items,
        vec![key(Key::Enter, Modifiers::NONE)],
    );
    assert_eq!(result.activated, Some(11));
    state.query = "definitely missing".into();
    let (output, result) = frame(
        &ctx,
        &mut state,
        &items,
        vec![key(Key::Enter, Modifiers::NONE)],
    );
    assert!(state.selected.is_none());
    assert_eq!(result.inner, None);
    assert_eq!(result.activated, None);
    text_rect(&output, "No matching parameters");

    state.query.clear();
    state.focus_search = true;
    for _ in 0..2 {
        frame(&ctx, &mut state, &items, vec![]);
    }
    let (_, result) = frame(&ctx, &mut state, &items, vec![Event::Text("cutoff".into())]);
    assert_eq!(
        result.inner,
        Some(11),
        "name-prefix matches rank ahead of later substrings"
    );
    let mut percentage_items = items.clone();
    for unit in ["%", " % "] {
        percentage_items[3].unit = unit;
        state.query = "%".into();
        let (output, result) = frame(&ctx, &mut state, &percentage_items, vec![]);
        assert_eq!(result.inner, Some(13), "Unit metadata remains searchable");
        assert_eq!(percentage_items[3].unit, unit);
        text_rect(&output, "30.0% · Automated");
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text().contains("% · %")
                || text.galley.text().contains("% ·  %")
        )));
    }
}

#[test]
fn modified_navigation_keys_preserve_selection_and_never_activate_a_parameter() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let items = items();
    let mut state = ParameterBrowserState::default();
    for _ in 0..3 {
        frame(&ctx, &mut state, &items, vec![]);
    }
    assert_eq!(state.selected, Some(10));
    for modifiers in [
        Modifiers::SHIFT,
        Modifiers::CTRL,
        Modifiers::ALT,
        Modifiers::COMMAND,
    ] {
        for navigation in [Key::ArrowDown, Key::Enter] {
            // The frame's final modifiers are NONE: the chord's own event
            // must decide whether this is result navigation.
            let (_, response) = frame(&ctx, &mut state, &items, vec![key(navigation, modifiers)]);
            assert_eq!(state.selected, Some(10));
            assert_eq!(response.activated, None);
        }
    }
    let (_, response) = frame(
        &ctx,
        &mut state,
        &items,
        vec![key(Key::ArrowDown, Modifiers::NONE)],
    );
    assert_eq!(state.selected, Some(11));
    assert_eq!(response.activated, None);
    let (_, response) = frame(
        &ctx,
        &mut state,
        &items,
        vec![key(Key::Enter, Modifiers::NONE)],
    );
    assert_eq!(response.activated, Some(11));
}

#[test]
fn scope_clicks_and_rows_update_the_detail_pane_immediately_and_activate_with_enter() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let items = items();
    let mut state = ParameterBrowserState::default();
    let mut output = frame(&ctx, &mut state, &items, vec![]).0;
    for _ in 0..3 {
        output = frame(&ctx, &mut state, &items, vec![]).0;
    }
    let automated = text_rect(&output, "Automated").center();
    frame(&ctx, &mut state, &items, pointer(automated, true));
    let (output, result) = frame(&ctx, &mut state, &items, pointer(automated, false));
    assert_eq!(state.scope, ParameterScope::Automated);
    assert_eq!(result.inner, Some(11));
    let unicode = text_rect(&output, "明るさ 🎛").center();
    frame(&ctx, &mut state, &items, pointer(unicode, true));
    let (_, result) = frame(&ctx, &mut state, &items, pointer(unicode, false));
    assert_eq!(
        result.inner,
        Some(13),
        "details see the row selection in the same frame"
    );
    let (_, result) = frame(
        &ctx,
        &mut state,
        &items,
        vec![key(Key::Enter, Modifiers::NONE)],
    );
    assert_eq!(result.activated, Some(13));
    let output = frame(&ctx, &mut state, &items, vec![]).0;
    let last_touched = text_rect(&output, "Last touched").center();
    frame(&ctx, &mut state, &items, pointer(last_touched, true));
    let (_, result) = frame(&ctx, &mut state, &items, pointer(last_touched, false));
    assert_eq!(state.scope, ParameterScope::LastTouched);
    assert_eq!(result.inner, Some(12));
    frame(
        &ctx,
        &mut state,
        &items,
        vec![key(Key::F, Modifiers::COMMAND)],
    );
    frame(&ctx, &mut state, &items, vec![Event::Text("reso".into())]);
    assert_eq!(
        state.query, "reso",
        "the search shortcut restores text focus"
    );
    let output = frame(&ctx, &mut state, &items, vec![]).0;
    let resonance = text_rect(&output, "Resonance").center();
    let mut activated = None;
    for (index, pressed) in [true, false, true, false].into_iter().enumerate() {
        let (_, response) = frame(&ctx, &mut state, &items, pointer(resonance, pressed));
        if index < 2 {
            assert!(
                response.activated.is_none(),
                "a first row click after other controls only selects"
            );
        }
        activated = response.activated.or(activated);
    }
    assert_eq!(
        activated,
        Some(12),
        "double-click activates the chosen parameter"
    );
}

#[test]
fn large_catalog_virtualizes_rows_and_stays_bounded_in_both_themes_and_densities() {
    let names: Vec<_> = (0..10_000)
        .map(|index| format!("Parameter {index:05} · 音 🎛 long name"))
        .collect();
    let items: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(id, name)| ParameterItem {
            id: id as u32,
            name,
            unit: "Hz",
            value: if id == 0 { f64::NAN } else { 0.5 },
            automated: false,
            last_touched: false,
        })
        .collect();
    for (appearance, density) in [
        (theme::Appearance::Light, 1.0),
        (theme::Appearance::Dark, 2.0),
    ] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(&ctx, appearance);
        ctx.set_pixels_per_point(density);
        let mut state = ParameterBrowserState::default();
        for _ in 0..3 {
            frame(&ctx, &mut state, &items, vec![]);
        }
        let (output, _) = frame(&ctx, &mut state, &items, vec![]);
        let mut rendered_rows = 0;
        for shape in output.shapes {
            match shape.shape {
                egui::Shape::Text(text) => {
                    if text.galley.text().starts_with("Parameter ") {
                        rendered_rows += 1;
                    }
                    assert!(text.pos.x >= 0.0 && text.pos.y >= 0.0);
                    assert!(text.pos.x + text.galley.size().x <= 760.0);
                }
                egui::Shape::Rect(rect) => assert!(rect.rect.is_finite()),
                _ => {}
            }
        }
        assert!(
            rendered_rows > 0 && rendered_rows < 10,
            "only visible catalog rows should be painted"
        );
    }
}

#[test]
fn disabled_browser_and_an_open_native_popup_do_not_activate_parameters() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let items = items();
    let mut state = ParameterBrowserState::default();
    let _ = ctx.run(
        RawInput {
            events: vec![key(Key::Enter, Modifiers::NONE)],
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_enabled_ui(false, |ui| {
                    let response = daw_ui::parameter_browser(
                        ui,
                        "disabled",
                        &mut state,
                        &items,
                        400.0,
                        |_, _| {},
                    );
                    assert!(response.activated.is_none());
                });
            });
        },
    );
    for _ in 0..3 {
        frame(&ctx, &mut state, &items, vec![]);
    }
    egui::Popup::open_id(&ctx, egui::Id::new("other_popup"));
    let (_, response) = frame(
        &ctx,
        &mut state,
        &items,
        vec![key(Key::Enter, Modifiers::NONE)],
    );
    assert!(response.activated.is_none());
}

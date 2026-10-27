//! Searchable, bounded parameter discovery shared by instrument and effect tools.
//! Parameter identities and values remain with the caller; browsing never edits them.
use crate::{Icon, IconButton, text::TextRole, theme, tokens};
use egui::{Key, Modifiers, Rect, Response, Ui};
use std::hash::Hash;

#[derive(Clone, Copy, Debug)]
pub struct ParameterItem<'a> {
    pub id: u32,
    pub name: &'a str,
    pub unit: &'a str,
    pub value: f64,
    pub automated: bool,
    pub last_touched: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParameterScope {
    #[default]
    All,
    Automated,
    LastTouched,
}

#[derive(Clone, Debug, Default)]
pub struct ParameterBrowserState {
    pub query: String,
    pub selected: Option<u32>,
    pub focus_search: bool,
    pub scope: ParameterScope,
}

pub struct ParameterBrowserResponse<R> {
    pub activated: Option<u32>,
    pub inner: R,
}

const ROW_HEIGHT: f32 = 64.0;

fn matches(items: &[ParameterItem<'_>], state: &ParameterBrowserState) -> Vec<usize> {
    let query = state.query.trim().to_lowercase();
    let tokens: Vec<_> = query.split_whitespace().collect();
    let mut matches: Vec<_> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| match state.scope {
            ParameterScope::All => true,
            ParameterScope::Automated => item.automated,
            ParameterScope::LastTouched => item.last_touched,
        })
        .filter_map(|(index, item)| {
            if tokens.is_empty() {
                return Some((index, 0));
            }
            let name = item.name.to_lowercase();
            let unit = item.unit.to_lowercase();
            tokens
                .iter()
                .all(|token| name.contains(token) || unit.contains(token))
                .then(|| {
                    let rank = if name == query {
                        0
                    } else if name.starts_with(&query) {
                        1
                    } else {
                        2
                    };
                    (index, rank)
                })
        })
        .collect();
    matches.sort_by_key(|(_, rank)| *rank);
    matches.into_iter().map(|(index, _)| index).collect()
}

fn parameter_row(ui: &mut Ui, item: &ParameterItem<'_>, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(0.0), ROW_HEIGHT),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            item.name,
        )
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let colors = theme::palette(ui.ctx());
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        tokens::SURFACE_RADIUS,
        if selected {
            colors.track_selected
        } else if response.hovered() {
            colors.control_hover
        } else {
            colors.panel
        },
    );
    let icon = Rect::from_center_size(
        egui::pos2(rect.left() + 28.0, rect.center().y),
        egui::Vec2::splat(22.0),
    );
    painter.circle_filled(icon.center(), 18.0, colors.control_fill);
    if item.automated {
        Icon::Automation
    } else if item.last_touched {
        Icon::Cursor
    } else {
        Icon::Settings
    }
    .paint(
        &painter,
        icon,
        if selected || item.automated || item.last_touched {
            colors.accent
        } else {
            colors.muted
        },
    );
    let text_left = (rect.left() + 54.0).min(rect.right());
    let text_right = (rect.right() - tokens::SPACE_MD).max(text_left);
    crate::text::paint_row(
        &painter,
        Rect::from_min_max(
            egui::pos2(text_left, rect.top() + 6.0),
            egui::pos2(text_right, rect.top() + 30.0),
        ),
        TextRole::Label,
        item.name,
    );
    let value = if item.value.is_finite() {
        item.value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut detail = format!("{:.1}%", value * 100.0);
    if !item.unit.is_empty() {
        detail.push_str(" · ");
        detail.push_str(item.unit);
    }
    if item.last_touched {
        detail.push_str(" · Last touched");
    } else if item.automated {
        detail.push_str(" · Automated");
    }
    crate::text::paint_row(
        &painter,
        Rect::from_min_max(
            egui::pos2(text_left, rect.top() + 30.0),
            egui::pos2(text_right, rect.bottom() - 10.0),
        ),
        TextRole::Note,
        &detail,
    );
    let rail = Rect::from_min_max(
        egui::pos2(text_left, rect.bottom() - 7.0),
        egui::pos2(text_right, rect.bottom() - 5.0),
    );
    painter.rect_filled(rail, 1.0, colors.line);
    painter.rect_filled(
        Rect::from_min_size(
            rail.min,
            egui::vec2(rail.width() * value as f32, rail.height()),
        ),
        1.0,
        colors.accent,
    );
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            tokens::SURFACE_RADIUS,
            egui::Stroke::new(1.0_f32, colors.accent),
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text(format!("{}\n{detail}\nDouble-click to automate", item.name))
}

/// A total height budget including search, scope controls and two equal panes.
/// The details closure receives the current visible selection after list input.
/// Its pane scrolls independently, so callers should place primary actions first.
pub fn parameter_browser<R>(
    ui: &mut Ui,
    id_salt: impl Hash,
    state: &mut ParameterBrowserState,
    items: &[ParameterItem<'_>],
    height: f32,
    details: impl FnOnce(&mut Ui, Option<&ParameterItem<'_>>) -> R,
) -> ParameterBrowserResponse<R> {
    ui.push_id(id_salt, |ui| {
        let height = if height.is_finite() {
            height.max(0.0).min(ui.available_height().max(0.0))
        } else {
            ui.available_height().clamp(0.0, 480.0)
        };
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(0.0), height),
            egui::Sense::hover(),
        );
        let mut body = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("browser_body")
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        body.set_clip_rect(ui.clip_rect().intersect(rect));
        let ui = &mut body;
        let enabled = ui.is_enabled() && !egui::Popup::is_any_open(ui.ctx());
        if enabled && ui.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::F)) {
            state.focus_search = true;
        }
        // Read the key before TextEdit processes it, but activate only when the
        // search/list owns focus. Match each event's modifiers so text-selection
        // chords and released modifiers do not become list navigation.
        let (up, down, enter) = ui.input(|input| {
            let pressed = |key| {
                input.events.iter().any(|event| {
                    matches!(event,
                        egui::Event::Key { key: candidate, pressed: true, modifiers, .. }
                            if *candidate == key && !modifiers.any()
                    )
                })
            };
            (
                pressed(Key::ArrowUp),
                pressed(Key::ArrowDown),
                pressed(Key::Enter),
            )
        });
        let previous_query = state.query.clone();
        let previous_scope = state.scope;
        let search = ui
            .horizontal(|ui| {
                let (icon, _) = ui.allocate_exact_size(
                    egui::vec2(22.0, tokens::CONTROL_HEIGHT),
                    egui::Sense::hover(),
                );
                Icon::Search.paint(
                    ui.painter(),
                    icon.shrink(2.0),
                    theme::palette(ui.ctx()).muted,
                );
                let width = (ui.available_width()
                    - 4.0 * tokens::BUTTON_PADDING_X
                    - 18.0
                    - tokens::SPACE_SM)
                    .max(0.0);
                let search = crate::text_field(ui, &mut state.query, "Search parameters", width);
                if ui
                    .add_enabled(
                        !state.query.is_empty(),
                        IconButton::icon_only(Icon::Close, "Clear search"),
                    )
                    .on_hover_text("Clear search")
                    .clicked()
                {
                    state.query.clear();
                    state.focus_search = true;
                }
                search
            })
            .inner;
        if state.focus_search && enabled {
            search.request_focus();
            if search.has_focus() {
                state.focus_search = false;
            }
        }
        ui.horizontal_wrapped(|ui| {
            for (scope, label) in [
                (ParameterScope::All, "All"),
                (ParameterScope::Automated, "Automated"),
                (ParameterScope::LastTouched, "Last touched"),
            ] {
                crate::selectable_value(ui, &mut state.scope, scope, label);
            }
        });
        if state.scope != previous_scope && enabled {
            search.request_focus();
            state.focus_search = true;
        }
        let matches = matches(items, state);
        let query_changed = state.query != previous_query || state.scope != previous_scope;
        let click_key = ui.make_persistent_id("parameter_row_click");
        let mut last_click = ui.data_mut(|data| data.get_temp::<(u32, f64, egui::Pos2)>(click_key));
        if query_changed {
            last_click = None;
        }
        let pointer_clicked = ui.input(|input| input.pointer.primary_clicked());
        let mut clicked_row = false;
        if query_changed
            || !matches
                .iter()
                .any(|&index| Some(items[index].id) == state.selected)
        {
            state.selected = matches.first().map(|&index| items[index].id);
        }
        let list_focus = ui.make_persistent_id("parameter_list_focus");
        ui.interact(
            Rect::from_min_size(ui.cursor().min, egui::Vec2::ZERO),
            list_focus,
            egui::Sense::focusable_noninteractive(),
        );
        let keyboard = enabled
            && (search.has_focus()
                || state.focus_search
                || ui.memory(|memory| memory.has_focus(list_focus))
                || !ui.ctx().wants_keyboard_input());
        let step = if keyboard && down {
            1
        } else if keyboard && up {
            -1
        } else {
            0
        };
        if step != 0 && !matches.is_empty() {
            let selected = matches
                .iter()
                .position(|&index| Some(items[index].id) == state.selected)
                .unwrap_or(0);
            let selected = (selected as isize + step).clamp(0, matches.len() as isize - 1) as usize;
            state.selected = Some(items[matches[selected]].id);
            ui.input_mut(|input| {
                input.consume_key(
                    Modifiers::NONE,
                    if step > 0 {
                        Key::ArrowDown
                    } else {
                        Key::ArrowUp
                    },
                );
            });
        }
        let mut activated = (keyboard && enter).then_some(state.selected).flatten();
        if activated.is_some() {
            ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
        }
        let count = if matches.len() == 1 {
            "1 parameter".to_owned()
        } else {
            format!("{} parameters", matches.len())
        };
        crate::text::note(ui, count);
        let mut details = Some(details);
        let mut inner = None;
        let pane_height = ui.available_height().max(0.0);
        crate::bounded_columns(ui, "parameter_panes", pane_height, |pane, ui| {
            if pane == 0 {
                if matches.is_empty() {
                    crate::text::subtitle(ui, "No matching parameters");
                    crate::text::note(ui, "Try a shorter search or another filter.");
                    return;
                }
                let mut scroll = egui::ScrollArea::vertical()
                    .id_salt("parameter_results")
                    .min_scrolled_height(0.0)
                    .max_height(ui.available_height().max(0.0))
                    .auto_shrink([false, false]);
                if query_changed {
                    scroll = scroll.vertical_scroll_offset(0.0);
                } else if step != 0 {
                    let selected = matches
                        .iter()
                        .position(|&index| Some(items[index].id) == state.selected)
                        .unwrap_or(0);
                    scroll = scroll.vertical_scroll_offset(
                        (selected as f32 * (ROW_HEIGHT + ui.spacing().item_spacing.y)
                            - pane_height * 0.4)
                            .max(0.0),
                    );
                }
                scroll.show_rows(ui, ROW_HEIGHT, matches.len(), |ui, range| {
                    for result in range {
                        let item = &items[matches[result]];
                        let row = parameter_row(ui, item, state.selected == Some(item.id));
                        if row.clicked() {
                            state.selected = Some(item.id);
                            ui.memory_mut(|memory| memory.request_focus(list_focus));
                            if pointer_clicked && let Some(position) = row.interact_pointer_pos() {
                                clicked_row = true;
                                let now = ui.input(|input| input.time);
                                let (delay, distance) = ui.ctx().options(|options| {
                                    (
                                        options.input_options.max_double_click_delay,
                                        options.input_options.max_click_dist,
                                    )
                                });
                                // Native egui counts double/triple clicks globally,
                                // including a preceding filter click. Require two
                                // clicks on this row, independent of that count.
                                if last_click.is_some_and(|(id, time, point)| {
                                    id == item.id
                                        && now > time
                                        && now - time < delay
                                        && point.distance(position) <= distance
                                }) {
                                    activated = Some(item.id);
                                }
                                last_click = Some((item.id, now, position));
                            }
                        }
                    }
                });
            } else {
                let selected = matches
                    .iter()
                    .find(|&&index| Some(items[index].id) == state.selected)
                    .map(|&index| &items[index]);
                inner = Some(
                    crate::scroll_body(ui, "parameter_details", |ui| {
                        details.take().unwrap()(ui, selected)
                    })
                    .inner,
                );
            }
        });
        if pointer_clicked && !clicked_row {
            last_click = None;
        }
        ui.data_mut(|data| {
            if let Some(click) = last_click {
                data.insert_temp(click_key, click);
            } else {
                data.remove::<(u32, f64, egui::Pos2)>(click_key);
            }
        });
        ParameterBrowserResponse {
            activated,
            inner: inner.unwrap(),
        }
    })
    .inner
}

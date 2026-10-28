//! Cached device previews in a responsive, virtualized grid or list. Device
//! identities, image loading and insertion actions belong to the caller.
use crate::{Icon, text::TextRole, theme, tokens};
use egui::{Id, Rect, Response, Ui};
use std::hash::Hash;

pub const DEVICE_CARD_HEIGHT: f32 = 208.0;
pub const DEVICE_CARD_MIN_WIDTH: f32 = 196.0;
pub const DEVICE_ROW_HEIGHT: f32 = 56.0;
const PREVIEW_HEIGHT: f32 = 118.0;
const ROW_PREVIEW_WIDTH: f32 = 80.0;
const SCROLLBAR_GUTTER: f32 = 16.0;

pub struct DeviceCard<'a> {
    /// Stable identity independent of filtering, sorting and grid position.
    pub id: Id,
    pub name: &'a str,
    pub producer: &'a str,
    pub preview: Option<&'a egui::TextureHandle>,
    pub selected: bool,
    /// Marked with a star badge; toggling it is the caller's action.
    pub favorite: bool,
    pub icon: Icon,
}

pub struct DeviceCardResponse {
    pub response: Response,
    /// Two consecutive clicks on this card, or keyboard activation while focused.
    /// Native egui double-click state also includes clicks on other widgets.
    pub activated: bool,
    pub preview_rect: Rect,
}

impl DeviceCardResponse {
    /// Move focus with the caller's grid selection, cancelling egui's automatic
    /// spatial traversal for this key so it cannot override that selection.
    pub fn request_focus(&self) {
        self.response.ctx.memory_mut(|memory| {
            memory.move_focus(egui::FocusDirection::None);
            memory.request_focus(self.response.id);
        });
        lock_card_arrows(&self.response);
    }
}

fn lock_card_arrows(response: &Response) {
    response.ctx.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            response.id,
            egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                ..Default::default()
            },
        );
    });
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeviceGridLayout {
    pub columns: usize,
    pub card_width: f32,
    /// Includes the gap between consecutive rows.
    pub row_height: f32,
}

/// Use the entire outer grid width, including its scrollbar gutter. This is the
/// same calculation used by [`device_grid`], useful for Up/Down keyboard steps.
pub fn device_grid_layout(width: f32) -> DeviceGridLayout {
    let width = if width.is_finite() {
        (width - SCROLLBAR_GUTTER).max(0.0)
    } else {
        0.0
    };
    let gap = tokens::SPACE_MD;
    let columns = ((width + gap) / (DEVICE_CARD_MIN_WIDTH + gap))
        .floor()
        .max(1.0) as usize;
    DeviceGridLayout {
        columns,
        card_width: ((width - gap * columns.saturating_sub(1) as f32) / columns as f32).max(0.0),
        row_height: DEVICE_CARD_HEIGHT + gap,
    }
}

pub struct DeviceGridResponse<R> {
    /// Only rendered rows are returned; large catalogs do not build hidden cards.
    pub cards: Vec<(usize, R)>,
    pub rect: Rect,
    pub scroll_offset: f32,
    pub layout: DeviceGridLayout,
}

#[derive(Clone, Copy)]
struct ClickTracker {
    frame: u64,
    sequence: u64,
    last: Option<(Id, u64, f64, egui::Pos2)>,
}

fn click_tracker(ctx: &egui::Context) -> ClickTracker {
    let key = Id::new("device_card_clicks");
    let frame = ctx.cumulative_frame_nr();
    let clicked = ctx.input(|input| input.pointer.any_click());
    let mut tracker = ctx
        .data_mut(|data| data.get_temp::<ClickTracker>(key))
        .unwrap_or(ClickTracker {
            frame: u64::MAX,
            sequence: 0,
            last: None,
        });
    if tracker.frame != frame {
        tracker.frame = frame;
        if clicked {
            tracker.sequence = tracker.sequence.wrapping_add(1);
        }
        ctx.data_mut(|data| data.insert_temp(key, tracker));
    }
    tracker
}

/// Two consecutive clicks on the same stable identity, or keyboard activation.
fn activation(ui: &Ui, response: &Response, id: Id) -> bool {
    let mut tracker = click_tracker(ui.ctx());
    let pointer_click = ui.input(|input| input.pointer.primary_clicked());
    let mut activated = response.clicked() && !pointer_click;
    if response.clicked()
        && pointer_click
        && let Some(position) = response.interact_pointer_pos()
    {
        let now = ui.input(|input| input.time);
        let (delay, distance) = ui.ctx().options(|options| {
            (
                options.input_options.max_double_click_delay,
                options.input_options.max_click_dist,
            )
        });
        activated = tracker.last.is_some_and(|(last, sequence, time, point)| {
            last == id
                && sequence.wrapping_add(1) == tracker.sequence
                && now > time
                && now - time < delay
                && point.distance(position) <= distance
        });
        tracker.last = Some((id, tracker.sequence, now, position));
        ui.ctx()
            .data_mut(|data| data.insert_temp(Id::new("device_card_clicks"), tracker));
    }
    activated
}

fn producer<'a>(card: &DeviceCard<'a>) -> &'a str {
    if card.producer.trim().is_empty() {
        "Unknown developer"
    } else {
        card.producer
    }
}

/// Allocate one whole-item target with selected-item accessibility.
fn device_target(ui: &mut Ui, card: &DeviceCard<'_>, size: egui::Vec2) -> Response {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let response = ui.interact(rect, card.id, egui::Sense::click());
    lock_card_arrows(&response);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            card.selected,
            format!(
                "{} · {}{}",
                card.name,
                producer(card),
                if card.favorite { " · Favorite" } else { "" }
            ),
        )
    });
    response
}

fn paint_item_frame(
    painter: &egui::Painter,
    rect: Rect,
    card: &DeviceCard<'_>,
    response: &Response,
) {
    let colors = theme::palette(painter.ctx());
    painter.rect_filled(
        rect,
        tokens::SURFACE_RADIUS,
        if card.selected {
            colors.track_selected
        } else if response.hovered() {
            colors.control_hover
        } else {
            colors.raised
        },
    );
    let emphasized = card.selected || response.has_focus();
    painter.rect_stroke(
        rect,
        tokens::SURFACE_RADIUS,
        egui::Stroke::new(
            if emphasized { 1.5_f32 } else { 1.0_f32 },
            if emphasized {
                colors.accent
            } else {
                colors.line
            },
        ),
        egui::StrokeKind::Inside,
    );
}

/// Paint a cached preview fitted inside `rect`, or the device icon placeholder.
fn paint_preview(painter: &egui::Painter, rect: Rect, card: &DeviceCard<'_>) {
    let colors = theme::palette(painter.ctx());
    painter.rect_filled(rect, tokens::RADIUS, colors.bg);
    if let Some(texture) = card.preview {
        let original = texture.size_vec2();
        let image_space = rect.shrink(tokens::SPACE_XS);
        if original.x > 0.0 && original.y > 0.0 && image_space.is_positive() {
            let scale = (image_space.width() / original.x).min(image_space.height() / original.y);
            painter.image(
                texture.id(),
                Rect::from_center_size(image_space.center(), original * scale),
                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    } else {
        crate::paint_ambient(painter, rect);
        card.icon.paint(
            painter,
            Rect::from_center_size(
                rect.center(),
                egui::Vec2::splat((rect.height() * 0.3).min(34.0)),
            ),
            colors.muted,
        );
    }
}

/// A quiet favorite marker; it adds no hit target to the device item.
fn paint_favorite_badge(painter: &egui::Painter, corner: egui::Pos2, size: f32) {
    let colors = theme::palette(painter.ctx());
    let badge = Rect::from_min_size(corner - egui::vec2(size, 0.0), egui::Vec2::splat(size));
    painter.rect_filled(badge, tokens::RADIUS, colors.panel);
    Icon::StarFilled.paint(painter, badge.shrink(size * 0.18), colors.accent);
}

/// One quiet, whole-card input target. Preview arrival never moves its labels,
/// changes the target bounds, or alters the caller's selection.
pub fn device_card(ui: &mut Ui, card: &DeviceCard<'_>, width: f32) -> DeviceCardResponse {
    let width = if width.is_finite() {
        width.max(0.0)
    } else {
        0.0
    };
    let response = device_target(ui, card, egui::vec2(width, DEVICE_CARD_HEIGHT));
    let producer = producer(card);
    let activated = activation(ui, &response, card.id);
    let rect = response.rect;
    let inset = rect.shrink2(egui::vec2(
        tokens::SPACE_MD.min(width * 0.5),
        tokens::SPACE_MD,
    ));
    let preview_rect = Rect::from_min_size(inset.min, egui::vec2(inset.width(), PREVIEW_HEIGHT));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        paint_item_frame(&painter, rect, card, &response);
        paint_preview(&painter, preview_rect, card);
        if card.favorite {
            paint_favorite_badge(
                &painter,
                preview_rect.right_top() + egui::vec2(-tokens::SPACE_XS, tokens::SPACE_XS),
                22.0,
            );
        }
        crate::text::paint_bounded_block(
            &painter,
            Rect::from_min_max(
                egui::pos2(inset.left(), preview_rect.bottom() + tokens::SPACE_SM),
                egui::pos2(
                    inset.right(),
                    preview_rect.bottom() + tokens::SPACE_SM + 40.0,
                ),
            ),
            TextRole::Label,
            card.name,
            2,
        );
        crate::text::paint_row(
            &painter,
            Rect::from_min_max(
                egui::pos2(inset.left(), rect.bottom() - 28.0),
                egui::pos2(inset.right(), rect.bottom() - 10.0),
            ),
            TextRole::Note,
            producer,
        );
    }
    DeviceCardResponse {
        response: response.on_hover_text(format!("{}\n{producer}", card.name)),
        activated,
        preview_rect,
    }
}

/// The compact list counterpart of [`device_card`]: a small preview, the name
/// and producer, and an optional trailing detail such as a category.
pub fn device_row(
    ui: &mut Ui,
    card: &DeviceCard<'_>,
    detail: &str,
    width: f32,
) -> DeviceCardResponse {
    let width = if width.is_finite() {
        width.max(0.0)
    } else {
        0.0
    };
    let response = device_target(ui, card, egui::vec2(width, DEVICE_ROW_HEIGHT));
    let producer = producer(card);
    let activated = activation(ui, &response, card.id);
    let rect = response.rect;
    let inset = rect.shrink(tokens::SPACE_SM);
    let preview_rect = Rect::from_min_size(
        inset.min,
        egui::vec2(ROW_PREVIEW_WIDTH.min(inset.width() * 0.3), inset.height()),
    );
    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        paint_item_frame(&painter, rect, card, &response);
        paint_preview(&painter, preview_rect, card);
        let star = 18.0;
        let detail_width = if detail.is_empty() {
            0.0
        } else {
            (inset.width() * 0.3).min(220.0)
        };
        let trailing = inset.right() - star - tokens::SPACE_SM;
        let text = Rect::from_min_max(
            egui::pos2(preview_rect.right() + tokens::SPACE_MD, inset.top()),
            egui::pos2(trailing - detail_width - tokens::SPACE_SM, inset.bottom()),
        );
        let half = text.height() * 0.5;
        crate::text::paint_row(
            &painter,
            text.with_max_y(text.top() + half),
            TextRole::Label,
            card.name,
        );
        crate::text::paint_row(
            &painter,
            text.with_min_y(text.top() + half),
            TextRole::Note,
            producer,
        );
        if detail_width > 0.0 {
            crate::text::paint_row(
                &painter,
                Rect::from_min_max(
                    egui::pos2(trailing - detail_width, inset.top()),
                    egui::pos2(trailing, inset.bottom()),
                ),
                TextRole::Note,
                detail,
            );
        }
        if card.favorite {
            paint_favorite_badge(
                &painter,
                egui::pos2(inset.right(), inset.center().y - star * 0.5),
                star,
            );
        }
    }
    DeviceCardResponse {
        response: response.on_hover_text(format!("{}\n{producer}", card.name)),
        activated,
        preview_rect,
    }
}

/// A large preview for a selected device, fitted without changing the
/// allocated size when the image arrives later.
pub fn device_preview(
    ui: &mut Ui,
    preview: Option<&egui::TextureHandle>,
    icon: Icon,
    size: egui::Vec2,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(size.max(egui::Vec2::ZERO), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_preview(
            &ui.painter_at(rect),
            rect,
            &DeviceCard {
                id: response.id,
                name: "",
                producer: "",
                preview,
                selected: false,
                favorite: false,
                icon,
            },
        );
    }
    response
}

/// Toggle a device's favorite mark. The caller stores the choice.
pub fn favorite_toggle(ui: &mut Ui, favorite: &mut bool) -> Response {
    let mut response = ui.add(
        crate::IconButton::with_icon(
            if *favorite {
                Icon::StarFilled
            } else {
                Icon::Star
            },
            "Favorite",
        )
        .selected(*favorite),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            ui.is_enabled(),
            *favorite,
            "Favorite",
        )
    });
    if response.clicked() {
        *favorite = !*favorite;
        response.mark_changed();
    }
    response
}

/// Fixed-height, responsive card rows with bounded rendering and image-loading
/// work. `scroll_to` reveals a keyboard selection even when its row is virtualized.
pub fn device_grid<R>(
    ui: &mut Ui,
    id_salt: impl Hash,
    count: usize,
    height: f32,
    scroll_to: Option<usize>,
    render: impl FnMut(&mut Ui, usize, f32) -> R,
) -> DeviceGridResponse<R> {
    virtual_items(
        ui,
        id_salt,
        count,
        height,
        scroll_to,
        DEVICE_CARD_HEIGHT,
        device_grid_layout,
        render,
    )
}

/// The single-column layout used by [`device_list`].
pub fn device_list_layout(width: f32) -> DeviceGridLayout {
    let width = if width.is_finite() {
        (width - SCROLLBAR_GUTTER).max(0.0)
    } else {
        0.0
    };
    DeviceGridLayout {
        columns: 1,
        card_width: width,
        row_height: DEVICE_ROW_HEIGHT + tokens::SPACE_XS,
    }
}

/// Virtualized [`device_row`] items with the same scrolling contract as
/// [`device_grid`]; render each row with the supplied width.
pub fn device_list<R>(
    ui: &mut Ui,
    id_salt: impl Hash,
    count: usize,
    height: f32,
    scroll_to: Option<usize>,
    render: impl FnMut(&mut Ui, usize, f32) -> R,
) -> DeviceGridResponse<R> {
    virtual_items(
        ui,
        id_salt,
        count,
        height,
        scroll_to,
        DEVICE_ROW_HEIGHT,
        device_list_layout,
        render,
    )
}

#[allow(clippy::too_many_arguments)]
fn virtual_items<R>(
    ui: &mut Ui,
    id_salt: impl Hash,
    count: usize,
    height: f32,
    scroll_to: Option<usize>,
    item_height: f32,
    layout: fn(f32) -> DeviceGridLayout,
    mut render: impl FnMut(&mut Ui, usize, f32) -> R,
) -> DeviceGridResponse<R> {
    ui.push_id(id_salt, |ui| {
        let layout = layout(ui.available_width());
        // Keep a stable gutter independently of scrollbar appearance/animation.
        ui.spacing_mut().scroll.floating = true;
        ui.spacing_mut().scroll.floating_allocated_width = 0.0;
        ui.spacing_mut().item_spacing =
            egui::vec2(tokens::SPACE_MD, layout.row_height - item_height);
        let height = if height.is_finite() {
            height.max(0.0).min(ui.available_height().max(0.0))
        } else {
            0.0
        };
        let rows = count.div_ceil(layout.columns);
        // Track unrelated clicks too, including search/filter clicks when the
        // grid is empty. Two cards cannot share a double-click sequence.
        click_tracker(ui.ctx());
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("cards")
            .min_scrolled_height(0.0)
            .max_height(height)
            .auto_shrink([false, false]);
        if let Some(index) = scroll_to.filter(|index| *index < count) {
            scroll = scroll.vertical_scroll_offset(
                ((index / layout.columns) as f32 * layout.row_height
                    - (height - item_height).max(0.0) * 0.5)
                    .max(0.0),
            );
        }
        let result = scroll.show_rows(ui, item_height, rows, |ui, visible| {
            let mut cards = Vec::with_capacity(visible.len() * layout.columns);
            for row in visible {
                ui.horizontal(|ui| {
                    for column in 0..layout.columns {
                        let index = row * layout.columns + column;
                        if index >= count {
                            break;
                        }
                        cards.push((index, render(ui, index, layout.card_width)));
                    }
                });
            }
            cards
        });
        DeviceGridResponse {
            cards: result.inner,
            rect: result.inner_rect,
            scroll_offset: result.state.offset.y,
            layout,
        }
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(width: f32, time: f64, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 700.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        }
    }

    fn render_grid(
        ctx: &egui::Context,
        width: f32,
        count: usize,
        height: f32,
        scroll_to: Option<usize>,
    ) -> DeviceGridResponse<DeviceCardResponse> {
        let mut result = None;
        let _ = ctx.run(input(width, 0.0, vec![]), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = Some(device_grid(
                    ui,
                    "test",
                    count,
                    height,
                    scroll_to,
                    |ui, index, width| {
                        device_card(
                            ui,
                            &DeviceCard {
                                id: Id::new(("test_card", index)),
                                name: "Synth Ω 🎹",
                                producer: "音楽 Producer",
                                preview: None,
                                selected: index == 0,
                                favorite: false,
                                icon: Icon::Instrument,
                            },
                            width,
                        )
                    },
                ));
            });
        });
        result.unwrap()
    }

    #[test]
    fn responsive_grid_fits_columns_without_clipping_or_building_hidden_cards() {
        for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
            for width in [280.0, 640.0, 940.0, 1440.0] {
                let ctx = egui::Context::default();
                theme::apply(&ctx);
                ctx.set_theme(appearance.preference());
                let grid = render_grid(&ctx, width, 10_000, 420.0, None);
                assert!(
                    grid.cards.len() <= grid.layout.columns * 4,
                    "virtualized rows"
                );
                assert!(grid.cards.len() < 40);
                for (_, card) in &grid.cards {
                    let rect = card.response.rect;
                    assert!(rect.left() >= grid.rect.left() - 0.01);
                    assert!(rect.right() <= grid.rect.right() - SCROLLBAR_GUTTER + 0.01);
                    assert_eq!(rect.height(), DEVICE_CARD_HEIGHT);
                    assert!(rect.contains_rect(card.preview_rect));
                }
                let first_row = &grid.cards[..grid.layout.columns];
                for pair in first_row.windows(2) {
                    assert!(
                        (pair[1].1.response.rect.left()
                            - pair[0].1.response.rect.right()
                            - tokens::SPACE_MD)
                            .abs()
                            < 0.01
                    );
                    assert_eq!(pair[0].1.response.rect.top(), pair[1].1.response.rect.top());
                }
                let scrolled = render_grid(&ctx, width, 10_000, 420.0, Some(9_999));
                assert!(scrolled.cards.iter().any(|(index, _)| *index == 9_999));
                assert!(scrolled.scroll_offset > 0.0);
            }
        }
    }

    #[test]
    fn keyboard_scroll_reveals_the_entire_selected_card_in_short_viewports() {
        for height in [260.0, 400.0] {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            render_grid(&ctx, 940.0, 10_000, height, None);
            let grid = render_grid(&ctx, 940.0, 10_000, height, Some(1_009));
            let selected = grid
                .cards
                .iter()
                .find(|(index, _)| *index == 1_009)
                .unwrap();
            assert!(
                grid.rect.contains_rect(selected.1.response.rect),
                "entire selected card {:?} must fit viewport {:?}",
                selected.1.response.rect,
                grid.rect
            );
        }
    }

    #[test]
    fn requested_card_focus_wins_over_automatic_spatial_navigation() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        for (time, target, key) in [
            (0.0, 3, None),
            (0.1, 1, Some(egui::Key::ArrowUp)),
            (0.2, 0, Some(egui::Key::ArrowLeft)),
        ] {
            let events = key
                .map(|key| egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                })
                .into_iter()
                .collect();
            let _ = ctx.run(input(520.0, time, events), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = ui.button("Search above grid");
                    device_grid(ui, "focus_grid", 4, 500.0, None, |ui, index, width| {
                        let card = device_card(
                            ui,
                            &DeviceCard {
                                id: Id::new(("focus_card", index)),
                                name: "Instrument",
                                producer: "Producer",
                                preview: None,
                                selected: index == target,
                                favorite: false,
                                icon: Icon::Instrument,
                            },
                            width,
                        );
                        if index == target {
                            card.request_focus();
                        }
                    });
                });
            });
            assert!(ctx.memory(|memory| memory.has_focus(Id::new(("focus_card", target)))));
        }
    }

    #[test]
    fn preview_arrival_preserves_card_geometry_and_full_unicode_text() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let texture = ctx.load_texture(
            "fixture",
            egui::ColorImage::filled([80, 40], egui::Color32::BLUE),
            egui::TextureOptions::LINEAR,
        );
        let name = "音楽 · Fréquence Ω 🎹 — A very long instrument name that needs two rows";
        let producer = "製作者 — Unicode producer 🎛";
        let mut geometry = None;
        for preview in [None, Some(&texture)] {
            let mut card_rect = Rect::NOTHING;
            let mut preview_rect = Rect::NOTHING;
            let output = ctx.run(input(400.0, 0.0, vec![]), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let card = device_card(
                        ui,
                        &DeviceCard {
                            id: Id::new("same"),
                            name,
                            producer,
                            preview,
                            selected: true,
                            favorite: true,
                            icon: Icon::Instrument,
                        },
                        220.0,
                    );
                    card_rect = card.response.rect;
                    preview_rect = card.preview_rect;
                });
            });
            if let Some(expected) = geometry {
                assert_eq!((card_rect, preview_rect), expected);
            }
            geometry = Some((card_rect, preview_rect));
            let mut names = false;
            let mut producers = false;
            for shape in output.shapes {
                if let egui::Shape::Text(text) = shape.shape {
                    if text.galley.job.text == name {
                        names = true;
                        assert!(text.galley.rows.len() <= 2);
                        assert!(shape.clip_rect.intersect(card_rect) == shape.clip_rect);
                    }
                    if text.galley.job.text == producer {
                        producers = true;
                    }
                }
            }
            assert!(
                names && producers,
                "full text remains available in real text shapes"
            );
        }
    }

    #[test]
    fn card_activation_requires_consecutive_clicks_on_same_stable_identity() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let frame = |time: f64, events: Vec<egui::Event>| {
            let mut activated = Vec::new();
            let mut positions = Vec::new();
            let _ = ctx.run(input(700.0, time, events), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let outside = ui.button("Search/filter");
                    positions.push(outside.rect.center());
                    device_grid(ui, "test", 2, 420.0, None, |ui, index, width| {
                        let card = device_card(
                            ui,
                            &DeviceCard {
                                id: Id::new(("card", index)),
                                name: if index == 0 { "Synth A" } else { "Synth B" },
                                producer: "Producer",
                                preview: None,
                                selected: false,
                                favorite: false,
                                icon: Icon::Instrument,
                            },
                            width,
                        );
                        positions.push(card.response.rect.center());
                        if card.activated {
                            activated.push(index);
                        }
                    });
                });
            });
            (positions, activated)
        };
        let (positions, _) = frame(0.0, vec![]);
        let click = |target: usize, time: f64| {
            let pos = positions[target];
            frame(
                time,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            frame(
                time + 0.01,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            )
            .1
        };
        assert!(click(1, 0.10).is_empty());
        assert!(click(2, 0.20).is_empty(), "different card never activates");
        assert_eq!(click(2, 0.30), vec![1], "same card double-click activates");
        assert!(click(0, 0.40).is_empty());
        assert!(
            click(2, 0.50).is_empty(),
            "a filter click breaks card sequence"
        );
        assert_eq!(click(2, 0.60), vec![1]);
    }

    fn click_at(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn list_rows_virtualize_keep_text_inside_and_double_click_activates() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let texture = ctx.load_texture(
            "row_fixture",
            egui::ColorImage::filled([160, 90], egui::Color32::RED),
            egui::TextureOptions::LINEAR,
        );
        let name = "Row Synth Ω — a long name that must stay within its row";
        let frame = |time: f64, events, scroll_to| {
            let mut rows = Vec::new();
            let mut grid_rect = Rect::NOTHING;
            let output = ctx.run(input(640.0, time, events), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let grid =
                        device_list(ui, "rows", 5_000, 300.0, scroll_to, |ui, index, width| {
                            let row = device_row(
                                ui,
                                &DeviceCard {
                                    id: Id::new(("row", index)),
                                    name,
                                    producer: "",
                                    preview: (index % 2 == 0).then_some(&texture),
                                    selected: index == 1,
                                    favorite: index == 1,
                                    icon: Icon::Effect,
                                },
                                "Delay · Reverb",
                                width,
                            );
                            (row.response.rect, row.preview_rect, row.activated)
                        });
                    grid_rect = grid.rect;
                    assert_eq!(grid.layout.columns, 1);
                    rows = grid.cards;
                });
            });
            (rows, grid_rect, output)
        };
        let (rows, grid_rect, output) = frame(0.0, vec![], None);
        assert!(rows.len() < 12, "only visible rows are built");
        for (_, (rect, preview, _)) in &rows {
            assert_eq!(rect.height(), DEVICE_ROW_HEIGHT);
            assert!(rect.contains_rect(*preview));
            assert!(rect.right() <= grid_rect.right() - SCROLLBAR_GUTTER + 0.01);
        }
        assert_eq!(
            rows[0].1.1.size(),
            rows[1].1.1.size(),
            "a missing preview keeps the row geometry"
        );
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((text.galley.job.text.clone(), shape.clip_rect)),
                _ => None,
            })
            .collect();
        for expected in [name, "Unknown developer", "Delay · Reverb"] {
            assert!(
                texts.iter().any(|(text, clip)| text == expected
                    && rows.iter().any(|(_, (rect, ..))| rect.contains_rect(*clip))),
                "{expected} is painted within a row"
            );
        }
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Mesh(_))),
            "the favorite badge is a filled star"
        );
        let target = rows[2].1.0.center();
        let mut activated = Vec::new();
        for (time, pressed) in [(0.2, true), (0.21, false), (0.3, true), (0.31, false)] {
            let (rows, ..) = frame(time, click_at(target, pressed), None);
            activated.extend(
                rows.iter()
                    .filter(|(_, (.., activated))| *activated)
                    .map(|(index, _)| *index),
            );
        }
        assert_eq!(activated, vec![2]);
        let (scrolled, ..) = frame(0.4, vec![], Some(4_999));
        assert!(scrolled.iter().any(|(index, _)| *index == 4_999));
    }

    #[test]
    fn favorite_toggle_count_items_and_filter_chips_report_their_actions() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut favorite = false;
        let mut clicked_item = false;
        let mut removed = false;
        let mut frame = |time: f64, events: Vec<egui::Event>| {
            let mut targets = Vec::new();
            let output = ctx.run(input(400.0, time, events), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    targets.push(favorite_toggle(ui, &mut favorite).rect.center());
                    let item = crate::count_item(ui, true, "Favorites", 1_234);
                    clicked_item |= item.clicked();
                    targets.push(item.rect.center());
                    let chip = crate::filter_chip(ui, "Acme Audio");
                    removed |= chip.clicked();
                    targets.push(chip.rect.center());
                });
            });
            (targets, output)
        };
        let (targets, output) = frame(0.0, vec![]);
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                _ => None,
            })
            .collect();
        for expected in ["Favorite", "Favorites", "1234", "Acme Audio"] {
            assert!(
                texts.iter().any(|text| text == expected),
                "{expected}: {texts:?}"
            );
        }
        for (step, target) in targets.into_iter().enumerate() {
            let time = 0.1 * (step + 1) as f64;
            frame(time, click_at(target, true));
            frame(time + 0.01, click_at(target, false));
        }
        assert!(favorite, "the toggle stores the new favorite state");
        assert!(clicked_item && removed);
    }
}

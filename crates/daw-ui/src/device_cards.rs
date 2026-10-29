//! Cached device previews in a responsive, virtualized grid. Device identities,
//! image loading and insertion actions belong to the caller.
use crate::{Icon, text::TextRole, theme, tokens};
use egui::{Id, Rect, Response, Ui};
use std::hash::Hash;

pub const DEVICE_CARD_HEIGHT: f32 = 208.0;
pub const DEVICE_CARD_MIN_WIDTH: f32 = 196.0;
const MIN_CARD_HEIGHT: f32 = 144.0;
const PREVIEW_HEIGHT: f32 = 118.0;
const SCROLLBAR_GUTTER: f32 = 16.0;

fn card_height(available: f32) -> f32 {
    if available.is_finite() {
        available.floor().clamp(MIN_CARD_HEIGHT, DEVICE_CARD_HEIGHT)
    } else {
        DEVICE_CARD_HEIGHT
    }
}

pub struct DeviceCard<'a> {
    /// Stable identity independent of filtering, sorting and grid position.
    pub id: Id,
    pub name: &'a str,
    pub producer: &'a str,
    pub preview: Option<&'a egui::TextureHandle>,
    pub selected: bool,
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

/// One quiet, whole-card input target. Preview arrival never moves its labels,
/// changes the target bounds, or alters the caller's selection. Short parent
/// rows reduce the preview area while keeping both name rows and producer text.
pub fn device_card(ui: &mut Ui, card: &DeviceCard<'_>, width: f32) -> DeviceCardResponse {
    let width = if width.is_finite() {
        width.max(0.0)
    } else {
        0.0
    };
    let height = card_height(ui.available_height());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let response = ui.interact(rect, card.id, egui::Sense::click());
    lock_card_arrows(&response);
    let producer = if card.producer.trim().is_empty() {
        "Unknown developer"
    } else {
        card.producer
    };
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            card.selected,
            format!("{} · {producer}", card.name),
        )
    });
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
        activated = tracker.last.is_some_and(|(id, sequence, time, point)| {
            id == card.id
                && sequence.wrapping_add(1) == tracker.sequence
                && now > time
                && now - time < delay
                && point.distance(position) <= distance
        });
        tracker.last = Some((card.id, tracker.sequence, now, position));
        ui.ctx()
            .data_mut(|data| data.insert_temp(Id::new("device_card_clicks"), tracker));
    }
    let inset = rect.shrink2(egui::vec2(
        tokens::SPACE_MD.min(width * 0.5),
        tokens::SPACE_MD,
    ));
    let preview_height = PREVIEW_HEIGHT - (DEVICE_CARD_HEIGHT - height);
    let preview_rect = Rect::from_min_size(inset.min, egui::vec2(inset.width(), preview_height));
    if ui.is_rect_visible(rect) {
        let colors = theme::palette(ui.ctx());
        let painter = ui.painter_at(rect);
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
        painter.rect_stroke(
            rect,
            tokens::SURFACE_RADIUS,
            egui::Stroke::new(
                if card.selected || response.has_focus() {
                    1.5_f32
                } else {
                    1.0_f32
                },
                if card.selected || response.has_focus() {
                    colors.accent
                } else {
                    colors.line
                },
            ),
            egui::StrokeKind::Inside,
        );
        painter.rect_filled(preview_rect, tokens::RADIUS, colors.bg);
        if let Some(texture) = card.preview {
            let original = texture.size_vec2();
            let image_space = preview_rect.shrink(tokens::SPACE_XS);
            if original.x > 0.0 && original.y > 0.0 && image_space.is_positive() {
                let scale =
                    (image_space.width() / original.x).min(image_space.height() / original.y);
                painter.image(
                    texture.id(),
                    Rect::from_center_size(image_space.center(), original * scale),
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        } else {
            crate::paint_ambient(&painter, preview_rect);
            card.icon.paint(
                &painter,
                Rect::from_center_size(preview_rect.center(), egui::Vec2::splat(34.0)),
                colors.muted,
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

/// Equal-height, responsive card rows with bounded rendering and image-loading
/// work. Short viewports reduce previews so a complete row remains usable.
/// `scroll_to` reveals a keyboard selection even when its row is virtualized.
pub fn device_grid<R>(
    ui: &mut Ui,
    id_salt: impl Hash,
    count: usize,
    height: f32,
    scroll_to: Option<usize>,
    mut render: impl FnMut(&mut Ui, usize, f32) -> R,
) -> DeviceGridResponse<R> {
    ui.push_id(id_salt, |ui| {
        // Keep a stable gutter independently of scrollbar appearance/animation.
        ui.spacing_mut().scroll.floating = true;
        ui.spacing_mut().scroll.floating_allocated_width = 0.0;
        ui.spacing_mut().item_spacing = egui::Vec2::splat(tokens::SPACE_MD);
        let mut layout = device_grid_layout(ui.available_width());
        let height = if height.is_finite() {
            height.max(0.0).min(ui.available_height().max(0.0))
        } else {
            0.0
        };
        let card_height = card_height(height);
        layout.row_height = card_height + tokens::SPACE_MD;
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
                    - (height - card_height).max(0.0) * 0.5)
                    .max(0.0),
            );
        }
        let result = scroll.show_rows(ui, card_height, rows, |ui, visible| {
            let mut cards = Vec::with_capacity(visible.len() * layout.columns);
            for row in visible {
                ui.horizontal(|ui| {
                    ui.set_max_height(card_height);
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
        for height in [144.0, 183.5, 260.0, 400.0] {
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
}

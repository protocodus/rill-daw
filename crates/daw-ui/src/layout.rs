//! Quiet, composable workspace structure. Interaction remains native egui;
//! project state, navigation and history remain with the caller.
use crate::{theme, tokens};
use egui::{InnerResponse, Response, Ui, WidgetText};
use std::hash::Hash;

/// A compact action row which wraps as complete controls when space is limited.
pub fn toolbar<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    toolbar_with_row_height(ui, tokens::TOOLBAR_CONTROL_HEIGHT, contents)
}

/// Apply the same compact, centered command-bar spacing at every window width.
/// Shared toolbars already apply this; callers may use it in custom command rows.
pub fn compact_toolbar_spacing(ui: &mut Ui) {
    ui.spacing_mut().button_padding = egui::vec2(
        tokens::TOOLBAR_BUTTON_PADDING_X,
        tokens::TOOLBAR_BUTTON_PADDING_Y,
    );
    ui.spacing_mut().item_spacing = egui::Vec2::splat(tokens::TOOLBAR_ITEM_SPACING);
    ui.spacing_mut().icon_spacing = tokens::ICON_LABEL_GAP;
    ui.spacing_mut().icon_width = tokens::TOOLBAR_ICON_SIZE;
    ui.spacing_mut().icon_width_inner = tokens::SPACE_SM;
    // Native ComboBox starts a nested horizontal row at interact_size. Reserve
    // the measured label height there too, before it paints any child controls.
    ui.spacing_mut().interact_size.y = tokens::TOOLBAR_CONTROL_HEIGHT.max(
        (ui.text_style_height(&egui::TextStyle::Button) + 2.0 * tokens::TOOLBAR_BUTTON_PADDING_Y)
            .ceil(),
    );
}

pub(crate) fn compact_toolbar(ui: &Ui) -> bool {
    !egui::containers::menu::is_in_menu(ui)
        && ui.spacing().button_padding.y == tokens::TOOLBAR_BUTTON_PADDING_Y
        && ui.spacing().icon_width == tokens::TOOLBAR_ICON_SIZE
}

pub(crate) fn control_height(ui: &Ui) -> f32 {
    ui.spacing()
        .interact_size
        .y
        .max(ui.text_style_height(&egui::TextStyle::Button) + 2.0 * ui.spacing().button_padding.y)
}

pub(crate) fn button_icon_size(ui: &Ui) -> f32 {
    if compact_toolbar(ui) {
        tokens::TOOLBAR_ICON_SIZE
    } else {
        tokens::ICON_SIZE
    }
}

/// A toolbar containing calibrated sliders. Reserve the whole row before any
/// controls are painted so preceding labels, choices and buttons share its axis.
pub fn parameter_toolbar<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    toolbar_with_row_height(ui, tokens::PARAMETER_ROW_HEIGHT, contents)
}

/// A centered parameter row. Scale captions belong below a slider's
/// rail; they must not pull the rail above adjacent fields, labels or actions.
pub fn parameter_row<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    row_with_height(ui, tokens::PARAMETER_ROW_HEIGHT, false, contents)
}

/// A compact, vertically centered row for actions, labels and plain fields.
/// Captioned sliders use `parameter_row` to reserve room for their scales.
pub fn control_row<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    row_with_height(ui, tokens::CONTROL_HEIGHT, false, contents)
}

/// Compact control rows that wrap complete controls when the workspace narrows.
pub fn control_row_wrapped<R>(
    ui: &mut Ui,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    row_with_height(ui, tokens::CONTROL_HEIGHT, true, contents)
}

/// A responsive parameter row that wraps complete controls onto centered rows.
pub fn parameter_row_wrapped<R>(
    ui: &mut Ui,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    row_with_height(ui, tokens::PARAMETER_ROW_HEIGHT, true, contents)
}

fn row_with_height<R>(
    ui: &mut Ui,
    height: f32,
    wrap: bool,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let layout = if ui.layout().prefer_right_to_left() {
        egui::Layout::right_to_left(egui::Align::Center)
    } else {
        egui::Layout::left_to_right(egui::Align::Center)
    };
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_size_before_wrap().x, height),
        layout.with_main_wrap(wrap),
        |ui| {
            ui.set_min_height(height);
            contents(ui)
        },
    )
}

fn toolbar_with_row_height<R>(
    ui: &mut Ui,
    row_height: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let colors = theme::palette(ui.ctx());
    let row = egui::Frame::new()
        .fill(colors.panel)
        .inner_margin(egui::Margin::symmetric(
            tokens::SPACE_SM as i8,
            tokens::SPACE_XS as i8,
        ))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            compact_toolbar_spacing(ui);
            row_with_height(ui, row_height.max(control_height(ui)), true, contents).inner
        });
    ui.painter().hline(
        row.response.rect.x_range(),
        row.response.rect.bottom(),
        egui::Stroke::new(1.0_f32, colors.line),
    );
    row
}

/// One row of a panel grid: each cell is a card `spans[index]` columns wide
/// of as many columns as the spans add up to, and every card takes the
/// height of the tallest. Rows whose spans add up to the same number share
/// the grid, so their cards' edges line up like the modules of a hardware
/// front panel. `cell` draws the contents of cell `index`.
pub fn grid_row<R>(ui: &mut Ui, spans: &[u8], cell: impl FnMut(&mut Ui, usize) -> R) -> Vec<R> {
    let cells: Vec<GridCell> = spans
        .iter()
        .map(|span| GridCell {
            span: *span,
            framed: true,
        })
        .collect();
    grid_row_cells(ui, &cells, cell)
}

/// One cell of a [`grid_row_cells`] row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridCell {
    /// Width in grid columns.
    pub span: u8,
    /// A card. A bare cell has no frame and no margin: its contents take
    /// the card's whole outer rectangle, as a picture beside cards does.
    pub framed: bool,
}

/// A [`grid_row`] whose cells may be bare. Bare cells are laid out after
/// the cards, so `ui.available_height()` in them is the height of the row;
/// `cell` is still called once per index and the results come in index order.
pub fn grid_row_cells<R>(
    ui: &mut Ui,
    cells: &[GridCell],
    mut cell: impl FnMut(&mut Ui, usize) -> R,
) -> Vec<R> {
    let frame = crate::card(ui.ctx());
    let margin = frame.total_margin();
    let gap = tokens::SPACE_SM;
    let origin = ui.cursor().min;
    let width = ui.available_width();
    let columns = cells
        .iter()
        .map(|cell| f32::from(cell.span))
        .sum::<f32>()
        .max(1.0);
    let column = (width - gap * (columns - 1.0)).max(0.0) / columns;
    // Left edge and outer width of every cell.
    let mut x = origin.x;
    let places: Vec<(f32, f32)> = cells
        .iter()
        .map(|cell| {
            let span = f32::from(cell.span);
            let outer = span * column + (span - 1.0).max(0.0) * gap;
            let place = (x, outer);
            x += outer + gap;
            place
        })
        .collect();
    let mut results: Vec<Option<R>> = cells.iter().map(|_| None).collect();
    let mut cards = Vec::with_capacity(cells.len());
    let mut height: f32 = 0.0;
    for (index, (left, outer)) in places.iter().enumerate() {
        if !cells[index].framed {
            continue;
        }
        let content = egui::Rect::from_min_size(
            egui::pos2(left + margin.left, origin.y + margin.top),
            egui::vec2(
                (outer - margin.sum().x).max(0.0),
                ui.available_height().max(0.0),
            ),
        );
        // Painted once the row's height is known, beneath the contents.
        let background = ui.painter().add(egui::Shape::Noop);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("grid_cell", index))
                .max_rect(content)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        results[index] = Some(cell(&mut child, index));
        height = height.max(child.min_rect().height());
        cards.push((background, content.min, content.width()));
    }
    let framed = !cards.is_empty();
    let mut outer_height = if framed {
        height + margin.sum().y
    } else {
        ui.available_height().max(0.0)
    };
    let mut bare_height: f32 = 0.0;
    for (index, (left, outer)) in places.iter().enumerate() {
        if cells[index].framed {
            continue;
        }
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("grid_cell", index))
                .max_rect(egui::Rect::from_min_size(
                    egui::pos2(*left, origin.y),
                    egui::vec2(*outer, outer_height),
                ))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        results[index] = Some(cell(&mut child, index));
        bare_height = bare_height.max(child.min_rect().height());
    }
    if !framed {
        outer_height = bare_height;
    }
    for (background, min, width) in cards {
        let content = egui::Rect::from_min_size(min, egui::vec2(width, height));
        ui.painter().set(background, frame.paint(content));
    }
    ui.advance_cursor_after_rect(egui::Rect::from_min_size(
        origin,
        egui::vec2(width, outer_height),
    ));
    results.into_iter().flatten().collect()
}

/// A compact visual family of tools. The measured width lets a wrapping toolbar
/// move the entire family together; the background never owns input or focus.
pub fn tool_group<R>(
    ui: &mut Ui,
    width: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let colors = theme::palette(ui.ctx());
    let background = ui.painter().add(egui::Shape::Noop);
    let group = toolbar_group_sized_with_spacing(ui, width, tokens::SPACE_XS, contents);
    ui.painter().set(
        background,
        crate::surface::component_shape(
            ui.painter(),
            group.response.rect,
            tokens::SURFACE_RADIUS,
            colors.bg,
            egui::Stroke::new(
                tokens::BORDER_WIDTH,
                colors.line.gamma_multiply(tokens::GOLDEN_RATIO_INVERSE),
            ),
            crate::surface::ComponentMaterial::default(),
        ),
    );
    group
}

/// Keep related controls together inside a wrapping toolbar.
pub fn toolbar_group<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_SM;
        ui.horizontal(contents).inner
    })
}

/// Reserve a known-width group before drawing its left-to-right contents. The
/// wrapping parent can move the complete group to the next row without adding
/// unused trailing space or changing the behavior of right-aligned controls.
pub fn toolbar_group_sized<R>(
    ui: &mut Ui,
    minimum_width: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    toolbar_group_sized_with_spacing(ui, minimum_width, tokens::SPACE_SM, contents)
}

/// Reserve a complete group before painting, with compact parent spacing.
pub fn toolbar_group_sized_with_spacing<R>(
    ui: &mut Ui,
    minimum_width: f32,
    spacing: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let width = minimum_width.max(0.0).min(ui.max_rect().width().max(0.0));
    ui.allocate_ui_with_layout(
        egui::vec2(width, control_height(ui)),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = spacing.max(0.0);
            contents(ui)
        },
    )
}

/// A single shared surface for a family of commands or a musical readout.
/// Reserve the complete group before wrapping; its background never takes focus.
pub fn command_group<R>(
    ui: &mut Ui,
    minimum_width: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let colors = theme::palette(ui.ctx());
    let width = minimum_width.max(0.0).min(ui.max_rect().width().max(0.0));
    ui.allocate_ui_with_layout(
        egui::vec2(width, control_height(ui) + tokens::SPACE_XS * 2.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let background = ui.painter().add(egui::Shape::Noop);
            let frame = egui::Frame::new()
                .inner_margin(tokens::SPACE_XS as i8)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = tokens::SPACE_XS;
                    ui.horizontal(contents).inner
                });
            ui.painter().set(
                background,
                crate::surface::component_shape(
                    ui.painter(),
                    frame.response.rect,
                    tokens::SURFACE_RADIUS,
                    colors.bg,
                    egui::Stroke::new(
                        tokens::BORDER_WIDTH,
                        colors.line.gamma_multiply(tokens::GOLDEN_RATIO_INVERSE),
                    ),
                    crate::surface::ComponentMaterial::default(),
                ),
            );
            frame.inner
        },
    )
}

/// Reserve trailing controls before right alignment, wrapping the whole group
/// when the remaining row cannot fit it. This keeps its hit area off prior tools.
pub fn toolbar_trailing<R>(
    ui: &mut Ui,
    minimum_width: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    // In a wrapping layout available_width includes subsequent rows. Asking
    // for that width always pushes trailing controls onto a fresh row, even
    // when there is enough room beside the leading tools.
    // Earlier unbounded measuring groups can enlarge max_rect beyond the
    // window. Keep the trailing anchor inside the actual visible toolbar.
    let visible = ui
        .max_rect()
        .intersect(ui.clip_rect())
        .intersect(ui.ctx().content_rect());
    let remaining = ui
        .available_rect_before_wrap()
        .intersect(visible)
        .width()
        .max(0.0);
    if remaining < minimum_width.min(visible.width()).max(0.0) {
        ui.end_row();
    }
    let width = ui
        .available_rect_before_wrap()
        .intersect(visible)
        .width()
        .max(0.0);
    ui.allocate_ui_with_layout(
        egui::vec2(width, control_height(ui)),
        egui::Layout::right_to_left(egui::Align::Center),
        contents,
    )
}

/// Secondary actions appear on demand, using native menu focus and dismissal.
pub fn action_menu<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    crate::widgets::semantic_menu(ui, label, crate::widgets::MenuRole::Action, contents)
}

/// A text-first action menu for narrow columns. The opener retains its full
/// control height and native menu behavior, with compact horizontal padding.
pub fn compact_action_menu<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    crate::widgets::semantic_compact_action_menu(ui, label, contents)
}

/// Presentation settings use a quiet outlined eye button. Unlike action menus,
/// toggling a setting keeps the menu open; Escape or an outside click dismisses it.
/// View changes remain the caller's responsibility and must not edit project data.
pub fn view_menu<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    crate::widgets::semantic_menu(ui, label, crate::widgets::MenuRole::View, contents)
}

/// A field-like dropdown for related choices, labelled with the current value.
/// Use `ui.close()` to accept a choice, or leave it open for several settings.
/// Nested choice menus use native submenus rather than competing popup state.
pub fn choice_menu<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<Option<R>> {
    crate::widgets::semantic_menu(ui, label, crate::widgets::MenuRole::Choice, contents)
}

/// A small section heading and its controls, grouped by spacing rather than a box.
pub fn context_section<R>(
    ui: &mut Ui,
    title: impl Into<String>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    ui.vertical(|ui| {
        crate::text::label(ui, title.into());
        contents(ui)
    })
}

/// A utility body that hugs short content and scrolls when it reaches the
/// window's available height. Use inside a window with its own scrolling off.
pub fn scroll_body<R>(
    ui: &mut Ui,
    id: impl Hash,
    contents: impl FnOnce(&mut Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .min_scrolled_height(0.0)
        .max_height(ui.available_height().max(0.0))
        .auto_shrink([false, true])
        .show(ui, contents)
}

/// Two equal, bounded content panes. Each pane owns its scrolling; headers can
/// stay fixed while only the long list or preview beneath them scrolls.
pub fn bounded_columns<R>(
    ui: &mut Ui,
    id: impl Hash,
    height: f32,
    mut contents: impl FnMut(usize, &mut Ui) -> R,
) -> InnerResponse<[R; 2]> {
    ui.push_id(id, |ui| {
        let height = if height.is_finite() {
            height.max(0.0).min(ui.available_height().max(0.0))
        } else {
            0.0
        };
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(0.0), height),
            egui::Sense::hover(),
        );
        let gap = tokens::SPACE_LG.min(rect.width());
        let width = (rect.width() - gap) * 0.5;
        [0, 1].map(|index| {
            let pane = egui::Rect::from_min_size(
                rect.min + egui::vec2(index as f32 * (width + gap), 0.0),
                egui::vec2(width, height),
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(index)
                    .max_rect(pane)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            child.set_clip_rect(ui.clip_rect().intersect(pane));
            child.set_width(width);
            contents(index, &mut child)
        })
    })
}

/// Sidebars keep their chosen widths across browser visits. When the window
/// narrows, fit both sides around the catalog without losing those preferences.
pub struct BrowserPanes {
    id: egui::Id,
    sides: [f32; 2],
    minimums: [f32; 2],
    budget: f32,
    pub center_width: f32,
}

impl BrowserPanes {
    pub fn new(ui: &Ui, id: egui::Id, defaults: [f32; 2], center_min: f32) -> Self {
        let width = ui.available_width().max(0.0);
        let visible = defaults.map(|w| w > 0.0);
        let gaps = visible.iter().filter(|side| **side).count() as f32 * tokens::SPACE_MD;
        let usable = (width - gaps).max(0.0);
        let mut minimums = [
            tokens::SELECT_WIDTH,
            tokens::SELECT_WIDTH + tokens::SPACE_XL,
        ];
        for i in 0..2 {
            if !visible[i] {
                minimums[i] = 0.0;
            }
        }
        let min_sum = minimums.iter().sum::<f32>();
        let budget = (usable - center_min).max(0.0);
        let scale = (budget / min_sum.max(1.0)).min(1.0);
        minimums = minimums.map(|v| v * scale);
        let mut extra = [0.0; 2];
        for i in 0..2 {
            if visible[i] {
                let preferred = ui
                    .data(|d| d.get_temp::<f32>(id.with(i)))
                    .unwrap_or(defaults[i]);
                extra[i] = (preferred - minimums[i]).max(0.0);
            }
        }
        let extra_scale = ((budget - minimums.iter().sum::<f32>()).max(0.0)
            / extra.iter().sum::<f32>().max(1.0))
        .min(1.0);
        let sides = std::array::from_fn(|i| minimums[i] + extra[i] * extra_scale);
        Self {
            id,
            sides,
            minimums,
            budget,
            center_width: usable - sides.iter().sum::<f32>(),
        }
    }

    pub fn show(
        self,
        ui: &mut Ui,
        height: f32,
        mut contents: impl FnMut(usize, &mut Ui),
    ) -> [egui::Rect; 3] {
        let height = height.max(0.0).min(ui.available_height().max(0.0));
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::hover(),
        );
        let mut edges = [rect.left(), rect.right()];
        for (i, center_edge) in edges.iter_mut().enumerate() {
            let width = self.sides[i];
            if width <= 0.0 {
                continue;
            }
            let sign = if i == 0 { 1.0 } else { -1.0 };
            let edge = *center_edge + sign * width;
            let gap = tokens::SPACE_MD;
            *center_edge = edge + sign * gap;
            let handle = egui::Rect::from_center_size(
                egui::pos2(edge + sign * gap * 0.5, rect.center().y),
                egui::vec2(gap, height),
            );
            // Express both handles as a left-to-right coordinate for the shared drag control.
            let mut at = sign * width;
            let min = sign * self.minimums[i];
            let max = sign * (self.budget - self.sides[1 - i]).max(self.minimums[i]);
            let response = crate::resize_value_at(
                ui,
                self.id.with(("divider", i)),
                handle,
                &mut at,
                min.min(max)..=min.max(max),
                crate::ResizeAxis::Horizontal,
                "Drag to resize sidebar",
            );
            let color = if response.hovered() || response.dragged() {
                theme::palette(ui.ctx()).accent
            } else {
                theme::palette(ui.ctx()).line
            };
            ui.painter().vline(
                handle.center().x,
                handle.y_range(),
                egui::Stroke::new(tokens::SPACE_XS, color),
            );
            if response.changed() {
                ui.data_mut(|d| {
                    d.insert_temp(self.id.with(i), sign * at);
                    if self.sides[1 - i] > 0.0 {
                        d.insert_temp(self.id.with(1 - i), self.sides[1 - i]);
                    }
                });
            }
        }
        let spans = [
            rect.left()..=rect.left() + self.sides[0],
            edges[0]..=edges[1].max(edges[0]),
            rect.right() - self.sides[1]..=rect.right(),
        ];
        let panes = spans.map(|x| egui::Rect::from_x_y_ranges(x, rect.y_range()));
        for (index, pane) in panes.into_iter().enumerate() {
            if !pane.is_positive() {
                continue;
            }
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(self.id.with(index))
                    .max_rect(pane)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            child.set_clip_rect(ui.clip_rect().intersect(pane));
            child.set_width(pane.width());
            contents(index, &mut child);
        }
        panes
    }
}

pub struct WorkspaceHeaderResponse<R> {
    pub back: Response,
    pub title: Response,
    pub context: Option<Response>,
    pub inner: R,
    pub response: Response,
}

/// One workspace title, Back, and trailing actions. Actions are laid out from
/// right to left. The caller decides what Back does; this never owns navigation.
pub fn workspace_header<R>(
    ui: &mut Ui,
    title: impl Into<WidgetText>,
    actions: impl FnOnce(&mut Ui) -> R,
) -> WorkspaceHeaderResponse<R> {
    workspace_header_with_context(ui, title, None, actions)
}

/// A workspace identity with an optional reserved context line. Measure both
/// lines and their breathing room before laying out the centered actions.
/// The first trailing action stays anchored to the right edge,
/// independent of the identity length or the remaining actions.
/// The caller can attach one complete explanation to the returned context
/// response; no automatic elision tooltip competes with that explanation.
pub fn workspace_header_with_context<R>(
    ui: &mut Ui,
    title: impl Into<WidgetText>,
    context: Option<&str>,
    actions: impl FnOnce(&mut Ui) -> R,
) -> WorkspaceHeaderResponse<R> {
    let title = title.into();
    let full_title = title.text().to_owned();
    let row = crate::soft_surface(
        ui,
        egui::Frame::new().inner_margin(egui::Margin::symmetric(
            tokens::SPACE_MD as i8,
            tokens::SPACE_SM as i8,
        )),
        |ui| {
            let identity_height = if context.is_some() {
                ui.fonts_mut(|fonts| {
                    fonts.row_height(&egui::TextStyle::Body.resolve(ui.style()))
                        + tokens::SPACE_XS
                        + fonts.row_height(&crate::text::TextRole::Note.font())
                })
            } else {
                tokens::CONTROL_HEIGHT
            }
            .max(tokens::CONTROL_HEIGHT);
            ui.horizontal(|ui| {
                ui.set_min_height(identity_height);
                ui.set_row_height(identity_height);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.set_min_height(identity_height);
                    ui.set_row_height(identity_height);
                    let inner = actions(ui);
                    ui.add_space(tokens::SPACE_MD);
                    let back = ui
                        .with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.set_min_height(identity_height);
                            ui.set_row_height(identity_height);
                            let back = crate::quiet_button(ui, "Back")
                                .on_hover_text("Return to the previous workspace");
                            ui.add_space(tokens::SPACE_XS);
                            let identity = if let Some(context) = context {
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = tokens::SPACE_XS;
                                    ui.set_min_height(identity_height);
                                    let title = ui
                                        .add(
                                            egui::Label::new(
                                                title.text_style(egui::TextStyle::Body),
                                            )
                                            .truncate()
                                            .show_tooltip_when_elided(false),
                                        )
                                        .on_hover_text(&full_title);
                                    let context = ui.add(
                                        egui::Label::new(
                                            crate::text::TextRole::Note.rich(ui.ctx(), context),
                                        )
                                        .truncate()
                                        .show_tooltip_when_elided(false),
                                    );
                                    (title, Some(context))
                                })
                                .inner
                            } else {
                                (
                                    ui.add(
                                        egui::Label::new(
                                            title.text_style(egui::TextStyle::Heading),
                                        )
                                        .truncate()
                                        .show_tooltip_when_elided(false),
                                    )
                                    .on_hover_text(&full_title),
                                    None,
                                )
                            };
                            (back, identity.0, identity.1)
                        })
                        .inner;
                    (back.0, back.1, back.2, inner)
                })
                .inner
            })
            .inner
        },
    );
    WorkspaceHeaderResponse {
        back: row.inner.0,
        title: row.inner.1,
        context: row.inner.2,
        inner: row.inner.3,
        response: row.response,
    }
}

/// A uniform mixer channel with a quiet wash of its arranger track color.
/// Size includes the internal margins; all channels can share the viewport
/// height while their controls remain stacked. Uncolored channels stay neutral.
pub fn mixer_strip<R>(
    ui: &mut Ui,
    id: impl Hash,
    size: egui::Vec2,
    color: Option<egui::Color32>,
    selected: bool,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    ui.push_id(id, |ui| {
        let colors = theme::palette(ui.ctx());
        let base = if selected {
            colors.track_selected
        } else {
            colors.panel
        };
        let fill = color.map_or(base, |color| {
            let amount = match (ui.ctx().theme(), selected) {
                (egui::Theme::Dark, false) => 0.08,
                (egui::Theme::Dark, true) => 0.14,
                (egui::Theme::Light, false) => 0.06,
                (egui::Theme::Light, true) => 0.12,
            };
            base.lerp_to_gamma(color, amount)
        });
        // The parent owns the gutter between cards. Keep a full, consistent
        // inset here so controls never borrow space from the adjacent channel.
        let frame = egui::Frame::new()
            .fill(fill)
            .corner_radius(tokens::RADIUS)
            .inner_margin(egui::Margin::same(crate::MIXER_CHANNEL_MARGIN as i8));
        let content = |ui: &mut Ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width((size.x - 2.0 * crate::MIXER_CHANNEL_MARGIN).max(0.0));
                ui.set_height((size.y - 2.0 * crate::MIXER_CHANNEL_MARGIN).max(0.0));
                contents(ui)
            })
            .inner
        };
        let result = if selected {
            crate::soft_surface(ui, frame, content)
        } else {
            frame.show(ui, content)
        };
        let rect = result.response.rect;
        // Fill-only card; only the selected strip gains an outline.
        if selected {
            crate::paint_selection_glow(
                ui.painter(),
                rect,
                tokens::RADIUS,
                color.unwrap_or(colors.accent),
            );
            ui.painter().rect_stroke(
                egui::emath::GuiRounding::round_to_pixels(rect, ui.painter().pixels_per_point()),
                tokens::RADIUS,
                egui::Stroke::new(1.0 / ui.ctx().pixels_per_point(), colors.accent),
                egui::StrokeKind::Inside,
            );
        }
        result.inner
    })
}

/// Stereo peak display, −60 dBFS to 0 dBFS. Non-finite readings show silence;
/// normalized peaks at/above unity use the warning color. No smoothing state.
pub fn stereo_meter(ui: &mut Ui, peaks: [f32; 2], size: egui::Vec2) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(size.max(egui::Vec2::splat(1.0)), egui::Sense::hover());
    let colors = theme::palette(ui.ctx());
    let gap = (rect.width() * 0.15).min(tokens::SPACE_SM);
    let width = (rect.width() - gap) * 0.5;
    let mut db = [f32::NEG_INFINITY; 2];
    for (index, peak) in peaks.into_iter().enumerate() {
        let peak = if peak.is_finite() { peak.max(0.0) } else { 0.0 };
        db[index] = 20.0 * peak.log10();
        let level = ((db[index] + 60.0) / 60.0).clamp(0.0, 1.0);
        let bar = egui::Rect::from_min_size(
            rect.min + egui::vec2(index as f32 * (width + gap), 0.0),
            egui::vec2(width, rect.height()),
        );
        ui.painter().rect_filled(bar, 1.0, colors.bg);
        if level > 0.0 {
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(bar.left(), bar.bottom() - bar.height() * level),
                    bar.max,
                ),
                1.0,
                if peak >= 1.0 {
                    colors.warning
                } else {
                    colors.guide
                },
            );
        }
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Other,
            ui.is_enabled(),
            format!("Stereo peak: L {:.1} dBFS, R {:.1} dBFS", db[0], db[1]),
        )
    });
    response
}

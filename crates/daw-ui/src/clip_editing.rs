//! Precise clip fields and a small name editor shared by timeline views.
use egui::{Id, Response, Ui};

/// Edge resizing must respond to small corrections without the click/drag
/// threshold used by a clip body. Register these after the body and before any
/// higher-priority source handles (such as audio fades). The caller owns the
/// gesture and must ignore zero-motion presses and consume clicks after motion.
pub fn clip_edge_handle(
    ui: &mut Ui,
    id: Id,
    clip: egui::Rect,
    visible: egui::Rect,
    start: bool,
) -> Option<Response> {
    let edge = if start { clip.left() } else { clip.right() };
    // A viewport boundary through the middle of a clip is not its trim edge.
    if edge < visible.left() || edge > visible.right() {
        return None;
    }
    let width = 10.0_f32.min(clip.width() / 3.0);
    let target = egui::Rect::from_min_max(
        egui::pos2(if start { edge } else { edge - width }, clip.top()),
        egui::pos2(if start { edge + width } else { edge }, clip.bottom()),
    )
    .intersect(visible);
    target.is_positive().then(|| {
        ui.interact(target, id, egui::Sense::drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DurationEditMode {
    #[default]
    Trim,
    Stretch,
}

#[derive(Clone, Copy, Debug)]
pub struct ClipTimingValues {
    pub start: f64,
    pub duration: f64,
    pub source_offset: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ClipTimingEdit {
    pub start: Option<f64>,
    pub duration: Option<(f64, DurationEditMode)>,
    pub source_offset: Option<f64>,
    /// Opening a numeric entry or changing its interpretation ends the prior
    /// typing burst, even when the model value itself has not changed yet.
    pub new_gesture: bool,
}

impl ClipTimingEdit {
    pub fn changed(self) -> bool {
        self.start.is_some() || self.duration.is_some() || self.source_offset.is_some()
    }
}

/// Append timing rows to a shared form grid. Values are staged; owners validate
/// and commit them after releasing model borrows. Merely inspecting a value
/// outside a current editing range never clamps or changes it.
/// `stretch_duration_range` comes from the occurrence's current source window
/// and the owner's supported playback rates, not the full source asset.
pub fn clip_timing_fields(
    ui: &mut Ui,
    mut values: ClipTimingValues,
    mode: &mut DurationEditMode,
    maximum_beat: f64,
    maximum_offset: f64,
    stretch_duration_range: std::ops::RangeInclusive<f64>,
) -> ClipTimingEdit {
    let mut edit = ClipTimingEdit::default();
    let label = crate::text::label(ui, "Start");
    let mut start = values.start + 1.0;
    let start_range = 1.0..=(maximum_beat - values.duration + 1.0).max(1.0);
    let stepper = crate::number_stepper(
        ui,
        &mut start,
        1.0,
        start_range.clone(),
        "start beat",
        |ui, value| {
            ui.add(
                crate::number(value)
                    .range(start_range)
                    .clamp_existing_to_range(false)
                    .speed(0.01)
                    .min_decimals(2)
                    .max_decimals(9)
                    .prefix("Beat "),
            )
        },
    );
    let response = stepper
        .response
        .labelled_by(label.id)
        .on_hover_text("Move the clip to an exact beat without changing its source window");
    edit.new_gesture |= stepper.stepped
        || response.gained_focus()
        || response.double_clicked()
        || response.drag_started();
    if response.changed() {
        edit.start = Some(start - 1.0);
    }
    ui.end_row();
    let label = crate::text::label(ui, "Duration");
    ui.horizontal(|ui| {
        let timeline_duration = maximum_beat - values.start;
        let duration_range = match *mode {
            DurationEditMode::Trim => 0.000_000_001..=timeline_duration.max(0.000_000_001),
            DurationEditMode::Stretch => {
                *stretch_duration_range.start()
                    ..=stretch_duration_range.end().min(timeline_duration)
            }
        };
        let stepper = crate::number_stepper(
            ui,
            &mut values.duration,
            1.0,
            duration_range.clone(),
            if *mode == DurationEditMode::Stretch {
                "Stretch duration (beats)"
            } else {
                "duration"
            },
            |ui, value| {
                ui.add(
                    crate::number(value)
                        .range(duration_range)
                        .clamp_existing_to_range(false)
                        .speed(0.01)
                        .min_decimals(2)
                        .max_decimals(9)
                        .suffix(" beats"),
                )
            },
        );
        let response = stepper
            .response
            .labelled_by(label.id)
            .on_hover_text("Trim changes the clip boundary; Stretch changes playback timing");
        edit.new_gesture |= stepper.stepped
            || response.gained_focus()
            || response.double_clicked()
            || response.drag_started();
        if response.changed() {
            edit.duration = Some((values.duration, *mode));
        }
        let previous_mode = *mode;
        crate::select(
            "clip_duration_mode",
            if *mode == DurationEditMode::Trim {
                "Trim"
            } else {
                "Stretch"
            },
        )
        .width(96.0)
        .show_ui(ui, |ui| {
            crate::selectable_value(ui, mode, DurationEditMode::Trim, "Trim");
            crate::selectable_value(ui, mode, DurationEditMode::Stretch, "Stretch");
        })
        .response
        .on_hover_text("Choose how the next duration edit affects this clip");
        edit.new_gesture |= *mode != previous_mode;
    });
    ui.end_row();
    let label = crate::text::label(ui, "Source offset");
    let offset_range = 0.0..=maximum_offset.max(0.0);
    let stepper = crate::number_stepper(
        ui,
        &mut values.source_offset,
        1.0,
        offset_range.clone(),
        "source offset",
        |ui, value| {
            ui.add(
                crate::number(value)
                    .range(offset_range)
                    .clamp_existing_to_range(false)
                    .speed(0.01)
                    .min_decimals(2)
                    .max_decimals(9)
                    .suffix(" beats"),
            )
        },
    );
    let response = stepper
        .response
        .labelled_by(label.id)
        .on_hover_text("Slip the source window while keeping the clip's position and duration");
    edit.new_gesture |= stepper.stepped
        || response.gained_focus()
        || response.double_clicked()
        || response.drag_started();
    if response.changed() {
        edit.source_offset = Some(values.source_offset);
    }
    ui.end_row();
    edit
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineNameAction {
    Editing,
    Commit,
    Cancel,
}

pub struct InlineNameResponse {
    pub response: Response,
    pub action: InlineNameAction,
}

/// Full surface width for a name field's requested text width and its three actions.
/// Native TextEdit adds its margins outside `desired_width`; measure the action
/// labels and icons before anchoring the row near a viewport edge.
pub fn inline_name_editor_width(ctx: &egui::Context, text_width: f32) -> f32 {
    let style = ctx.style();
    let font = egui::TextStyle::Button.resolve(&style);
    let color = style.visuals.text_color();
    let density = ctx.pixels_per_point();
    let actions = ctx.fonts_mut(|fonts| {
        ["Keyboard", "Apply name", "Cancel"]
            .into_iter()
            .map(|label| {
                let text = fonts
                    .layout_no_wrap(label.to_owned(), font.clone(), color)
                    .size()
                    .x;
                let icon = if crate::Icon::for_label(label).is_some() {
                    18.0 + style.spacing.icon_spacing
                } else {
                    0.0
                };
                ((text + icon + 2.0 * crate::tokens::SPACE_SM) * density).ceil() / density
            })
            .sum::<f32>()
    });
    text_width
        + 2.0 * crate::tokens::BUTTON_PADDING_X
        + actions
        + 3.0 * style.spacing.item_spacing.x
        + crate::card(ctx).total_margin().sum().x
}

/// Measured opaque editor bounds, including the optional pointer keyboard.
/// Anchored hosts reserve this complete surface before placing it at an edge.
pub fn inline_name_editor_size(ctx: &egui::Context, text_width: f32, keyboard: bool) -> egui::Vec2 {
    let margin = crate::card(ctx).total_margin().sum();
    let style = ctx.style();
    let font = egui::TextStyle::Button.resolve(&style);
    let row =
        ctx.fonts_mut(|fonts| fonts.row_height(&font)) + 2.0 * crate::tokens::BUTTON_PADDING_Y;
    let row = (row.max(crate::tokens::CONTROL_HEIGHT) * ctx.pixels_per_point()).ceil()
        / ctx.pixels_per_point();
    egui::vec2(
        inline_name_editor_width(ctx, text_width).max(if keyboard {
            crate::name_keyboard_width(ctx) + margin.x
        } else {
            0.0
        }),
        row + margin.y
            + if keyboard {
                crate::name_keyboard_height(ctx)
            } else {
                0.0
            },
    )
}

/// Single-line naming in the existing item context. The caller owns its draft,
/// anchor and transaction; Enter/focus loss commits and Escape discards it.
pub fn inline_name_editor(
    ui: &mut Ui,
    id: Id,
    value: &mut String,
    hint: &str,
    width: f32,
    initial_focus: bool,
) -> InlineNameResponse {
    let size = inline_name_editor_size(ui.ctx(), width, crate::name_keyboard_open(ui.ctx(), id));
    // Register the complete surface before its children: gaps consume pointer
    // input without stealing the field, action buttons, or keyboard targets.
    ui.interact(
        egui::Rect::from_min_size(ui.next_widget_position(), size),
        id.with("surface"),
        egui::Sense::CLICK,
    );
    crate::card(ui.ctx())
        .show(ui, |ui| {
            inline_name_editor_contents(ui, id, value, hint, width, initial_focus)
        })
        .inner
}

fn inline_name_editor_contents(
    ui: &mut Ui,
    id: Id,
    value: &mut String,
    hint: &str,
    width: f32,
    initial_focus: bool,
) -> InlineNameResponse {
    let cancel = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let row = ui
        .horizontal(|ui| {
            let output = egui::TextEdit::singleline(value)
                .id(id)
                .hint_text(hint)
                .char_limit(crate::NAME_MAX_CHARS)
                .desired_width(width.max(32.0))
                .margin(egui::vec2(
                    crate::tokens::BUTTON_PADDING_X,
                    crate::tokens::BUTTON_PADDING_Y,
                ))
                .min_size(egui::vec2(0.0, crate::tokens::CONTROL_HEIGHT))
                .show(ui);
            let keyboard =
                crate::small_button(ui, "Keyboard").on_hover_text("Enter this name with the mouse");
            if keyboard.clicked() {
                let open = crate::name_keyboard_open(ui.ctx(), id);
                crate::set_name_keyboard_open(ui.ctx(), id, !open);
            }
            let keyboard_pressed = keyboard.clicked() || keyboard.is_pointer_button_down_on();
            let apply = crate::small_button(ui, "Apply name").clicked();
            let cancel = crate::small_button(ui, "Cancel");
            // Text input can lose focus on pointer-down, before a button's
            // release click. Cancel must win over that ordinary blur commit.
            let cancel = cancel.clicked() || cancel.is_pointer_button_down_on();
            (output, apply, cancel, keyboard_pressed)
        })
        .inner;
    let (mut output, apply, cancel_clicked, keyboard_pressed) = row;
    if initial_focus {
        output.response.request_focus();
        output.state.clear_undoer();
        output
            .state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(value.chars().count()),
            )));
        output.state.store(ui.ctx(), id);
    }
    crate::name_keyboard(ui, id, value);
    let enter = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let action = if cancel || cancel_clicked {
        InlineNameAction::Cancel
    } else if apply
        || enter
        || (!initial_focus
            && output.response.lost_focus()
            && !keyboard_pressed
            && !crate::name_keyboard_open(ui.ctx(), id))
    {
        InlineNameAction::Commit
    } else {
        InlineNameAction::Editing
    };
    InlineNameResponse {
        response: output.response,
        action,
    }
}

/// Width reserved for the clip editors' view cluster before it has been
/// measured once: four zoom squares, View and Tool help.
const EDITOR_VIEW_CONTROLS_ESTIMATE: f32 =
    crate::tokens::CONTROL_HEIGHT * 7.0 + crate::tokens::SPACE_SM * 4.0;

/// The trailing end of a clip editor toolbar, left to right: caption slot,
/// zoom controls, View and Tool help. The reservation is the width the controls
/// used last frame, so the row wraps only when they genuinely do not fit.
/// `caption` fills whatever remains between the leading tools and the zoom.
pub fn editor_view_controls(
    ui: &mut Ui,
    tool: crate::EditorTool,
    context: crate::ToolHintContext,
    selection_available: bool,
    view_menu: impl FnOnce(&mut Ui),
    caption: impl FnOnce(&mut Ui),
) -> crate::TimelineViewAction {
    let id = Id::new(("editor_view_controls_width", context));
    let reserved = ui
        .ctx()
        .data(|data| data.get_temp::<f32>(id))
        .unwrap_or(EDITOR_VIEW_CONTROLS_ESTIMATE);
    crate::toolbar_trailing(ui, reserved, |ui| {
        // An absolute id keeps these widgets' ids (and open menus) stable
        // when controls before them come and go.
        ui.scope_builder(egui::UiBuilder::new().id(id.with("cluster")), |ui| {
            view_cluster(
                ui,
                id,
                tool,
                context,
                selection_available,
                view_menu,
                caption,
            )
        })
        .inner
    })
    .inner
}

fn view_cluster(
    ui: &mut Ui,
    id: Id,
    tool: crate::EditorTool,
    context: crate::ToolHintContext,
    selection_available: bool,
    view_menu: impl FnOnce(&mut Ui),
    caption: impl FnOnce(&mut Ui),
) -> crate::TimelineViewAction {
    {
        let help = crate::editor_tool_help(ui, tool, context);
        let view = crate::quiet_view_menu(ui, "View", view_menu).response;
        let action = crate::timeline_zoom_controls(ui, selection_available);
        // Everything placed so far, right to left: help, View, then zoom.
        let left = ui.min_rect().left().min(view.rect.left()) - ui.spacing().item_spacing.x;
        ui.ctx().data_mut(|data| {
            data.insert_temp(id, (help.rect.right() - left).ceil());
        });
        caption(ui);
        action
    }
}

/// Transient editor status (progress, errors, a selection readout) shown in
/// the toolbar caption slot instead of a separate row above the canvas.
pub fn editor_status_caption(ui: &mut Ui, status: &str) -> Option<Response> {
    let available = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    if available.width() < crate::tokens::CONTROL_HEIGHT {
        return None;
    }
    Some(
        ui.allocate_ui_with_layout(
            egui::vec2(available.width(), crate::tokens::CONTROL_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                // Drop whole trailing clauses before truncating, so a narrow
                // caption still reads "Click to add" rather than "Click to a…".
                let font = crate::text::TextRole::Note.font();
                let fits = |text: &str| {
                    ui.painter()
                        .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
                        .size()
                        .x
                        <= available.width()
                };
                let clauses: Vec<_> = status.split(" · ").collect();
                let shown = (1..=clauses.len())
                    .rev()
                    .map(|count| clauses[..count].join(" · "))
                    .find(|text| fits(text))
                    .unwrap_or_else(|| clauses[0].to_owned());
                ui.add(
                    egui::Label::new(crate::text::TextRole::Note.rich(ui.ctx(), shown))
                        .truncate()
                        .selectable(false),
                )
            },
        )
        .inner,
    )
}

/// A panel below a clip editor canvas for analysis tools (transients, grid
/// slicing). Its height is taken from the bottom of the remaining area, so the
/// ruler and canvas top never move when it opens, wraps or a tool changes.
/// Returns the height the caller must leave free under its canvas.
pub fn editor_bottom_drawer(ui: &mut Ui, id: Id, contents: impl FnOnce(&mut Ui)) -> f32 {
    let height = ui
        .ctx()
        .data(|data| data.get_temp::<f32>(id))
        .unwrap_or(crate::tokens::PARAMETER_ROW_HEIGHT + 2.0 * crate::tokens::SPACE_SM + 1.0);
    let available = ui.available_rect_before_wrap();
    let rect = egui::Rect::from_min_max(
        egui::pos2(available.left(), available.bottom() - height),
        available.max,
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    let colors = crate::theme::palette(ui.ctx());
    child.painter().hline(
        rect.x_range(),
        rect.top(),
        egui::Stroke::new(1.0_f32, colors.line),
    );
    crate::parameter_toolbar(&mut child, contents);
    let used = child.min_rect().height().ceil();
    if (used - height).abs() > 0.5 {
        ui.ctx().data_mut(|data| data.insert_temp(id, used));
        ui.ctx().request_repaint();
    }
    height
}

/// Nudge commands as a section of a clip editor's Clip menu: the increment
/// choice (shared with the arranger's Nudge menu) and both directions.
pub fn clip_nudge_section(ui: &mut Ui, enabled: bool) -> i32 {
    let id = Id::new("timeline_nudge_unit");
    let mut unit = crate::NudgeUnit::load(ui.ctx());
    let mut direction = 0;
    ui.horizontal(|ui| {
        crate::text::note(ui, "Nudge by");
        for (value, label) in [
            (crate::NudgeUnit::Grid, "Grid"),
            (crate::NudgeUnit::Millisecond, "1 ms"),
            (crate::NudgeUnit::Sample, "1 sample"),
        ] {
            crate::selectable_value(ui, &mut unit, value, label);
        }
    });
    for (value, label, hint) in [
        (-1, "Nudge left", "Move the clip left · Ctrl/Cmd+Left arrow"),
        (
            1,
            "Nudge right",
            "Move the clip right · Ctrl/Cmd+Right arrow",
        ),
    ] {
        if ui
            .add_enabled(enabled, crate::button_widget(label))
            .on_hover_text(hint)
            .clicked()
        {
            direction = value;
            ui.close();
        }
    }
    ui.ctx().data_mut(|data| data.insert_temp(id, unit));
    direction
}

/// A zero-based beat edited as a one-based "bar.beat.sixteenth" position, the
/// same notation as the rulers. Typing accepts "12", "12.3" or "12.3.2";
/// the caller sets the range.
pub fn musical_position_field<'a>(
    beat: &'a mut f64,
    beats_per_bar: u8,
    prefix: &str,
) -> crate::Number<'a> {
    crate::number(beat)
        .prefix(prefix)
        .custom_formatter(move |beat, _| crate::time::musical_position_label(beat, beats_per_bar))
        .custom_parser(move |text| parse_musical_position(text, beats_per_bar))
}

/// Parse "bar.beat.sixteenth" (or "bar.beat", or "bar") into a zero-based beat.
pub fn parse_musical_position(text: &str, beats_per_bar: u8) -> Option<f64> {
    let mut parts = text
        .trim()
        .split('.')
        .map(|part| part.trim().parse::<f64>());
    let bar = parts.next()?.ok()?;
    let beat = parts.next().transpose().ok()?.unwrap_or(1.0);
    let sixteenth = parts.next().transpose().ok()?.unwrap_or(1.0);
    if parts.next().is_some() || bar < 1.0 || beat < 1.0 || sixteenth < 1.0 {
        return None;
    }
    Some((bar - 1.0) * f64::from(beats_per_bar.max(1)) + (beat - 1.0) + (sixteenth - 1.0) / 4.0)
}

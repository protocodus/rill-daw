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
    meter: &crate::MeterMap,
) -> ClipTimingEdit {
    let mut edit = ClipTimingEdit::default();
    let label = crate::text::label(ui, "Start");
    let mut start = values.start + 1.0;
    let start_range = 1.0..=(maximum_beat - values.duration + 1.0).max(1.0);
    let (stepper, position) = ui
        .horizontal_wrapped(|ui| {
            let stepper = crate::number_stepper(
                ui,
                &mut start,
                1.0,
                start_range.clone(),
                "start beat",
                |ui, value| {
                    ui.add(
                        crate::number(value)
                            .range(start_range.clone())
                            .clamp_existing_to_range(false)
                            .speed(0.01)
                            .min_decimals(2)
                            .max_decimals(9)
                            .prefix("Beat "),
                    )
                },
            );
            let position = musical_position_entry(
                ui,
                "clip_start_position",
                if stepper.response.changed() {
                    start - 1.0
                } else {
                    values.start
                },
                meter,
                0.0..=(*start_range.end() - 1.0),
            );
            (stepper, position)
        })
        .inner;
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
    edit.new_gesture |= position.0.clicked() || position.1.is_some();
    if let Some(start) = position.1 {
        edit.start = Some(start);
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
        crate::choice(
            ui,
            "clip_duration_mode",
            mode,
            &[
                (DurationEditMode::Trim, "Trim"),
                (DurationEditMode::Stretch, "Stretch"),
            ],
        )
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

#[derive(Clone, Copy, Hash)]
enum PositionField {
    Bar,
    Beat,
}

#[derive(Clone)]
struct PositionDraft {
    original: f64,
    meter: crate::MeterMap,
    bar: f64,
    beat: f64,
    initial_bar: f64,
    initial_beat: f64,
    keypad: Option<PositionField>,
    last_frame: u64,
}

impl PositionDraft {
    fn new(position: f64, meter: &crate::MeterMap, frame: u64) -> Self {
        // Exact floor: a position just below a bar line stays in its bar,
        // unlike the map's display tolerance.
        let (mut bar, _) = meter.position(position);
        if bar > 1 && position < meter.bar_start(bar) {
            bar -= 1;
        }
        let beat = position - meter.bar_start(bar) + 1.0;
        let bar = f64::from(bar);
        Self {
            original: position,
            meter: meter.clone(),
            bar,
            beat,
            initial_bar: bar,
            initial_beat: beat,
            keypad: None,
            last_frame: frame,
        }
    }

    /// Beats in the staged bar.
    fn bar_meter(&self) -> u8 {
        self.meter.meter_at(
            self.meter
                .bar_start(self.bar.clamp(1.0, f64::from(u32::MAX)) as u32),
        )
    }

    fn position(&self, range: &std::ops::RangeInclusive<f64>) -> Option<f64> {
        if !self.bar.is_finite()
            || self.bar < 1.0
            || self.bar.fract() != 0.0
            || self.bar > f64::from(u32::MAX)
        {
            return None;
        }
        let beat_unchanged = self.beat.to_bits() == self.initial_beat.to_bits();
        let limit = f64::from(self.bar_meter()) + 1.0;
        if !beat_unchanged && (!self.beat.is_finite() || self.beat < 1.0 || self.beat >= limit) {
            return None;
        }
        // A kept beat must also exist in a bar of another meter.
        let initial_limit = f64::from(self.meter.meter_at(self.original)) + 1.0;
        if beat_unchanged && limit < initial_limit && self.beat >= limit {
            return None;
        }
        let bar_start = |bar: f64| self.meter.bar_start(bar as u32);
        let position = if self.bar.to_bits() == self.initial_bar.to_bits() && beat_unchanged {
            self.original
        } else if beat_unchanged {
            // Moving only the bar preserves a sub-beat remainder that may be
            // too small to survive displaying it as a one-based beat.
            self.original + (bar_start(self.bar) - bar_start(self.initial_bar))
        } else {
            bar_start(self.bar) + (self.beat - 1.0)
        };
        (position.is_finite() && range.contains(&position)).then_some(position)
    }
}

fn position_beat_display(beat: f64, meter: u8) -> String {
    let limit = f64::from(meter.max(1)) + 1.0;
    if !beat.is_finite() || !(1.0..limit).contains(&beat) {
        return crate::exact_value_text(beat);
    }
    let rounded = format!("{beat:.9}");
    if rounded.parse::<f64>().is_ok_and(|value| value >= limit) {
        // A concise field must not suggest a beat in the next bar while its
        // staged bar is unchanged. Keep the exact near-boundary remainder.
        crate::exact_value_text(beat)
    } else {
        rounded
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

/// A visible bar/beat alternative to scalar timing fields. The menu stages
/// both values and commits one zero-based beat only on Apply position. Its
/// numeric keypads provide the same operation without typing. Untouched values
/// remain exact, including positions between sixteenths and beyond display
/// precision; merely opening, closing or changing meter never retimes them.
pub fn musical_position_entry(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash,
    current: f64,
    meter: &crate::MeterMap,
    range: std::ops::RangeInclusive<f64>,
) -> (Response, Option<f64>) {
    let id = ui.make_persistent_id(("musical_position_entry", id_salt));
    let caption = crate::duration::musical_position(current, meter);
    let mut applied = None;
    let response = ui.push_id(id, |ui| {
        crate::choice_menu(ui, caption, |ui| {
            let frame = ui.ctx().cumulative_frame_nr();
            let mut draft = ui.data(|data| data.get_temp::<PositionDraft>(id))
                .filter(|draft| {
                    frame <= draft.last_frame.saturating_add(1)
                        && draft.original.to_bits() == current.to_bits()
                        && draft.meter == *meter
                })
                .unwrap_or_else(|| PositionDraft::new(current, meter, frame));
            draft.last_frame = frame;
            let maximum_bar = f64::from(meter.position(*range.end()).0);
            let bar_meter = draft.bar_meter();
            // The final beat may contain any fraction below the next bar.
            let beat_limit = f64::from_bits((f64::from(bar_meter) + 1.0).to_bits() - 1);
            if let Some(field) = draft.keypad {
                let (value, field_range, name, whole_numbers) = match field {
                    PositionField::Bar => (
                        &mut draft.bar, 1.0..=maximum_bar.max(1.0), "Start bar", true,
                    ),
                    PositionField::Beat => (
                        &mut draft.beat, 1.0..=beat_limit, "Beat in bar", false,
                    ),
                };
                // A native submenu forwards its Apply/Cancel close to the
                // entire menu chain. Keep this edit step in the same popup,
                // with a close boundary that returns to the staged position.
                let entry = ui.scope_builder(
                    egui::UiBuilder::new().id_salt(("keypad", field)).closable(),
                    |ui| {
                        // egui clears focus before drawing an Escape frame.
                        // This edit step owns Escape even after its text field
                        // loses focus, so dismiss only the keypad draft.
                        if ui.is_enabled() && ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                        }) {
                            crate::value_entry::clear_value_entry(ui, id.with(field));
                            ui.close();
                            None
                        } else {
                            crate::value_entry::numeric_value_entry(
                                ui, id.with(field), *value, field_range, name, whole_numbers,
                            )
                        }
                    },
                );
                if let Some(next) = entry.inner {
                    *value = next;
                }
                if entry.response.should_close() {
                    draft.keypad = None;
                }
                ui.data_mut(|data| data.insert_temp(id, draft));
                return;
            }
            crate::text::label(ui, "Start position");
            crate::form_grid(ui, id.with("fields"), |ui| {
                crate::form_label(ui, "Bar");
                let (_, keypad) = crate::number_stepper::number_stepper_controls(ui, &mut draft.bar, 1.0,
                    1.0..=maximum_bar.max(1.0), "start bar", |ui, value| {
                        ui.add(crate::number(value).speed(1.0)
                            .clamp_existing_to_range(false).update_while_editing(false)
                            .custom_formatter(|value, _| crate::exact_value_text(value)))
                    });
                if keypad.clicked() {
                    draft.keypad = Some(PositionField::Bar);
                }
                ui.end_row();
                crate::form_label(ui, "Beat in bar");
                let (_, keypad) = crate::number_stepper::number_stepper_controls(ui, &mut draft.beat, 1.0,
                    1.0..=beat_limit, "beat in bar", |ui, value| {
                        let number_id = ui.next_auto_id();
                        let context = ui.ctx().clone();
                        ui.add(crate::number(value).speed(0.01)
                            .clamp_existing_to_range(false).update_while_editing(false)
                            .custom_formatter(move |value, _| {
                                // Query after DragValue registers keyboard focus,
                                // including focus acquired by Tab in this frame.
                                // Only idle paint is concise; native text and its
                                // subsequent blur parse always use exact digits.
                                if context.memory(|memory| memory.has_focus(number_id)) {
                                    crate::exact_value_text(value)
                                } else {
                                    position_beat_display(value, bar_meter)
                                }
                            }))
                    });
                if keypad.clicked() {
                    draft.keypad = Some(PositionField::Beat);
                }
                ui.end_row();
            });
            let candidate = draft.position(&range);
            if candidate.is_none() {
                crate::text::note(ui, format!(
                    "Use a whole bar and a beat from 1 to below {} within the clip's range.", u16::from(bar_meter) + 1
                ));
            }
            let (apply, cancel) = crate::dialog_actions(ui, |ui| {
                let apply = ui.add_enabled_ui(candidate.is_some(), |ui| {
                    crate::dialog_primary(ui, "Apply position")
                }).inner.clicked();
                let cancel = crate::dialog_button(ui, "Cancel").clicked();
                (apply, cancel)
            }, |_| {}).inner.0;
            if apply || cancel {
                ui.data_mut(|data| data.remove::<PositionDraft>(id));
                if apply {
                    applied = candidate.filter(|value| value.to_bits() != current.to_bits());
                }
                ui.close();
            } else {
                ui.data_mut(|data| data.insert_temp(id, draft));
            }
        }).response
    }).inner.on_hover_text("Place the clip by bar and beat; fractional beats stay exact");
    (response, applied)
}

#[cfg(test)]
mod position_entry_tests {
    use super::PositionDraft;
    use crate::MeterMap;

    #[test]
    fn untouched_positions_keep_every_bit_in_all_supported_meters() {
        for meter in 1..=32 {
            for start in [
                0.0,
                -0.0,
                1e-20,
                0.123_456_789_123_456,
                3.999_999_999_999,
                f64::from_bits(4.0_f64.to_bits() - 1),
                21.125_123_456_789,
                999_990.123_456_789,
            ] {
                let draft = PositionDraft::new(start, &MeterMap::uniform(meter), 0);
                assert_eq!(
                    draft.position(&(0.0..=1_000_000.0)).unwrap().to_bits(),
                    start.to_bits()
                );
            }
        }
    }

    #[test]
    fn bar_moves_preserve_the_fraction_and_beat_entry_uses_the_current_meter() {
        for meter in [3, 4, 7, 11] {
            let start = 1.123_456_789_123;
            let mut draft = PositionDraft::new(start, &MeterMap::uniform(meter), 0);
            draft.bar = 5.0;
            assert_eq!(
                draft.position(&(0.0..=1_000.0)),
                Some(start + 4.0 * f64::from(meter))
            );
            draft.beat = 2.125;
            assert_eq!(
                draft.position(&(0.0..=1_000.0)),
                Some(4.0 * f64::from(meter) + 1.125)
            );
        }
    }

    #[test]
    fn invalid_positions_and_clip_end_overflows_cannot_apply() {
        for (bar, beat) in [
            (0.0, 1.0),
            (1.5, 1.0),
            (f64::NAN, 1.0),
            (f64::INFINITY, 1.0),
            (2.0, 0.0),
            (2.0, 4.0),
            (2.0, f64::NAN),
            (2.0, f64::INFINITY),
        ] {
            let mut draft = PositionDraft::new(0.0, &MeterMap::uniform(3), 0);
            draft.bar = bar;
            draft.beat = beat;
            assert!(draft.position(&(0.0..=100.0)).is_none(), "{bar}:{beat}");
        }
        let mut draft = PositionDraft::new(0.0, &MeterMap::uniform(3), 0);
        draft.bar = 4.0;
        draft.beat = 3.5;
        assert_eq!(draft.position(&(0.0..=11.5)), Some(11.5));
        assert!(draft.position(&(0.0..=11.25)).is_none());
    }

    #[test]
    fn bar_and_beat_entry_follow_meter_changes() {
        // 4/4 for bar 1, then 3/4: bar 3 starts at beat 7.
        let meter = MeterMap::new(
            4,
            &[daw_timeline::MeterChange {
                bar: 2,
                beats_per_bar: 3,
            }],
        );
        let mut draft = PositionDraft::new(0.5, &meter, 0);
        draft.bar = 3.0;
        assert_eq!(draft.position(&(0.0..=100.0)), Some(7.5));
        draft.beat = 3.75;
        assert_eq!(draft.position(&(0.0..=100.0)), Some(9.75));
        draft.beat = 4.0;
        assert!(draft.position(&(0.0..=100.0)).is_none());
        // Beat 4 of bar 1 has no counterpart in a 3/4 bar.
        let mut draft = PositionDraft::new(3.5, &meter, 0);
        draft.bar = 2.0;
        assert!(draft.position(&(0.0..=100.0)).is_none());
        assert_eq!(super::parse_musical_position("3.2.3", &meter), Some(8.5));
    }
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
                    crate::tokens::ICON_SIZE + style.spacing.icon_spacing
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
                .desired_width(width.max(crate::tokens::SPACE_XL))
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

fn header_zoom_id(context: crate::ToolHintContext) -> Id {
    Id::new(("editor_header_zoom", context))
}

/// Below this window width the header needs its room for the clip's identity
/// and song span, so zoom stays in the toolbar's view cluster.
const HEADER_ZOOM_MIN_WIDTH: f32 = 1280.0;

/// A clip editor's zoom controls, drawn in its workspace header so the tool
/// row does not spend a second line on view controls. The click is handed to
/// [`editor_view_controls`], which the editor lays out later in the same frame.
/// On narrow windows this draws nothing and the view cluster keeps the zoom.
pub fn editor_header_zoom(ui: &mut Ui, context: crate::ToolHintContext) {
    if ui.ctx().content_rect().width() < HEADER_ZOOM_MIN_WIDTH {
        return;
    }
    let id = header_zoom_id(context);
    let frame = ui.ctx().cumulative_frame_nr();
    ui.ctx()
        .data_mut(|data| data.insert_temp(id.with("frame"), frame));
    let selection = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(id.with("selection")))
        .unwrap_or(false);
    let action = crate::timeline_zoom_controls(ui, selection);
    if action != crate::TimelineViewAction::None {
        ui.ctx().data_mut(|data| data.insert_temp(id, action));
    }
}

/// The trailing end of a clip editor toolbar, left to right: caption slot,
/// zoom (unless the header shows it), View and Tool help. Returns the zoom
/// chosen here or in the header this frame. The reservation is the width the controls
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
        let zoom = header_zoom_id(context);
        let frame = ui.ctx().cumulative_frame_nr();
        let (in_header, header_action) = ui.ctx().data_mut(|data| {
            data.insert_temp(zoom.with("selection"), selection_available);
            (
                data.get_temp::<u64>(zoom.with("frame")) == Some(frame),
                data.remove_temp::<crate::TimelineViewAction>(zoom)
                    .unwrap_or_default(),
            )
        });
        let action = if in_header {
            header_action
        } else {
            crate::timeline_zoom_controls(ui, selection_available)
        };
        // Everything placed so far, right to left: help, then View.
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
    meter: &'a crate::MeterMap,
    prefix: &str,
) -> crate::Number<'a> {
    crate::number(beat)
        .prefix(prefix)
        .custom_formatter(move |beat, _| crate::time::musical_position_label(beat, meter))
        .custom_parser(move |text| parse_musical_position(text, meter))
}

/// Parse "bar.beat.sixteenth" (or "bar.beat", or "bar") into a zero-based beat.
pub fn parse_musical_position(text: &str, meter: &crate::MeterMap) -> Option<f64> {
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
    if bar > f64::from(u32::MAX) {
        return None;
    }
    // A fractional bar is a fraction of that bar's own length.
    let whole = bar.floor() as u32;
    let bar_start = meter.bar_start(whole);
    let length = meter.bar_start(whole.saturating_add(1)) - bar_start;
    Some(bar_start + bar.fract() * length + (beat - 1.0) + (sixteenth - 1.0) / 4.0)
}

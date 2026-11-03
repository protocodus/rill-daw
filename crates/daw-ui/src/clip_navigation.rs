//! Compact navigation and slicing controls shared by clip editors.
use crate::{text::TextRole, tokens};
use egui::{Ui, WidgetText};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipStep {
    Previous,
    Next,
}

/// Previous/Next as two square chevrons for the slot right after Back.
/// Their accessible names stay "Previous" and "Next".
pub fn clip_navigation_header_controls(
    ui: &mut Ui,
    previous: bool,
    next: bool,
) -> Option<ClipStep> {
    if !previous && !next {
        return None;
    }
    let square = egui::Vec2::splat(tokens::CONTROL_HEIGHT);
    let mut step = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.spacing_mut().button_padding.x = tokens::SPACE_SM;
        for (enabled, icon, label, hint, value) in [
            (
                previous,
                crate::Icon::Back,
                "Previous",
                "Previous clip · Shift+Tab",
                ClipStep::Previous,
            ),
            (
                next,
                crate::Icon::ChevronRight,
                "Next",
                "Next clip · Tab",
                ClipStep::Next,
            ),
        ] {
            if ui
                .add_enabled_ui(enabled, |ui| {
                    ui.add_sized(
                        square,
                        crate::IconButton::icon_only(icon, label)
                            .presentation()
                            .quiet(),
                    )
                })
                .inner
                .on_hover_text(hint)
                .clicked()
            {
                step = Some(value);
            }
        }
    });
    step
}

/// Navigation changes the inspected clip, without changing its musical data.
pub fn clip_navigation_controls(ui: &mut Ui, previous: bool, next: bool) -> Option<ClipStep> {
    let mut step = None;
    if ui
        .add_enabled(previous, crate::view_button_widget("Previous clip"))
        .on_hover_text("Inspect the previous clip, retaining tool and zoom · Shift+Tab")
        .clicked()
    {
        step = Some(ClipStep::Previous);
        ui.close();
    }
    if ui
        .add_enabled(next, crate::view_button_widget("Next clip"))
        .on_hover_text("Inspect the next clip, retaining tool and zoom · Tab")
        .clicked()
    {
        step = Some(ClipStep::Next);
        ui.close();
    }
    step
}

/// Previewed grid cuts use a choice control; committing them is an action.
pub fn audio_grid_slice_controls(ui: &mut Ui, grid: &mut f64, cuts: usize) -> bool {
    crate::select("audio_slice_grid", format!("Slice grid {grid} beats"))
        .width(155.0)
        .show_ui(ui, |ui| {
            for value in [0.125, 0.25, 0.5, 1.0, 2.0, 4.0] {
                crate::selectable_value(ui, grid, value, format!("{value} beats"));
            }
        });
    ui.label(format!("{cuts} cuts"));
    ui.add_enabled(cuts > 0 && cuts <= 2048, crate::button_widget("Split grid"))
        .on_hover_text("Split at the previewed grid positions; keep every resulting cue selected")
        .clicked()
}

/// Which part of [`clip_workspace_header`] its contents closure is drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderSlot {
    /// Trailing actions, laid out right to left from the right edge.
    Actions,
    /// The slot right after Back, for Previous/Next.
    Navigation,
}

/// The Clip Editor and Mixer header: Back, a navigation slot, then the title
/// in `TextRole::Subtitle` followed on the same line by an optional context. The title keeps one
/// size in both workspaces. `contents` runs once per [`HeaderSlot`] (actions
/// first, so the identity truncates into the remaining width); one closure
/// lets both slots borrow the same application state.
pub fn clip_workspace_header<R>(
    ui: &mut Ui,
    title: impl Into<WidgetText>,
    context: Option<&str>,
    mut contents: impl FnMut(&mut Ui, HeaderSlot) -> R,
) -> crate::WorkspaceHeaderResponse<R> {
    let full_title = title.into().text().to_owned();
    let row = crate::soft_surface(
        ui,
        egui::Frame::new().inner_margin(egui::Margin::symmetric(
            tokens::SPACE_MD as i8,
            tokens::SPACE_SM as i8,
        )),
        |ui| {
            // Title and context share one line, so the header costs one
            // control height in every workspace.
            let height = tokens::CONTROL_HEIGHT;
            ui.horizontal(|ui| {
                ui.set_min_height(height);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.set_min_height(height);
                    let inner = contents(ui, HeaderSlot::Actions);
                    ui.add_space(tokens::SPACE_MD);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_height(height);
                        let back = crate::quiet_button(ui, "Back")
                            .on_hover_text("Return to the previous workspace");
                        ui.add_space(tokens::SPACE_XS);
                        contents(ui, HeaderSlot::Navigation);
                        ui.add_space(tokens::SPACE_XS);
                        // The context (the clip's song span) stays whole; a long
                        // title truncates first.
                        let context_width = context.map_or(0.0, |context| {
                            ui.painter()
                                .layout_no_wrap(
                                    context.to_owned(),
                                    TextRole::Note.font(),
                                    egui::Color32::PLACEHOLDER,
                                )
                                .size()
                                .x
                                + tokens::SPACE_SM
                        });
                        let title_width = (ui.available_width() - context_width).max(0.0);
                        let title = ui
                            .scope(|ui| {
                                ui.set_max_width(title_width);
                                ui.add(
                                    egui::Label::new(
                                        TextRole::Subtitle.rich(ui.ctx(), &full_title),
                                    )
                                    .truncate()
                                    .show_tooltip_when_elided(false),
                                )
                            })
                            .inner
                            .on_hover_text(&full_title);
                        let context = context.map(|context| {
                            ui.add_space(tokens::SPACE_SM);
                            ui.add(
                                egui::Label::new(TextRole::Note.rich(ui.ctx(), context))
                                    .truncate()
                                    .show_tooltip_when_elided(false),
                            )
                            .on_hover_text(context)
                        });
                        (back, title, context, inner)
                    })
                    .inner
                })
                .inner
            })
            .inner
        },
    );
    let (back, title, context, inner) = row.inner;
    crate::WorkspaceHeaderResponse {
        back,
        title,
        context,
        inner,
        response: row.response,
    }
}

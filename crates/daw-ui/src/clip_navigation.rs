//! Compact navigation and slicing controls shared by clip editors.
use egui::Ui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipStep {
    Previous,
    Next,
}

/// The existing workspace header can host one-click navigation without taking
/// another row from the editing canvas. Its pair has a bounded width.
pub fn clip_navigation_header_controls(
    ui: &mut Ui,
    previous: bool,
    next: bool,
) -> Option<ClipStep> {
    if !previous && !next {
        return None;
    }
    let compact = ui.ctx().content_rect().width() < 1000.0;
    let width = (if compact { 160.0_f32 } else { 220.0_f32 }).min(ui.available_width());
    let gap = crate::tokens::SPACE_XS;
    // "Previous" needs more room than "Next" after icon and native padding.
    let previous_width = ((width - gap) * if compact { 0.60 } else { 0.57 }).max(1.0);
    let next_width = ((width - gap) - previous_width).max(1.0);
    ui.allocate_ui_with_layout(
        egui::vec2(width, crate::tokens::CONTROL_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let previous = ui
                .add_enabled_ui(previous, |ui| {
                    ui.add_sized(
                        egui::vec2(previous_width, crate::tokens::CONTROL_HEIGHT),
                        (if compact {
                            crate::IconButton::new("Previous")
                        } else {
                            crate::IconButton::with_icon(crate::Icon::Back, "Previous")
                        })
                        .presentation()
                        .quiet()
                        .wrap_mode(egui::TextWrapMode::Extend),
                    )
                })
                .inner
                .on_hover_text("Previous clip · Shift+Tab")
                .clicked();
            let next = ui
                .add_enabled_ui(next, |ui| {
                    ui.add_sized(
                        egui::vec2(next_width, crate::tokens::CONTROL_HEIGHT),
                        (if compact {
                            crate::IconButton::new("Next")
                        } else {
                            crate::IconButton::with_icon(crate::Icon::ChevronRight, "Next")
                        })
                        .presentation()
                        .quiet()
                        .wrap_mode(egui::TextWrapMode::Extend),
                    )
                })
                .inner
                .on_hover_text("Next clip · Tab")
                .clicked();
            if previous {
                Some(ClipStep::Previous)
            } else if next {
                Some(ClipStep::Next)
            } else {
                None
            }
        },
    )
    .inner
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

//! Shared mouse access to automated controls and their retained curves.
use crate::AutomationState;
use egui::{Response, Ui};

pub fn automation_channel_menu<R>(
    ui: &mut Ui,
    state: Option<AutomationState>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    match state {
        Some(state) => {
            crate::widgets::semantic_narrow_action_menu(ui, state.rich_label(ui.ctx()), contents)
        }
        None => crate::compact_action_menu(ui, "Channel", contents),
    }
}

#[derive(Clone, Copy, Debug)]
pub enum AutomationNavigation {
    Create(AutomationControl),
    Edit(u64),
    Resume(u64),
}

/// A native parameter's owner, carried as UI intent without depending on the
/// application's song model. Every surface uses the same exact target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationControl {
    Gain { track: u64 },
    Pan { track: u64 },
    Send { track: u64, bus: u64 },
}

pub fn automatable_menu_contents(
    ui: &mut Ui,
    control: AutomationControl,
    automation: Option<(u64, AutomationState)>,
) -> Option<AutomationNavigation> {
    if let Some((lane, state)) = automation {
        automation_menu_contents(ui, lane, state)
    } else if crate::button(ui, "Create automation").clicked() {
        ui.close();
        Some(AutomationNavigation::Create(control))
    } else {
        None
    }
}

/// Expose creation before a lane exists, then the same Edit/Resume menu used by
/// every other automated surface. Creating a lane never relies on selection.
pub fn automatable_parameter_context(
    ui: &Ui,
    response: &Response,
    control: AutomationControl,
    automation: Option<(u64, AutomationState)>,
) -> Option<AutomationNavigation> {
    if let Some((lane, state)) = automation {
        return automation_parameter_context(ui, response, lane, state);
    }
    response
        .clone()
        .on_hover_text("Right-click to create automation for this control");
    let mut action = None;
    response.context_menu(|ui| action = automatable_menu_contents(ui, control, None));
    action
}

pub fn automation_state_label(state: AutomationState) -> &'static str {
    match state {
        AutomationState::Read => "Automated",
        AutomationState::Overridden => "Overridden",
        AutomationState::Bypassed => "Bypassed",
    }
}

pub fn automation_menu_contents(
    ui: &mut Ui,
    lane: u64,
    state: AutomationState,
) -> Option<AutomationNavigation> {
    crate::automation_badge(ui, state);
    let mut action = None;
    if crate::button(ui, "Edit automation").clicked() {
        action = Some(AutomationNavigation::Edit(lane));
        ui.close();
    }
    if state == AutomationState::Overridden && crate::button(ui, "Resume automation").clicked() {
        action = Some(AutomationNavigation::Resume(lane));
        ui.close();
    }
    action
}

/// Keep a control's native drag and keyboard semantics. Its context menu gives
/// the mouse the same editing and resume actions as the dedicated lane view.
pub fn automation_parameter_context(
    ui: &Ui,
    response: &Response,
    lane: u64,
    state: AutomationState,
) -> Option<AutomationNavigation> {
    crate::paint_automation_control(ui, response, state);
    response.clone().on_hover_text(format!(
        "{} · Adjust to override · Right-click to edit or resume automation",
        automation_state_label(state)
    ));
    let mut action = None;
    response.context_menu(|ui| action = automation_menu_contents(ui, lane, state));
    action
}

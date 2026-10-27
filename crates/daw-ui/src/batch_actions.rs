//! Contextual commands for a selection of signal channels.
use egui::Ui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchRouteChoice {
    Output(Option<u64>),
    Send { bus: u64, pre_fader: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelSelectionChoice {
    SelectOnly,
    Add,
    Remove,
    All,
    Clear,
}

/// Mouse equivalents for additive keyboard selection, presented on a channel
/// header's context menu using the same action widgets as other item menus.
pub fn channel_selection_choices(ui: &mut Ui, selected: bool) -> Option<ChannelSelectionChoice> {
    let mut choice = None;
    for (label, action) in [
        ("Select channel", ChannelSelectionChoice::SelectOnly),
        (
            if selected {
                "Remove channel from selection"
            } else {
                "Add channel to selection"
            },
            if selected {
                ChannelSelectionChoice::Remove
            } else {
                ChannelSelectionChoice::Add
            },
        ),
        ("Select all channels", ChannelSelectionChoice::All),
        ("Clear channel selection", ChannelSelectionChoice::Clear),
    ] {
        if crate::button(ui, label).clicked() {
            choice = Some(action);
            ui.close();
        }
    }
    choice
}

/// The caller supplies destinations valid for every selected channel. Main
/// output is one click after opening the menu; pre/post sends stay grouped.
pub fn batch_route_choices(
    ui: &mut Ui,
    outputs: &[(u64, String)],
    sends: &[(u64, String)],
) -> Option<BatchRouteChoice> {
    let mut choice = None;
    crate::text::note(ui, "Main output");
    if crate::button(ui, "Master output").clicked() {
        choice = Some(BatchRouteChoice::Output(None));
        ui.close();
    }
    for (id, name) in outputs {
        if crate::button(ui, name).clicked() {
            choice = Some(BatchRouteChoice::Output(Some(*id)));
            ui.close();
        }
    }
    ui.separator();
    crate::text::note(ui, "Existing sends keep their settings");
    for (label, pre_fader) in [("Add post-fader send", false), ("Add pre-fader send", true)] {
        crate::menu_button(ui, label, |ui| {
            if sends.is_empty() {
                crate::text::note(ui, "No eligible buses");
            }
            for (id, name) in sends {
                if crate::button(ui, name).clicked() {
                    choice = Some(BatchRouteChoice::Send {
                        bus: *id,
                        pre_fader,
                    });
                    ui.close();
                }
            }
        });
    }
    choice
}

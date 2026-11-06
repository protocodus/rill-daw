use crate::app::{ChannelDestination, DawApp};
use daw_core::*;
use daw_ui as widgets;
use eframe::egui;

impl DawApp {
    /// The shared header of the channel utility windows: page links, then the
    /// contextual AI action when `ai_scope` is given. Returns whether a link
    /// navigated away and whether AI was requested.
    pub(crate) fn utility_header(
        &mut self,
        ui: &mut egui::Ui,
        current: ChannelDestination,
        ai_scope: Option<&str>,
    ) -> (bool, bool) {
        // Five links and the full AI label overflow a medium window; the
        // short AI label keeps the row on one line instead of wrapping.
        let links = self.channel_links(current);
        let compact = !widgets::links_fit(ui, current, &links, "AI for this context");
        widgets::utility_header(
            ui,
            |ui| self.channel_navigation(ui, current),
            |ui| {
                ai_scope.is_some_and(|scope| {
                    if compact {
                        widgets::contextual_ai_button_compact(ui, scope)
                    } else {
                        widgets::contextual_ai_button(ui, scope)
                    }
                    .clicked()
                })
            },
        )
    }

    pub fn channel_rack(&mut self, ctx: &egui::Context) {
        if !self.effects_open {
            return;
        }
        let Some(track_id) = self.selected_track else {
            self.effects_open = false;
            return;
        };
        let Some(track) = self.project.track(track_id) else {
            self.effects_open = false;
            return;
        };
        if matches!(track.kind, TrackKind::Folder | TrackKind::Automation) {
            self.effects_open = false;
            return;
        }
        let track_name = track.name.clone();
        let mut open = true;
        let mut edit_effect = None;
        let mut automate_effect = None;
        let mut add_effect = false;
        widgets::popup(format!("Effects of {track_name}"), ctx)
            .id(egui::Id::new("effects"))
            .open(&mut open)
            .default_width(620.0)
            .default_height(620.0)
            .vscroll(false)
            .show(ctx, |ui| {
                let scope = daw_tools::ToolScope::EffectChain { track_id };
                let label = scope.label(&self.project);
                let (navigated, open_ai) =
                    self.utility_header(ui, ChannelDestination::Effects, Some(&label));
                if navigated {
                    return;
                }
                if open_ai {
                    self.open_context_assistant(scope);
                    return;
                }
                let inserts = &self.project.track(track_id).unwrap().inserts;
                widgets::effect_chain_header(
                    ui,
                    inserts.len(),
                    inserts.iter().filter(|slot| slot.enabled).count(),
                );
                ui.add_space(widgets::tokens::SPACE_SM);
                let mut footer = widgets::popup_footer(ui, "effects_actions");
                let window_ui = ui;
                let ui = footer.body();
                widgets::popup_scroll_body(ui, "effects_body", 0.0, |ui| {
                    let mut remove = None;
                    let mut move_up = None;
                    let mut move_down = None;
                    let mut dragged = None;
                    let mut changed = false;
                    let track = self.project.track_mut(track_id).unwrap();
                    let count = track.inserts.len();
                    for (index, slot) in track.inserts.iter_mut().enumerate() {
                        ui.push_id(slot.id, |ui| {
                            let Effect::Vst3 { path, class_id } = &slot.effect;
                            let model =
                                crate::library::plugin_model(&self.vst_catalog, path, class_id);
                            let row = widgets::effect_slot(
                                ui,
                                (track_id, slot.id),
                                index,
                                &slot.name,
                                &model,
                                &mut slot.enabled,
                                |ui| {
                                    if widgets::small_button(ui, "Open editor")
                                        .on_hover_text("Open native effect editor")
                                        .clicked()
                                    {
                                        edit_effect = Some(slot.id);
                                    }
                                    if widgets::small_button(ui, "Automate…")
                                        .on_hover_text("Search this effect's automation parameters")
                                        .clicked()
                                    {
                                        automate_effect = Some(slot.id);
                                    }
                                    widgets::action_menu(ui, "More", |ui| {
                                        if ui
                                            .add_enabled(index > 0, widgets::button_widget("Up"))
                                            .clicked()
                                        {
                                            move_up = Some(index);
                                            ui.close();
                                        }
                                        if ui
                                            .add_enabled(
                                                index + 1 < count,
                                                widgets::button_widget("Down"),
                                            )
                                            .clicked()
                                        {
                                            move_down = Some(index);
                                            ui.close();
                                        }
                                        ui.separator();
                                        if widgets::button(ui, "Remove insert").clicked() {
                                            remove = Some(index);
                                            ui.close();
                                        }
                                    })
                                    .response
                                    .on_hover_text("Insert actions");
                                },
                            );
                            let Effect::Vst3 { path, .. } = &slot.effect;
                            changed |= row.inner.0.on_hover_text(path).changed();
                            if let Some(payload) =
                                widgets::reorder_drop_target::<(u64, u64)>(ui, &row.response)
                                && payload.0 == track_id
                                && payload.1 != slot.id
                            {
                                dragged = Some((payload.1, index));
                            }
                        });
                    }
                    if let Some(index) = remove {
                        track.inserts.remove(index);
                        changed = true;
                    } else if let Some(index) = move_up {
                        track.inserts.swap(index, index - 1);
                        changed = true;
                    } else if let Some(index) = move_down {
                        track.inserts.swap(index, index + 1);
                        changed = true;
                    }
                    if let Some((id, destination)) = dragged
                        && let Some(source) = track.inserts.iter().position(|slot| slot.id == id)
                    {
                        let slot = track.inserts.remove(source);
                        track.inserts.insert(destination, slot);
                        changed = true;
                    }
                    if track.inserts.is_empty() {
                        widgets::empty_effect_chain(ui);
                    }
                    if changed {
                        self.changed();
                    }
                });
                add_effect = footer
                    .show(window_ui, |ui| {
                        widgets::dialog_actions(
                            ui,
                            |ui| {
                                widgets::primary_button(ui, "+ Insert")
                                    .on_hover_text("Add an effect to the end of this chain")
                                    .clicked()
                            },
                            |_| {},
                        )
                        .inner
                        .0
                    })
                    .inner;
            });
        if !open {
            self.effects_open = false;
        }
        if add_effect {
            self.open_effect_library(track_id, false);
        }
        if let Some(effect) = edit_effect {
            self.open_effect_editor(track_id, effect);
        }
        if let Some(effect) = automate_effect {
            self.open_effect_automation(track_id, effect);
        }
    }

    pub fn routing_popup(&mut self, ctx: &egui::Context) {
        if !self.routing_open {
            return;
        }
        let Some(track_id) = self.selected_track else {
            self.routing_open = false;
            return;
        };
        let Some(track) = self.project.track(track_id) else {
            self.routing_open = false;
            return;
        };
        if matches!(track.kind, TrackKind::Folder | TrackKind::Automation) {
            self.routing_open = false;
            return;
        }
        let track_name = track.name.clone();
        let automation: std::collections::HashMap<_, _> = track
            .sends
            .iter()
            .filter_map(|send| {
                let target = AutomationTarget {
                    track: track_id,
                    parameter: AutomationParameter::SendGain {
                        bus: send.target_bus,
                    },
                };
                self.automation_control_state(target)
                    .map(|state| (send.target_bus, state))
            })
            .collect();
        let mut automation_navigation = None;
        let mut automation_overrides = Vec::new();
        let mut create_send = false;
        let mut create_bus_send = false;
        let eligible: std::collections::HashSet<_> = self
            .project
            .eligible_output_buses(track_id)
            .unwrap_or_default()
            .into_iter()
            .map(|track| track.id)
            .collect();
        let buses: Vec<_> = self
            .project
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Bus && track.id != track_id)
            .map(|track| (track.id, track.name.clone()))
            .collect();
        let mut open = true;
        widgets::popup(format!("Routing of {track_name}"), ctx)
            .id(egui::Id::new("routing"))
            .open(&mut open)
            .default_width(widgets::tokens::POPUP_WIDTH_MD)
            // A send row is one unwrapped parameter_row; keep it whole. The
            // height hugs the content and the body scrolls in short windows.
            .min_width(widgets::popup_body_size(ctx).x.min(widgets::tokens::POPUP_WIDTH_MD))
            .vscroll(false)
            .show(ctx, |ui| {
                let scope = daw_tools::ToolScope::Routing { track_id };
                let label = scope.label(&self.project);
                let (navigated, open_ai) =
                    self.utility_header(ui, ChannelDestination::Routing, Some(&label));
                if navigated {
                    return;
                }
                if open_ai {
                    self.open_context_assistant(scope);
                    return;
                }
                let mut footer = widgets::popup_footer(ui, "routing_actions");
                let window_ui = ui;
                let ui = footer.body();
                widgets::scroll_body(ui, "routing_body", |ui| {
                    // One vertical list: headings, the output and send rows.
                    ui.spacing_mut().item_spacing.y = widgets::tokens::SPACE_MD;
                    // Signal order, as on the strip: the main output, then sends
                    // (pre-fader above post-fader).
                    widgets::text::section_heading(ui, "Output");
                    let current = self.project.track(track_id).unwrap().output_bus;
                    let mut output = current;
                    widgets::select(
                        "channel_output",
                        current
                            .and_then(|id| buses.iter().find(|bus| bus.0 == id))
                            .map(|bus| bus.1.as_str())
                            .unwrap_or("Master output"),
                    )
                    .show_ui(ui, |ui| {
                        widgets::selectable_value(ui, &mut output, None, "Master output");
                        for (id, name) in buses.iter().filter(|(id, _)| eligible.contains(id)) {
                            widgets::selectable_value(ui, &mut output, Some(*id), name);
                        }
                    })
                    .response
                    .on_hover_text(
                        "Routes cannot form feedback cycles; all paths are delay compensated",
                    );
                    if output != current {
                        let result = self.project.set_output(track_id, output);
                        if result.is_ok() {
                            self.changed();
                        }
                        self.message(result, "Output routing updated");
                    }
                    ui.add_space((widgets::tokens::SPACE_LG - ui.spacing().item_spacing.y).max(0.0));
                    widgets::text::section_heading(ui, "Sends");
                    let mut remove = None;
                    let mut changed = false;
                    let track = self.project.track_mut(track_id).unwrap();
                    let mut order: Vec<usize> = (0..track.sends.len()).collect();
                    order.sort_by_key(|index| !track.sends[*index].pre_fader);
                    for index in order {
                        let send = &mut track.sends[index];
                        ui.push_id(send.target_bus, |ui| {
                            let parameter = AutomationParameter::SendGain { bus: send.target_bus };
                            let state = automation.get(&send.target_bus).copied();
                            let mut gain = state
                                .filter(|(_, status, _)| *status != daw_ui::AutomationState::Bypassed)
                                .map(|(_, _, value)| parameter.denormalize(value))
                                .unwrap_or(send.gain);
                            let destination = buses
                                .iter()
                                .find(|bus| bus.0 == send.target_bus)
                                .map(|bus| bus.1.as_str())
                                .unwrap_or("Unavailable bus");
                            let name = format!("{track_name} → {destination} · Send level");
                            let control = daw_ui::AutomationControl::Send {
                                track: track_id,
                                bus: send.target_bus,
                            };
                            let lane = state.map(|(lane, status, _)| (lane, status));
                            // The top of the send range in dB, rounded for display (16× is +24.08 dB).
                            let top = (f64::from(daw_ui::channel_gain_to_db(*parameter.range().end())) * 100.0).round() / 100.0;
                            widgets::parameter_row(ui, |ui| {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(SEND_NAME_WIDTH, widgets::tokens::PARAMETER_ROW_HEIGHT),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_min_width(SEND_NAME_WIDTH);
                                        widgets::text::truncated(ui, widgets::text::TextRole::Label, destination)
                                    },
                                );
                                let response = ui
                                    .add(
                                        widgets::knob(&mut gain, parameter.range())
                                            .compact()
                                            .diameter(SEND_KNOB_DIAMETER)
                                            .perceptual_gain()
                                            .pointer()
                                            .clamping(egui::SliderClamping::Edits)
                                            .automation(state.map(|(_, status, _)| status))
                                            .text("Send level"),
                                    )
                                    .on_hover_text(format!(
                                        "{name}: {}\nDrag up/right to increase; down/left to decrease. Shift for fine adjustment. Right-click for exact entry.",
                                        daw_ui::channel_gain_label(gain),
                                    ));
                                let (context_action, context_exact) =
                                    daw_ui::automatable_parameter_value_context_named(
                                        ui,
                                        &response,
                                        control,
                                        lane,
                                        f64::from(daw_ui::channel_gain_to_db(gain)),
                                        -60.0..=top,
                                        "dB",
                                        &name,
                                    );
                                let (action, exact) = daw_ui::automatable_parameter_value_button(
                                    ui,
                                    "send_exact",
                                    send_level_text(gain),
                                    control,
                                    lane,
                                    f64::from(daw_ui::channel_gain_to_db(gain)),
                                    -60.0..=top,
                                    "dB",
                                    &name,
                                );
                                if widgets::selectable(ui, send.pre_fader, "Pre")
                                    .on_hover_text("Send after inserts, before track gain and pan")
                                    .clicked()
                                    && !send.pre_fader
                                {
                                    send.pre_fader = true;
                                    changed = true;
                                }
                                if widgets::selectable(ui, !send.pre_fader, "Post")
                                    .on_hover_text("Send after inserts, track gain, and pan")
                                    .clicked()
                                    && send.pre_fader
                                {
                                    send.pre_fader = false;
                                    changed = true;
                                }
                                daw_ui::action_menu(
                                    ui,
                                    state
                                        .map(|(_, status, _)| status.rich_label(ui.ctx()))
                                        .unwrap_or_else(|| egui::RichText::new("Automation")),
                                    |ui| {
                                        if let Some(action) = daw_ui::automatable_menu_contents(ui, control, lane) {
                                            automation_navigation = Some(action);
                                        }
                                    },
                                );
                                if ui
                                    .add(widgets::IconButton::icon_only(widgets::Icon::Delete, "Remove send").quiet())
                                    .on_hover_text(format!("Remove the send to {destination}"))
                                    .clicked()
                                {
                                    remove = Some(index);
                                }
                                if let Some(action) = action.or(context_action) {
                                    automation_navigation = Some(action);
                                }
                                if let Some(db) = exact.or(context_exact) {
                                    gain = daw_ui::channel_gain_from_db(db as f32);
                                }
                                if response.changed() || exact.or(context_exact).is_some() {
                                    send.gain = gain;
                                    changed = true;
                                    automation_overrides.push((
                                        AutomationTarget { track: track_id, parameter },
                                        parameter.normalize(gain),
                                    ));
                                }
                            });
                        });
                    }
                    if let Some(index) = remove {
                        track.sends.remove(index);
                        changed = true;
                    }
                    let mut add_bus = None;
                    widgets::menu_button(ui, "Add send…", |ui| {
                        let available: Vec<_> = buses
                            .iter()
                            .filter(|(id, _)| {
                                eligible.contains(id)
                                    && !track.sends.iter().any(|send| send.target_bus == *id)
                            })
                            .collect();
                        if available.is_empty() {
                            ui.label("No eligible buses");
                        }
                        for (id, name) in available {
                            if widgets::button(ui, name).clicked() {
                                add_bus = Some(*id);
                                ui.close();
                            }
                        }
                        ui.separator();
                        if widgets::button(ui, "New bus and send").clicked() {
                            create_bus_send = true;
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text("Send to a bus; set its effect to 100% wet for a shared return");
                    if changed {
                        for (target, value) in automation_overrides.drain(..) {
                            self.override_automated_parameter(target, value);
                        }
                        self.changed();
                    }
                    if let Some(bus) = add_bus {
                        let result = self.project.add_send(track_id, bus, 0.25, false);
                        if result.is_ok() {
                            self.changed();
                        }
                        self.message(result, "Send added");
                    }
                });
                footer.show(window_ui, |ui| {
                    widgets::dialog_actions(
                        ui,
                        |ui| {
                            create_send = widgets::button(ui, "New return with effect…")
                                .on_hover_text("Choose an effect for a new bus and send to it")
                                .clicked();
                        },
                        |_| {},
                    );
                });
            });
        if !open {
            self.routing_open = false;
        }
        self.apply_automation_navigation(automation_navigation);
        if create_send {
            self.open_effect_library(track_id, true);
        }
        if create_bus_send {
            self.mixer_create_send(track_id, None);
        }
    }
}

/// Destination names lead each send row; the knob ring sits in a 55 pt row.
const SEND_NAME_WIDTH: f32 = 89.0;
const SEND_KNOB_DIAMETER: f32 = 55.0;

/// The send's level on the console's dB vocabulary ("−12.0 dB", "−∞ dB").
fn send_level_text(gain: f32) -> String {
    if gain > 0.0 {
        format!("{:.1} dB", 20.0 * gain.log10()).replace('-', "−")
    } else {
        "−∞ dB".to_owned()
    }
}

/// Preorder traversal preserves track order within each parent and hides descendants
/// of collapsed folders. The core model validates parent cycles before persistence.
pub fn visible_tracks(project: &Project) -> Vec<(usize, usize)> {
    visible_tracks_with_expanded(project, &std::collections::HashSet::new())
}

/// Presentation-only folder expansion when jumping from a mixer channel.
pub(crate) fn visible_tracks_with_expanded(
    project: &Project,
    expanded: &std::collections::HashSet<u64>,
) -> Vec<(usize, usize)> {
    fn visit(
        project: &Project,
        expanded: &std::collections::HashSet<u64>,
        parent: Option<u64>,
        depth: usize,
        out: &mut Vec<(usize, usize)>,
    ) {
        if depth > 64 {
            return;
        }
        for (index, track) in project.tracks.iter().enumerate() {
            if track.parent_folder == parent && track.kind != TrackKind::Automation {
                out.push((index, depth));
                for (automation_index, automation) in project.tracks.iter().enumerate() {
                    if automation
                        .automation
                        .as_ref()
                        .is_some_and(|lane| lane.target.track == track.id)
                    {
                        out.push((automation_index, depth + 1));
                    }
                }
                if track.kind == TrackKind::Folder
                    && (!track.collapsed || expanded.contains(&track.id))
                {
                    visit(project, expanded, Some(track.id), depth + 1, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    visit(project, expanded, None, 0, &mut out);
    out
}

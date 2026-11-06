//! Compact mixer routing controls. Destination color identifies the receiving
//! channel; compact destination menus keep routine routing beside its source.
use crate::{theme, tokens};
use egui::{Color32, Response, Ui, Vec2};

/// Main output named by its destination only ("Master", "Drum bus"), as a
/// quiet row with the destination's color rail. Long names show on hover.
pub fn mixer_output_button(
    ui: &mut Ui,
    width: f32,
    destination: &str,
    destination_color: Option<Color32>,
) -> Response {
    let colors = theme::palette(ui.ctx());
    crate::mixer_row(
        ui,
        width,
        tokens::CONTROL_HEIGHT,
        destination,
        crate::MixerRowText::Body,
        Some(destination_color.unwrap_or(colors.accent)),
        None,
    )
    .on_hover_text(format!(
        "Main output → {destination}\nAfter inserts, volume and pan · Click to choose an output or create a bus"
    ))
}

/// One parallel path in the compact sends list. Names and colors are borrowed
/// from the project so rendering does not retain or copy a second routing graph.
pub struct MixerSend<'a> {
    pub destination: &'a str,
    pub color: Color32,
    pub gain: f32,
    pub pre_fader: bool,
}

fn send_level(gain: f32) -> String {
    if gain > 0.0 {
        format!("{:.1}", 20.0 * gain.log10()).replace('-', "−")
    } else {
        "−∞".to_owned()
    }
}

/// One compact send: destination rail, name, "PRE" when pre-fader and the
/// level in dB. The whole row is one mouse target.
pub fn mixer_send_row(ui: &mut Ui, width: f32, send: &MixerSend<'_>) -> Response {
    let colors = theme::palette(ui.ctx());
    let level = send_level(send.gain);
    let trailing = if send.pre_fader {
        format!("PRE {level}")
    } else {
        level.clone()
    };
    let placement = if send.pre_fader {
        "Pre-fader"
    } else {
        "Post-fader"
    };
    let response = crate::mixer_row(
        ui,
        width,
        crate::MIXER_SEND_ROW_HEIGHT,
        send.destination,
        crate::MixerRowText::Caption,
        Some(send.color),
        Some((&trailing, colors.muted)),
    );
    crate::hover_text(
        response,
        format!(
            "Send → {} · {placement} · {level} dB\nClick to edit sends",
            send.destination,
        ),
    )
}

/// A summary row for sends beyond the visible rows ("+2 more").
pub fn mixer_more_sends(ui: &mut Ui, width: f32, hidden: usize) -> Response {
    crate::mixer_row(
        ui,
        width,
        crate::MIXER_SEND_ROW_HEIGHT,
        &format!("+{hidden} more"),
        crate::MixerRowText::Placeholder,
        None,
        None,
    )
    .on_hover_text("Click to edit all sends")
}

/// The add-send opener at the end of the sends list.
pub fn mixer_add_send_button(ui: &mut Ui, width: f32) -> Response {
    crate::mixer_row(
        ui,
        width,
        crate::MIXER_SEND_ROW_HEIGHT,
        "Add send…",
        crate::MixerRowText::Action,
        None,
        None,
    )
    .on_hover_text("Add a parallel send to a bus")
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MixerRouteKind {
    Output,
    Send,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MixerRouteChoice {
    Master,
    Bus(u64),
    NewBus,
    Details,
}

pub struct MixerDestination<'a> {
    pub id: u64,
    pub name: &'a str,
}

/// Source-labelled, bounded destination chooser. The application supplies only
/// graph-safe destinations and validates again before committing a route.
pub fn mixer_route_choices(
    ui: &mut Ui,
    source: &str,
    kind: MixerRouteKind,
    current: Option<u64>,
    destinations: &[MixerDestination<'_>],
    details: bool,
) -> Option<MixerRouteChoice> {
    crate::command_menu_contents(ui, |ui| {
        ui.set_max_width(tokens::SEARCH_CHOICE_WIDTH);
        ui.set_min_width(tokens::SELECT_WIDTH);
        crate::text::truncated(
            ui,
            crate::text::TextRole::Label,
            format!(
                "{} {source}",
                if kind == MixerRouteKind::Output {
                    "Output of"
                } else {
                    "Send from"
                }
            ),
        );
        ui.separator();
        let mut choice = None;
        let height = (ui.ctx().content_rect().height() - 233.0).clamp(68.0, 377.0);
        crate::scroll_area(ui, "mixer_route_destinations", [false, true])
            .max_height(height)
            .min_scrolled_height(0.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if kind == MixerRouteKind::Output
                    && crate::selectable(ui, current.is_none(), "Master").clicked()
                {
                    choice = Some(MixerRouteChoice::Master);
                }
                for destination in destinations {
                    let selected =
                        kind == MixerRouteKind::Output && current == Some(destination.id);
                    let response = ui
                        .add_sized(
                            Vec2::new(ui.available_width(), tokens::CONTROL_HEIGHT),
                            crate::button_widget(destination.name)
                                .selected(selected)
                                .truncate(),
                        )
                        .on_hover_text(destination.name);
                    if response.clicked() {
                        choice = Some(MixerRouteChoice::Bus(destination.id));
                    }
                }
                if destinations.is_empty() && kind == MixerRouteKind::Send {
                    crate::text::note(ui, "No eligible buses");
                }
            });
        ui.separator();
        if crate::button(
            ui,
            if kind == MixerRouteKind::Output {
                "New bus and route"
            } else {
                "New bus and send"
            },
        )
        .clicked()
        {
            choice = Some(MixerRouteChoice::NewBus);
        }
        if details
            && crate::view_button(
                ui,
                if kind == MixerRouteKind::Output {
                    "Routing…"
                } else {
                    "Edit sends…"
                },
            )
            .clicked()
        {
            choice = Some(MixerRouteChoice::Details);
        }
        if choice.is_some() {
            ui.close();
        }
        choice
    })
}

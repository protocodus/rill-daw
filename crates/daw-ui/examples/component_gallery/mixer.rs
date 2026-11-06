use daw_ui::{self as ui, text, tokens};
use egui::{Color32, Id, Sense, Ui, vec2};

pub struct State {
    gain: [f32; 3],
    pan: [f32; 3],
    muted: [bool; 3],
    solo: [bool; 3],
    precount: u8,
    click: bool,
    click_gain: f32,
    canvas_gain: f32,
    canvas_pan: f32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            gain: [0.49, 0.30, 0.72],
            pan: [0.0, -0.2, 0.1],
            muted: [false, true, false],
            solo: [false, false, true],
            precount: 1,
            click: true,
            click_gain: 0.35,
            canvas_gain: 0.65,
            canvas_pan: -0.25,
        }
    }
}

pub fn show(ui: &mut Ui, state: &mut State) {
    let available = ui.available_width();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(508.0);
            text::subtitle(ui, "Mixer channels");
            text::note(
                ui,
                "mixer_role_heading · mixer_name_plate_with_grip · mixer_fader",
            );
            ui.add_space(tokens::SPACE_SM);
            let mut endpoints = Vec::new();
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = tokens::SPACE_SM;
                for (index, (name, kind, color, peaks)) in [
                    (
                        "Rhythm",
                        "Audio",
                        Color32::from_rgb(92, 176, 168),
                        [0.48, 0.39],
                    ),
                    (
                        "Music",
                        "Bus",
                        Color32::from_rgb(184, 166, 95),
                        [0.28, 0.34],
                    ),
                    (
                        "Reverb",
                        "Send return",
                        Color32::from_rgb(145, 137, 190),
                        [0.65, 0.57],
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let role = match index {
                        1 => Some(ui::MixerChannelRole::Bus),
                        2 => Some(ui::MixerChannelRole::SendReturn),
                        _ => None,
                    };
                    let strip = ui
                        .vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let offset = if let Some(role) = role {
                                ui::mixer_role_heading(ui, role, 160.0, color);
                                ui::MIXER_ROLE_HEIGHT
                            } else {
                                0.0
                            };
                            ui::mixer_strip(
                                ui,
                                ("gallery_channel", index),
                                vec2(160.0, 560.0 - offset),
                                Some(color),
                                index == 1,
                                |ui| {
                                    let width = ui.available_width();
                                    ui::mixer_name_plate_with_grip(
                                        ui,
                                        if role.is_some() {
                                            ui::MixerNamePlateKind::Bus
                                        } else {
                                            ui::MixerNamePlateKind::Track
                                        },
                                        name,
                                        width,
                                        color,
                                        index as u64,
                                        true,
                                    );
                                    ui.spacing_mut().item_spacing.y = tokens::SPACE_SM;
                                    text::note(ui, format!("Input · {kind}"));
                                    let mut db = ui::channel_gain_to_db(state.gain[index]);
                                    if ui.add(ui::channel_gain_number(&mut db)).changed() {
                                        state.gain[index] = ui::channel_gain_from_db(db);
                                    }
                                    ui::mixer_pan_row(
                                        ui,
                                        &mut state.pan[index],
                                        &mut state.muted[index],
                                        &mut state.solo[index],
                                        width,
                                    );
                                    ui.horizontal(|ui| {
                                        ui::mixer_fader(ui, &mut state.gain[index], 216.0 - offset);
                                        ui::channel_meter(
                                            ui,
                                            Id::new(("gallery_meter", index)),
                                            peaks,
                                            216.0 - offset,
                                            ui::MIXER_FADER_INSET,
                                        );
                                    });
                                    ui::view_button(
                                        ui,
                                        if index == 2 { "Master" } else { "Music bus" },
                                    );
                                },
                            )
                        })
                        .inner;
                    endpoints.push(ui::MixerRouteEndpoint {
                        position: strip.response.rect.center_bottom(),
                        color,
                    });
                }
            });
            ui::mixer_route_lane(
                ui,
                endpoints[0],
                endpoints[1],
                "Output → Music bus",
                ui.clip_rect(),
            );
        });
        ui.add_space(tokens::SPACE_LG);
        ui.vertical(|ui| {
            ui.set_width((available - 532.0).max(300.0));
            text::subtitle(ui, "Recording and timing");
            text::note(ui, "record_button · record_arm_at · input_meter");
            ui.add_space(tokens::SPACE_SM);
            ui.horizontal(|ui| {
                ui::record_button(ui, false, true);
                ui::record_button(ui, true, true);
                let (rect, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                ui::record_arm_at(ui, rect, Id::new("gallery_armed"), true, true);
            });
            ui.add_space(tokens::SPACE_SM);
            text::label(ui, "Example input level");
            ui::input_meter(ui, [0.42, 0.34]);
            ui.add_space(tokens::SPACE_MD);
            ui::recording_timing_controls(
                ui,
                &mut state.precount,
                &mut state.click,
                &mut state.click_gain,
                true,
            );
            ui.add_space(tokens::SPACE_LG);
            text::subtitle(ui, "Canvas-owned channel controls");
            text::note(ui, "channel_controls_at · precise level, pan and peaks");
            ui.add_space(tokens::SPACE_SM);
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
            ui::channel_controls_at(
                ui,
                rect,
                "gallery_channel_controls",
                &mut state.canvas_gain,
                &mut state.canvas_pan,
                [0.36, 0.29],
                false,
            );
            ui.add_space(tokens::SPACE_MD);
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
            ui::paint_recording_region(ui.painter(), rect);
            ui.add_space(tokens::SPACE_SM);
            text::note(
                ui,
                "Static demonstration levels; controls have no audio or recording backend.",
            );
        });
    });
}

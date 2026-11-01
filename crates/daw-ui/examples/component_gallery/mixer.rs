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
                "mixer_fader · mixer_pan · mixer_mute_solo · mixer_level_meter",
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
                        "Warm keys",
                        "MIDI",
                        Color32::from_rgb(184, 166, 95),
                        [0.28, 0.34],
                    ),
                    (
                        "Music",
                        "Bus",
                        Color32::from_rgb(145, 137, 190),
                        [0.65, 0.57],
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let strip = ui::mixer_strip(
                        ui,
                        ("gallery_channel", index),
                        vec2(160.0, 460.0),
                        Some(color),
                        index == 1,
                        |ui| {
                            let width = ui.available_width();
                            if index == 2 {
                                ui::mixer_bus_button(ui, name, width, color);
                            } else {
                                ui::mixer_channel_button(ui, name, width, color);
                            }
                            ui.spacing_mut().item_spacing.y = tokens::SPACE_SM;
                            text::note(ui, format!("Input · {kind}"));
                            let mut db = ui::channel_gain_to_db(state.gain[index]);
                            if ui.add(ui::channel_gain_number(&mut db)).changed() {
                                state.gain[index] = ui::channel_gain_from_db(db);
                            }
                            ui.vertical_centered(|ui| {
                                ui::mixer_fader(ui, &mut state.gain[index], 116.0);
                            });
                            ui::mixer_pan(ui, &mut state.pan[index], width);
                            ui::mixer_mute_solo(
                                ui,
                                &mut state.muted[index],
                                &mut state.solo[index],
                                width,
                            );
                            ui::mixer_level_meter(ui, peaks, width);
                            ui::view_button(ui, if index == 2 { "Master" } else { "Music bus" });
                        },
                    );
                    endpoints.push(ui::MixerRouteEndpoint {
                        position: strip.response.rect.center_bottom(),
                        color,
                    });
                }
            });
            ui::mixer_route_lane(
                ui,
                endpoints[1],
                endpoints[2],
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

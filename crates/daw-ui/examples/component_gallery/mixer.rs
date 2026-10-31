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
            gain: [-6.2, -10.4, -2.8],
            pan: [0.0, -0.2, 0.1],
            muted: [false; 3],
            solo: [false; 3],
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
            text::note(ui, "mixer_strip · mixer_channel_button · stereo_meter");
            ui.add_space(tokens::SPACE_SM);
            let mut endpoints = Vec::new();
            ui.horizontal_top(|ui| {
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
                        vec2(160.0, 382.0),
                        Some(color),
                        index == 1,
                        |ui| {
                            if index == 2 {
                                ui::mixer_bus_button(ui, name, 136.0, color);
                            } else {
                                ui::mixer_channel_button(ui, name, 136.0, color);
                            }
                            text::note(ui, kind);
                            ui.add(ui::channel_gain_number(&mut state.gain[index]));
                            ui.horizontal(|ui| {
                                ui.spacing_mut().slider_width = 176.0;
                                ui.add_sized(
                                    vec2(35.0, 176.0),
                                    ui::slider(&mut state.gain[index], -60.0..=3.5)
                                        .vertical()
                                        .show_value(false),
                                );
                                ui::stereo_meter(ui, peaks, vec2(20.0, 176.0));
                            });
                            ui.add(
                                ui::slider(&mut state.pan[index], -1.0..=1.0)
                                    .text("Pan")
                                    .show_value(false),
                            );
                            ui.horizontal(|ui| {
                                ui::checkbox(ui, &mut state.muted[index], "Mute");
                                ui::checkbox(ui, &mut state.solo[index], "Solo");
                            });
                            ui.add_space(tokens::SPACE_SM);
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

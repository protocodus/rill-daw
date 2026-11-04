use daw_ui::{self as ui, text, tokens};
use egui::{Ui, vec2};

pub fn show(ui: &mut Ui) {
    ui.add_space(tokens::SPACE_LG);
    text::subtitle(ui, "Routing choices");
    text::note(ui, "mixer_route_choices · source-specific output and sends");
    ui.add_space(tokens::SPACE_SM);
    ui.horizontal_top(|ui| {
        for kind in [ui::MixerRouteKind::Output, ui::MixerRouteKind::Send] {
            ui.push_id(
                if kind == ui::MixerRouteKind::Output {
                    "output_choices"
                } else {
                    "send_choices"
                },
                |ui| {
                    ui.allocate_ui_with_layout(
                        vec2(tokens::SEARCH_CHOICE_WIDTH + 2.0 * tokens::SPACE_SM, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui::card(ui.ctx())
                                .inner_margin(tokens::SPACE_SM)
                                .show(ui, |ui| {
                                    let choices = [
                                        ui::MixerDestination {
                                            id: 1,
                                            name: "Music",
                                        },
                                        ui::MixerDestination {
                                            id: 2,
                                            name: "Room reverb — long destination name",
                                        },
                                    ];
                                    ui::mixer_route_choices(
                                        ui,
                                        "Warm keys",
                                        kind,
                                        Some(1),
                                        &choices,
                                        true,
                                    );
                                });
                        },
                    );
                },
            );
        }
    });
}

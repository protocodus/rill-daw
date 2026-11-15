use daw_ui::{self as ui, DeviceBrowserIdentity, DeviceCard, Icon};
use egui::{Id, Ui};

#[derive(Default)]
pub struct State {
    query: String,
    selected: usize,
    favorite: bool,
}

pub fn show(ui: &mut Ui, state: &mut State) {
    ui::device_browser_search(ui, &mut state.query, "Search instruments");
    ui.add_space(ui::tokens::SPACE_MD);
    ui::device_browser_results_heading(ui, 3, "instrument", "instruments", Some("All devices"));
    ui.add_space(ui::tokens::SPACE_SM);
    let names = [
        "Expressive grand piano",
        "Warm analog keys",
        "Studio drum kit",
    ];
    let categories = ["Piano · Acoustic", "Synth · Keys", "Drums · Acoustic"];
    let formats = ["VST3", "Built-in", "CLAP"];
    let width = ui.available_width();
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(width * 0.67, 290.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                for (index, name) in names.iter().enumerate() {
                    let device = DeviceCard {
                        id: Id::new(("browser_gallery", index)),
                        name,
                        producer: "Gallery fixture",
                        preview: None,
                        selected: state.selected == index,
                        favorite: state.favorite && state.selected == index,
                        icon: Icon::Instrument,
                        format: formats[index],
                    };
                    if ui::device_row(ui, &device, categories[index], ui.available_width())
                        .response
                        .clicked()
                    {
                        state.selected = index;
                    }
                }
            },
        );
        ui.add_space(ui::tokens::SPACE_MD);
        ui.vertical(|ui| {
            ui::device_browser_identity(
                ui,
                &DeviceBrowserIdentity {
                    name: names[state.selected],
                    developer: "Gallery fixture",
                    kind_format: &format!("Instrument · {}", formats[state.selected]),
                    categories: categories[state.selected],
                    preview: None,
                    icon: Icon::Instrument,
                },
            );
            ui.add_space(ui::tokens::SPACE_SM);
            ui::favorite_toggle(ui, &mut state.favorite);
            ui::view_button(ui, "Organize");
            ui::view_button(ui, "Device details");
        });
    });
    ui.separator();
    ui::action_row(
        ui,
        |ui| {
            ui::primary_button(ui, "Replace instrument");
            ui::view_button(ui, "More");
            ui::view_button(ui, "Details");
        },
        |ui| {
            ui::device_browser_selection_summary(
                ui,
                names[state.selected],
                "Keys · Current instrument: Warm analog keys",
            );
        },
    );
    ui.add_space(ui::tokens::SPACE_LG);
    ui::text::note(
        ui,
        "Illustrative catalogue data · search and insertion are owned by the application.",
    );
}

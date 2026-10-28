use daw_ui::{Icon, ParameterBrowserState, ParameterItem};
use egui::{Id, Ui};

pub struct State {
    selected: usize,
    browser: ParameterBrowserState,
}

impl Default for State {
    fn default() -> Self {
        Self {
            selected: 0,
            browser: ParameterBrowserState {
                selected: Some(11),
                ..Default::default()
            },
        }
    }
}

pub fn show(ui: &mut Ui, state: &mut State) {
    ui.columns(2, |columns| {
        let left = &mut columns[0];
        daw_ui::text::subtitle(left, "Device cards");
        daw_ui::text::note(left, "device_card · selected and idle states");
        left.add_space(daw_ui::tokens::SPACE_MD);
        let width = (left.available_width() - daw_ui::tokens::SPACE_MD) * 0.5;
        left.horizontal(|ui| {
            for (index, (name, icon)) in [
                ("Sample instrument", Icon::Instrument),
                ("Sample effect", Icon::Effect),
            ]
            .into_iter()
            .enumerate()
            {
                let response = daw_ui::device_card(
                    ui,
                    &daw_ui::DeviceCard {
                        id: Id::new(("gallery_device", index)),
                        name,
                        producer: "Gallery fixture",
                        preview: None,
                        selected: state.selected == index,
                        favorite: false,
                        icon,
                    },
                    width,
                );
                if response.response.clicked() {
                    state.selected = index;
                }
            }
        });
        left.add_space(daw_ui::tokens::SPACE_LG);
        daw_ui::text::body(
            left,
            "Stable previews keep cards aligned while the library loads.",
        );
        daw_ui::text::note(
            left,
            "These sample cards use the built-in preview fallback.",
        );
        left.add_space(daw_ui::tokens::SPACE_LG);
        daw_ui::view_links(
            left,
            0,
            &[
                daw_ui::ViewLink {
                    value: 0,
                    label: "Instrument",
                    enabled: true,
                },
                daw_ui::ViewLink {
                    value: 1,
                    label: "Effects",
                    enabled: true,
                },
                daw_ui::ViewLink {
                    value: 2,
                    label: "Routing",
                    enabled: true,
                },
            ],
        );

        let right = &mut columns[1];
        daw_ui::text::subtitle(right, "Parameter browser");
        daw_ui::text::note(
            right,
            "parameter_browser · searchable controls and metadata",
        );
        right.add_space(daw_ui::tokens::SPACE_MD);
        let parameters = [
            ParameterItem {
                id: 10,
                name: "Cutoff frequency",
                unit: "Hz",
                value: 0.62,
                automated: true,
                last_touched: false,
            },
            ParameterItem {
                id: 11,
                name: "Resonance",
                unit: "",
                value: 0.28,
                automated: false,
                last_touched: true,
            },
            ParameterItem {
                id: 12,
                name: "Attack",
                unit: "ms",
                value: 0.15,
                automated: false,
                last_touched: false,
            },
            ParameterItem {
                id: 13,
                name: "Release",
                unit: "ms",
                value: 0.70,
                automated: true,
                last_touched: false,
            },
        ];
        daw_ui::parameter_browser(
            right,
            "gallery_parameters",
            &mut state.browser,
            &parameters,
            430.0,
            |ui, selected| {
                if let Some(item) = selected {
                    daw_ui::text::subtitle(ui, item.name);
                    daw_ui::text::note(
                        ui,
                        format!("Parameter {} · normalized value {:.2}", item.id, item.value),
                    );
                    ui.add_space(daw_ui::tokens::SPACE_MD);
                    daw_ui::button(ui, "Automate parameter");
                }
            },
        );
    });
}

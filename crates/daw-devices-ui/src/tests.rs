use super::*;
use daw_devices::{devices::filter, registry};
use std::collections::HashMap;

/// A plain value table, as the gallery and tests use instead of an engine.
#[derive(Default)]
struct TableHost {
    values: HashMap<u32, f64>,
    defaults: HashMap<u32, f64>,
    batches: usize,
    saved: Vec<Preset>,
    navigation: Vec<AutomationNavigation>,
}

impl TableHost {
    fn new(descriptor: &DeviceDescriptor) -> Self {
        let defaults: HashMap<_, _> = descriptor
            .parameters
            .iter()
            .map(|spec| (spec.id, f64::from(spec.default_normalized())))
            .collect();
        Self {
            values: defaults.clone(),
            defaults,
            ..Default::default()
        }
    }
}

impl DeviceHost for TableHost {
    fn value(&self, id: u32) -> f64 {
        self.values
            .get(&id)
            .or(self.defaults.get(&id))
            .copied()
            .unwrap_or(0.0)
    }
    fn set_value(&mut self, id: u32, normalized: f64) {
        self.values.insert(id, normalized);
    }
    fn set_values(&mut self, values: &[(u32, f64)]) {
        self.batches += 1;
        self.values.extend(values.iter().copied());
    }
    fn automation(&self, _: u32) -> Option<(u64, AutomationState)> {
        None
    }
    fn navigate(&mut self, action: AutomationNavigation) {
        self.navigation.push(action);
    }
    fn user_presets(&self) -> Vec<Preset> {
        self.saved.clone()
    }
    fn save_preset(&mut self, preset: Preset) {
        self.saved.push(preset);
    }
}

fn render(
    descriptor: &DeviceDescriptor,
    host: &mut TableHost,
    state: &mut PanelState,
    appearance: daw_ui::theme::Appearance,
) -> egui::Rect {
    let ctx = egui::Context::default();
    daw_ui::theme::apply(&ctx);
    ctx.set_theme(appearance.preference());
    let mut used = egui::Rect::NOTHING;
    for _ in 0..2 {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                used = ui
                    .scope(|ui| show(ui, descriptor, host, state))
                    .response
                    .rect;
            });
        });
    }
    used
}

#[test]
fn every_registered_panel_renders_within_its_width() {
    for appearance in [
        daw_ui::theme::Appearance::Light,
        daw_ui::theme::Appearance::Dark,
    ] {
        for descriptor in registry() {
            let mut host = TableHost::new(descriptor);
            let used = render(
                descriptor,
                &mut host,
                &mut PanelState::default(),
                appearance,
            );
            assert!(
                used.width() <= daw_ui::tokens::POPUP_WIDTH_MD + 1.0,
                "{} is {} points wide",
                descriptor.id,
                used.width()
            );
            assert_eq!(host.values, host.defaults, "rendering changed values");
        }
    }
}

#[test]
fn filter_hides_controls_that_do_not_apply() {
    let layout = panels::layout(&filter::DESCRIPTOR);
    let model = |index: f32| {
        let spec = filter::DESCRIPTOR.parameter(filter::MODEL).unwrap();
        let value = f64::from(spec.to_normalized(index));
        move |id: u32| if id == filter::MODEL { value } else { 0.0 }
    };
    let clean = model(0.0);
    assert!((layout.visible)(filter::SLOPE, &clean));
    assert!(!(layout.visible)(filter::MORPH, &clean));
    let vowel = model(8.0);
    assert!((layout.visible)(filter::MORPH, &vowel));
    assert!(!(layout.visible)(filter::TYPE, &vowel));
    let sallen_key = model(2.0);
    assert_eq!(
        (layout.options)(filter::TYPE, &sallen_key).unwrap().len(),
        2
    );
    let pole_mix = model(3.0);
    assert!((layout.visible)(filter::SLOPE, &pole_mix));
    assert_eq!((layout.options)(filter::TYPE, &pole_mix).unwrap().len(), 4);
}

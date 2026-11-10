use super::*;
use daw_devices::{
    devices::{filter, mini},
    registry,
};
use std::collections::HashMap;

/// A plain value table, as the gallery and tests use instead of an engine.
#[derive(Default)]
struct TableHost {
    values: HashMap<u32, f64>,
    defaults: HashMap<u32, f64>,
    batches: usize,
    saved: Vec<Preset>,
    navigation: Vec<AutomationNavigation>,
    auditions: Vec<Option<(u8, f32)>>,
    wheels: Vec<(f32, f32)>,
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
    fn audition(&mut self, note: Option<(u8, f32)>) {
        self.auditions.push(note);
    }
    fn wheels(&mut self, pitch: f32, modulation: f32) {
        self.wheels.push((pitch, modulation));
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
                used.width() <= panel_width(descriptor) + 1.0,
                "{} is {} points wide",
                descriptor.id,
                used.width()
            );
            assert_eq!(host.values, host.defaults, "rendering changed values");
            assert!(host.auditions.is_empty() && host.wheels.is_empty());
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
    let ota_cascade = model(3.0);
    assert!((layout.visible)(filter::SLOPE, &ota_cascade));
    assert_eq!(
        (layout.options)(filter::TYPE, &ota_cascade).unwrap().len(),
        4
    );
}

#[test]
fn mini_uses_the_wide_panel_and_shows_pulse_width_only_for_pulse() {
    assert_eq!(
        panel_width(&mini::DESCRIPTOR),
        daw_ui::tokens::POPUP_WIDTH_LG
    );
    let layout = panels::layout(&mini::DESCRIPTOR);
    let wave = |index: f32| {
        let spec = mini::DESCRIPTOR.parameter(mini::OSC1_WAVE).unwrap();
        let value = f64::from(spec.to_normalized(index));
        move |id: u32| {
            if id == mini::OSC1_WAVE {
                value
            } else {
                f64::from(mini::DESCRIPTOR.parameter(id).unwrap().default_normalized())
            }
        }
    };
    assert!(!(layout.visible)(mini::OSC1_WIDTH, &wave(0.0)));
    assert!((layout.visible)(mini::OSC1_WIDTH, &wave(1.0)));
    assert!((layout.visible)(mini::LFO_WIDTH, &wave(1.0)));
    assert!(!(layout.visible)(mini::OSC2_WIDTH, &wave(1.0)));
    // Every parameter has a section, so every one has a control and a
    // right-click Automate.
    let placed: Vec<u32> = layout
        .sections
        .iter()
        .flat_map(|section| section.parameters.clone())
        .collect();
    assert_eq!(placed.len(), mini::DESCRIPTOR.parameters.len());
}

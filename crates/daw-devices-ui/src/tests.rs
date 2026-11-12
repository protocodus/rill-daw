use super::*;
use daw_devices::{
    devices::{compressor, equalizer, filter, mini, modulator, saturator, spacer},
    registry,
};
use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use std::collections::HashMap;

/// A plain value table, as the gallery and tests use instead of an engine.
#[derive(Default)]
struct TableHost {
    values: HashMap<u32, f64>,
    defaults: HashMap<u32, f64>,
    batches: usize,
    /// The IDs of every `set_values` call and of every `set_value`, in order.
    batch_ids: Vec<Vec<u32>>,
    singles: Vec<u32>,
    telemetry: Option<Arc<Telemetry>>,
    saved: Vec<Preset>,
    navigation: Vec<AutomationNavigation>,
    auditions: Vec<Option<(u8, f32)>>,
    wheels: Vec<(f32, f32)>,
    /// What is routed to the sidechain input, and how often Routing was asked for.
    sources: Option<Vec<String>>,
    routed: usize,
    tempo: Option<f64>,
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
        self.singles.push(id);
        self.values.insert(id, normalized);
    }
    fn set_values(&mut self, values: &[(u32, f64)]) {
        self.batches += 1;
        self.batch_ids
            .push(values.iter().map(|(id, _)| *id).collect());
        self.values.extend(values.iter().copied());
    }
    fn telemetry(&self) -> Option<Arc<Telemetry>> {
        self.telemetry.clone()
    }
    fn tempo(&self) -> Option<f64> {
        self.tempo
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
    fn sidechain_sources(&self) -> Option<Vec<String>> {
        self.sources.clone()
    }
    fn route_sidechain(&mut self) {
        self.routed += 1;
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
        daw_ui::tokens::POPUP_WIDTH_LG + daw_ui::DEVICE_FRAME_INSET
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
        .flat_map(|section| section.ids())
        .collect();
    assert_eq!(placed.len(), mini::DESCRIPTOR.parameters.len());
}

#[test]
fn card_lines_share_columns_and_wrap_only_when_too_wide() {
    let lines = vec![vec![("a", 50.0), ("b", 30.0)], vec![("c", 40.0)]];
    // Columns as wide as their widest control, leftover shared evenly.
    assert_eq!(columns(&lines, 100.0), vec![Some(60.0), Some(40.0)]);
    assert_eq!(columns(&lines, 70.0), vec![None, None]);
    assert_eq!(wrap(&lines, 100.0), lines);
    assert_eq!(
        wrap(&lines, 60.0),
        vec![vec![("a", 50.0)], vec![("b", 30.0)], vec![("c", 40.0)]]
    );
}

#[test]
fn mini_places_every_parameter_once() {
    let layout = panels::layout(&mini::DESCRIPTOR);
    let mut placed: Vec<u32> = layout
        .sections
        .iter()
        .flat_map(|section| section.ids())
        .collect();
    placed.sort_unstable();
    placed.dedup();
    assert_eq!(placed.len(), mini::DESCRIPTOR.parameters.len());
}

/// One frame of a panel in a context that lives across frames, so gestures
/// can span them.
fn frame(
    ctx: &egui::Context,
    events: Vec<Event>,
    descriptor: &DeviceDescriptor,
    host: &mut TableHost,
    state: &mut PanelState,
) -> egui::Rect {
    let mut used = egui::Rect::NOTHING;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 1000.0))),
            time: Some(ctx.cumulative_frame_nr() as f64 * 0.1),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                used = ui
                    .scope(|ui| show(ui, descriptor, host, state))
                    .response
                    .rect;
            });
        },
    );
    used
}

fn button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

struct Panel {
    ctx: egui::Context,
    descriptor: &'static DeviceDescriptor,
    host: TableHost,
    state: PanelState,
}

impl Panel {
    fn new(descriptor: &'static DeviceDescriptor) -> Self {
        let ctx = egui::Context::default();
        daw_ui::theme::apply(&ctx);
        let mut panel = Self {
            ctx,
            descriptor,
            host: TableHost::new(descriptor),
            state: PanelState::default(),
        };
        panel.frame(vec![]);
        panel.frame(vec![]);
        panel
    }

    fn frame(&mut self, events: Vec<Event>) -> egui::Rect {
        frame(
            &self.ctx,
            events,
            self.descriptor,
            &mut self.host,
            &mut self.state,
        )
    }

    /// Hover, then press and release in separate frames, as a real pointer does.
    fn click(&mut self, at: Pos2, times: usize) {
        self.frame(vec![Event::PointerMoved(at)]);
        for _ in 0..times {
            self.frame(vec![button(at, true)]);
            self.frame(vec![button(at, false)]);
        }
        self.frame(vec![]);
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.frame(vec![Event::PointerMoved(from)]);
        self.frame(vec![button(from, true)]);
        for step in 1..=4 {
            self.frame(vec![Event::PointerMoved(from.lerp(to, step as f32 / 4.0))]);
        }
        self.frame(vec![button(to, false)]);
        self.frame(vec![]);
    }

    fn set(&mut self, id: u32, plain: f32) {
        let spec = self.descriptor.parameter(id).unwrap();
        self.host
            .values
            .insert(id, f64::from(spec.to_normalized(plain)));
        self.frame(vec![]);
    }

    fn plain(&self, id: u32) -> f32 {
        let spec = self.descriptor.parameter(id).unwrap();
        spec.to_plain(self.host.value(id) as f32)
    }

    /// Where the response draws a node at this frequency and gain.
    fn node(&self, frequency: f32, gain_db: f32, range_db: f32) -> Pos2 {
        let inner = self.state.response_rect.unwrap().shrink(1.0);
        pos2(
            inner.left() + inner.width() * (frequency / 20.0).ln() / 1000f32.ln(),
            inner.center().y - inner.height() * 0.5 * gain_db / range_db,
        )
    }

    fn cell(&self, id: u32) -> egui::Rect {
        let cell = self
            .state
            .control_rects
            .iter()
            .find(|(cell, _)| *cell == id);
        cell.unwrap().1
    }
}

#[test]
fn equalizer_fits_its_window_in_both_themes() {
    for appearance in [
        daw_ui::theme::Appearance::Light,
        daw_ui::theme::Appearance::Dark,
    ] {
        let mut host = TableHost::new(&equalizer::DESCRIPTOR);
        let used = render(
            &equalizer::DESCRIPTOR,
            &mut host,
            &mut PanelState::default(),
            appearance,
        );
        assert_eq!(
            panel_width(&equalizer::DESCRIPTOR),
            daw_ui::tokens::POPUP_WIDTH_LG + daw_ui::DEVICE_FRAME_INSET
        );
        // What the effect window shows of a panel without scrolling in the
        // app's default 1480 x 960 window is about 634 points.
        assert!(used.height() <= 620.0, "{} points high", used.height());
    }
}

#[test]
fn equalizer_places_every_parameter_once() {
    let layout = panels::layout(&equalizer::DESCRIPTOR);
    let mut placed: Vec<u32> = layout
        .sections
        .iter()
        .flat_map(|section| section.ids())
        .collect();
    placed.sort_unstable();
    let mut all: Vec<u32> = equalizer::PARAMETERS.iter().map(|spec| spec.id).collect();
    all.sort_unstable();
    assert_eq!(placed, all);
}

#[test]
fn equalizer_bands_stand_in_six_equal_columns() {
    use equalizer::*;
    let panel = Panel::new(&DESCRIPTOR);
    let lamps: Vec<egui::Rect> = [HP_ON, 10, 20, 30, 40, LP_ON]
        .into_iter()
        .map(|id| panel.cell(id))
        .collect();
    let column = lamps[1].center().x - lamps[0].center().x;
    assert!((98.0..=99.0).contains(&column), "{column}");
    for pair in lamps.windows(2) {
        let step = pair[1].center().x - pair[0].center().x;
        // Equal but for the snap to the pixel grid.
        assert!((step - column).abs() < 0.05, "{step} {column}");
    }
    // No control leaves its column, and the widest value fits its cell.
    for (index, base) in [0, 10, 20, 30, 40, 50].into_iter().enumerate() {
        let center = lamps[index].center().x;
        for (id, rect) in &panel.state.control_rects {
            if (base..base + 10).contains(id) && (index > 0 || *id < 10) {
                assert!(
                    rect.left() >= center - column * 0.5 - 0.01
                        && rect.right() <= center + column * 0.5 + 0.01,
                    "parameter {id}: {rect:?} outside the column at {center}"
                );
            }
        }
    }
    let ui_width = |text: &str| {
        let mut width = 0.0;
        let _ = panel.ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| width = daw_ui::inline_knob_width(ui, text));
        });
        width
    };
    // With room left, so a value reads with its own dial and not the next.
    assert!(ui_width("20.00 kHz") <= column - 8.0);
    assert!(ui_width("+24.0 dB") <= column - 8.0);
}

#[test]
fn equalizer_notch_disables_gain_in_place() {
    use equalizer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let gain = BANDS[1] + BAND_GAIN;
    let before = (panel.frame(vec![]), panel.cell(gain));
    panel.set(BANDS[1] + BAND_TYPE, 3.0);
    assert_eq!((panel.frame(vec![]), panel.cell(gain)), before);
    let at = panel.cell(gain).center();
    panel.drag(at, at - vec2(0.0, 60.0));
    assert_eq!(panel.plain(gain), 0.0, "a notch has no gain to drag");
    panel.set(BANDS[1] + BAND_TYPE, 0.0);
    panel.drag(at, at - vec2(0.0, 60.0));
    assert!(panel.plain(gain) > 1.0, "a bell's gain drags");
    assert_eq!(panel.state.selected, Some(2), "using a column selects it");
}

#[test]
fn equalizer_node_drag_moves_frequency_and_gain_as_one_gesture() {
    use equalizer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let (frequency, gain) = (BANDS[1] + BAND_FREQUENCY, BANDS[1] + BAND_GAIN);
    let from = panel.node(400.0, 0.0, 12.0);
    panel.drag(from, from + vec2(60.0, -40.0));
    assert!(panel.plain(frequency) > 500.0);
    assert!(panel.plain(gain) > 3.0);
    assert_eq!(panel.state.selected, Some(2));
    // Both parameters in every call: the host makes the drag one Undo step.
    assert!(panel.host.batch_ids.len() >= 3);
    assert!(
        panel
            .host
            .batch_ids
            .iter()
            .all(|ids| ids == &[frequency, gain])
    );
    assert!(panel.host.singles.is_empty());
}

#[test]
fn equalizer_axis_holds_still_while_a_node_is_dragged_past_it() {
    use equalizer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let gain = BANDS[1] + BAND_GAIN;
    let from = panel.node(400.0, 0.0, 12.0);
    panel.frame(vec![Event::PointerMoved(from)]);
    panel.frame(vec![button(from, true)]);
    // The whole height of the plot is 24 dB on the ±12 dB axis.
    for step in 1..=5 {
        panel.frame(vec![Event::PointerMoved(
            from - vec2(0.0, 30.0 * step as f32),
        )]);
    }
    assert!(panel.plain(gain) > 16.0, "{}", panel.plain(gain));
    assert_eq!(panel.state.held_range, Some(12.0));
    panel.frame(vec![button(from - vec2(0.0, 150.0), false)]);
    panel.frame(vec![]);
    assert_eq!(panel.state.held_range, None, "the axis widens on release");
}

#[test]
fn equalizer_cut_and_notch_nodes_move_along_the_frequency_axis_only() {
    use equalizer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let from = panel.node(30.0, 0.0, 12.0);
    panel.drag(from, from + vec2(50.0, -40.0));
    assert!(panel.plain(HP_FREQUENCY) > 40.0);
    assert_eq!(panel.host.singles.last(), Some(&HP_FREQUENCY));
    assert!(panel.host.batch_ids.is_empty());
    assert_eq!(panel.state.selected, Some(0));
}

#[test]
fn equalizer_node_double_click_toggles_and_the_wheel_turns_q_or_steps_slope() {
    use equalizer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let band = panel.node(2500.0, 0.0, 12.0);
    assert_eq!(panel.plain(BANDS[2] + BAND_ON), 1.0);
    panel.click(band, 2);
    assert_eq!(panel.plain(BANDS[2] + BAND_ON), 0.0);
    panel.click(band, 2);
    assert_eq!(panel.plain(BANDS[2] + BAND_ON), 1.0);

    let wheel = |notches: f32| Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, notches),
        modifiers: Modifiers::NONE,
    };
    let q = panel.plain(BANDS[2] + BAND_Q);
    panel.frame(vec![wheel(4.0)]);
    panel.frame(vec![]);
    assert!(panel.plain(BANDS[2] + BAND_Q) > q * 1.5, "four notches up");

    let cut = panel.node(18_000.0, 0.0, 12.0);
    panel.frame(vec![Event::PointerMoved(cut)]);
    panel.frame(vec![]);
    for expected in [1.0, 2.0, 2.0] {
        panel.frame(vec![wheel(1.0)]);
        panel.frame(vec![]);
        assert_eq!(panel.plain(LP_SLOPE), expected);
    }
    panel.frame(vec![wheel(-1.0)]);
    panel.frame(vec![]);
    assert_eq!(panel.plain(LP_SLOPE), 1.0);
    assert_eq!(panel.state.selected, Some(5));
}

#[test]
fn equalizer_table_controls_edit_their_band() {
    use equalizer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    panel.click(panel.cell(LP_ON).center(), 1);
    assert_eq!(panel.plain(LP_ON), 1.0);
    assert_eq!(panel.state.selected, Some(5));
    // The fourth glyph of band 1's Type is the notch.
    let track = panel.cell(BANDS[0] + BAND_TYPE);
    panel.click(pos2(track.right() - 10.0, track.center().y), 1);
    assert_eq!(panel.plain(BANDS[0] + BAND_TYPE), 3.0);
    assert_eq!(panel.state.selected, Some(1));
    let slope = panel.cell(HP_SLOPE);
    panel.click(slope.center(), 1);
    assert_eq!(panel.plain(HP_SLOPE), 1.0);
    // A double-click on a dial is Reset.
    let frequency = panel.cell(BANDS[2] + BAND_FREQUENCY).center();
    panel.drag(frequency, frequency - vec2(0.0, 40.0));
    assert!(panel.plain(BANDS[2] + BAND_FREQUENCY) > 2600.0);
    panel.click(frequency, 2);
    assert!((panel.plain(BANDS[2] + BAND_FREQUENCY) - 2500.0).abs() < 1.0);
}

#[test]
fn equalizer_axis_and_auto_gain_readout_follow_the_values() {
    use equalizer::*;
    let layout = panels::layout(&DESCRIPTOR);
    let view = layout.response().unwrap();
    let mut host = TableHost::new(&DESCRIPTOR);
    let set = |host: &mut TableHost, id: u32, plain: f32| {
        let spec = DESCRIPTOR.parameter(id).unwrap();
        host.values.insert(id, f64::from(spec.to_normalized(plain)));
    };
    assert_eq!((view.range_db)(&|id| host.value(id)), 12.0);
    // Nothing is applied while Auto gain is off: its value line is empty.
    assert_eq!((layout.readout)(AUTO_GAIN, &|id| host.value(id)), None);
    set(&mut host, AUTO_GAIN, 1.0);
    assert_eq!(
        (layout.readout)(AUTO_GAIN, &|id| host.value(id)).as_deref(),
        Some("+0.0 dB")
    );
    set(&mut host, BANDS[1] + BAND_GAIN, 6.0);
    set(&mut host, AUTO_GAIN, 1.0);
    assert_eq!(
        (layout.readout)(AUTO_GAIN, &|id| host.value(id)).as_deref(),
        Some("-1.6 dB")
    );
    assert_eq!((layout.readout)(OUTPUT, &|id| host.value(id)), None);
    set(&mut host, BANDS[1] + BAND_GAIN, 18.0);
    assert_eq!((view.range_db)(&|id| host.value(id)), 24.0);
    // A band that is off does not widen the axis; nor does a notch.
    set(&mut host, BANDS[1] + BAND_ON, 0.0);
    assert_eq!((view.range_db)(&|id| host.value(id)), 12.0);
    let readout = view.readout.unwrap();
    assert_eq!(readout(0, &|id| host.value(id)), "30 Hz   12 dB/oct");
    set(&mut host, BANDS[1] + BAND_TYPE, 3.0);
    assert!(!readout(2, &|id| host.value(id)).contains("dB"));
}

#[test]
fn equalizer_spectrum_needs_telemetry_and_a_source() {
    use equalizer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    assert!(
        panel.state.spectrum.analyzer.is_none(),
        "nothing to analyze"
    );
    let telemetry = Arc::new(Telemetry::default());
    telemetry.set_value(TELEMETRY_SAMPLE_RATE, 96_000.0);
    let block = vec![0.25; 512];
    telemetry.record_output(&block, &block);
    panel.host.telemetry = Some(telemetry);
    panel.set(ANALYZER, 0.0);
    assert!(panel.state.spectrum.analyzer.is_none(), "Analyzer is Off");
    panel.set(ANALYZER, 2.0);
    // At the device's own rate.
    let (rate, analyzer) = panel.state.spectrum.analyzer.as_ref().unwrap();
    assert_eq!((*rate, analyzer.size()), (96_000.0, 16_384));
    assert_eq!(panel.host.values.len(), DESCRIPTOR.parameters.len());
}

#[test]
fn filter_response_keeps_its_one_node_on_the_cutoff() {
    use filter::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let cutoff = panel.plain(CUTOFF);
    let from = panel.node(cutoff, 0.0, 24.0);
    panel.drag(from, from + vec2(80.0, -30.0));
    assert!(panel.plain(CUTOFF) > cutoff * 1.5);
    assert!(panel.host.singles.iter().all(|id| *id == CUTOFF));
    assert!(panel.host.batch_ids.is_empty());
    // No band to switch or reset: a double-click and the wheel do nothing.
    let at = panel.node(panel.plain(CUTOFF), 0.0, 24.0);
    let values = panel.host.values.clone();
    panel.click(at, 2);
    panel.frame(vec![Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, 3.0),
        modifiers: Modifiers::NONE,
    }]);
    assert_eq!(panel.host.values, values);
}

fn placed(descriptor: &DeviceDescriptor) -> Vec<u32> {
    let mut placed: Vec<u32> = panels::layout(descriptor)
        .sections
        .iter()
        .flat_map(|section| section.ids())
        .collect();
    placed.sort_unstable();
    placed
}

fn wheel(notches: f32) -> Event {
    Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, notches),
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn saturator_and_compressor_fit_their_window_in_both_themes() {
    for descriptor in [&saturator::DESCRIPTOR, &compressor::DESCRIPTOR] {
        assert_eq!(
            panel_width(descriptor),
            daw_ui::tokens::POPUP_WIDTH_LG + daw_ui::DEVICE_FRAME_INSET
        );
        for appearance in [
            daw_ui::theme::Appearance::Light,
            daw_ui::theme::Appearance::Dark,
        ] {
            let mut host = TableHost::new(descriptor);
            let used = render(
                descriptor,
                &mut host,
                &mut PanelState::default(),
                appearance,
            );
            // As the Equalizer: what the effect window shows without scrolling.
            assert!(
                used.height() <= 620.0,
                "{} is {} points high",
                descriptor.id,
                used.height()
            );
        }
        let mut all: Vec<u32> = descriptor.parameters.iter().map(|spec| spec.id).collect();
        all.sort_unstable();
        assert_eq!(placed(descriptor), all, "every parameter once");
    }
}

#[test]
fn saturator_picture_fills_the_width_and_only_shows() {
    use saturator::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let [curve, bars] = panel.state.picture_rects[..] else {
        panic!("a curve and the harmonics");
    };
    assert_eq!(curve.size(), vec2(233.0, 233.0));
    assert_eq!(bars.height(), 233.0);
    assert!(bars.left() > curve.right());
    let cards = panel.cell(OUTPUT).right();
    assert!(bars.right() >= cards, "the picture is as wide as the cards");
    // Nothing to grab: a drag, the wheel and a double-click leave the values.
    let values = panel.host.values.clone();
    panel.drag(curve.center(), curve.center() + vec2(40.0, -40.0));
    panel.click(bars.center(), 2);
    panel.frame(vec![wheel(3.0)]);
    assert_eq!(panel.host.values, values);
}

#[test]
fn saturator_character_is_named_for_the_model_without_moving_a_control() {
    use saturator::*;
    let layout = panels::layout(&DESCRIPTOR);
    let mut panel = Panel::new(&DESCRIPTOR);
    let spec = DESCRIPTOR.parameter(CHARACTER).unwrap();
    let before = panel.state.control_rects.clone();
    for (model, name) in [(0.0, "Hardness"), (1.0, "Bias"), (2.0, "Compress")] {
        panel.set(MODEL, model);
        assert_eq!(caption(&DESCRIPTOR, spec, &panel.host, &layout), name);
        assert_eq!(panel.state.control_rects, before);
    }
}

#[test]
fn saturator_readouts_say_what_the_device_does() {
    use saturator::*;
    let layout = panels::layout(&DESCRIPTOR);
    let mut host = TableHost::new(&DESCRIPTOR);
    let set = |host: &mut TableHost, id: u32, plain: f32| {
        let spec = DESCRIPTOR.parameter(id).unwrap();
        host.values.insert(id, f64::from(spec.to_normalized(plain)));
    };
    let readout = |host: &TableHost, id: u32| (layout.readout)(id, &|id| host.value(id));
    set(&mut host, FOCUS, 0.0);
    assert_eq!(readout(&host, FOCUS).as_deref(), Some("Off"));
    set(&mut host, FOCUS, 50.0);
    assert_eq!(readout(&host, FOCUS), None);
    assert_eq!(readout(&host, DRIVE), None);
    // Nothing is applied while Auto gain is off.
    set(&mut host, AUTO_GAIN, 0.0);
    assert_eq!(readout(&host, AUTO_GAIN), None);
    set(&mut host, AUTO_GAIN, 1.0);
    // More drive, more to take back.
    let gain = |host: &TableHost| -> f32 {
        let text = readout(host, AUTO_GAIN).unwrap();
        text.trim_end_matches(" dB").parse().unwrap()
    };
    set(&mut host, DRIVE, 6.0);
    let low = gain(&host);
    set(&mut host, DRIVE, 30.0);
    assert!(gain(&host) < low - 6.0, "{} {low}", gain(&host));
}

#[test]
fn compressor_controls_share_columns_and_keep_their_places_in_every_mode() {
    use compressor::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let used = panel.frame(vec![]);
    let before = panel.state.control_rects.clone();
    // The one-cell controls of a line are one cell apart.
    let line = [THRESHOLD, RATIO, KNEE, CEILING].map(|id| panel.cell(id));
    let cell = line[1].center().x - line[0].center().x;
    for pair in line.windows(2) {
        assert!((pair[1].center().x - pair[0].center().x - cell).abs() < 0.6);
        assert_eq!(pair[0].top(), pair[1].top());
    }
    for (upper, lower) in [
        (THRESHOLD, ATTACK),
        (RATIO, RELEASE),
        (KNEE, AUTO_RELEASE),
        (CEILING, LOOKAHEAD),
        (MAKEUP, MIX),
        (AUTO_MAKEUP, OUTPUT),
    ] {
        let (upper, lower) = (panel.cell(upper), panel.cell(lower));
        assert!((upper.center().x - lower.center().x).abs() < 0.6);
        assert!(lower.top() >= upper.bottom());
    }
    // The two narrow cards are cut alike.
    let narrow =
        |first: u32, second: u32| panel.cell(second).center().x - panel.cell(first).center().x;
    assert!((narrow(SIDECHAIN_HIGH_PASS, STEREO_LINK) - narrow(MAKEUP, AUTO_MAKEUP)).abs() < 0.6);
    // Both lines start level in all four cards.
    for line in [
        [MODE, THRESHOLD, SIDECHAIN, MAKEUP],
        [CHARACTER, ATTACK, SIDECHAIN_HIGH_PASS, MIX],
    ] {
        let top = panel.cell(line[0]).top();
        assert!(line.iter().all(|id| panel.cell(*id).top() == top));
    }
    for mode in [1.0, 2.0, 0.0] {
        panel.set(MODE, mode);
        assert_eq!(panel.frame(vec![]), used, "mode {mode}");
        assert_eq!(panel.state.control_rects, before, "mode {mode}");
        let settings = panels::compressor_settings(&|id| panel.host.value(id));
        let mut off: Vec<u32> = PARAMETERS
            .iter()
            .map(|spec| spec.id)
            .filter(|id| !applies(*id, &settings))
            .collect();
        off.sort_unstable();
        let mut disabled = panel.state.disabled.clone();
        disabled.sort_unstable();
        assert_eq!(disabled, off, "mode {mode}");
    }
}

#[test]
fn compressor_disabled_controls_do_not_move() {
    use compressor::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let ceiling = panel.cell(CEILING).center();
    let before = panel.plain(CEILING);
    panel.drag(ceiling, ceiling - vec2(0.0, 60.0));
    assert_eq!(panel.plain(CEILING), before, "no ceiling on a compressor");
    panel.set(MODE, 1.0);
    panel.drag(ceiling, ceiling + vec2(0.0, 60.0));
    assert!(panel.plain(CEILING) < before, "a limiter's ceiling drags");
}

#[test]
fn compressor_readouts_say_what_the_device_uses() {
    use compressor::*;
    let layout = panels::layout(&DESCRIPTOR);
    let mut host = TableHost::new(&DESCRIPTOR);
    let set = |host: &mut TableHost, id: u32, plain: f32| {
        let spec = DESCRIPTOR.parameter(id).unwrap();
        host.values.insert(id, f64::from(spec.to_normalized(plain)));
    };
    let readout = |host: &TableHost, id: u32| (layout.readout)(id, &|id| host.value(id));
    let own = |id: u32, plain: f32| {
        let spec = DESCRIPTOR.parameter(id).unwrap();
        Some(spec.format(spec.to_normalized(plain)))
    };
    // Digital at its defaults: every knob shows its own value.
    for id in [THRESHOLD, RATIO, KNEE, ATTACK, RELEASE, CEILING] {
        assert_eq!(readout(&host, id), None, "parameter {id}");
    }
    assert_eq!(readout(&host, LOOKAHEAD).as_deref(), Some("Off"));
    assert_eq!(readout(&host, SIDECHAIN_HIGH_PASS).as_deref(), Some("Off"));
    // Auto makeup is off by default: nothing is applied, nothing is shown.
    assert_eq!(readout(&host, AUTO_MAKEUP), None);
    set(&mut host, AUTO_MAKEUP, 1.0);
    assert!(readout(&host, AUTO_MAKEUP).unwrap().ends_with(" dB"));
    set(&mut host, AUTO_RELEASE, 1.0);
    assert_eq!(readout(&host, RELEASE).as_deref(), Some("Auto"));
    // Opto cannot follow faster than 10 ms; Tube has no hard knee.
    set(&mut host, ATTACK, 1.0);
    set(&mut host, KNEE, 0.0);
    set(&mut host, CHARACTER, 2.0);
    assert_eq!(readout(&host, ATTACK), own(ATTACK, 10.0));
    assert_eq!(readout(&host, KNEE), None);
    set(&mut host, CHARACTER, 1.0);
    assert_eq!(readout(&host, ATTACK), None);
    assert_eq!(readout(&host, KNEE), own(KNEE, 12.0));
    // A limiter always looks ahead, and has no ratio to choose.
    set(&mut host, MODE, 1.0);
    assert_eq!(readout(&host, RATIO).as_deref(), Some("∞:1"));
    assert_eq!(readout(&host, LOOKAHEAD), own(LOOKAHEAD, 5.0));
    assert_eq!(readout(&host, AUTO_MAKEUP), None);
    set(&mut host, LOOKAHEAD, 2.0);
    assert_eq!(readout(&host, LOOKAHEAD), None);
    let makeup = DESCRIPTOR.parameter(MAKEUP).unwrap();
    assert_eq!(caption(&DESCRIPTOR, makeup, &host, &layout), "Gain");
    set(&mut host, MODE, 2.0);
    assert_eq!(caption(&DESCRIPTOR, makeup, &host, &layout), "Makeup");
}

/// Where the Compressor's curve draws its handle for a level in dB.
fn handle(panel: &Panel, db: f32) -> Pos2 {
    let inner = panel.state.picture_rects[0].shrink(1.0);
    let fraction = (db + 60.0) / 60.0;
    pos2(
        inner.left() + inner.width() * fraction,
        inner.bottom() - inner.height() * fraction,
    )
}

#[test]
fn compressor_handle_sets_threshold_and_the_wheel_ratio() {
    use compressor::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let [curve, history, meter] = panel.state.picture_rects[..] else {
        panic!("a curve, the history and the meter");
    };
    assert_eq!(curve.size(), vec2(208.0, 208.0));
    assert_eq!((history.height(), meter.size()), (208.0, vec2(48.0, 208.0)));
    assert!(history.left() > curve.right() && meter.left() > history.right());
    assert!(meter.right() >= panel.cell(OUTPUT).right());

    let from = handle(&panel, -18.0);
    panel.drag(from, from + vec2(-40.0, 0.0));
    let threshold = panel.plain(THRESHOLD);
    assert!((-31.5..=-28.5).contains(&threshold), "{threshold}");
    assert!(panel.host.singles.iter().all(|id| *id == THRESHOLD));
    assert!(panel.host.batch_ids.is_empty());

    // The pointer rests on the handle: the wheel turns Ratio.
    let ratio = panel.plain(RATIO);
    panel.frame(vec![Event::PointerMoved(handle(&panel, threshold))]);
    panel.frame(vec![]);
    panel.frame(vec![wheel(4.0)]);
    panel.frame(vec![]);
    assert!(panel.plain(RATIO) > ratio * 1.2, "{}", panel.plain(RATIO));
    assert_eq!(panel.plain(THRESHOLD), threshold);

    panel.click(handle(&panel, threshold), 2);
    assert_eq!(panel.plain(THRESHOLD), -18.0, "a double-click resets");
}

#[test]
fn compressor_handle_is_the_ceiling_in_front_of_a_limiter() {
    use compressor::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    panel.set(MODE, 1.0);
    let ceiling = panel.plain(CEILING);
    let threshold = panel.plain(THRESHOLD);
    let ratio = panel.plain(RATIO);
    let from = handle(&panel, ceiling);
    panel.drag(from, from + vec2(-30.0, 0.0));
    assert!(
        panel.plain(CEILING) < ceiling - 5.0,
        "{}",
        panel.plain(CEILING)
    );
    panel.frame(vec![Event::PointerMoved(handle(
        &panel,
        panel.plain(CEILING),
    ))]);
    panel.frame(vec![]);
    panel.frame(vec![wheel(4.0)]);
    panel.frame(vec![]);
    assert_eq!(
        (panel.plain(THRESHOLD), panel.plain(RATIO)),
        (threshold, ratio)
    );
    assert!(panel.host.singles.iter().all(|id| *id == CEILING));
}

#[test]
fn compressor_history_line_drags_the_threshold_and_a_click_clears_the_hold() {
    use compressor::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let history = panel.state.picture_rects[1].shrink(1.0);
    // The line stands at the height of the curve's handle.
    let at = pos2(history.center().x, handle(&panel, -18.0).y);
    panel.drag(at, at + vec2(0.0, 35.0));
    let threshold = panel.plain(THRESHOLD);
    assert!((-29.0..=-27.0).contains(&threshold), "{threshold}");
    assert!(panel.host.singles.iter().all(|id| *id == THRESHOLD));

    panel.state.reduction_hold = 7.5;
    panel.frame(vec![]);
    assert_eq!(panel.state.reduction_hold, 7.5, "held until clicked");
    let meter = panel.state.picture_rects[2];
    panel.click(meter.center(), 1);
    assert_eq!(panel.state.reduction_hold, 0.0);
}

#[test]
fn compressor_meter_holds_the_largest_reduction_while_the_device_plays() {
    use compressor::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let telemetry = Arc::new(Telemetry::default());
    panel.host.telemetry = Some(telemetry.clone());
    let block = vec![0.25; 512];
    for reduction in [3.0, 9.0, 4.0] {
        telemetry.set_value(TELEMETRY_REDUCTION, reduction);
        telemetry.record_output(&block, &block);
        panel.frame(vec![]);
    }
    assert_eq!(panel.state.reduction_hold, 9.0);
    assert_eq!(
        panel.host.values, panel.host.defaults,
        "showing edits nothing"
    );
}

#[test]
fn compressor_external_sidechain_names_its_source_and_opens_routing() {
    use compressor::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    panel.host.sources = Some(Vec::new());
    // The value line of the Sidechain cell.
    let link = |panel: &Panel| {
        let cell = panel.cell(SIDECHAIN);
        pos2(cell.center().x, cell.bottom() - 8.0)
    };
    // Internal: nothing to route.
    panel.click(link(&panel), 1);
    assert_eq!(panel.host.routed, 0);
    let before = panel.state.control_rects.clone();
    panel.set(SIDECHAIN, 1.0);
    assert_eq!(
        panel.state.control_rects, before,
        "the line was there already"
    );
    panel.click(link(&panel), 1);
    assert_eq!(panel.host.routed, 1, "\"No source\" opens Routing");
    panel.host.sources = Some(vec!["Kick".into(), "Snare".into()]);
    panel.click(link(&panel), 1);
    assert_eq!(panel.host.routed, 2);
    assert_eq!(panel.plain(SIDECHAIN), 1.0);
    // A limiter listens to its own input only.
    panel.set(MODE, 1.0);
    panel.click(link(&panel), 1);
    assert_eq!(panel.host.routed, 2);
}

/// Where every control stands, whichever parameter a place shows.
fn places(panel: &Panel) -> Vec<[i32; 4]> {
    let mut places: Vec<[i32; 4]> = panel
        .state
        .control_rects
        .iter()
        .map(|(_, rect)| {
            [rect.left(), rect.top(), rect.right(), rect.bottom()].map(|edge| edge.round() as i32)
        })
        .collect();
    places.sort_unstable();
    places
}

fn disabled(panel: &Panel) -> Vec<u32> {
    let mut disabled = panel.state.disabled.clone();
    disabled.sort_unstable();
    disabled
}

fn key(key: egui::Key) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn modulator_and_spacer_fit_their_window_in_both_themes() {
    for descriptor in [&modulator::DESCRIPTOR, &spacer::DESCRIPTOR] {
        assert_eq!(
            panel_width(descriptor),
            daw_ui::tokens::POPUP_WIDTH_LG + daw_ui::DEVICE_FRAME_INSET
        );
        for appearance in [
            daw_ui::theme::Appearance::Light,
            daw_ui::theme::Appearance::Dark,
        ] {
            let mut host = TableHost::new(descriptor);
            let used = render(
                descriptor,
                &mut host,
                &mut PanelState::default(),
                appearance,
            );
            // As the Equalizer: what the effect window shows without scrolling.
            assert!(
                used.height() <= 620.0,
                "{} is {} points high",
                descriptor.id,
                used.height()
            );
        }
        let mut all: Vec<u32> = descriptor.parameters.iter().map(|spec| spec.id).collect();
        all.sort_unstable();
        assert_eq!(placed(descriptor), all, "every parameter once");
    }
}

#[test]
fn modulator_controls_keep_their_places_in_every_mode() {
    use modulator::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let used = panel.frame(vec![]);
    let before = places(&panel);
    // Thirteen places for seventeen parameters, none of them empty.
    assert_eq!(before.len(), 13);
    for (mode, sync) in [(1.0, 0.0), (2.0, 1.0), (0.0, 1.0), (1.0, 1.0), (2.0, 0.0)] {
        panel.set(MODE, mode);
        panel.set(SYNC, sync);
        assert_eq!(panel.frame(vec![]), used, "mode {mode} sync {sync}");
        assert_eq!(places(&panel), before, "mode {mode} sync {sync}");
        let shown: Vec<u32> = panel
            .state
            .control_rects
            .iter()
            .map(|(id, _)| *id)
            .collect();
        let center = [CENTER_CHORUS, CENTER_FLANGER, CENTER_PHASER][mode as usize];
        assert!(shown.contains(&center));
        assert!(shown.contains(if sync > 0.5 { &DIVISION } else { &RATE }));
        // A flanger has neither voices nor stages: Voices stays, disabled.
        let (count, off) = match mode as usize {
            0 => (VOICES, vec![]),
            1 => (VOICES, vec![VOICES]),
            _ => (STAGES, vec![]),
        };
        assert!(shown.contains(&count));
        assert_eq!(disabled(&panel), off, "mode {mode}");
    }
    // The lines start level in all four cards, and a swapped place is the
    // same place.
    panel.set(MODE, 0.0);
    panel.set(SYNC, 0.0);
    for line in [
        [MODE, SHAPE, CENTER_CHORUS, MIX],
        [CHARACTER, RATE, VOICES, OUTPUT],
    ] {
        let top = panel.cell(line[0]).top();
        assert!(line.iter().all(|id| panel.cell(*id).top() == top));
    }
    let voices = panel.cell(VOICES);
    panel.set(MODE, 2.0);
    assert_eq!(panel.cell(STAGES), voices);
}

#[test]
fn modulator_mode_click_sends_the_mode_defaults_as_one_edit() {
    use modulator::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    panel.set(FEEDBACK, -30.0);
    panel.set(CENTER_FLANGER, 6.0);
    // The middle one of the three segments.
    let mode = panel.cell(MODE).center() + vec2(0.0, 6.0);
    panel.click(mode, 1);
    assert_eq!(panel.plain(MODE), 1.0);
    assert_eq!(
        panel.host.batch_ids,
        vec![vec![MODE, FEEDBACK, CENTER_FLANGER]],
        "one call: one Undo step"
    );
    assert!(panel.host.singles.is_empty());
    assert_eq!(panel.plain(FEEDBACK), 50.0);
    assert!((panel.plain(CENTER_FLANGER) - 1.5).abs() < 0.01);
    panel.click(mode + vec2(70.0, 0.0), 1);
    assert_eq!(panel.plain(MODE), 2.0);
    assert_eq!(
        panel.host.batch_ids.last().unwrap(),
        &vec![MODE, FEEDBACK, CENTER_PHASER, STAGES]
    );
    // Any other choice is a plain edit.
    let character = panel.cell(CHARACTER).center() + vec2(20.0, 6.0);
    panel.click(character, 1);
    assert_eq!(panel.plain(CHARACTER), 1.0);
    assert_eq!(panel.host.singles, vec![CHARACTER]);
    // A value that arrives any other way brings nothing along.
    panel.set(FEEDBACK, 10.0);
    panel.set(MODE, 0.0);
    assert!((panel.plain(FEEDBACK) - 10.0).abs() < 1e-3);
}

#[test]
fn modulator_division_is_a_detented_knob_whose_value_lists_the_options() {
    use modulator::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    panel.set(SYNC, 1.0);
    assert_eq!(panel.plain(DIVISION), 3.0);
    let cell = panel.cell(DIVISION);
    assert_eq!(cell, panel.cell(DEPTH).translate(vec2(-cell.width(), 0.0)));
    panel.drag(cell.center(), cell.center() - vec2(0.0, 30.0));
    let division = panel.plain(DIVISION);
    assert!(division > 3.0 && division.fract() == 0.0, "{division}");
    assert!(panel.host.singles.iter().all(|id| *id == DIVISION));
    panel.click(cell.center(), 2);
    assert_eq!(panel.plain(DIVISION), 3.0, "a double-click resets");
    // The value under the knob opens the list.
    assert!(!egui::Popup::is_any_open(&panel.ctx));
    panel.click(pos2(cell.center().x, cell.bottom() - 8.0), 1);
    assert!(egui::Popup::is_any_open(&panel.ctx));
}

#[test]
fn modulator_readouts_word_the_values() {
    use modulator::*;
    let layout = panels::layout(&DESCRIPTOR);
    let mut host = TableHost::new(&DESCRIPTOR);
    let set = |host: &mut TableHost, id: u32, plain: f32| {
        let spec = DESCRIPTOR.parameter(id).unwrap();
        host.values.insert(id, f64::from(spec.to_normalized(plain)));
    };
    let readout = |host: &TableHost, id: u32| (layout.readout)(id, &|id| host.value(id));
    assert_eq!(readout(&host, LOW_CUT).as_deref(), Some("Off"));
    assert_eq!(readout(&host, FEEDBACK).as_deref(), Some("+0 %"));
    assert_eq!(readout(&host, STEREO).as_deref(), Some("90°"));
    assert_eq!(readout(&host, VOICES).as_deref(), Some("2 voices"));
    assert_eq!(readout(&host, STAGES).as_deref(), Some("4 stages"));
    assert_eq!(readout(&host, RATE), None);
    set(&mut host, LOW_CUT, 120.0);
    set(&mut host, FEEDBACK, -70.0);
    set(&mut host, VOICES, 1.0);
    assert_eq!(readout(&host, LOW_CUT), None);
    assert_eq!(readout(&host, FEEDBACK).as_deref(), Some("-70 %"));
    assert_eq!(readout(&host, VOICES).as_deref(), Some("1 voice"));
    // Another wording of the same value, which the parameter reads back.
    let feedback = DESCRIPTOR.parameter(FEEDBACK).unwrap();
    assert_eq!(feedback.parse("-70 %"), Some(host.value(FEEDBACK) as f32));
    let spec = |id: u32| DESCRIPTOR.parameter(id).unwrap();
    for (id, name) in [
        (CENTER_CHORUS, "Center"),
        (CENTER_FLANGER, "Center"),
        (CENTER_PHASER, "Center"),
        (LOW_CUT, "Low cut"),
        (FEEDBACK, "Feedback"),
    ] {
        assert_eq!(caption(&DESCRIPTOR, spec(id), &host, &layout), name);
    }
}

#[test]
fn modulator_lfo_drag_turns_rate_or_steps_division() {
    use modulator::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let [lfo, plot] = panel.state.picture_rects[..] else {
        panic!("the LFO and the response");
    };
    assert_eq!(lfo.size(), vec2(208.0, 208.0));
    assert_eq!(plot.height(), 208.0);
    assert!(plot.left() > lfo.right());
    assert!(plot.right() >= panel.cell(OUTPUT).right());

    let rate = panel.plain(RATE);
    let from = lfo.center();
    panel.drag(from, from + vec2(60.0, 30.0));
    assert!(panel.plain(RATE) > rate * 2.0, "{}", panel.plain(RATE));
    assert!(panel.host.singles.iter().all(|id| *id == RATE));
    panel.click(from, 2);
    assert!(
        (panel.plain(RATE) - rate).abs() < 1e-3,
        "a double-click resets"
    );

    panel.set(SYNC, 1.0);
    panel.host.singles.clear();
    panel.drag(from, from + vec2(60.0, 0.0));
    // Sixty points are ten notches: two steps towards shorter divisions.
    assert_eq!(panel.plain(DIVISION), 5.0);
    assert_eq!(panel.host.singles, vec![DIVISION, DIVISION]);
    assert!((panel.plain(RATE) - rate).abs() < 1e-3);
}

/// Where the Modulator's response draws a frequency on its 0 dB line.
fn swept(panel: &Panel, frequency: f32) -> Pos2 {
    let inner = panel.state.picture_rects[1].shrink(1.0);
    pos2(
        inner.left() + inner.width() * (frequency / 20.0).ln() / 1000f32.ln(),
        inner.center().y,
    )
}

#[test]
fn modulator_handle_sets_the_modes_center_and_the_wheel_depth() {
    use modulator::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    // A chorus at 12 ms: the comb's first tooth is at 83 Hz.
    let from = swept(&panel, 1000.0 / 12.0);
    panel.drag(from, from + vec2(40.0, -30.0));
    let center = panel.plain(CENTER_CHORUS);
    assert!((7.0..=9.5).contains(&center), "{center}: right is shorter");
    assert!(panel.host.singles.iter().all(|id| *id == CENTER_CHORUS));
    assert!(panel.host.batch_ids.is_empty());

    let depth = panel.plain(DEPTH);
    panel.frame(vec![Event::PointerMoved(swept(&panel, 1000.0 / center))]);
    panel.frame(vec![]);
    panel.frame(vec![wheel(4.0)]);
    panel.frame(vec![]);
    assert!(
        (panel.plain(DEPTH) - depth - 10.0).abs() < 0.5,
        "four notches"
    );
    assert_eq!(panel.plain(CENTER_CHORUS), center);

    panel.click(swept(&panel, 1000.0 / center), 2);
    assert!((panel.plain(CENTER_CHORUS) - 12.0).abs() < 0.01, "reset");

    // A phaser's handle is the stages' corner itself.
    panel.set(MODE, 2.0);
    let from = swept(&panel, 800.0);
    panel.drag(from, from + vec2(50.0, 0.0));
    assert!(panel.plain(CENTER_PHASER) > 1100.0);
    assert!((panel.plain(CENTER_CHORUS) - 12.0).abs() < 0.01);
}

#[test]
fn modulator_picture_follows_the_playing_device_without_editing() {
    use modulator::*;
    for mode in [0.0, 1.0, 2.0] {
        let mut panel = Panel::new(&DESCRIPTOR);
        panel.set(MODE, mode);
        panel.set(VOICES, 4.0);
        panel.set(FEEDBACK, 80.0);
        let values = panel.host.values.clone();
        let telemetry = Arc::new(Telemetry::default());
        telemetry.set_value(TELEMETRY_SAMPLE_RATE, 44_100.0);
        telemetry.set_value(TELEMETRY_PHASE, 0.7);
        telemetry.set_value(TELEMETRY_SWEEP_LEFT, -0.6);
        telemetry.set_value(TELEMETRY_SWEEP_RIGHT, f32::NAN);
        let block = vec![0.25; 512];
        telemetry.record_output(&block, &block);
        panel.host.telemetry = Some(telemetry);
        let rects = panel.state.picture_rects.clone();
        panel.frame(vec![]);
        assert_eq!(panel.state.picture_rects, rects, "mode {mode}");
        assert_eq!(panel.host.values, values, "showing edits nothing");
    }
}

#[test]
fn spacer_models_stand_on_the_title_lines_and_pick_with_one_click() {
    use spacer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    for (lamp, model, last, labels) in [
        (ECHO_ON, ECHO_MODEL, ECHO_LEVEL, ECHO_MODELS),
        (REVERB_ON, REVERB_MODEL, REVERB_LEVEL, REVERB_MODELS),
    ] {
        let (title, choice) = (panel.cell(lamp), panel.cell(model));
        // On the title's line, clear of the name, flush with the card's
        // last column, above the first line of controls.
        assert!(title.right() + 8.0 < choice.left(), "{title:?} {choice:?}");
        assert!((choice.center().y - title.center().y).abs() < 6.0);
        assert!(choice.right() <= panel.cell(last).right() + 0.5);
        assert!(choice.right() > panel.cell(last).center().x);
        assert!(choice.bottom() < panel.cell(last).top());
        // Three equal buttons, each wide enough for its name.
        let third = choice.width() / 3.0;
        for (index, label) in labels.iter().enumerate() {
            let at = pos2(
                choice.left() + third * (index as f32 + 0.5),
                choice.center().y,
            );
            panel.click(at, 1);
            assert_eq!(panel.plain(model), index as f32, "{label}");
        }
        // A section that is off keeps its model as it is.
        panel.set(lamp, 0.0);
        panel.click(pos2(choice.left() + third * 0.5, choice.center().y), 1);
        assert_eq!(panel.plain(model), 2.0);
    }
    // Digital was on already; Hall was, so the first two clicks moved it.
    assert_eq!(
        panel.host.singles,
        [&[ECHO_MODEL; 2][..], &[REVERB_MODEL; 3][..]].concat()
    );
}

#[test]
fn spacer_routings_stack_in_their_cells_and_pick_with_one_click() {
    use spacer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let routing = panel
        .state
        .grid_rects
        .iter()
        .find(|(id, _)| *id == ROUTING)
        .unwrap()
        .1;
    // Stacked in their own cells, clear of Mix.
    assert!(routing.height() > routing.width() / 3.0, "{routing:?}");
    assert!(
        routing.right() <= panel.cell(MIX).left() + 0.5,
        "{routing:?}"
    );
    let half = routing.height() / 2.0;
    panel.click(pos2(routing.center().x, routing.top() + half * 1.5), 1);
    assert_eq!(panel.plain(ROUTING), 1.0, "Parallel is the lower option");
    panel.click(pos2(routing.center().x, routing.top() + half * 0.5), 1);
    assert_eq!(
        panel.plain(ROUTING),
        0.0,
        "Echo → Reverb is the upper option"
    );
}

#[test]
fn spacer_seed_is_a_stepper_that_fits_its_cell() {
    use spacer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let cell = panel.cell(SEED);
    assert!((cell.width() - panel.cell(REVERB_COLOR).width()).abs() < 0.5);
    assert_eq!(cell.top(), panel.cell(REVERB_COLOR).top());
    assert_eq!(cell.bottom(), panel.cell(REVERB_COLOR).bottom());
    // Previous, die, next: centred in the cell, level with the knobs.
    let track = egui::Rect::from_center_size(cell.center(), daw_ui::VARIATION_STEPPER_SIZE);
    assert!(cell.contains_rect(track));
    let button = |index: f32| {
        pos2(
            track.left() + track.width() * (index + 0.5) / 3.0,
            panel.cell(REVERB_COLOR).center().y,
        )
    };
    let (previous, die, next) = (button(0.0), button(1.0), button(2.0));
    panel.click(next, 2);
    assert_eq!(panel.plain(SEED), 3.0);
    panel.click(previous, 3);
    assert_eq!(panel.plain(SEED), 32.0, "wraps");
    panel.click(die, 1);
    assert_ne!(panel.plain(SEED), 32.0);
    assert!((1.0..=32.0).contains(&panel.plain(SEED)));
    assert!(panel.host.singles.iter().all(|id| *id == SEED));
    // Two quick clicks are two steps, not a knob's double-click Reset.
    panel.set(SEED, 9.0);
    panel.click(previous, 2);
    assert_eq!(panel.plain(SEED), 7.0);
}

#[test]
fn spacer_controls_keep_their_places_whatever_is_switched() {
    use spacer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let used = panel.frame(vec![]);
    let before = places(&panel);
    // Thirty-one parameters in thirty places: Time and Division share one.
    assert_eq!(before.len(), 30);
    assert_eq!(disabled(&panel), Vec::<u32>::new());
    let echo: Vec<u32> = (ECHO_MODEL..=ECHO_DUCK).collect();
    let reverb: Vec<u32> = (REVERB_MODEL..=REVERB_DUCK).collect();
    for (echo_on, reverb_on, sync, model) in [
        (0.0, 1.0, 1.0, 0.0),
        (1.0, 0.0, 0.0, 1.0),
        (0.0, 0.0, 0.0, 2.0),
        (1.0, 1.0, 0.0, 2.0),
        (1.0, 1.0, 1.0, 1.0),
    ] {
        panel.set(ECHO_ON, echo_on);
        panel.set(REVERB_ON, reverb_on);
        panel.set(ECHO_SYNC, sync);
        panel.set(ECHO_MODEL, model);
        panel.set(REVERB_MODEL, model);
        let case = format!("echo {echo_on} reverb {reverb_on} sync {sync} model {model}");
        assert_eq!(panel.frame(vec![]), used, "{case}");
        assert_eq!(places(&panel), before, "{case}");
        // A section that is off stays in place, disabled; Routing needs
        // both. Nothing else ever is: every control applies in every model.
        let mut off: Vec<u32> = Vec::new();
        if echo_on < 0.5 {
            // The cell Time and Division share shows the one in use, also
            // while the echo is off.
            let other = if sync >= 0.5 {
                ECHO_TIME
            } else {
                ECHO_DIVISION
            };
            off.extend(echo.iter().copied().filter(|id| *id != other));
        }
        if reverb_on < 0.5 {
            off.extend(reverb.iter().copied());
        }
        if echo_on < 0.5 || reverb_on < 0.5 {
            off.push(ROUTING);
        }
        off.sort_unstable();
        let mut shown = disabled(&panel);
        shown.sort_unstable();
        assert_eq!(shown, off, "{case}");
    }
    // The picture is as tall as the Output card beside it and shares the
    // edges of the Echo card below.
    let picture = panel.state.picture_rects[0];
    let below = panel.cell(ECHO_ON);
    assert!((picture.left() - below.left()).abs() <= 14.0);
    assert!(picture.right() < panel.cell(ROUTING).left());
    assert!(
        picture.right() > panel.cell(PRE_DELAY).left(),
        "seven columns"
    );
    assert!(picture.bottom() < below.top());
    assert!(picture.height() >= 150.0, "{picture:?}");
    // Both sections start their lines level, and Duck and Level close each.
    for line in [
        [ECHO_SYNC, PRE_DELAY],
        [ECHO_DIFFUSION, SEED],
        [ECHO_DUCK, REVERB_DUCK],
        [ECHO_LEVEL, REVERB_LEVEL],
    ] {
        assert_eq!(panel.cell(line[0]).top(), panel.cell(line[1]).top());
    }
    assert_eq!(panel.cell(ECHO_MODEL).top(), panel.cell(REVERB_MODEL).top());
}

#[test]
fn spacer_title_lamps_switch_their_sections() {
    use spacer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    for id in [ECHO_ON, REVERB_ON] {
        let lamp = panel.cell(id);
        assert_eq!(panel.plain(id), 1.0);
        panel.click(lamp.left_center() + vec2(5.0, 0.0), 1);
        assert_eq!(panel.plain(id), 0.0);
        // The name beside the lamp is part of the switch.
        panel.click(lamp.right_center() - vec2(10.0, 0.0), 1);
        assert_eq!(panel.plain(id), 1.0);
    }
    assert_eq!(
        panel.host.singles,
        vec![ECHO_ON, ECHO_ON, REVERB_ON, REVERB_ON]
    );
    // A control of a section that is off does not move.
    panel.set(REVERB_ON, 0.0);
    let decay = panel.cell(DECAY).center();
    let before = panel.plain(DECAY);
    panel.drag(decay, decay - vec2(0.0, 40.0));
    assert_eq!(panel.plain(DECAY), before);
}

#[test]
fn spacer_readouts_and_hints_say_what_a_value_means_in_its_model() {
    use spacer::*;
    let layout = panels::layout(&DESCRIPTOR);
    let mut host = TableHost::new(&DESCRIPTOR);
    let set = |host: &mut TableHost, id: u32, plain: f32| {
        let spec = DESCRIPTOR.parameter(id).unwrap();
        host.values.insert(id, f64::from(spec.to_normalized(plain)));
    };
    let readout = |host: &TableHost, id: u32| (layout.readout)(id, &|id| host.value(id));
    let hint = |host: &TableHost, id: u32| (layout.hint)(id, &|id| host.value(id));
    for id in [
        ECHO_LEVEL,
        REVERB_LEVEL,
        ECHO_LOW_CUT,
        REVERB_COLOR,
        DECAY,
        MIX,
    ] {
        assert_eq!(readout(&host, id), None, "parameter {id}");
    }
    for id in [ECHO_DUCK, REVERB_DUCK] {
        assert_eq!(readout(&host, id).as_deref(), Some("Off"));
        set(&mut host, id, 40.0);
        assert_eq!(readout(&host, id), None);
    }
    for id in [ECHO_LEVEL, REVERB_LEVEL] {
        set(&mut host, id, -60.0);
        assert_eq!(readout(&host, id).as_deref(), Some("−∞ dB"));
    }
    assert_eq!(readout(&host, SEED).as_deref(), Some("#1"));
    set(&mut host, SEED, 17.0);
    assert_eq!(readout(&host, SEED).as_deref(), Some("#17"));
    set(&mut host, DECAY, 0.35);
    assert_eq!(readout(&host, DECAY).as_deref(), Some("350 ms"));
    // Digital's Color is the corner of its cut: the roadmap's 8 kHz at
    // the centre, none at the right end.
    assert_eq!(readout(&host, ECHO_COLOR).as_deref(), Some("8.0 kHz"));
    set(&mut host, ECHO_COLOR, -100.0);
    assert_eq!(readout(&host, ECHO_COLOR).as_deref(), Some("1.0 kHz"));
    set(&mut host, ECHO_COLOR, 100.0);
    assert_eq!(readout(&host, ECHO_COLOR).as_deref(), Some("Off"));
    set(&mut host, ECHO_LOW_CUT, 20.0);
    set(&mut host, REVERB_LOW_CUT, 20.0);
    for id in [ECHO_LOW_CUT, REVERB_LOW_CUT] {
        assert_eq!(readout(&host, id).as_deref(), Some("Off"), "parameter {id}");
    }
    let digital = [hint(&host, ECHO_COLOR), hint(&host, ECHO_WOBBLE)];
    // A bucket-brigade or tape loop keeps its low cut, and Color is the
    // model's own, in percent.
    for model in [1.0, 2.0] {
        set(&mut host, ECHO_MODEL, model);
        assert_eq!(readout(&host, ECHO_LOW_CUT), None);
        assert_eq!(readout(&host, ECHO_COLOR), None);
        for (id, digital) in [ECHO_COLOR, ECHO_WOBBLE].into_iter().zip(digital) {
            assert!(hint(&host, id).is_some() && hint(&host, id) != digital);
        }
    }
    assert!(hint(&host, REVERB_COLOR).is_some() && hint(&host, SEED).is_some());
    assert_eq!(hint(&host, ECHO_FEEDBACK), None);
    let spec = |id: u32| DESCRIPTOR.parameter(id).unwrap();
    for (id, name) in [
        (ECHO_LEVEL, "Level"),
        (ECHO_COLOR, "Color"),
        (REVERB_COLOR, "Color"),
        (REVERB_DUCK, "Duck"),
        (PRE_DELAY, "Pre-delay"),
        (SEED, "Seed"),
    ] {
        assert_eq!(caption(&DESCRIPTOR, spec(id), &host, &layout), name);
    }
}

/// Where the Spacer's picture draws a time and a level on a span.
fn echo_point(panel: &Panel, span: f32, seconds: f32, db: f32, below: bool) -> Pos2 {
    let view = daw_ui::DelayView {
        taps: &[],
        tails: &[],
        grid: &[],
        span_seconds: span,
        floor_db: -60.0,
        handles: &[],
        reductions: &[],
        size: egui::Vec2::ZERO,
    };
    daw_ui::delay_point(&view, panel.state.picture_rects[0], seconds, db, below)
}

#[test]
fn spacer_echo_handles_set_time_level_and_feedback() {
    use spacer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    // A dotted eighth at 120 bpm is 375 ms; six of them need four seconds.
    let from = echo_point(&panel, 4.0, 0.375, -6.0, false);
    let to = echo_point(&panel, 4.0, 0.75, -18.0, false);
    panel.frame(vec![Event::PointerMoved(from)]);
    panel.frame(vec![button(from, true)]);
    for step in 1..=4 {
        panel.frame(vec![Event::PointerMoved(from.lerp(to, step as f32 / 4.0))]);
        assert_eq!(panel.state.held_span, Some(4.0), "the axis holds still");
    }
    panel.frame(vec![button(to, false)]);
    panel.frame(vec![]);
    assert_eq!(panel.state.held_span, None);
    // The division nearest to 750 ms: a dotted quarter.
    assert_eq!(panel.plain(ECHO_DIVISION), 12.0);
    assert!((panel.plain(ECHO_LEVEL) + 18.0).abs() < 0.6);
    assert!(panel.host.singles.is_empty());
    assert!(
        panel
            .host
            .batch_ids
            .iter()
            .all(|ids| ids == &[ECHO_DIVISION, ECHO_LEVEL]),
        "time and level are one gesture"
    );

    // Free of the tempo, the same handle is Time.
    panel.set(ECHO_DIVISION, 9.0);
    panel.set(ECHO_LEVEL, -6.0);
    panel.set(ECHO_SYNC, 0.0);
    let from = echo_point(&panel, 4.0, 0.35, -6.0, false);
    panel.drag(from, echo_point(&panel, 4.0, 0.6, -6.0, false));
    assert!((panel.plain(ECHO_TIME) - 600.0).abs() < 15.0);
    assert!((panel.plain(ECHO_LEVEL) + 6.0).abs() < 0.1);

    // The second echo, on the right in ping-pong: its height is Feedback.
    panel.set(ECHO_TIME, 350.0);
    let level = |feedback: f32| -6.0 + 20.0 * (feedback / 100.0).log10();
    let from = echo_point(&panel, 4.0, 0.7, level(35.0), true);
    panel.host.singles.clear();
    panel.drag(from, echo_point(&panel, 4.0, 0.7, level(70.0), true));
    assert!((panel.plain(ECHO_FEEDBACK) - 70.0).abs() < 2.0);
    assert!(panel.host.singles.iter().all(|id| *id == ECHO_FEEDBACK));
    assert!((panel.plain(ECHO_TIME) - 350.0).abs() < 1.0);
    let at = echo_point(&panel, 4.0, 0.7, level(panel.plain(ECHO_FEEDBACK)), true);
    panel.click(at, 2);
    assert_eq!(panel.plain(ECHO_FEEDBACK), 35.0, "a double-click resets");
}

#[test]
fn spacer_reverb_handles_set_pre_delay_level_and_decay() {
    use spacer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    let from = echo_point(&panel, 4.0, 0.015, -9.0, false);
    panel.drag(from, echo_point(&panel, 4.0, 0.115, -21.0, false));
    assert!((panel.plain(PRE_DELAY) - 115.0).abs() < 8.0);
    assert!((panel.plain(REVERB_LEVEL) + 21.0).abs() < 0.6);
    assert!(
        panel
            .host
            .batch_ids
            .iter()
            .all(|ids| ids == &[PRE_DELAY, REVERB_LEVEL])
    );
    panel.set(PRE_DELAY, 15.0);
    panel.set(REVERB_LEVEL, -9.0);

    // The tail has fallen from −9 dB to the floor after 51/60 of Decay.
    let end = |decay: f32| 0.015 + decay * 51.0 / 60.0;
    let from = echo_point(&panel, 4.0, end(1.8), -60.0, false);
    panel.host.singles.clear();
    panel.drag(from, echo_point(&panel, 4.0, end(3.0), -60.0, false));
    let decay = panel.plain(DECAY);
    assert!((decay - 3.0).abs() < 0.1, "{decay}");
    assert!(panel.host.singles.iter().all(|id| *id == DECAY));
    // The handle keeps the focus: the arrow keys turn the same value.
    panel.frame(vec![key(egui::Key::ArrowRight)]);
    panel.frame(vec![]);
    assert!(panel.plain(DECAY) > decay);
    panel.frame(vec![key(egui::Key::ArrowUp)]);
    assert!(panel.host.singles.iter().all(|id| *id == DECAY));

    // A section that is off shows no handles.
    panel.set(DECAY, 1.8);
    panel.set(REVERB_ON, 0.0);
    let values = panel.host.values.clone();
    let from = echo_point(&panel, 2.0, end(1.8), -60.0, false);
    panel.drag(from, from + vec2(60.0, 0.0));
    assert_eq!(panel.host.values, values);
}

#[test]
fn spacer_picture_follows_the_playing_device_without_editing() {
    use spacer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    panel.set(ECHO_DUCK, 70.0);
    panel.set(REVERB_DUCK, 30.0);
    let values = panel.host.values.clone();
    let telemetry = Arc::new(Telemetry::default());
    telemetry.set_value(TELEMETRY_SAMPLE_RATE, 96_000.0);
    telemetry.set_value(TELEMETRY_TEMPO, 60.0);
    telemetry.set_value(TELEMETRY_DUCK, 12.0);
    let block = vec![0.25; 512];
    telemetry.record_output(&block, &block);
    panel.host.telemetry = Some(telemetry);
    let rects = panel.state.picture_rects.clone();
    panel.frame(vec![]);
    assert_eq!(panel.state.picture_rects, rects);
    assert_eq!(panel.host.values, values, "showing edits nothing");
    // At 60 bpm the dotted eighth is 750 ms: T stands there.
    let from = echo_point(&panel, 8.0, 0.75, -6.0, false);
    panel.drag(from, from - vec2(0.0, -20.0));
    assert!(panel.plain(ECHO_LEVEL) < -8.0);
}

#[test]
fn spacer_picture_places_a_synced_echo_at_the_song_tempo_while_nothing_plays() {
    use spacer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    // No telemetry: the device has not run. At 90 bpm the dotted eighth is
    // 500 ms.
    panel.host.tempo = Some(90.0);
    panel.frame(vec![]);
    let from = echo_point(&panel, 4.0, 0.5, -6.0, false);
    panel.drag(from, from - vec2(0.0, -20.0));
    assert!(panel.plain(ECHO_LEVEL) < -8.0);
}

#[test]
fn a_random_roll_sets_the_whole_device_as_one_step() {
    let host = TableHost::new(&spacer::DESCRIPTOR);
    let values = random_roll(&spacer::DESCRIPTOR, &host, 0.1, 3);
    assert_eq!(values.len(), spacer::DESCRIPTOR.parameters.len());
    assert!(values.iter().any(|(id, value)| *value != host.value(*id)));
    let output = values.iter().find(|(id, _)| *id == spacer::OUTPUT).unwrap();
    assert_eq!(output.1, host.value(spacer::OUTPUT));
}

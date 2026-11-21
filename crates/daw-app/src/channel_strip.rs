//! A mixer channel's R1 Strip: its values on the track, the mixer's filter
//! knobs and stage buttons, and the window that shows the whole strip.
//!
//! The strip's trim, polarity, width, pan and output are the channel's own
//! input gain, polarity, width, pan and volume, so the window edits those
//! fields and the stored strip never holds them.
use crate::app::{DawApp, PopupKind};
use daw_core::{PluginParameterValue, Track};
use daw_devices::devices::strip::{self, *};
use eframe::egui;

/// The largest channel volume, as the fader's.
const MAX_GAIN: f32 = 1.5;
/// The largest input gain, as the input row's.
const MAX_INPUT_GAIN: f32 = 16.0;

fn spec(id: u32) -> &'static daw_devices::ParamSpec {
    strip::DESCRIPTOR.parameter(id).expect("strip parameter")
}

/// The normalized value of a strip parameter on this channel.
pub(crate) fn value(track: &Track, id: u32) -> f64 {
    let spec = spec(id);
    let db = |gain: f32| daw_ui::channel_gain_to_db(gain).clamp(-24.0, 24.0);
    let plain = match id {
        TRIM => db(track.input_gain),
        POLARITY => f32::from(u8::from(track.invert_polarity)),
        WIDTH => track.stereo_width * 100.0,
        PAN => track.pan * 100.0,
        OUTPUT => db(track.gain),
        _ => {
            return track
                .strip
                .iter()
                .find(|value| value.id == id)
                .map_or(f64::from(spec.default_normalized()), |value| value.value);
        }
    };
    f64::from(spec.to_normalized(plain))
}

fn plain(track: &Track, id: u32) -> f32 {
    spec(id).to_plain(value(track, id) as f32)
}

fn on(track: &Track, id: u32) -> bool {
    plain(track, id) >= 0.5
}

/// Sets a strip parameter on this channel: one of the channel's own fields,
/// or the stored strip value.
pub(crate) fn set(track: &mut Track, id: u32, normalized: f64) {
    if !normalized.is_finite() {
        return;
    }
    let normalized = normalized.clamp(0.0, 1.0);
    let plain = spec(id).to_plain(normalized as f32);
    match id {
        TRIM => track.input_gain = daw_ui::channel_gain_from_db(plain).min(MAX_INPUT_GAIN),
        POLARITY => track.invert_polarity = plain >= 0.5,
        WIDTH => track.stereo_width = (plain / 100.0).clamp(0.0, 2.0),
        PAN => track.pan = (plain / 100.0).clamp(-1.0, 1.0),
        OUTPUT => track.gain = daw_ui::channel_gain_from_db(plain).min(MAX_GAIN),
        _ => match track.strip.iter_mut().find(|value| value.id == id) {
            Some(value) => value.value = normalized,
            None => track.strip.push(PluginParameterValue {
                id,
                value: normalized,
            }),
        },
    }
}

/// A channel that has a strip: folders and automation tracks have none.
pub(crate) fn has_strip(track: &Track) -> bool {
    !matches!(
        track.kind,
        daw_core::TrackKind::Folder | daw_core::TrackKind::Automation
    )
}

/// Where the mixer's filter knobs stand, 0–1: the frequency's position when
/// the filter is on, the end of the travel when it is off.
pub(crate) fn filter_positions(track: &Track) -> [f32; 2] {
    [
        if on(track, HP_ON) {
            value(track, HP_FREQUENCY) as f32
        } else {
            0.0
        },
        if on(track, LP_ON) {
            value(track, LP_FREQUENCY) as f32
        } else {
            1.0
        },
    ]
}

/// The knobs' short readouts: the frequency, or the filter's name while it
/// is off.
pub(crate) fn filter_texts(track: &Track) -> [String; 2] {
    let text = |switch: u32, frequency: u32, name: &str| {
        if !on(track, switch) {
            return name.to_owned();
        }
        let hz = plain(track, frequency);
        if hz < 1000.0 {
            format!("{hz:.0}")
        } else if hz < 10_000.0 {
            format!("{:.1}k", hz / 1000.0)
        } else {
            format!("{:.0}k", hz / 1000.0)
        }
    };
    [
        text(HP_ON, HP_FREQUENCY, "HP"),
        text(LP_ON, LP_FREQUENCY, "LP"),
    ]
}

/// Moves a filter knob: the end of its travel switches the filter off,
/// anywhere else sets its frequency and switches it on.
pub(crate) fn set_filter(track: &mut Track, low_pass: bool, position: f32) {
    // A hair from the end still reads as the end on a small knob.
    const END: f32 = 0.005;
    let (switch, frequency, off) = if low_pass {
        (LP_ON, LP_FREQUENCY, position >= 1.0 - END)
    } else {
        (HP_ON, HP_FREQUENCY, position <= END)
    };
    set(track, switch, if off { 0.0 } else { 1.0 });
    if !off {
        set(track, frequency, f64::from(position));
    }
}

/// Which stages are doing something: the equalizer when it is on and not
/// flat, the gate and the compressor when they are on.
pub(crate) fn stages_lit(track: &Track) -> [bool; 3] {
    let flat = [LOW_GAIN, MID_GAIN, HIGH_GAIN]
        .iter()
        .all(|&id| plain(track, id) == 0.0);
    [
        on(track, EQ_ON) && !flat,
        on(track, GATE_ON),
        on(track, COMP_ON),
    ]
}

pub(crate) struct MixerControls {
    pub changed: bool,
    /// A stage button asked for the channel strip window.
    pub open: bool,
    /// Both rows, for the input section's band.
    pub rect: egui::Rect,
    pub diagnostics: serde_json::Value,
}

/// The strip's rows on a mixer channel, right under its input: the filters
/// as knobs, the other stages as buttons into its window. `held` ignores
/// edits while another gesture owns the mixer.
pub(crate) fn mixer_controls(
    ui: &mut egui::Ui,
    track: &mut Track,
    width: f32,
    held: bool,
) -> MixerControls {
    let [hp_text, lp_text] = filter_texts(track);
    let [mut high_pass, mut low_pass] = filter_positions(track);
    let filters = daw_ui::mixer_filter_row(
        ui,
        width,
        &mut high_pass,
        &mut low_pass,
        [&hp_text, &lp_text],
    );
    let mut changed = false;
    if filters.high_pass.changed() && !held {
        set_filter(track, false, high_pass);
        changed = true;
    }
    if filters.low_pass.changed() && !held {
        set_filter(track, true, low_pass);
        changed = true;
    }
    let lit = stages_lit(track);
    let stages = daw_ui::mixer_strip_buttons(
        ui,
        width,
        [
            ("EQ", lit[0], "Equalizer · opens the channel strip"),
            ("Gate", lit[1], "Gate · opens the channel strip"),
            ("Comp", lit[2], "Compressor · opens the channel strip"),
        ],
    );
    let rect = |response: &egui::Response| {
        serde_json::json!({"min":[response.rect.min.x,response.rect.min.y],
            "max":[response.rect.max.x,response.rect.max.y]})
    };
    MixerControls {
        changed,
        open: stages.iter().any(egui::Response::clicked),
        rect: filters
            .high_pass
            .rect
            .union(filters.low_pass.rect)
            .union(stages[2].rect)
            .union(stages[0].rect),
        diagnostics: serde_json::json!({"track":track.id,"strip":{
            "high_pass":rect(&filters.high_pass),"low_pass":rect(&filters.low_pass),
            "eq":rect(&stages[0]),"gate":rect(&stages[1]),"comp":rect(&stages[2])}}),
    }
}

#[derive(Default)]
pub(crate) struct ChannelStripWindow {
    pub open: bool,
    pub track: u64,
    panel: daw_devices_ui::PanelState,
    /// Read when the window first draws and after a save, not every frame.
    presets: Option<Vec<daw_devices::Preset>>,
}

/// What the panel asked for while it drew.
struct Host<'a> {
    track: &'a Track,
    presets: &'a [daw_devices::Preset],
    updates: Vec<(u32, f64)>,
    batch: Vec<(u32, f64)>,
    saved: Option<daw_devices::Preset>,
}

impl daw_devices_ui::DeviceHost for Host<'_> {
    fn value(&self, id: u32) -> f64 {
        self.batch
            .iter()
            .chain(&self.updates)
            .rev()
            .find(|(update, _)| *update == id)
            .map_or_else(|| value(self.track, id), |(_, value)| *value)
    }
    fn set_value(&mut self, id: u32, normalized: f64) {
        self.updates.push((id, normalized));
    }
    fn set_values(&mut self, values: &[(u32, f64)]) {
        if values.len() < strip::PARAMETERS.len() {
            self.updates.extend_from_slice(values);
        } else {
            // A preset or Reset sets how the strip processes, not where the
            // channel sits in the mix: its own fields stay.
            self.batch = values
                .iter()
                .copied()
                .filter(|(id, _)| !CHANNEL_FIELDS.contains(id))
                .collect();
        }
    }
    fn automation(&self, _: u32) -> Option<(u64, daw_ui::AutomationState)> {
        None
    }
    fn navigate(&mut self, _: daw_ui::AutomationNavigation) {}
    fn user_presets(&self) -> Vec<daw_devices::Preset> {
        self.presets.to_vec()
    }
    fn save_preset(&mut self, preset: daw_devices::Preset) {
        self.saved = Some(preset);
    }
    fn audition(&mut self, _: Option<(u8, f32)>) {}
    fn wheels(&mut self, _: f32, _: f32) {}
}

impl DawApp {
    pub(crate) fn open_channel_strip(&mut self, track: u64) {
        if !self.project.track(track).is_some_and(has_strip) {
            return;
        }
        self.open_popup(PopupKind::ChannelStrip);
        self.selected_track = Some(track);
        self.channel_strip.track = track;
        self.channel_strip.open = true;
    }

    pub(crate) fn channel_strip_page(&mut self, ctx: &egui::Context) {
        if !self.channel_strip.open {
            return;
        }
        let track_id = self.channel_strip.track;
        let Some(track) = self
            .project
            .track(track_id)
            .filter(|track| has_strip(track))
        else {
            self.close_popup();
            return;
        };
        let title = format!("Channel strip · {}", track.name);
        let window = &mut self.channel_strip;
        let presets = window
            .presets
            .get_or_insert_with(|| crate::builtin_devices::user_presets(strip::DESCRIPTOR.id));
        let mut host = Host {
            track,
            presets,
            updates: Vec::new(),
            batch: Vec::new(),
            saved: None,
        };
        let mut open = true;
        let panel = &mut window.panel;
        daw_ui::page(title)
            .id(egui::Id::new("channel_strip"))
            .open(&mut open)
            .show(ctx, |ui| {
                daw_devices_ui::show(ui, &strip::DESCRIPTOR, &mut host, panel);
            });
        let Host {
            updates,
            batch,
            saved,
            ..
        } = host;
        if let Some(preset) = saved {
            match crate::builtin_devices::save_user_preset(&preset) {
                Ok(_) => self.channel_strip.presets = None,
                Err(error) => self.message(Err(error.context("Preset not saved")), ""),
            }
        }
        if !updates.is_empty() || !batch.is_empty() {
            let track = self.project.track_mut(track_id).expect("drawn above");
            for (id, value) in batch.into_iter().chain(updates) {
                set(track, id, value);
            }
            self.changed();
        }
        if !open {
            self.close_popup();
        }
    }
}

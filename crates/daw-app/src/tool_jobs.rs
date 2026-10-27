//! Song-scoped worker jobs. Never call these helpers on the UI or audio thread.
//! Jobs return measurements and discovered controls, not filesystem capabilities.
use daw_core::{
    AudioData, AudioSilenceSettings, ClipContent, Effect, Instrument, Project, TrackKind,
    TransientAlgorithm, TransientSettings, detect_audio_silence, detect_transients,
    normalization_gain,
};
use daw_plugins::{ParameterInfo, VstEffect, VstInstrument};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_ANALYSIS_SECONDS: f64 = 30.0;
const MAX_END_SECONDS: f64 = 120.0;
const MAX_PARAMETER_PAGE: usize = 200;

pub(crate) fn handles(name: &str) -> bool {
    matches!(
        name,
        "audio_analyze" | "audio_analyze_clip" | "plugin_describe_parameters"
    )
}

pub(crate) fn execute(
    project: Project,
    sample_rate: u32,
    name: &str,
    args: Value,
    cancel: &AtomicBool,
) -> Result<Value, String> {
    check_cancelled(cancel)?;
    match name {
        "audio_analyze" => analyze(project, sample_rate, decode(args)?, cancel),
        "audio_analyze_clip" => analyze_clip(&project, decode(args)?, cancel),
        "plugin_describe_parameters" => {
            describe_parameters(&project, sample_rate, decode(args)?, cancel)
        }
        _ => Err("Unknown worker tool".into()),
    }
}

fn decode<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|error| format!("Invalid tool arguments: {error}"))
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Tool job cancelled".into())
    } else {
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnalyzeArgs {
    #[serde(default)]
    start_beat: f64,
    end_beat: Option<f64>,
    track_id: Option<u64>,
}

fn analyze(
    mut project: Project,
    sample_rate: u32,
    args: AnalyzeArgs,
    cancel: &AtomicBool,
) -> Result<Value, String> {
    project
        .validate()
        .map_err(|_| "The current song is invalid".to_string())?;
    if !(8_000..=192_000).contains(&sample_rate) {
        return Err("Audio analysis supports sample rates from 8000 to 192000 Hz".into());
    }
    let end = args.end_beat.unwrap_or_else(|| project.end_beat());
    if !args.start_beat.is_finite()
        || !end.is_finite()
        || args.start_beat < 0.0
        || end <= args.start_beat
    {
        return Err("Choose a finite nonempty range with nonnegative beat positions".into());
    }
    let seconds_per_beat = 60.0 / project.tempo;
    if (end - args.start_beat) * seconds_per_beat > MAX_ANALYSIS_SECONDS + 1e-9 {
        return Err(
            "Analyze at most 30 seconds per request; specify start_beat and end_beat".into(),
        );
    }
    if end * seconds_per_beat > MAX_END_SECONDS + 1e-9 {
        return Err("Audio analysis currently supports ranges within the first 120 seconds of the song; preceding audio is rendered to preserve effect history".into());
    }
    let target = if let Some(id) = args.track_id {
        let track = project
            .track(id)
            .ok_or_else(|| "Track not found".to_string())?;
        if !matches!(
            track.kind,
            TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
        ) {
            return Err("Analyze an audio, MIDI, or bus track".into());
        }
        let name = bounded_text(&track.name, 120);
        // Existing solo traversal selects upstream bus feeds and downstream
        // output/send paths, without adding unrelated sources sharing a bus.
        // Existing mutes, folder mutes, faders and automation remain in effect.
        for track in &mut project.tracks {
            track.solo = track.id == id;
        }
        json!({"track_id":id,"name":name,"mode":"contribution_at_master_with_sends_and_existing_mutes"})
    } else {
        json!({"mode":"master_with_current_mute_and_solo"})
    };
    check_cancelled(cancel)?;
    let samples = daw_engine::render_range_cancellable(
        &project,
        sample_rate,
        args.start_beat,
        end,
        daw_engine::RenderOptions::default(),
        cancel,
    )
    .map_err(|_| {
        if cancel.load(Ordering::Acquire) {
            "Tool job cancelled".to_string()
        } else {
            // Renderer diagnostics may contain asset/plugin paths. These are
            // application diagnostics, never provider-visible tool payloads.
            "Unable to render this range for analysis; check the song's audio sources and plug-ins"
                .to_string()
        }
    })?;
    let mut measurements = measure(&samples, sample_rate, cancel)?;
    measurements["target"] = target;
    measurements["range"] =
        json!({"start_beat":args.start_beat,"end_beat":end,"end_exclusive":true});
    measurements["render_history"] = json!("Processed from song start; preceding audio discarded");
    if args.track_id.is_some() {
        measurements["isolation_note"] = json!(
            "The selected track or bus is rendered in isolation through its downstream routes. Nonlinear shared effects may respond differently than in the complete mix."
        );
    }
    measurements["listening"] =
        json!("Numeric audio measurements only; no audio was sent to a model");
    Ok(measurements)
}

fn dbfs(amplitude: f64) -> Option<f64> {
    (amplitude > 0.0).then(|| 20.0 * amplitude.log10())
}

fn measure(samples: &[f32], sample_rate: u32, cancel: &AtomicBool) -> Result<Value, String> {
    if samples.is_empty() || !samples.len().is_multiple_of(2) {
        return Err("Rendered analysis audio must contain stereo frames".into());
    }
    let frames = samples.len() / 2;
    let low_alpha = 1.0 - (-std::f64::consts::TAU * 250.0 / f64::from(sample_rate)).exp();
    let high_alpha = 1.0 - (-std::f64::consts::TAU * 4_000.0 / f64::from(sample_rate)).exp();
    let mut peaks = [0.0_f64; 2];
    let mut powers = [0.0_f64; 2];
    let mut sums = [0.0_f64; 2];
    let mut cross = 0.0;
    let mut low = [0.0; 2];
    let mut high_lowpass = [0.0; 2];
    let mut band_powers = [0.0; 3];
    let mut clipping_frames = 0_u64;
    for (index, frame) in samples.as_chunks::<2>().0.iter().enumerate() {
        if index.is_multiple_of(1024) {
            check_cancelled(cancel)?;
        }
        let left = f64::from(frame[0]);
        let right = f64::from(frame[1]);
        if !left.is_finite() || !right.is_finite() {
            return Err("Rendered audio contains nonfinite samples".into());
        }
        if left.abs() >= 1.0 || right.abs() >= 1.0 {
            clipping_frames += 1;
        }
        for channel in 0..2 {
            let sample = f64::from(frame[channel]);
            peaks[channel] = peaks[channel].max(sample.abs());
            powers[channel] += sample * sample;
            sums[channel] += sample;
            low[channel] += low_alpha * (sample - low[channel]);
            high_lowpass[channel] += high_alpha * (sample - high_lowpass[channel]);
            let bands = [
                low[channel],
                high_lowpass[channel] - low[channel],
                sample - high_lowpass[channel],
            ];
            for (power, value) in band_powers.iter_mut().zip(bands) {
                *power += value * value;
            }
        }
        cross += left * right;
    }
    check_cancelled(cancel)?;
    let count = frames as f64;
    let rms = ((powers[0] + powers[1]) / (2.0 * count)).sqrt();
    let centered_left = (powers[0] - sums[0] * sums[0] / count).max(0.0);
    let centered_right = (powers[1] - sums[1] * sums[1] / count).max(0.0);
    let correlation_scale = (centered_left * centered_right).sqrt();
    let correlation = (correlation_scale > 1e-20)
        .then(|| ((cross - sums[0] * sums[1] / count) / correlation_scale).clamp(-1.0, 1.0));
    let band_rms = band_powers.map(|power| (power / (2.0 * count)).sqrt());
    Ok(json!({
        "sample_rate":sample_rate,"frames":frames,"duration_seconds":count/f64::from(sample_rate),
        "silent":peaks[0] == 0.0 && peaks[1] == 0.0,
        "sample_peak_dbfs":dbfs(peaks[0].max(peaks[1])),
        "sample_peak_linear":{"left":peaks[0],"right":peaks[1]},
        "rms_dbfs":dbfs(rms),"rms_linear":rms,
        "channel_rms_dbfs":{"left":dbfs((powers[0]/count).sqrt()),"right":dbfs((powers[1]/count).sqrt())},
        "frames_at_or_above_full_scale":clipping_frames,
        "stereo_correlation":correlation,
        "band_rms_estimates_dbfs":{"below_250_hz":dbfs(band_rms[0]),"250_to_4000_hz":dbfs(band_rms[1]),"above_4000_hz":dbfs(band_rms[2])},
        "band_estimate_method":"First-order crossover filters, approximate overlapping bands; not calibrated loudness measurements",
        "measurement_limits":"Sample peaks, not true peak; RMS, not LUFS. Silent amplitudes and undefined correlation are null."
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParameterArgs {
    track_id: u64,
    insert_id: Option<u64>,
    #[serde(default)]
    query: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_page_limit")]
    limit: usize,
}

fn default_page_limit() -> usize {
    100
}

fn describe_parameters(
    project: &Project,
    sample_rate: u32,
    args: ParameterArgs,
    cancel: &AtomicBool,
) -> Result<Value, String> {
    if args.limit == 0 || args.limit > MAX_PARAMETER_PAGE {
        return Err("Parameter page limit must be between 1 and 200".into());
    }
    if args.query.len() > 256 {
        return Err("Parameter search query is limited to 256 bytes".into());
    }
    let track = project
        .track(args.track_id)
        .ok_or_else(|| "Track not found".to_string())?;
    let metadata_error = || {
        "Unable to discover this plug-in's parameters; check that the assigned VST3 and helper are available".to_string()
    };
    let parameters = if let Some(insert_id) = args.insert_id {
        let insert = track
            .inserts
            .iter()
            .find(|insert| insert.id == insert_id)
            .ok_or_else(|| "Insert not found on this track".to_string())?;
        let Effect::Vst3 { path, class_id } = &insert.effect;
        check_cancelled(cancel)?;
        let mut plugin = VstEffect::load_class(
            Path::new(path),
            class_id.as_deref(),
            f64::from(sample_rate),
            256,
        )
        .map_err(|_| metadata_error())?;
        check_cancelled(cancel)?;
        if !insert.state.is_empty() {
            plugin
                .load_state(&insert.state)
                .map_err(|_| metadata_error())?;
        }
        for parameter in &insert.parameters {
            check_cancelled(cancel)?;
            plugin
                .set_parameter(parameter.id, parameter.value)
                .map_err(|_| metadata_error())?;
        }
        plugin.parameters().map_err(|_| metadata_error())?
    } else {
        let Instrument::Vst3 { path, class_id } = &track.instrument else {
            return Err("This track has no assigned VST3 instrument".into());
        };
        check_cancelled(cancel)?;
        let mut plugin = VstInstrument::load_class(
            Path::new(path),
            class_id.as_deref(),
            f64::from(sample_rate),
            256,
        )
        .map_err(|_| metadata_error())?;
        check_cancelled(cancel)?;
        if !track.instrument_state.is_empty() {
            plugin
                .load_state(&track.instrument_state)
                .map_err(|_| metadata_error())?;
        }
        for parameter in &track.instrument_parameters {
            check_cancelled(cancel)?;
            plugin
                .set_parameter(parameter.id, parameter.value)
                .map_err(|_| metadata_error())?;
        }
        plugin.parameters().map_err(|_| metadata_error())?
    };
    check_cancelled(cancel)?;
    parameter_page(parameters, args)
}

fn parameter_page(parameters: Vec<ParameterInfo>, args: ParameterArgs) -> Result<Value, String> {
    let query = args.query.to_lowercase();
    let total = parameters.len();
    let matching: Vec<_> = parameters
        .into_iter()
        .filter(|parameter| {
            parameter.name.to_lowercase().contains(&query) || parameter.id.to_string() == query
        })
        .collect();
    let matched = matching.len();
    let page: Vec<_> = matching
        .into_iter()
        .skip(args.offset)
        .take(args.limit)
        .map(|parameter| json!({
            "id":parameter.id,"name":bounded_text(&parameter.name,160),
            "normalized_value":parameter.value,"normalized_default":parameter.default,
            "unit":bounded_text(&parameter.unit,32),"steps":parameter.steps,"read_only":parameter.read_only
        }))
        .collect();
    let next_offset = args.offset.saturating_add(page.len());
    Ok(json!({
        "track_id":args.track_id,"insert_id":args.insert_id,
        "total_parameters":total,"matched_parameters":matched,"parameters":page,
        "next_offset":(next_offset<matched).then_some(next_offset),
        "value_format":"Normalized 0..1 values; unit labels do not establish a conversion to physical units",
        "automation_note":"Returned values are stored manual/preset values. Enabled automation may override them at playback time."
    }))
}

fn bounded_text(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OnsetAlgorithm {
    #[default]
    EnergyRise,
    AmplitudeGate,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipAnalysisArgs {
    track_id: u64,
    clip_id: u64,
    start_beat: Option<f64>,
    end_beat: Option<f64>,
    #[serde(default)]
    transient_algorithm: OnsetAlgorithm,
    #[serde(default = "default_sensitivity")]
    sensitivity: f32,
    #[serde(default = "default_gap")]
    minimum_gap_ms: f32,
    #[serde(default = "default_silence_threshold")]
    silence_threshold_db: f32,
    #[serde(default = "default_minimum_silence")]
    minimum_silence_ms: f32,
    #[serde(default = "default_padding")]
    padding_ms: f32,
    #[serde(default = "default_normalization_target")]
    normalize_target_db: f32,
}

fn default_sensitivity() -> f32 {
    0.5
}
fn default_gap() -> f32 {
    80.0
}
fn default_silence_threshold() -> f32 {
    -40.0
}
fn default_minimum_silence() -> f32 {
    100.0
}
fn default_padding() -> f32 {
    5.0
}
fn default_normalization_target() -> f32 {
    -1.0
}

fn analyze_clip(
    project: &Project,
    args: ClipAnalysisArgs,
    cancel: &AtomicBool,
) -> Result<Value, String> {
    project
        .validate()
        .map_err(|_| "The current song is invalid".to_string())?;
    if !args.sensitivity.is_finite()
        || !(0.0..=1.0).contains(&args.sensitivity)
        || !args.minimum_gap_ms.is_finite()
        || !(20.0..=2_000.0).contains(&args.minimum_gap_ms)
        || !args.silence_threshold_db.is_finite()
        || !(-96.0..=0.0).contains(&args.silence_threshold_db)
        || !args.minimum_silence_ms.is_finite()
        || !(1.0..=10_000.0).contains(&args.minimum_silence_ms)
        || !args.padding_ms.is_finite()
        || !(0.0..=1_000.0).contains(&args.padding_ms)
        || !args.normalize_target_db.is_finite()
        || !(-60.0..=0.0).contains(&args.normalize_target_db)
    {
        return Err("Invalid clip analysis settings: sensitivity 0..1, minimum gap 20..2000 ms, silence threshold -96..0 dBFS, minimum silence 1..10000 ms, padding 0..1000 ms, normalization target -60..0 dBFS".into());
    }
    let clip = project
        .clip(args.track_id, args.clip_id)
        .ok_or_else(|| "Audio clip not found".to_string())?;
    let ClipContent::Audio { path, .. } = &clip.content else {
        return Err("Analyze an audio clip".into());
    };
    if clip.is_take_source() {
        return Err("Choose a take before analyzing its editable clip".into());
    }
    let start = args.start_beat.unwrap_or(clip.start);
    let end = args.end_beat.unwrap_or_else(|| clip.end());
    let region = clip.audio_region(start, end).map_err(|_| {
        "Analysis range must be finite, nonempty, and inside the audio clip".to_string()
    })?;
    if (end - start) * 60.0 / project.tempo > MAX_ANALYSIS_SECONDS + 1e-9 {
        return Err(
            "Analyze at most 30 seconds of a clip per request; specify start_beat and end_beat"
                .into(),
        );
    }
    check_cancelled(cancel)?;
    let data = AudioData::load_wav_with_limit(path, 64 * 1024 * 1024).map_err(|_| {
        "Unable to decode this clip's audio source (maximum decoded size 64 MiB)".to_string()
    })?;
    check_cancelled(cancel)?;
    if data.channels > 32 || data.sample_rate > 192_000 {
        return Err(
            "Clip analysis supports at most 32 channels and source sample rates up to 192000 Hz"
                .into(),
        );
    }
    let source_samples = (end - start) * 60.0 / project.tempo
        * clip.source_rate
        * f64::from(data.sample_rate)
        * data.channels as f64;
    if source_samples > 16_777_216.0 {
        return Err(
            "The source range is too large for clip analysis; choose a shorter range".into(),
        );
    }
    let settings = TransientSettings {
        algorithm: match args.transient_algorithm {
            OnsetAlgorithm::EnergyRise => TransientAlgorithm::EnergyRise,
            OnsetAlgorithm::AmplitudeGate => TransientAlgorithm::AmplitudeGate,
        },
        sensitivity: args.sensitivity,
        minimum_gap_ms: args.minimum_gap_ms,
    };
    let region_onsets =
        detect_transients(&region, &data, project.tempo, settings).map_err(|_| {
            "Unable to analyze transients; shorten the range or increase the minimum gap"
                .to_string()
        })?;
    check_cancelled(cancel)?;
    let clip_onsets: Vec<_> = region_onsets
        .iter()
        .map(|beat| start - clip.start + beat)
        .collect();
    let arrangement_onsets: Vec<_> = region_onsets.iter().map(|beat| start + beat).collect();
    let retained = detect_audio_silence(
        clip,
        &data,
        project.tempo,
        (start, end),
        AudioSilenceSettings {
            threshold_db: args.silence_threshold_db,
            minimum_silence_ms: args.minimum_silence_ms,
            padding_ms: args.padding_ms,
        },
    )
    .map_err(|_| {
        "Unable to analyze silence; shorten the range or increase minimum silence".to_string()
    })?;
    check_cancelled(cancel)?;
    let gain = normalization_gain(clip, &data, project.tempo, args.normalize_target_db).ok();
    let normalization_peak = gain
        .map(|gain| 10.0_f64.powf(f64::from(args.normalize_target_db) / 20.0) / f64::from(gain));
    let normalize_edit = gain.map(
        |gain| json!({"op":"set_audio_gain","track":args.track_id,"clip":args.clip_id,"gain":gain}),
    );
    let transient_edit = (!clip_onsets.is_empty()).then(||json!({"op":"split_audio_transients","track":args.track_id,"clip":args.clip_id,"beats":clip_onsets}));
    let silence_edit = (retained.len()<=2048).then(||json!({"op":"strip_audio_silence","track":args.track_id,"clip":args.clip_id,"start":start,"end":end,"retained":retained}));
    let linked_occurrences = clip.link_group.map_or(1, |group| {
        project
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .filter(|other| other.link_group == Some(group))
            .count()
    });
    check_cancelled(cancel)?;
    Ok(json!({
        "track_id":args.track_id,"clip_id":args.clip_id,"range":{"start_beat":start,"end_beat":end,"end_exclusive":true},
        "source":{"sample_rate":data.sample_rate,"channels":data.channels,"frames":data.frames(),"duration_seconds":data.duration_seconds(),"source_beats":data.duration_seconds()*project.tempo/60.0},
        "transients":{"clip_relative_beats":clip_onsets,"arrangement_beats":arrangement_onsets},
        "silence":{"retained_arrangement_ranges":retained,"threshold_dbfs":args.silence_threshold_db,"minimum_silence_ms":args.minimum_silence_ms,"padding_ms":args.padding_ms},
        "normalization":{"target_dbfs":args.normalize_target_db,"source_peak_linear":normalization_peak,"recommended_absolute_gain":gain,"scope":"Entire current clip source window, before gain/fades/pitch processing, even when a smaller analysis range is selected","linked_occurrences_affected":linked_occurrences,"unavailable_reason":gain.is_none().then_some("Source window is silent or below the normalization floor")},
        "suggested_edits":{"normalize":normalize_edit,"split_transients":transient_edit,"strip_silence":silence_edit},
        "apply_note":"Analysis only. Choose the edit matching the user's request and submit it in an edit batch for the same song revision. The topology edits are alternatives: applying one invalidates the other analysis for this clip. Transient edit beats are clip-relative; silence ranges are arrangement beats.",
        "measurement_limits":"Source analysis respects source offset, reverse and stretch; it excludes fades, clip/channel gain, inserts and pitch processing. No source files were modified."
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frequency: f64, amplitude: f64, inverted_right: bool) -> Vec<f32> {
        (0..48_000)
            .flat_map(|frame| {
                let value = (std::f64::consts::TAU * frequency * f64::from(frame) / 48_000.0).sin()
                    * amplitude;
                [
                    value as f32,
                    if inverted_right {
                        -value as f32
                    } else {
                        value as f32
                    },
                ]
            })
            .collect()
    }

    #[test]
    fn measurement_detects_rms_full_scale_and_opposite_phase() {
        let result = measure(&tone(100.0, 1.2, true), 48_000, &AtomicBool::new(false)).unwrap();
        assert!((result["rms_linear"].as_f64().unwrap() - 1.2 / 2.0_f64.sqrt()).abs() < 1e-6);
        assert!((result["sample_peak_linear"]["left"].as_f64().unwrap() - 1.2).abs() < 1e-6);
        assert_eq!(result["stereo_correlation"].as_f64().unwrap(), -1.0);
        assert!(result["frames_at_or_above_full_scale"].as_u64().unwrap() > 0);
        assert!(
            result["band_rms_estimates_dbfs"]["below_250_hz"]
                .as_f64()
                .unwrap()
                > result["band_rms_estimates_dbfs"]["above_4000_hz"]
                    .as_f64()
                    .unwrap()
        );
    }

    #[test]
    fn silence_and_cancellation_do_not_report_nonfinite_metrics() {
        let result = measure(&[0.0; 128], 48_000, &AtomicBool::new(false)).unwrap();
        assert_eq!(result["silent"], true);
        assert!(result["rms_dbfs"].is_null());
        assert!(result["stereo_correlation"].is_null());
        assert!(measure(&tone(100.0, 0.5, false), 48_000, &AtomicBool::new(true)).is_err());
    }

    #[test]
    fn parameter_paging_filters_and_never_exposes_plugin_paths_or_state() {
        let parameters = (0..5)
            .map(|id| ParameterInfo {
                id,
                name: format!("Filter {id}"),
                value: 0.5,
                default: 0.25,
                unit: "Hz".into(),
                steps: 0,
                read_only: false,
            })
            .collect();
        let result = parameter_page(
            parameters,
            ParameterArgs {
                track_id: 42,
                insert_id: Some(43),
                query: "filter".into(),
                offset: 1,
                limit: 2,
            },
        )
        .unwrap();
        assert_eq!(result["parameters"].as_array().unwrap().len(), 2);
        assert_eq!(result["parameters"][0]["id"], 1);
        assert_eq!(result["next_offset"], 3);
        assert!(result.get("path").is_none());
        assert!(result.get("state").is_none());
    }

    #[test]
    fn analysis_refuses_expensive_ranges_before_rendering() {
        let project = Project::new();
        let result = execute(
            project,
            48_000,
            "audio_analyze",
            json!({"end_beat":200}),
            &AtomicBool::new(false),
        );
        assert!(result.unwrap_err().contains("30 seconds"));
    }

    #[test]
    fn analyze_selected_track_preserves_downstream_bus_and_excludes_other_sources() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("tone.wav");
        daw_core::AudioData {
            samples: tone(100.0, 0.2, false),
            sample_rate: 48_000,
            channels: 2,
        }
        .write_wav(&source)
        .unwrap();
        let mut project = Project::new();
        let first = project.add_audio_track("First");
        let second = project.add_audio_track("Second");
        let bus = project.add_bus("Bus");
        for id in [first, second] {
            project.track_mut(id).unwrap().gain = 1.0;
            project
                .track_mut(id)
                .unwrap()
                .clips
                .push(daw_core::Clip::audio(
                    "Tone",
                    0.0,
                    2.0,
                    source.to_string_lossy(),
                ));
            project.set_output(id, Some(bus)).unwrap();
        }
        project.track_mut(bus).unwrap().gain = 0.5;
        let result = execute(
            project,
            48_000,
            "audio_analyze",
            json!({"track_id":first,"end_beat":2.0}),
            &AtomicBool::new(false),
        )
        .unwrap();
        let peak = result["sample_peak_linear"]["left"].as_f64().unwrap();
        assert!((peak - 0.1).abs() < 1e-5, "actual peak: {peak}");
    }

    #[test]
    fn clip_analysis_maps_reversed_onsets_and_supplies_valid_alternative_edits() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pulses.wav");
        let mut samples = vec![0.0; 8_000];
        samples[1_000..2_000].fill(0.5);
        samples[5_000..6_000].fill(0.5);
        AudioData {
            samples: samples.clone(),
            sample_rate: 8_000,
            channels: 1,
        }
        .write_wav(&path)
        .unwrap();
        let mut project = Project::new();
        let track = project.add_audio_track("Source");
        let mut clip = daw_core::Clip::audio("Pulses", 8.0, 2.0, path.to_string_lossy());
        let clip_id = clip.id;
        if let ClipContent::Audio { reverse, .. } = &mut clip.content {
            *reverse = true;
        }
        project.track_mut(track).unwrap().clips.push(clip);
        let result = execute(
            project.clone(),
            48_000,
            "audio_analyze_clip",
            json!({"track_id":track,"clip_id":clip_id}),
            &AtomicBool::new(false),
        )
        .unwrap();
        let onsets = result["transients"]["arrangement_beats"]
            .as_array()
            .unwrap();
        assert_eq!(onsets.len(), 2);
        assert!((onsets[0].as_f64().unwrap() - 8.5).abs() < 0.011);
        assert!((onsets[1].as_f64().unwrap() - 9.5).abs() < 0.011);
        assert_eq!(result["source"]["source_beats"], 2.0);
        let gain = result["normalization"]["recommended_absolute_gain"]
            .as_f64()
            .unwrap();
        assert!((gain - 10_f64.powf(-1.0 / 20.0) / 0.5).abs() < 1e-6);
        for action in ["normalize", "split_transients", "strip_silence"] {
            let mut edited = project.clone();
            let batch: daw_core::EditBatch = serde_json::from_value(
                json!({"label":action,"edits":[result["suggested_edits"][action].clone()]}),
            )
            .unwrap();
            assert!(batch.apply(&mut edited).unwrap().changed);
        }
        assert_eq!(AudioData::load_wav(path).unwrap().samples, samples);
    }

    #[test]
    fn clip_analysis_rejects_file_arguments_and_redacts_source_failures() {
        let mut project = Project::new();
        let track = project.add_audio_track("Missing");
        let clip = daw_core::Clip::audio("Source", 0.0, 2.0, "/private/sensitive/missing.wav");
        let clip_id = clip.id;
        project.track_mut(track).unwrap().clips.push(clip);
        let args = json!({"track_id":track,"clip_id":clip_id});
        let error = execute(
            project.clone(),
            48_000,
            "audio_analyze_clip",
            args.clone(),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(!error.contains("private"));
        assert!(!error.contains("sensitive"));
        let mut unsafe_args = args;
        unsafe_args["path"] = json!("/another/file.wav");
        assert!(
            execute(
                project,
                48_000,
                "audio_analyze_clip",
                unsafe_args,
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .contains("unknown field")
        );
    }
}

//! Enumerate devices and play a quiet verification tone on the default output.
//! `device_smoke [DRIVER [capture]]` uses another audio driver such as JACK, and
//! with `capture` also records half a second from its default input. A third
//! argument keeps the stream open that many extra milliseconds for external probes.
use daw_core::{AudioData, AudioInputChannels, Clip, Project};
use daw_engine::{
    AudioEngine, DeviceSettings, InputRoute, InputSettings, RecordingConfig, list_hosts,
    list_input_devices, list_output_devices,
};
use std::{thread, time::Duration};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let host = args.next();
    let capture = args.next().as_deref() == Some("capture");
    let hold: u64 = args.next().and_then(|ms| ms.parse().ok()).unwrap_or(0);
    println!("Drivers: {:?}", list_hosts());
    for device in list_output_devices(host.as_deref())? {
        println!(
            "{}{}: {:?} Hz, buffers {:?}",
            if device.is_default { "* " } else { "  " },
            device.name,
            device.supported_sample_rates,
            device.buffer_frames
        );
    }
    let directory = std::env::temp_dir().join("rill-device-smoke");
    std::fs::create_dir_all(&directory)?;
    let tone = directory.join("tone.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..192_000)
            .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin() * 0.1)
            .collect(),
    }
    .write_wav(&tone)?;
    let mut project = Project::new();
    let playback = project.add_audio_track("Smoke test");
    project
        .track_mut(playback)
        .expect("new track")
        .clips
        .push(Clip::audio("Tone", 0.0, 8.0, tone.to_string_lossy()));
    let mut settings = DeviceSettings {
        host: host.clone(),
        ..DeviceSettings::default()
    };
    if let Some(device) = list_output_devices(host.as_deref())?.first() {
        // A server-dictated driver offers exactly one rate and buffer size.
        if let [rate] = device.supported_sample_rates[..] {
            settings.sample_rate = rate;
        }
        if let Some((min, max)) = device.buffer_frames {
            settings.buffer_frames = settings.buffer_frames.clamp(min, max);
        }
    }
    let mut recording = None;
    if capture {
        let id = project.add_audio_track("Capture");
        println!("Inputs: {:?}", list_input_devices(host.as_deref())?);
        recording = Some(id);
    }
    let mut engine = AudioEngine::start(&project, settings)?;
    thread::sleep(Duration::from_millis(30));
    if let Some(track) = recording {
        engine.configure_input(
            &project,
            InputSettings {
                device_name: None,
                routes: vec![InputRoute {
                    track,
                    channels: AudioInputChannels::Mono { channel: 0 },
                    monitor: false,
                }],
            },
        )?;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !engine.recording_snapshot().input_ready {
            engine.poll_input();
            anyhow::ensure!(std::time::Instant::now() < deadline, "Input did not open");
            thread::sleep(Duration::from_millis(10));
        }
        engine.start_recording(RecordingConfig {
            directory: directory.clone(),
            tracks: vec![track],
        })?;
    }
    engine.set_playing(true);
    thread::sleep(Duration::from_millis(
        if capture { 500 } else { 250 } + hold,
    ));
    let snapshot = engine.snapshot();
    if recording.is_some() {
        engine.request_stop_recording();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            engine.poll_input();
            if let Some(result) = engine.take_recording_result() {
                println!("Capture: {result:?}");
                anyhow::ensure!(result.error.is_none(), "Capture reported an error");
                anyhow::ensure!(
                    result.takes.iter().any(|take| take.frames > 0),
                    "Capture recorded no frames"
                );
                break;
            }
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "Capture did not finish"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    engine.stop();
    println!("Device stream: {snapshot:?}");
    anyhow::ensure!(
        snapshot.position_beats > 0.0,
        "Device callback did not advance playback"
    );
    anyhow::ensure!(
        snapshot.peak_left > 0.0,
        "Device callback did not receive rendered audio"
    );
    anyhow::ensure!(
        snapshot.render_errors == 0,
        "Device reported an output or rendering error"
    );
    Ok(())
}

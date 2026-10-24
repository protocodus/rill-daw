//! Enumerate devices and play a quiet 250 ms verification tone on the default output.
use daw_core::{Clip, Note, Project, Track, TrackKind};
use daw_engine::{AudioEngine, DeviceSettings, list_output_devices};
use std::{thread, time::Duration};

fn main() -> anyhow::Result<()> {
    for device in list_output_devices()? {
        println!(
            "{}{}: {:?} Hz",
            if device.is_default { "* " } else { "  " },
            device.name,
            device.supported_sample_rates
        );
    }
    let mut project = Project::new();
    let mut track = Track::new("Smoke test", TrackKind::Midi);
    track.gain = 0.08;
    track.clips.push(Clip::midi(
        "Tone",
        0.0,
        1.0,
        vec![Note::new(69, 0.0, 1.0, 80)],
    ));
    project.tracks.push(track);
    let mut engine = AudioEngine::start(&project, DeviceSettings::default())?;
    thread::sleep(Duration::from_millis(30));
    engine.set_playing(true);
    thread::sleep(Duration::from_millis(250));
    let snapshot = engine.snapshot();
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

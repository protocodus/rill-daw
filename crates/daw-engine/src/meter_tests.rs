use super::*;
use crate::meter::ChannelMeters;
use daw_core::{AutomationPoint, AutomationTarget, Clip, Interpolation};

fn fixture() -> (tempfile::TempDir, Project, [u64; 5]) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("channels.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000).flat_map(|_| [0.8, 0.4]).collect(),
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let lead = project.add_audio_track("Lead");
    let bass = project.add_audio_track("Bass");
    let muted = project.add_audio_track("Muted");
    let bus = project.add_bus("Group");
    let send = project.add_bus("Pre-fader return");
    for id in [lead, bass, muted] {
        project.track_mut(id).unwrap().clips.push(Clip::audio(
            "Source",
            0.0,
            2.0,
            path.to_string_lossy(),
        ));
    }
    project.track_mut(lead).unwrap().gain = 0.5;
    project.track_mut(lead).unwrap().pan = -0.5;
    project.track_mut(bass).unwrap().gain = 0.25;
    project.track_mut(bass).unwrap().pan = 1.0;
    project.track_mut(muted).unwrap().mute = true;
    project.track_mut(bus).unwrap().gain = 0.5;
    project.track_mut(bus).unwrap().pan = -0.25;
    project.track_mut(send).unwrap().gain = 0.5;
    project.set_output(lead, Some(bus)).unwrap();
    project.set_output(bass, Some(bus)).unwrap();
    project.add_send(lead, send, 0.25, true).unwrap();
    (directory, project, [lead, bass, muted, bus, send])
}

fn assert_peaks(actual: [f32; 2], expected: [f32; 2]) {
    for channel in 0..2 {
        assert!(
            (actual[channel] - expected[channel]).abs() < 1e-6,
            "channel {channel}: {actual:?} != {expected:?}"
        );
    }
}

#[test]
fn channel_meters_follow_own_fader_pan_bus_and_send_without_changing_audio() {
    let (_directory, project, [lead, bass, muted, bus, send]) = fixture();
    for workers in [1, 2] {
        let meters = ChannelMeters::new(&project);
        let mut metered = Renderer::new_with_workers(&project, 48_000, false, workers).unwrap();
        metered.attach_channel_meters(&meters);
        let mut reference = Renderer::new_with_workers(&project, 48_000, false, workers).unwrap();
        let mut output = [[0.0; 2]; BLOCK];
        let mut positions = [0.0; BLOCK];
        let mut dry = output;
        let mut dry_positions = positions;
        // Prime persistent worker-channel bookkeeping before measuring the
        // steady-state dispatcher, as the existing allocation suite does.
        metered.render(&mut output, &mut positions).unwrap();
        reference.render(&mut dry, &mut dry_positions).unwrap();
        assert_eq!(output, dry);
        for _ in 0..4 {
            assert_eq!(
                crate::allocation_test_support::count_allocations(|| {
                    metered.render(&mut output, &mut positions).unwrap();
                }),
                0,
                "metering must not allocate during processing"
            );
            reference.render(&mut dry, &mut dry_positions).unwrap();
            assert_eq!(output, dry);
            assert_eq!(positions, dry_positions);
            assert_peaks(meters.peaks(lead), [0.4, 0.1]);
            assert_peaks(meters.peaks(bass), [0.0, 0.1]);
            assert_peaks(meters.peaks(bus), [0.2, 0.075]);
            assert_peaks(meters.peaks(send), [0.1, 0.05]);
            assert_eq!(meters.peaks(muted), [0.0; 2]);
            assert_eq!(meters.peaks(u64::MAX), [0.0; 2]);
            assert_ne!(
                meters.peaks(lead),
                output[0],
                "individual channels must not mirror master"
            );
        }
        metered.seek(3.0).unwrap();
        for id in [lead, bass, muted, bus, send] {
            assert_eq!(meters.peaks(id), [0.0; 2], "seek clears stale peaks");
        }
        metered.render(&mut output, &mut positions).unwrap();
        assert_eq!(meters.peaks(lead), [0.0; 2]);
    }
}

#[test]
fn channel_meter_uses_automated_fader_and_respects_solo_selection() {
    let (_directory, mut project, [lead, bass, muted, bus, send]) = fixture();
    project.track_mut(lead).unwrap().solo = true;
    let parameter = AutomationParameter::TrackGain;
    let lane = project
        .add_automation_track(
            "Gain",
            AutomationTarget {
                track: lead,
                parameter,
            },
        )
        .unwrap();
    let automation = project.automation_lane_mut(lane).unwrap();
    automation.interpolation = Interpolation::Step;
    automation.points = vec![AutomationPoint::new(0.0, parameter.normalize(0.75))];
    let folder = project.add_folder("Folder");
    let meters = ChannelMeters::new(&project);
    assert!(meters.channel(lane).is_none());
    assert!(meters.channel(folder).is_none());
    let mut renderer = Renderer::new_with_workers(&project, 48_000, false, 2).unwrap();
    renderer.attach_channel_meters(&meters);
    renderer
        .render(&mut [[0.0; 2]; BLOCK], &mut [0.0; BLOCK])
        .unwrap();
    assert_peaks(meters.peaks(lead), [0.6, 0.15]);
    assert_eq!(meters.peaks(bass), [0.0; 2]);
    assert_eq!(meters.peaks(muted), [0.0; 2]);
    assert_peaks(meters.peaks(bus), [0.3, 0.05625]);
    assert_peaks(meters.peaks(send), [0.1, 0.05]);
}

#[test]
fn failed_channel_clears_meter_before_subsequent_blocks() {
    let (_directory, project, [lead, ..]) = fixture();
    let meters = ChannelMeters::new(&project);
    let mut renderer = Renderer::new(&project, 48_000, false).unwrap();
    renderer.attach_channel_meters(&meters);
    renderer
        .render(&mut [[0.0; 2]; BLOCK], &mut [0.0; BLOCK])
        .unwrap();
    assert!(meters.peaks(lead)[0] > 0.0);
    let track = renderer
        .tracks
        .iter_mut()
        .flatten()
        .find(|track| track.id == lead)
        .unwrap();
    finish_processing(track, Err(anyhow::anyhow!("isolated plugin failed")), true).unwrap();
    assert_eq!(meters.peaks(lead), [0.0; 2]);
    renderer
        .render(&mut [[0.0; 2]; BLOCK], &mut [0.0; BLOCK])
        .unwrap();
    assert_eq!(meters.peaks(lead), [0.0; 2]);
}

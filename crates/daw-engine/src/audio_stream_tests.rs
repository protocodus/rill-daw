//! Large sparse fixtures exercise actual file windows without allocating source PCM.
use super::*;
use daw_core::{AUDIO_STREAM_THRESHOLD_BYTES, AudioFile, Clip};
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
};
const RATE: u32 = 48000;
fn source() -> (tempfile::TempDir, std::path::PathBuf, usize) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("long.wav");
    let frames = (AUDIO_STREAM_THRESHOLD_BYTES / 8) as usize + 17;
    let bytes = (frames * 8) as u32;
    let mut file = File::create(&path).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(bytes + 36).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&3u16.to_le_bytes()).unwrap();
    file.write_all(&2u16.to_le_bytes()).unwrap();
    file.write_all(&RATE.to_le_bytes()).unwrap();
    file.write_all(&(RATE * 8).to_le_bytes()).unwrap();
    file.write_all(&8u16.to_le_bytes()).unwrap();
    file.write_all(&32u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&bytes.to_le_bytes()).unwrap();
    file.set_len(44 + u64::from(bytes)).unwrap();
    for (frame, value) in [
        (0, 0.2_f32),
        (4095, 0.3),
        (4096, -0.4),
        (8191, 0.6),
        (8192, -0.7),
        (2_000_000, 0.8),
        (frames - 1, 0.9),
    ] {
        file.seek(SeekFrom::Start(44 + frame as u64 * 8)).unwrap();
        file.write_all(&value.to_le_bytes()).unwrap();
        file.write_all(&(-value).to_le_bytes()).unwrap();
    }
    file.sync_all().unwrap();
    assert!(AudioFile::open(&path).unwrap().info().should_stream());
    (directory, path, frames)
}
fn project(path: &Path, frames: usize) -> (Project, u64, u64) {
    let mut project = Project::new();
    let track = project.add_audio_track("Disk source");
    let clip = Clip::audio(
        "Large take",
        0.0,
        frames as f64 / 24000.0,
        path.to_string_lossy(),
    );
    let id = clip.id;
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(clip);
    (project, track, id)
}
fn block(renderer: &mut Renderer, frames: usize) -> Vec<[f32; 2]> {
    let mut audio = vec![[0.0; 2]; frames];
    renderer.render(&mut audio, &mut vec![0.0; frames]).unwrap();
    audio
}
fn near(actual: [f32; 2], expected: f32) {
    assert!(
        (actual[0] - expected).abs() < 1e-6,
        "{actual:?} != {expected}"
    );
    assert!((actual[1] + expected).abs() < 1e-6);
}
#[test]
fn disk_windows_seek_reverse_and_loop_without_whole_file_pcm() {
    let (_directory, path, frames) = source();
    let (mut project, track, id) = project(&path, frames);
    let mut renderer = Renderer::new(&project, RATE, true).unwrap();
    assert_eq!(renderer.streamed_clips().as_slice(), &[(track, id)]);
    let stats = renderer.audio_streaming_snapshot();
    assert_eq!(stats.resident_bytes, 0);
    assert!(stats.window_bytes >= 2 * crate::audio_stream::WINDOW_BYTES as u64);
    for (frame, value) in [
        (0, 0.2),
        (4095, 0.3),
        (4096, -0.4),
        (8191, 0.6),
        (8192, -0.7),
        (2_000_000, 0.8),
    ] {
        renderer.seek(frame as f64 / 24000.0).unwrap();
        near(block(&mut renderer, 1)[0], value);
    }
    let stats = renderer.audio_streaming_snapshot();
    assert!(stats.disk_read_operations > 0);
    assert!(
        stats.disk_frames_read < frames as u64 / 10,
        "Only requested bounded windows are read"
    );
    if let ClipContent::Audio { reverse, .. } =
        &mut project.track_mut(track).unwrap().clips[0].content
    {
        *reverse = true;
    }
    let mut reverse = Renderer::new(&project, RATE, false).unwrap();
    near(block(&mut reverse, 1)[0], 0.9);
    let mut looped = project.clone();
    if let ClipContent::Audio { reverse, .. } =
        &mut looped.track_mut(track).unwrap().clips[0].content
    {
        *reverse = false;
    }
    looped.loop_enabled = true;
    looped.loop_start = 8191.0 / 24000.0;
    looped.loop_end = 8193.0 / 24000.0;
    let mut renderer = Renderer::new(&looped, RATE, true).unwrap();
    renderer.seek(looped.loop_start).unwrap();
    let audio = block(&mut renderer, 8);
    for pair in audio.as_chunks::<2>().0 {
        near(pair[0], 0.6);
        near(pair[1], -0.7);
    }
}
#[test]
fn streamed_live_edit_reuses_graph_and_retains_old_new_windows_for_bridge() {
    let (_directory, path, frames) = source();
    let (mut project, track, _) = project(&path, frames);
    let mut renderer = Renderer::new(&project, RATE, true).unwrap();
    block(&mut renderer, 256);
    let mut preparer = renderer.live_edit_preparer();
    project.track_mut(track).unwrap().clips[0].offset = 8192.0 / 24000.0;
    let mut edit = preparer.prepare(&project).unwrap();
    renderer.apply_live_edit(&mut edit).unwrap();
    renderer.seek(0.0).unwrap();
    near(block(&mut renderer, 1)[0], -0.7);
    assert_eq!(renderer.audio_streaming_snapshot().resident_bytes, 0);
}
#[test]
fn changed_streamed_source_fails_offline_without_replacing_previous_export() {
    let (_directory, path, frames) = source();
    let (project, _, _) = project(&path, frames);
    let mut renderer =
        Renderer::new_offline_with_options(&project, RATE, RenderOptions::default()).unwrap();
    block(&mut renderer, 256);
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(44 + 1024)
        .unwrap();
    let error = renderer
        .render(&mut [[0.0; 2]; 256], &mut [0.0; 256])
        .unwrap_err();
    assert!(format!("{error:#}").contains("changed"));
    let output = path.with_file_name("keep.wav");
    std::fs::write(&output, b"existing output").unwrap();
    assert!(export_project_wav(&project, &output, RATE).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"existing output");
}

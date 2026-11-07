use daw_core::{
    AUDIO_FILE_READ_MAX_BYTES, AUDIO_STREAM_THRESHOLD_BYTES, AudioData, AudioFile,
    cached_audio_file_handles,
};
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    sync::Arc,
};

#[test]
fn positional_readers_match_resident_pcm_and_have_independent_cursors() {
    let directory = tempfile::tempdir().unwrap();
    for bits in [8, 16, 24, 32] {
        let path = directory.path().join(format!("pcm-{bits}.wav"));
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48_000,
                bits_per_sample: bits,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for frame in 0..257_i32 {
            let value = ((frame % 63) - 31) << (bits - 8);
            writer.write_sample(value).unwrap();
            writer.write_sample(-value).unwrap();
        }
        writer.finalize().unwrap();
        let resident = Arc::new(AudioData::load_wav(&path).unwrap());
        let file = AudioFile::open(&path).unwrap();
        assert!(!file.info().should_stream());
        assert_eq!(file.info().frames, resident.frames());
        let threads: Vec<_> = (0..4)
            .map(|offset| {
                let file = file.clone();
                let resident = Arc::clone(&resident);
                std::thread::spawn(move || {
                    let mut reader = file.reader().unwrap();
                    for step in 0..35 {
                        let start = (step * 71 + offset * 31) % 257;
                        let mut block = [123.0; 38];
                        let count = reader.read_frames(start, &mut block).unwrap();
                        assert_eq!(count, 19.min(257 - start));
                        assert_eq!(
                            &block[..count * 2],
                            &resident.samples[start * 2..(start + count) * 2]
                        );
                        assert!(block[count * 2..].iter().all(|sample| *sample == 0.0));
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }
}

#[test]
fn large_sparse_float_source_supports_far_seeks_without_resident_pcm() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("long.wav");
    let bytes = AUDIO_STREAM_THRESHOLD_BYTES + 8;
    let frames = bytes as usize / 8;
    let mut file = File::create(&path).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&((bytes + 36) as u32).to_le_bytes())
        .unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16_u32.to_le_bytes()).unwrap();
    file.write_all(&3_u16.to_le_bytes()).unwrap();
    file.write_all(&2_u16.to_le_bytes()).unwrap();
    file.write_all(&48_000_u32.to_le_bytes()).unwrap();
    file.write_all(&(48_000_u32 * 8).to_le_bytes()).unwrap();
    file.write_all(&8_u16.to_le_bytes()).unwrap();
    file.write_all(&32_u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&(bytes as u32).to_le_bytes()).unwrap();
    file.set_len(bytes + 44).unwrap();
    file.seek(SeekFrom::Start(bytes + 44 - 8)).unwrap();
    file.write_all(&0.75_f32.to_le_bytes()).unwrap();
    file.write_all(&(-0.25_f32).to_le_bytes()).unwrap();
    file.sync_all().unwrap();
    let source = AudioFile::open(path).unwrap();
    assert!(source.info().should_stream());
    assert_eq!(source.info().frames, frames);
    let mut reader = source.reader().unwrap();
    let mut block = [1.0; 8];
    assert_eq!(reader.read_frames(frames - 2, &mut block).unwrap(), 2);
    assert_eq!(block, [0.0, 0.0, 0.75, -0.25, 0.0, 0.0, 0.0, 0.0]);
    assert_eq!(reader.read_frames(0, &mut block).unwrap(), 4);
    assert_eq!(block, [0.0; 8]);
}

#[test]
fn changed_or_truncated_sources_never_return_stale_buffered_success() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("changed.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.25; 256],
    }
    .write_wav(&path)
    .unwrap();
    let source = AudioFile::open(&path).unwrap();
    let mut reader = source.reader().unwrap();
    let mut block = [0.0; 4];
    reader.read_frames(0, &mut block).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(16)
        .unwrap();
    assert!(source.check_unchanged().is_err());
    assert!(reader.read_frames(0, &mut block).is_err());
}

#[test]
fn nonfinite_audio_and_unbounded_window_requests_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("nonfinite.wav");
    let mut writer = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    writer.write_sample(f32::NAN).unwrap();
    writer.finalize().unwrap();
    let mut reader = AudioFile::open(path).unwrap().reader().unwrap();
    assert!(reader.read_frames(0, &mut [0.0]).is_err());
    let mut too_large = vec![0.0; AUDIO_FILE_READ_MAX_BYTES / 4 + 1];
    assert!(reader.read_frames(0, &mut too_large).is_err());
}

#[test]
fn many_descriptors_and_readers_use_a_bounded_handle_cache() {
    let directory = tempfile::tempdir().unwrap();
    let mut readers = Vec::new();
    for index in 0..80 {
        let path = directory.path().join(format!("{index}.wav"));
        AudioData {
            sample_rate: 48_000,
            channels: 1,
            samples: vec![index as f32; 128],
        }
        .write_wav(&path)
        .unwrap();
        let file = AudioFile::open(path).unwrap();
        readers.push(file.reader().unwrap());
        assert!(cached_audio_file_handles() <= 32);
    }
    for (index, reader) in readers.iter_mut().enumerate() {
        let mut sample = [0.0];
        assert_eq!(reader.read_frames(100, &mut sample).unwrap(), 1);
        assert_eq!(sample, [index as f32]);
    }
    assert!(cached_audio_file_handles() <= 32);
}

#[test]
fn metadata_admission_rejects_huge_unknown_header_chunks() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("huge-header.wav");
    let junk = 8 * 1024 * 1024_u32;
    let mut file = File::create(&path).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(junk + 48).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16_u32.to_le_bytes()).unwrap();
    file.write_all(&3_u16.to_le_bytes()).unwrap();
    file.write_all(&1_u16.to_le_bytes()).unwrap();
    file.write_all(&48_000_u32.to_le_bytes()).unwrap();
    file.write_all(&192_000_u32.to_le_bytes()).unwrap();
    file.write_all(&4_u16.to_le_bytes()).unwrap();
    file.write_all(&32_u16.to_le_bytes()).unwrap();
    file.write_all(b"JUNK").unwrap();
    file.write_all(&junk.to_le_bytes()).unwrap();
    file.seek(SeekFrom::Start(44 + u64::from(junk))).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&4_u32.to_le_bytes()).unwrap();
    file.write_all(&0.5_f32.to_le_bytes()).unwrap();
    file.sync_all().unwrap();
    let error = AudioFile::open(path).unwrap_err();
    assert!(format!("{error:#}").contains("header exceeds"), "{error:#}");
}

#[cfg(unix)]
#[test]
fn metadata_admission_rejects_fifos_without_waiting_for_a_writer() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("not-a-sample.wav");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        sender.send(AudioFile::open(path).is_err()).unwrap();
    });
    assert!(
        receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap()
    );
    worker.join().unwrap();
}

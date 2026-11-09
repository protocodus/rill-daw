//! Standalone test-only sandbox worker. The application speaks its production
//! process-isolation protocol to this deterministic instrument/delay fixture.
//! This is not a native VST compatibility test and is never shipped with Rill.
use std::io::{BufRead, Write};

fn number(line: &str, name: &str) -> f64 {
    line.split_once(&format!("\"{name}\":"))
        .unwrap()
        .1
        .split([',', '}'])
        .next()
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
fn encode(samples: &[f32]) -> String {
    let bytes: Vec<_> = samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(chunk.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}
fn decode(encoded: &str) -> Vec<f32> {
    let mut bytes = Vec::new();
    for chunk in encoded.as_bytes().as_chunks::<4>().0 {
        let digit = |v| match v {
            b'A'..=b'Z' => u32::from(v - b'A'),
            b'a'..=b'z' => u32::from(v - b'a' + 26),
            b'0'..=b'9' => u32::from(v - b'0' + 52),
            b'+' => 62,
            b'/' => 63,
            b'=' => 0,
            _ => panic!("Invalid fixture audio encoding"),
        };
        let n = (digit(chunk[0]) << 18)
            | (digit(chunk[1]) << 12)
            | (digit(chunk[2]) << 6)
            | digit(chunk[3]);
        bytes.push((n >> 16) as u8);
        if chunk[2] != b'=' {
            bytes.push((n >> 8) as u8);
        }
        if chunk[3] != b'=' {
            bytes.push(n as u8);
        }
    }
    bytes
        .as_chunks::<4>().0.iter()
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect()
}
#[derive(Clone, Copy)]
struct Voice {
    velocity: f32,
    channel: i16,
    note_id: Option<i32>,
}
#[derive(Clone, Copy)]
struct Event {
    offset: usize,
    pitch: u8,
    velocity: f32,
    on: bool,
    channel: i16,
    note_id: Option<i32>,
}
fn main() {
    assert_eq!(
        std::env::args().len(),
        1,
        "No editor or metadata fixture mode"
    );
    let log_path = std::env::current_exe()
        .unwrap()
        .with_file_name(format!("worker-{}.jsonl", std::process::id()));
    let mut log = std::fs::File::create(log_path).unwrap();
    let mut output = std::io::stdout().lock();
    let mut effect = false;
    let mut voices: [Vec<Voice>; 128] = std::array::from_fn(|_| Vec::new());
    let mut synth_value = 0.0;
    let mut events: Vec<Event> = Vec::new();
    let mut delay = [[0.0_f32; 2]; 32];
    let mut delay_cursor = 0;
    let mut processed = 0_u64;
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        if !line.contains("\"ProcessBuses\"") {
            writeln!(log, "{line}").unwrap();
        }
        if line == "\"Shutdown\"" {
            break;
        }
        let response = if line.contains("\"LoadPlugin\"") {
            effect = line.contains("33333333333333333333333333333333");
            format!(
                r#"{{"PluginInfo":{{"vendor":"Rill tests","name":"Live edit fixture","version":"1","category":"{}","uid":"{}","has_gui":false,"audio_inputs":{},"audio_outputs":1,"output_channels":2,"has_midi_input":{},"has_midi_output":false,"compatibility":[]}}}}"#,
                if effect { "Fx" } else { "Instrument|Synth" },
                if effect {
                    "33333333333333333333333333333333"
                } else {
                    "11111111111111111111111111111111"
                },
                if effect { 1 } else { 0 },
                !effect
            )
        } else if line == "\"ProcessingCapabilities\"" {
            // This fixture implements only the legacy per-command protocol.
            r#"{"ProcessingCapabilities":{"block_processing":false,"shared_audio":false}}"#
                .to_owned()
        } else if line == "\"AudioBusLayout\"" {
            format!(
                r#"{{"AudioBusLayout":{{"layout":{{"inputs":{},"outputs":[{{"channel_count":2,"active":true}}]}}}}}}"#,
                if effect {
                    r#"[{"channel_count":2,"active":true}]"#
                } else {
                    "[]"
                }
            )
        } else if line == "\"AudioBusDetails\"" {
            format!(
                r#"{{"AudioBusDetails":{{"details":{{"inputs":{},"outputs":[{{"name":"Main output","channels":2,"active":true,"main":true}}]}}}}}}"#,
                if effect {
                    r#"[{"name":"Main input","channels":2,"active":true,"main":true}]"#
                } else {
                    "[]"
                }
            )
        } else if line.contains("\"NoteExpressions\"") {
            r#"{"NoteExpressions":{"expressions":[]}}"#.to_owned()
        } else if line.contains("\"MidiControllerAssignments\"") {
            r#"{"MidiControllerAssignments":{"assignments":[]}}"#.to_owned()
        } else if matches!(
            line.as_str(),
            "\"ServiceHostRequests\"" | "\"TakeRestartFlags\""
        ) {
            r#"{"RestartFlags":{"bits":0}}"#.to_owned()
        } else if line == "\"BusArrangements\"" {
            format!(
                r#"{{"BusArrangements":{{"arrangements":{{"inputs":{},"outputs":[3]}}}}}}"#,
                if effect { "[3]" } else { "[]" }
            )
        } else if line == "\"LatencySamples\"" {
            format!(
                r#"{{"LatencySamples":{{"samples":{}}}}}"#,
                if effect { 32 } else { 0 }
            )
        } else if line == "\"TailSamples\"" {
            format!(
                r#"{{"TailSamples":{{"samples":{}}}}}"#,
                if effect { 32 } else { 0 }
            )
        } else if line == "\"SaveState\"" {
            r#"{"State":{"data":[]}}"#.to_owned()
        } else if line == "\"GetAllParameters\"" {
            r#"{"Parameters":{"params":[]}}"#.to_owned()
        } else if line.contains("\"SendMidiAt\"") {
            events.push(Event {
                offset: number(&line, "sample_offset") as usize,
                pitch: number(&line, "note") as u8,
                velocity: number(&line, "velocity") as f32,
                on: line.contains("\"NoteOn\""),
                channel: 0,
                note_id: None,
            });
            r#"{"Success":{"message":""}}"#.to_owned()
        } else if line.contains("\"SendPluginEvent\"") {
            if line.contains("\"NoteOn\"") || line.contains("\"NoteOff\"") {
                // Native note velocities are normalized floats. Preserve their
                // precision while retaining the fixture's historical DC scale.
                events.push(Event {
                    offset: number(&line, "sample_offset") as usize,
                    pitch: number(&line, "pitch") as u8,
                    velocity: number(&line, "velocity") as f32 * 127.0,
                    on: line.contains("\"NoteOn\""),
                    channel: number(&line, "channel") as i16,
                    note_id: Some(number(&line, "note_id") as i32),
                });
            }
            r#"{"Success":{"message":""}}"#.to_owned()
        } else if line == "\"MidiPanic\"" {
            voices.iter_mut().for_each(Vec::clear);
            synth_value = 0.0;
            r#"{"Success":{"message":""}}"#.to_owned()
        } else if line.contains("\"ProcessBuses\"") {
            let frames = number(&line, "frames") as usize;
            let inputs = if effect {
                let channels = line
                    .split_once("\"channels\":[")
                    .unwrap()
                    .1
                    .split(']')
                    .next()
                    .unwrap();
                channels
                    .split(',')
                    .map(|s| decode(s.trim_matches('"')))
                    .collect::<Vec<_>>()
            } else {
                vec![]
            };
            let mut left = Vec::with_capacity(frames);
            let mut right = Vec::with_capacity(frames);
            for frame in 0..frames {
                let mut changed = false;
                for event in events.iter().filter(|event| event.offset == frame) {
                    let active = &mut voices[usize::from(event.pitch)];
                    if event.on && (event.note_id.is_some() || event.velocity > 0.0) {
                        active.push(Voice {
                            velocity: event.velocity,
                            channel: event.channel,
                            note_id: event.note_id,
                        });
                    } else if let Some(at) = active.iter().position(|voice| {
                        voice.channel == event.channel
                            && (event.note_id.is_none() || voice.note_id == event.note_id)
                    }) {
                        active.remove(at);
                    }
                    changed = true;
                }
                // This fixture produces DC, so the exact sum changes only at
                // MIDI events. Rescanning 128 voice vectors for every sample
                // needlessly made debug fixtures contend for the 100 ms IPC
                // deadline while several live-edit scenarios ran together.
                if changed && !effect {
                    synth_value = voices
                        .iter()
                        .enumerate()
                        .flat_map(|(pitch, velocities)| {
                            velocities.iter().map(move |voice| {
                                (0.2 + (pitch as f32 - 60.0) * 0.01) * voice.velocity / 100.0
                            })
                        })
                        .sum::<f32>();
                }
                let stereo = if effect {
                    let previous = delay[delay_cursor];
                    delay[delay_cursor] = [inputs[0][frame] * 0.5, inputs[1][frame] * 0.5];
                    delay_cursor = (delay_cursor + 1) % delay.len();
                    previous
                } else {
                    [synth_value, synth_value]
                };
                left.push(stereo[0]);
                right.push(stereo[1]);
            }
            events.clear();
            processed += frames as u64;
            let active: Vec<_> = voices
                .iter()
                .enumerate()
                .filter_map(|(pitch, v)| (!v.is_empty()).then_some(pitch))
                .collect();
            writeln!(
                log,
                r#"{{"processed":{processed},"active":{active:?},"effect":{effect}}}"#
            )
            .unwrap();
            format!(
                r#"{{"BusAudioOutput":{{"outputs":[{{"active":true,"channels":["{}","{}"]}}],"output_events":[]}}}}"#,
                encode(&left),
                encode(&right)
            )
        } else {
            r#"{"Success":{"message":""}}"#.to_owned()
        };
        log.flush().unwrap();
        writeln!(output, "{response}").unwrap();
        output.flush().unwrap();
    }
}

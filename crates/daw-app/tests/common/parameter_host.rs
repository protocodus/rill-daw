//! Test-only metadata/state worker, compiled into a temporary directory by the
//! parameter browser E2E. It exercises the real child-process protocol, not DSP
//! or a native VST editor. It is never linked into or distributed with Rill.
use std::io::{BufRead, Write};

fn number(line: &str, name: &str) -> Option<f64> {
    line.split_once(&format!("\"{name}\":"))?
        .1
        .split([',', '}'])
        .next()?
        .trim()
        .parse()
        .ok()
}

fn state_value(line: &str) -> Option<f64> {
    let data = line.split_once("\"data\":\"")?.1.split('"').next()?;
    let digit = |byte: u8| match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let bytes = data.as_bytes();
    Some(f64::from((digit(*bytes.first()?)? << 2) | (digit(*bytes.get(1)?)? >> 4)) / 100.0)
}

fn main() {
    if std::env::args().len() > 1 {
        eprintln!("The parameter E2E fixture has no native editor");
        std::process::exit(2);
    }
    let mut values = std::collections::HashMap::<u32, f64>::new();
    let mut effect = false;
    let log = std::env::current_exe()
        .unwrap()
        .with_file_name("requests.log");
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .unwrap();
    let mut output = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        writeln!(log, "{line}").unwrap();
        if line == "\"Shutdown\"" {
            break;
        }
        let response = if line.contains("\"LoadPlugin\"") {
            effect = line.contains("33333333333333333333333333333333");
            if line.contains("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF") {
                r#"{"Error":{"message":"Fixture metadata unavailable"}}"#.to_owned()
            } else {
                format!(
                    r#"{{"PluginInfo":{{"vendor":"Rill tests","name":"Parameter Fixture","version":"1","category":"{}","uid":"{}","has_gui":false,"audio_inputs":{},"audio_outputs":1,"output_channels":2,"has_midi_input":{},"has_midi_output":false,"compatibility":[]}}}}"#,
                    if effect { "Fx" } else { "Instrument|Synth" },
                    if effect {
                        "33333333333333333333333333333333"
                    } else {
                        "11111111111111111111111111111111"
                    },
                    if effect { 1 } else { 0 },
                    !effect
                )
            }
        } else if line == "\"AudioBusLayout\"" {
            format!(
                r#"{{"AudioBusLayout":{{"layout":{{"inputs":{},"outputs":[{{"channel_count":2,"active":true}}]}}}}}}"#,
                if effect {
                    r#"[{"channel_count":2,"active":true}]"#
                } else {
                    "[]"
                }
            )
        } else if line == "\"LatencySamples\"" {
            r#"{"LatencySamples":{"samples":0}}"#.to_owned()
        } else if line == "\"TailSamples\"" {
            r#"{"TailSamples":{"samples":0}}"#.to_owned()
        } else if line.contains("\"LoadState\"") {
            if let Some(value) = state_value(&line) {
                values.insert(7, value);
            }
            r#"{"Success":{"message":""}}"#.to_owned()
        } else if line.contains("\"SetParameter\"") {
            let id = number(&line, "id").unwrap() as u32;
            values.insert(id, number(&line, "value").unwrap());
            r#"{"Success":{"message":""}}"#.to_owned()
        } else if line.contains("\"GetParameter\"") {
            let id = number(&line, "id").unwrap() as u32;
            let value = values.get(&id).copied().unwrap_or(if id == 44 {
                0.37
            } else if effect {
                0.25
            } else {
                0.5
            });
            format!(r#"{{"ParameterValue":{{"value":{value}}}}}"#)
        } else if line == "\"GetAllParameters\"" {
            let mut definitions = vec![
                (7, "Filter Cutoff".to_owned(), "Hz", false, 0),
                (8, "Filter Resonance".to_owned(), "%", false, 0),
                (9, "Envelope Attack".to_owned(), "ms", false, 0),
                (10, "Envelope Release".to_owned(), "ms", false, 0),
                (42, "Delay Feedback".to_owned(), "%", false, 0),
                (43, "Output Meter".to_owned(), "dB", true, 0),
                (44, "Oscillator Mode".to_owned(), "", false, 4),
            ];
            for index in 0..200 {
                definitions.push((1000 + index, format!("Macro {index:03}"), "%", false, 0));
            }
            definitions.push((9000, "Ω Motion Depth 🎛".to_owned(), "%", false, 0));
            let parameters = definitions.into_iter().map(|(id, name, unit, read_only, steps)| {
                let value = values.get(&id).copied().unwrap_or(if id == 44 {0.37} else if effect {0.25} else {0.5});
                format!(r#"{{"id":{id},"name":"{name}","value":{value},"min":0,"max":1,"default":0.5,"unit":"{unit}","step_count":{steps},"can_automate":{},"is_read_only":{read_only},"is_bypass":false,"flags":0}}"#, !read_only)
            }).collect::<Vec<_>>().join(",");
            format!(r#"{{"Parameters":{{"params":[{parameters}]}}}}"#)
        } else if line == "\"SaveState\"" {
            r#"{"State":{"data":[]}}"#.to_owned()
        } else {
            r#"{"Success":{"message":""}}"#.to_owned()
        };
        writeln!(output, "{response}").unwrap();
        output.flush().unwrap();
    }
}

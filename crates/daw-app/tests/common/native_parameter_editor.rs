// Test-only native-editor protocol actor. App tests compile this alongside the
// metadata worker into a temporary executable; no platform window is created.
fn main() {
    if std::env::args().any(|argument| argument == "--editor") {
        editor_main();
    } else {
        metadata_main();
    }
}

fn editor_main() {
    use std::{sync::mpsc, time::Duration};
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let executable = std::env::current_exe().unwrap();
    let prefix = format!("editor-{}", std::process::id());
    let actions = executable.with_file_name(format!("{prefix}.actions"));
    let handled = executable.with_file_name(format!("{prefix}.handled"));
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(executable.with_file_name(format!("{prefix}.log")))
        .unwrap();
    let mut values = std::collections::HashMap::from([(0, 0.4), (7, 0.5), (43, 0.1)]);
    let mut output = std::io::stdout().lock();
    let mut ready = false;
    loop {
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok(command) => {
                writeln!(log, "{command}").unwrap();
                if command.contains("\"command\":\"open\"") {
                    if let Some(state) = command.split_once("\"state\":[").map(|(_, state)| state)
                        && let Some(value) = state
                            .split([',', ']'])
                            .next()
                            .and_then(|value| value.parse::<u8>().ok())
                    {
                        values.insert(0, f64::from(value) / 100.0);
                    }
                    editor_ready(&mut output, &values);
                    ready = true;
                } else if command.contains("\"command\":\"set_parameter\"") {
                    let id = number(&command, "id").unwrap() as u32;
                    values.insert(id, number(&command, "value").unwrap());
                } else if command.contains("\"command\":\"save_state\"")
                    || command.contains("\"command\":\"close\"")
                {
                    let request = number(&command, "request_id")
                        .map(|id| (id as u64).to_string())
                        .unwrap_or("null".into());
                    let state = (values[&0] * 100.0).round() as u8;
                    writeln!(
                        output,
                        r#"{{"event":"state_saved","request_id":{request},"state":[{state}]}}"#
                    )
                    .unwrap();
                    if command.contains("\"command\":\"close\"") {
                        writeln!(output, r#"{{"event":"closed","request_id":{request}}}"#).unwrap();
                        output.flush().unwrap();
                        break;
                    }
                }
                output.flush().unwrap();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if ready && let Ok(batch) = std::fs::read_to_string(&actions) {
            std::fs::remove_file(&actions).unwrap();
            for action in batch.lines() {
                writeln!(log, "{action}").unwrap();
                let mut fields = action.split_whitespace();
                let action = fields.next().unwrap();
                let id = fields.next().unwrap().parse::<u32>().unwrap();
                let value = if let Some(value) = fields.next() {
                    value.parse::<f64>().unwrap()
                } else {
                    values.get(&id).copied().unwrap_or(0.5)
                };
                if action == "preset" {
                    // Preset loading can change opaque state without reporting
                    // individual beginEdit/performEdit parameter gestures.
                    values.insert(id, value);
                } else if action == "change" {
                    values.insert(id, value);
                    writeln!(
                        output,
                        r#"{{"event":"parameter_changed","id":{id},"value":{value}}}"#
                    )
                    .unwrap();
                } else {
                    // Match the real helper's ordering: capture opaque preset
                    // state before the app creates an automation Undo boundary.
                    let state = (values[&0] * 100.0).round() as u8;
                    writeln!(
                        output,
                        r#"{{"event":"state_saved","request_id":null,"state":[{state}]}}"#
                    )
                    .unwrap();
                    editor_ready(&mut output, &values);
                    // The fixture models selecting the native menu target. The
                    // real renderer/action validation has separate host tests.
                    writeln!(
                        output,
                        r#"{{"event":"automate_parameter","id":{id},"value":{value}}}"#
                    )
                    .unwrap();
                }
            }
            output.flush().unwrap();
            std::fs::write(&handled, batch).unwrap();
        }
        if ready {
            writeln!(output, r#"{{"event":"heartbeat"}}"#).unwrap();
            output.flush().unwrap();
        }
    }
}

fn editor_ready(output: &mut impl Write, values: &std::collections::HashMap<u32, f64>) {
    let parameters = [(0, "Filter Cutoff", false), (7, "Resonance", false), (43, "Output Meter", true)]
        .into_iter().map(|(id, name, read_only)| format!(
            r#"{{"id":{id},"name":"{name}","value":{},"default":0.5,"unit":"%","steps":0,"read_only":{read_only}}}"#,
            values[&id]
        )).collect::<Vec<_>>().join(",");
    writeln!(output, r#"{{"event":"ready","parameters":[{parameters}]}}"#).unwrap();
}

//! Shared driver for the real `rill --headless` application process.

#![allow(dead_code)] // Integration targets exercise different helpers.

use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
use tempfile::TempDir;

pub struct Harness {
    pub directory: TempDir,
    child: Child,
    input: ChildStdin,
    output: Receiver<String>,
}

impl Harness {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().expect("create isolated E2E directory");
        let binary = std::env::var_os("RILL_E2E_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_rill").into());
        let mut child = Command::new(binary)
            .arg("--headless")
            .env("RILL_AUTOMATION_DIR", directory.path())
            .env("RILL_CONFIG_DIR", directory.path().join("config"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("launch the production Rill executable");
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let harness = Self {
            directory,
            child,
            input,
            output: rx,
        };
        let ready = harness.response();
        assert_eq!(ready["event"], "ready", "{ready}");
        assert_eq!(ready["ok"], true, "{ready}");
        assert_eq!(ready["state"]["device_open"], false);
        assert!(
            ready["state"]["project"]["tracks"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        harness
    }

    fn response(&self) -> Value {
        let line = self
            .output
            .recv_timeout(Duration::from_secs(45))
            .expect("application did not return JSON within 45 seconds");
        serde_json::from_str(&line)
            .unwrap_or_else(|error| panic!("Invalid application output {line:?}: {error}"))
    }

    pub fn raw_command(&mut self, command: Value) -> Value {
        serde_json::to_writer(&mut self.input, &command).unwrap();
        writeln!(&mut self.input).unwrap();
        self.input.flush().unwrap();
        self.response()
    }

    pub fn command(&mut self, command: Value) -> Value {
        let response = self.raw_command(command.clone());
        assert_eq!(response["ok"], true, "Command {command} failed: {response}");
        response["state"].clone()
    }

    pub fn click(&mut self, label: &str) -> Value {
        self.command(json!({"command": "click", "label": label}))
    }

    pub fn inspect(&mut self) -> Value {
        self.command(json!({"command": "inspect"}))
    }

    pub fn key(&mut self, key: &str) -> Value {
        self.command(json!({"command": "key", "key": key}))
    }

    pub fn quit(mut self) {
        let result = self.raw_command(json!({"command": "quit"}));
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["event"], "exiting");
        assert_eq!(result["state"]["device_open"], false);
        self.wait_exit();
    }

    pub fn wait_exit(mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "Application exited with {status}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Application failed to exit after quit"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Failed assertions must not leave an application process or temp session.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

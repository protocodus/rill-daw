//! This test creates its own Xvfb server. It never connects to the user's display.
//! Run in a Linux container with Xvfb and Mesa installed:
//! cargo test -p daw-plugins --test native_context_menu -- --ignored

#![cfg(target_os = "linux")]

use anyhow::{Context, Result, ensure};
use daw_ui::{NativeContextMenu, NativeMenuItem};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection,
    protocol::{
        xproto::{
            AtomEnum, BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, ConnectionExt as _, InputFocus,
            KEY_PRESS_EVENT, KEY_RELEASE_EVENT, MOTION_NOTIFY_EVENT, MapState, Window,
        },
        xtest::ConnectionExt as _,
    },
    rust_connection::RustConnection,
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_window(connection: &RustConnection, root: Window) -> Result<Window> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        for window in connection.query_tree(root)?.reply()?.children {
            if let Ok(reply) = connection
                .get_property(false, window, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 1024)?
                .reply()
                && String::from_utf8_lossy(&reply.value) == "Rill parameter menu"
                && connection.get_window_attributes(window)?.reply()?.map_state
                    == MapState::VIEWABLE
            {
                return Ok(window);
            }
        }
        ensure!(
            Instant::now() < deadline,
            "Menu did not map on the private Xvfb display"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn pointer_click(connection: &RustConnection, root: Window, x: i16, y: i16) -> Result<()> {
    for (kind, button) in [
        (MOTION_NOTIFY_EVENT, 0),
        (BUTTON_PRESS_EVENT, 1),
        (BUTTON_RELEASE_EVENT, 1),
    ] {
        connection
            .xtest_fake_input(kind, button, x11rb::CURRENT_TIME, root, x, y, 0)?
            .check()?;
        connection.flush()?;
        std::thread::sleep(Duration::from_millis(30));
    }
    Ok(())
}

#[test]
#[ignore = "Creates a private Xvfb display; requires Xvfb and Mesa in a Linux container"]
fn real_menu_worker_selects_zero_id_honors_disabled_actions_and_cancels() -> Result<()> {
    let mut xvfb = Process(
        Command::new("Xvfb")
            .args([
                "-displayfd",
                "1",
                "-screen",
                "0",
                "1024x768x24",
                "-nolisten",
                "tcp",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Install Xvfb in the test container")?,
    );
    let stdout = xvfb.0.stdout.take().unwrap();
    let (display_sender, display_receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut display = String::new();
        let result = BufReader::new(stdout)
            .read_line(&mut display)
            .map(|_| display);
        let _ = display_sender.send(result);
    });
    let display = display_receiver.recv_timeout(Duration::from_secs(5))??;
    let display = format!(":{}", display.trim().parse::<u32>()?);
    let (connection, screen) = x11rb::connect(Some(&display))?;
    let root = connection.setup().roots[screen].root;
    let model = NativeContextMenu {
        position: [80.0, 80.0],
        items: vec![
            NativeMenuItem::Action {
                id: 0,
                label: "Automate fréquence 🎛".into(),
                enabled: true,
                checked: false,
            },
            NativeMenuItem::Action {
                id: 1,
                label: "Disabled".into(),
                enabled: false,
                checked: true,
            },
        ],
    };
    for (cancel, scale) in [(false, 1), (true, 1), (false, 2), (true, 2)] {
        let mut worker = Process(
            Command::new(env!("CARGO_BIN_EXE_daw-plugin-host"))
                .arg("--native-context-menu")
                .env("DISPLAY", &display)
                .env_remove("WAYLAND_DISPLAY")
                .env("WINIT_UNIX_BACKEND", "x11")
                .env("WINIT_X11_SCALE_FACTOR", scale.to_string())
                .env("LIBGL_ALWAYS_SOFTWARE", "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()?,
        );
        let mut input = worker.0.stdin.take().unwrap();
        serde_json::to_writer(&mut input, &model)?;
        writeln!(input)?;
        input.flush()?;
        let window = wait_window(&connection, root)?;
        connection
            .set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)?
            .check()?;
        connection.flush()?;
        std::thread::sleep(Duration::from_millis(150));
        let position = connection
            .translate_coordinates(window, root, 0, 0)?
            .reply()?;
        assert_eq!(
            (position.dst_x, position.dst_y),
            (80, 80),
            "Menu position must remain physical at {scale}× scale"
        );
        if cancel {
            let setup = connection.setup();
            let keys = connection
                .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)?
                .reply()?;
            let escape = keys
                .keysyms
                .chunks(keys.keysyms_per_keycode as usize)
                .position(|symbols| symbols.contains(&0xff1b))
                .context("Xvfb keyboard has no Escape")? as u8
                + setup.min_keycode;
            for kind in [KEY_PRESS_EVENT, KEY_RELEASE_EVENT] {
                connection
                    .xtest_fake_input(kind, escape, x11rb::CURRENT_TIME, root, 0, 0, 0)?
                    .check()?;
            }
            connection.flush()?;
        } else {
            pointer_click(
                &connection,
                root,
                position.dst_x + 40 * scale,
                position.dst_y + 57 * scale,
            )?;
            assert!(
                worker.0.try_wait()?.is_none(),
                "Disabled item must not select or close the menu"
            );
            pointer_click(
                &connection,
                root,
                position.dst_x + 40 * scale,
                position.dst_y + 20 * scale,
            )?;
        }
        let output = worker.0.stdout.take().unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut line = String::new();
            let mut output = BufReader::new(output);
            let result = (|| -> std::io::Result<String> {
                output.read_line(&mut line)?;
                assert_eq!(line, "ready\n");
                line.clear();
                output.read_line(&mut line)?;
                Ok(line)
            })();
            let _ = sender.send(result);
        });
        let line = receiver.recv_timeout(Duration::from_secs(5))??;
        let selected: Option<u32> = serde_json::from_str(&line)?;
        assert_eq!(selected, if cancel { None } else { Some(0) });
        drop(input);
    }
    Ok(())
}

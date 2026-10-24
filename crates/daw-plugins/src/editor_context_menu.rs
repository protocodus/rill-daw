//! A separate menu process keeps native menu tracking out of the plug-in loop.
//! Holding a menu open never suspends processing, state IPC, or heartbeats.

use anyhow::{Context, Result, bail, ensure};
use daw_ui::{NativeContextMenu, NativeMenuItem};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, TryRecvError},
    },
};
use vst3_host::ContextMenuItem;

const MAX_MENU_BYTES: usize = 1024 * 1024;

enum MenuMessage {
    Ready,
    Selected(Option<u32>),
}

pub(crate) struct ContextMenuProcess {
    menu_id: u64,
    child: Child,
    // EOF is the worker's parent-death signal. Keep this handle after the request.
    _input: Arc<Mutex<ChildStdin>>,
    result: Receiver<std::result::Result<MenuMessage, String>>,
    request_ready: Option<mpsc::SyncSender<()>>,
    ready: bool,
    started: std::time::Instant,
    #[cfg(target_os = "macos")]
    native_activation: bool,
    model: NativeContextMenu,
}

impl ContextMenuProcess {
    pub(crate) fn spawn(
        menu_id: u64,
        items: Vec<ContextMenuItem>,
        position: [f64; 2],
    ) -> Result<Self> {
        let model = menu_model(items, position)?;
        Self::spawn_at(&std::env::current_exe()?, menu_id, model, true)
    }

    fn spawn_at(
        executable: &Path,
        menu_id: u64,
        model: NativeContextMenu,
        _native_activation: bool,
    ) -> Result<Self> {
        model.validate().map_err(anyhow::Error::msg)?;
        let request = serde_json::to_vec(&model)?;
        ensure!(
            request.len() < MAX_MENU_BYTES,
            "Context menu request is too large"
        );
        let mut child = Command::new(executable)
            .arg("--native-context-menu")
            .arg(std::process::id().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Starting native context menu")?;
        let input = child.stdin.take().context("Menu worker stdin missing")?;
        let output = child.stdout.take().context("Menu worker stdout missing")?;
        let (sender, receiver) = mpsc::sync_channel(2);
        let (request_ready, ready_receiver) = mpsc::sync_channel(1);
        let input = Arc::new(Mutex::new(input));
        let process = Self {
            menu_id,
            child,
            _input: Arc::clone(&input),
            result: receiver,
            request_ready: Some(request_ready),
            ready: false,
            started: std::time::Instant::now(),
            #[cfg(target_os = "macos")]
            native_activation: _native_activation,
            model,
        };
        let write_sender = sender.clone();
        // A large plug-in menu can fill a pipe before the child finishes startup.
        // Writing it must not stall the editor's processing/heartbeat loop.
        std::thread::spawn(move || {
            if ready_receiver.recv().is_err() {
                return;
            }
            let written = (|| -> std::io::Result<()> {
                let mut input = input.lock().unwrap();
                input.write_all(&request)?;
                input.write_all(b"\n")?;
                input.flush()
            })();
            if let Err(error) = written {
                let _ = write_sender.try_send(Err(format!("Writing menu request: {error}")));
            }
        });
        std::thread::spawn(move || {
            let result = (|| -> std::result::Result<MenuMessage, String> {
                let mut output = BufReader::new(output);
                let mut line = String::new();
                (&mut output)
                    .take(256)
                    .read_line(&mut line)
                    .map_err(|error| error.to_string())?;
                if line != "ready\n" {
                    return Err("Menu worker did not initialize".into());
                }
                sender
                    .send(Ok(MenuMessage::Ready))
                    .map_err(|error| error.to_string())?;
                line.clear();
                output
                    .take(256)
                    .read_line(&mut line)
                    .map_err(|error| error.to_string())?;
                if !line.ends_with('\n') {
                    return Err("Menu worker closed without a valid response".into());
                }
                serde_json::from_str(&line)
                    .map(MenuMessage::Selected)
                    .map_err(|error| format!("Invalid menu response: {error}"))
            })();
            let _ = sender.send(result);
        });
        Ok(process)
    }

    pub(crate) fn menu_id(&self) -> u64 {
        self.menu_id
    }

    /// `None` means pending, `Some(None)` means dismissed.
    pub(crate) fn poll(&mut self) -> Result<Option<Option<u32>>> {
        if self.request_ready.is_some() {
            ensure!(
                self.started.elapsed() < std::time::Duration::from_secs(10),
                "Menu worker startup timed out"
            );
            if self.ready {
                #[cfg(target_os = "macos")]
                let can_send = !self.native_activation || yield_to_menu_worker(self.child.id());
                #[cfg(not(target_os = "macos"))]
                let can_send = true;
                if can_send && let Some(sender) = self.request_ready.take() {
                    let _ = sender.send(());
                }
            }
        }
        match self.result.try_recv() {
            Ok(Ok(MenuMessage::Ready)) => {
                self.ready = true;
                Ok(None)
            }
            Ok(Ok(MenuMessage::Selected(selected))) => {
                ensure!(
                    selected.is_none_or(|id| self.model.contains_enabled_action(id)),
                    "Menu worker selected an unavailable action"
                );
                Ok(Some(selected))
            }
            Ok(Err(error)) => bail!(error),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => bail!("Menu worker disconnected"),
        }
    }
}

impl Drop for ContextMenuProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn menu_model(items: Vec<ContextMenuItem>, position: [f64; 2]) -> Result<NativeContextMenu> {
    ensure!(items.len() <= 2048, "Context menu has too many items");
    let mut stack: Vec<(String, Vec<NativeMenuItem>)> = vec![(String::new(), Vec::new())];
    for item in items {
        // VST's begin flag includes disabled, and its end flag includes separator.
        // They are structural markers, not disabled submenu headings or lines.
        if item.is_group_start() {
            ensure!(stack.len() <= 16, "Context menu is too deeply nested");
            stack.push((item.name, Vec::new()));
        } else if item.is_group_end() {
            ensure!(stack.len() > 1, "Unmatched context menu group end");
            let (label, items) = stack.pop().unwrap();
            stack
                .last_mut()
                .unwrap()
                .1
                .push(NativeMenuItem::Submenu { label, items });
        } else {
            let entry = if item.is_separator() {
                NativeMenuItem::Separator
            } else {
                NativeMenuItem::Action {
                    id: item.item_id,
                    enabled: !item.is_disabled(),
                    checked: item.is_checked(),
                    label: item.name,
                }
            };
            stack.last_mut().unwrap().1.push(entry);
        }
    }
    ensure!(stack.len() == 1, "Unclosed context menu group");
    let model = NativeContextMenu {
        position,
        items: stack.pop().unwrap().1,
    };
    model.validate().map_err(anyhow::Error::msg)?;
    Ok(model)
}

/// Called only by the isolated helper executable's menu mode.
pub(crate) fn run_worker() -> Result<()> {
    let mut protocol = vst3_host::process_isolation::ProtocolChannel::claim();
    #[cfg(target_os = "macos")]
    daw_ui::initialize_native_context_menu_worker().map_err(anyhow::Error::msg)?;
    writeln!(protocol, "ready")?;
    protocol.flush()?;
    let mut input = BufReader::new(std::io::stdin());
    let mut line = String::new();
    (&mut input)
        .take(MAX_MENU_BYTES as u64 + 1)
        .read_line(&mut line)?;
    ensure!(
        line.len() <= MAX_MENU_BYTES && line.ends_with('\n'),
        "Invalid menu request size"
    );
    let model: NativeContextMenu = serde_json::from_str(&line)?;
    model.validate().map_err(anyhow::Error::msg)?;
    // If the editor is killed/crashes, its pipe closes. Do not leave an orphaned
    // native menu holding pointer/keyboard focus. No plug-in code lives here.
    std::thread::spawn(move || {
        let mut byte = [0];
        loop {
            match input.read(&mut byte) {
                Ok(0) | Err(_) => std::process::exit(0),
                Ok(_) => {}
            }
        }
    });
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let selected = daw_ui::show_native_context_menu(&model).map_err(anyhow::Error::msg)?;
    #[cfg(target_os = "linux")]
    let selected = linux::show(model)?;
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    let selected: Option<u32> = None;
    #[cfg(target_os = "macos")]
    return_activation_to_editor();
    serde_json::to_writer(&mut protocol, &selected)?;
    writeln!(protocol)?;
    protocol.flush()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn yield_to_menu_worker(pid: u32) -> bool {
    use objc2::sel;
    use objc2_app_kit::{NSApplication, NSRunningApplication};
    use objc2_foundation::NSObjectProtocol;
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return false;
    };
    let Some(worker) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as i32)
    else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    // Cooperative activation is macOS 14+. Older supported systems use the
    // worker's activateIgnoringOtherApps fallback.
    if app.respondsToSelector(sel!(yieldActivationToApplication:)) {
        app.yieldActivationToApplication(&worker);
    }
    true
}

#[cfg(target_os = "macos")]
fn return_activation_to_editor() {
    use objc2::sel;
    use objc2_app_kit::{NSApplication, NSApplicationActivationOptions, NSRunningApplication};
    use objc2_foundation::NSObjectProtocol;
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // Cancelling by choosing another application must not pull focus back.
    if !app.isActive() {
        return;
    }
    let Some(pid) = std::env::args()
        .nth(2)
        .and_then(|pid| pid.parse::<i32>().ok())
    else {
        return;
    };
    if let Some(editor) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
        if app.respondsToSelector(sel!(yieldActivationToApplication:)) {
            app.yieldActivationToApplication(&editor);
        }
        editor.activateWithOptions(NSApplicationActivationOptions::empty());
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use eframe::egui;
    use std::sync::{Arc, Mutex};

    struct MenuApp {
        model: NativeContextMenu,
        selected: Arc<Mutex<Option<u32>>>,
        was_focused: bool,
        positioned: bool,
    }

    impl eframe::App for MenuApp {
        fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            if !self.positioned {
                // X11 coordinates are physical pixels; egui viewport positions
                // are logical points. Wait until the actual display scale is
                // known, then position before revealing the menu.
                let scale = ctx.pixels_per_point();
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    self.model.position[0] as f32 / scale,
                    self.model.position[1] as f32 / scale,
                )));
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                self.positioned = true;
            }
            let focused = ctx.input(|input| input.focused);
            if (self.was_focused && !focused)
                || ctx.input(|input| input.key_pressed(egui::Key::Escape))
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            self.was_focused |= focused;
            egui::CentralPanel::default()
                .frame(egui::Frame::popup(&ctx.style()))
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if let Some(id) = daw_ui::native_context_menu_ui(ui, &self.model.items) {
                            *self.selected.lock().unwrap() = Some(id);
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
        }
    }

    pub(super) fn show(model: NativeContextMenu) -> Result<Option<u32>> {
        let selected = Arc::new(Mutex::new(None));
        let result = Arc::clone(&selected);
        let height = (model.items.len() as f32 * 35.0 + 16.0).clamp(60.0, 650.0);
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("Rill parameter menu")
                .with_decorations(false)
                .with_resizable(false)
                .with_taskbar(false)
                .with_always_on_top()
                .with_active(true)
                .with_visible(false)
                .with_inner_size([340.0, height]),
            ..Default::default()
        };
        eframe::run_native(
            "Rill parameter menu",
            options,
            Box::new(move |cc| {
                daw_ui::theme::apply(&cc.egui_ctx);
                Ok(Box::new(MenuApp {
                    model,
                    selected,
                    was_focused: false,
                    positioned: false,
                }))
            }),
        )
        .map_err(|error| anyhow::anyhow!("Opening native parameter menu: {error}"))?;
        let value = *result.lock().unwrap();
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u32, name: &str, flags: i32) -> ContextMenuItem {
        ContextMenuItem {
            item_id: id,
            name: name.into(),
            tag: 0,
            flags,
        }
    }

    #[test]
    fn vst_groups_keep_actions_checks_disabled_state_and_zero_id() {
        let model = menu_model(
            vec![
                item(1, "Plugin", 10),
                item(0, "Automate fréquence 🎛", 4),
                item(2, "", 1),
                item(3, "Unavailable", 2),
                item(4, "", 17),
                item(5, "Reset", 0),
            ],
            [50.0, 80.0],
        )
        .unwrap();
        assert!(model.contains_enabled_action(0));
        assert!(!model.contains_enabled_action(3));
        assert!(model.contains_enabled_action(5));
        let NativeMenuItem::Submenu { items, .. } = &model.items[0] else {
            panic!("Expected submenu")
        };
        assert!(matches!(
            items[0],
            NativeMenuItem::Action {
                checked: true,
                id: 0,
                ..
            }
        ));
        assert!(matches!(items[1], NativeMenuItem::Separator));
        assert!(menu_model(vec![item(0, "Bad end", 17)], [0.0; 2]).is_err());
        assert!(menu_model(vec![item(0, "Bad start", 10)], [0.0; 2]).is_err());
    }

    #[cfg(unix)]
    fn fixture(script: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("menu-worker");
        std::fs::write(
            &path,
            format!("#!/bin/sh\nprintf 'ready\\n'\nread request\n{script}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (temp, path)
    }

    #[cfg(unix)]
    fn await_result(process: &mut ContextMenuProcess) -> Result<Option<u32>> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if let Some(selected) = process.poll()? {
                return Ok(selected);
            }
            ensure!(
                std::time::Instant::now() < deadline,
                "Menu fixture timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[cfg(unix)]
    #[test]
    fn process_completion_cancellation_invalid_selection_and_crash_are_bounded() {
        let model = menu_model(
            vec![item(0, "Automate", 0), item(1, "Disabled", 2)],
            [0.0; 2],
        )
        .unwrap();
        for (script, expected) in [
            ("printf '0\\n'", Some(Some(0))),
            ("printf 'null\\n'", Some(None)),
            ("printf '1\\n'", None),
            ("printf '999\\n'", None),
            ("printf 'invalid\\n'", None),
            ("exit 2", None),
        ] {
            let (_temp, executable) = fixture(script);
            let mut process =
                ContextMenuProcess::spawn_at(&executable, 42, model.clone(), false).unwrap();
            assert_eq!(process.menu_id(), 42);
            match expected {
                Some(expected) => assert_eq!(await_result(&mut process).unwrap(), expected),
                None => assert!(await_result(&mut process).is_err()),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn holding_menu_never_blocks_poll_and_drop_kills_and_reaps_live_worker() {
        let (_temp, executable) = fixture("exec sleep 30");
        let model = menu_model(vec![item(0, "Automate", 0)], [0.0; 2]).unwrap();
        let mut process = ContextMenuProcess::spawn_at(&executable, 9, model, false).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while process.request_ready.is_some() {
            assert_eq!(process.poll().unwrap(), None);
            assert!(
                std::time::Instant::now() < deadline,
                "Worker never became ready"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let started = std::time::Instant::now();
        for _ in 0..1_000 {
            assert_eq!(process.poll().unwrap(), None);
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert!(process.child.try_wait().unwrap().is_none());
        let pid = process.child.id() as i32;
        let started = std::time::Instant::now();
        drop(process);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        unsafe extern "C" {
            fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
        }
        let mut status = 0;
        // SAFETY: POSIX waitpid receives this test's former child PID, writable
        // status and WNOHANG=1. A live child would return 0; an unreaped child its
        // PID. -1 proves Drop already consumed the child exit status.
        assert_eq!(unsafe { waitpid(pid, &mut status, 1) }, -1);
    }
}

//! Wakeable stdio bridge. The transport only queues requests; the app's control
//! thread remains the sole tool dispatcher and owner of the live song.
use daw_tools::{ToolError, ToolHost, ToolProfile};
use eframe::egui;
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
};

pub(crate) struct ToolRequest {
    pub profile: ToolProfile,
    pub cancel: Arc<AtomicBool>,
    pub name: String,
    pub arguments: Value,
    pub reply: Sender<Result<Value, ToolError>>,
}

struct ChannelHost {
    sender: Sender<ToolRequest>,
    profile: ToolProfile,
    ctx: egui::Context,
}

impl ToolHost for ChannelHost {
    fn tool_specs(&self) -> Vec<daw_tools::ToolSpec> {
        daw_tools::tool_specs_for(self.profile)
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        let (reply, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.sender
            .send(ToolRequest {
                profile: self.profile,
                cancel: cancel.clone(),
                name: name.into(),
                arguments,
                reply,
            })
            .map_err(|_| ToolError::new("unavailable", "Rill closed"))?;
        // The app can be asleep between user actions. Publish before waking so
        // the next frame can dispatch this request without periodic busy work.
        self.ctx.request_repaint();
        match receiver.recv_timeout(std::time::Duration::from_secs(90)) {
            Ok(result) => result,
            Err(_) => {
                cancel.store(true, Ordering::Release);
                Err(ToolError::new("timeout", "Tool dispatch timed out"))
            }
        }
    }
}

pub(crate) fn start_stdio(profile: ToolProfile, ctx: egui::Context) -> Receiver<ToolRequest> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        if let Err(error) = daw_mcp::serve(
            stdin.lock(),
            stdout.lock(),
            &mut ChannelHost {
                sender,
                profile,
                ctx,
            },
        ) {
            eprintln!("MCP transport ended: {error}");
        }
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn queued_tool_wakes_idle_ui_before_waiting_for_its_reply() {
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let _ = ctx.run(Default::default(), |_| {});
        }
        let (wake, wakes) = mpsc::channel();
        ctx.set_request_repaint_callback(move |info| {
            wake.send(info.delay).unwrap();
        });
        let (sender, requests) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            ChannelHost {
                sender,
                profile: ToolProfile::CurrentSong,
                ctx,
            }
            .call_tool("song_get", serde_json::json!({}))
        });
        assert_eq!(
            wakes.recv_timeout(Duration::from_secs(5)).unwrap(),
            Duration::ZERO
        );
        let request = requests.try_recv().expect("request published before wake");
        assert_eq!(request.name, "song_get");
        assert_eq!(request.profile, ToolProfile::CurrentSong);
        assert!(!request.cancel.load(Ordering::Acquire));
        request
            .reply
            .send(Ok(serde_json::json!({"awake":true})))
            .unwrap();
        assert_eq!(
            worker.join().unwrap().unwrap(),
            serde_json::json!({"awake":true})
        );
    }

    #[test]
    fn closed_app_fails_dispatch_without_waiting_for_timeout() {
        let (sender, requests) = mpsc::channel();
        drop(requests);
        let error = ChannelHost {
            sender,
            profile: ToolProfile::CurrentSong,
            ctx: egui::Context::default(),
        }
        .call_tool("song_get", Value::Null)
        .unwrap_err();
        assert_eq!(error.code, "unavailable");
    }
}

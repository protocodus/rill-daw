mod app;
mod arrangement;
mod automation;
mod editor;
mod routing;
mod settings;
mod theme;
#[cfg(test)]
mod ui_tests;

use daw_core::Project;
use std::path::PathBuf;

fn demo() -> Project {
    let scratch = settings::Settings::load().scratch_dir;
    Project::demo_with_audio(scratch.join("rill-demo-drums.wav"))
        .unwrap_or_else(|_| Project::demo())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|s| s == "--headless") {
        return automation::run();
    }
    if args.first().is_some_and(|s| s == "--help" || s == "-h") {
        println!(
            "Rill — modular Rust DAW\n\nrill [session.daw.json]\nrill --render-demo output.wav\nrill --render session.daw.json output.wav\nrill --screenshot output.png [--view clip|mixer|instrument|effects|routing|setup]\nrill --headless (isolated JSON automation; see docs/E2E.md)\n\nBuild all workspace binaries to enable isolated VST3 hosting."
        );
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "--render-demo") {
        let output = args
            .get(1)
            .ok_or_else(|| anyhow::anyhow!("Provide output.wav"))?;
        let p = demo();
        daw_engine::export_project_wav(&p, output, 48_000)?;
        println!("Rendered {} beats to {output}", p.end_beat());
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "--render") {
        anyhow::ensure!(
            args.len() >= 3,
            "Usage: rill --render session.daw.json output.wav"
        );
        let p = Project::load(&args[1])?;
        daw_engine::export_project_wav(&p, &args[2], 48_000)?;
        return Ok(());
    }
    let screenshot = if args.first().is_some_and(|s| s == "--screenshot") {
        Some(PathBuf::from(args.get(1).ok_or_else(|| {
            anyhow::anyhow!("Provide screenshot output path")
        })?))
    } else {
        None
    };
    let project = if screenshot.is_none() && !args.is_empty() {
        Project::load(&args[0])?
    } else {
        demo()
    };
    let project_path = if screenshot.is_none() && !args.is_empty() {
        Some(PathBuf::from(&args[0]))
    } else {
        None
    };
    let view = args
        .iter()
        .position(|a| a == "--view")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1480.0, 960.0])
            .with_min_inner_size([1100.0, 720.0]),
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "Rill — Audio Workstation",
        options,
        Box::new(move |cc| {
            let mut app = app::DawApp::new(cc, project, screenshot);
            app.project_path = project_path;
            match view.as_deref() {
                Some("setup") => {
                    app.open_popup(app::PopupKind::Setup);
                }
                Some("mixer") => app.view = app::WorkspaceView::Mixer,
                Some("clip") => app.view = app::WorkspaceView::Clip,
                Some("library") => app.open_popup(app::PopupKind::Library),
                Some("routing") => app.open_popup(app::PopupKind::Routing),
                Some("instrument") => app.open_popup(app::PopupKind::Instrument),
                Some("rack" | "effects") => {
                    let id = app.project.tracks[0].id;
                    let bus = app.project.add_bus("Space return");
                    app.project
                        .track_mut(bus)
                        .unwrap()
                        .inserts
                        .push(daw_core::EffectSlot::new(
                            "Stereo delay",
                            daw_core::Effect::Delay {
                                time_ms: 375.0,
                                feedback: 0.4,
                                mix: 1.0,
                            },
                        ));
                    app.project.add_send(id, bus, 0.2, false)?;
                    app.project
                        .track_mut(id)
                        .unwrap()
                        .inserts
                        .push(daw_core::EffectSlot::new(
                            "Trim",
                            daw_core::Effect::Gain { gain: 0.8 },
                        ));
                    app.open_popup(app::PopupKind::Effects);
                }
                _ => {}
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

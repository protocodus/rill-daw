mod app;
mod arrangement;
mod audio_editor;
mod automation;
mod automation_editor;
mod crossfade_ui;
mod document_ui;
mod editor;
mod engine_lifecycle;
mod error_ui;
mod export_ui;
mod history_ui;
mod join_ui;
mod library;
mod library_preview;
mod midi_editor;
mod midi_generate_ui;
mod plugin_ui;
mod project_io;
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
            "Rill — modular Rust DAW\n\nrill [session.rill]\nrill --render-demo output.wav\nrill --render session.rill output.wav\nrill --screenshot output.png [--project session.rill] [--view clip|mixer|channel|instrument|effects|routing|automation|library|setup|properties|crossfade|join|unsaved-changes]\nrill --headless (isolated JSON automation; see docs/E2E.md)\n\nLegacy .daw.json sessions remain supported. Build all workspace binaries to enable isolated VST3 hosting."
        );
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "--render-demo") {
        let output = args
            .get(1)
            .ok_or_else(|| anyhow::anyhow!("Provide output.wav"))?;
        let p = demo();
        let settings = settings::Settings::load();
        daw_engine::export_project_wav_with_options(
            &p,
            output,
            settings.sample_rate,
            daw_engine::RenderOptions {
                resample_quality: settings.resample_quality,
            },
        )?;
        println!("Rendered {} beats to {output}", p.end_beat());
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "--render") {
        anyhow::ensure!(
            args.len() >= 3,
            "Usage: rill --render session.rill output.wav"
        );
        let p = Project::load_with_assets(&args[1], settings::Settings::load().scratch_dir)?;
        let settings = settings::Settings::load();
        daw_engine::export_project_wav_with_options(
            &p,
            &args[2],
            settings.sample_rate,
            daw_engine::RenderOptions {
                resample_quality: settings.resample_quality,
            },
        )?;
        return Ok(());
    }
    let screenshot = if args.first().is_some_and(|s| s == "--screenshot") {
        Some(PathBuf::from(args.get(1).ok_or_else(|| {
            anyhow::anyhow!("Provide screenshot output path")
        })?))
    } else {
        None
    };
    let screenshot_project = screenshot.as_ref().and_then(|_| {
        args.iter()
            .position(|arg| arg == "--project")
            .and_then(|index| args.get(index + 1))
    });
    let project = if let Some(path) = screenshot_project {
        Project::load_with_assets(path, settings::Settings::load().scratch_dir)?
    } else if screenshot.is_none() && !args.is_empty() {
        Project::load_with_assets(&args[0], settings::Settings::load().scratch_dir)?
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
            .with_app_id(daw_ui::APP_ID)
            .with_icon(daw_ui::app_icon())
            .with_inner_size([1480.0, 960.0])
            .with_min_inner_size([1100.0, 720.0]),
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "RILL — Audio Workstation",
        options,
        Box::new(move |cc| {
            let mut app = app::DawApp::new(cc, project, screenshot);
            app.project_path = project_path;
            match view.as_deref() {
                Some("setup") => {
                    app.open_popup(app::PopupKind::Setup);
                }
                Some("mixer") => app.view = app::WorkspaceView::Mixer,
                Some("channel") => {
                    if let Some(track) = app.selected_track {
                        app.open_channel(track);
                    }
                }
                Some("clip") => app.view = app::WorkspaceView::Clip,
                Some("library") => app.open_popup(app::PopupKind::Library),
                Some("automation") => app.open_popup(app::PopupKind::Automation),
                Some("crossfade") => app.open_popup(app::PopupKind::Crossfade),
                Some("join") => app.open_popup(app::PopupKind::Join),
                Some("routing") => app.open_popup(app::PopupKind::Routing),
                Some("properties") => app.open_popup(app::PopupKind::Inspector),
                Some("unsaved-changes") if app.screenshot.is_some() => {
                    app.modified = true;
                    app.history.mark_unsaved();
                    app.request_document_transition(document_ui::DocumentTransition::New);
                }
                Some("instrument") => app.open_popup(app::PopupKind::Instrument),
                Some("rack" | "effects") => app.open_popup(app::PopupKind::Effects),
                _ => {}
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

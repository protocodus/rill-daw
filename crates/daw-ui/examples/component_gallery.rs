//! Interactive shared-component catalogue and reproducible documentation capture.
//!
//! cargo run -p daw-ui --example component_gallery
//! xvfb-run -a cargo run -p daw-ui --example component_gallery -- --capture docs/assets/ui-gallery
use daw_ui::{self as ui, theme};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[path = "component_gallery/actions.rs"]
mod actions;
#[path = "component_gallery/curves.rs"]
mod curves;
#[path = "component_gallery/device_controls.rs"]
mod device_controls;
#[path = "component_gallery/device_details.rs"]
mod device_details;
#[path = "component_gallery/device_panels.rs"]
mod device_panels;
#[path = "component_gallery/device_plots.rs"]
mod device_plots;
#[path = "component_gallery/devices.rs"]
mod devices;
#[path = "component_gallery/feedback.rs"]
mod feedback;
#[path = "component_gallery/inputs.rs"]
mod inputs;
#[path = "component_gallery/mixer.rs"]
mod mixer;
#[path = "component_gallery/navigation.rs"]
mod navigation;
#[path = "component_gallery/routing_choices.rs"]
mod routing_choices;
#[path = "component_gallery/signal_analysis.rs"]
mod signal_analysis;
#[path = "component_gallery/timeline.rs"]
mod timeline;
#[path = "component_gallery/tracks.rs"]
mod tracks;

const PAGES: [(&str, &str, &str); 15] = [
    (
        "actions",
        "Actions, tools, and views",
        "Action buttons, exclusive editing modes, presentation controls, and vector icons.",
    ),
    (
        "inputs",
        "Inputs and forms",
        "Aligned settings, precise numeric entry, text fields, and independent choices.",
    ),
    (
        "navigation",
        "Navigation and popups",
        "Workspace headers, related views, native menus, and on-demand utility windows.",
    ),
    (
        "timeline",
        "Musical canvases",
        "Shared rulers, audio waveforms, piano keys, and velocity markers.",
    ),
    (
        "mixer",
        "Mixer and recording",
        "Channel strips, stereo meters, input levels, and recording controls with fixed demo levels.",
    ),
    (
        "curves",
        "Curves and crossfades",
        "Editable automation points and complementary audio fades, using sample control data.",
    ),
    (
        "devices",
        "Devices and parameters",
        "Reusable device cards and searchable parameter controls with illustrative data.",
    ),
    (
        "feedback",
        "Feedback and assistant",
        "Notices and the provider-independent assistant, populated with labelled sample content.",
    ),
    (
        "tracks",
        "Track headers",
        "Identity, mix, routing, source and stereo metering, with fixed demonstration levels.",
    ),
    (
        "device-details",
        "Device details",
        "Inspected input/output counts and a searchable, read-only parameter catalogue with sample metadata.",
    ),
    (
        "routing-choices",
        "Mixer routing choices",
        "Choose the main output, create a group bus, add sends, or open their full controls beside the source channel.",
    ),
    (
        "signal-analysis",
        "Signal analysis",
        "Switchable spectrum, loudness, stereo correlation, and phase displays for mixer channels.",
    ),
    (
        "device-controls",
        "Device controls",
        "Knobs, selectors, pads, envelope, XY pad, keyboard, wheels and the modulation matrix shared by every device.",
    ),
    (
        "device-plots",
        "Device visualizations",
        "Response, transfer, envelope, sample and echo displays drawn from plain data.",
    ),
    (
        "device-panels",
        "Device panels",
        "The shared device frame and a panel generated from parameter rows.",
    ),
];

#[derive(Default)]
struct CaptureResult {
    complete: bool,
    error: Option<String>,
}

struct Gallery {
    capture: Option<PathBuf>,
    result: Arc<Mutex<CaptureResult>>,
    started: Instant,
    stage: usize,
    warmup: u8,
    requested: bool,
    page: usize,
    appearance: theme::Appearance,
    actions: actions::State,
    inputs: inputs::State,
    navigation: navigation::State,
    timeline: timeline::State,
    mixer: mixer::State,
    curves: curves::State,
    devices: devices::State,
    device_details: device_details::State,
    feedback: feedback::State,
    tracks: tracks::State,
    signal_analysis: signal_analysis::State,
    device_controls: device_controls::State,
    device_plots: device_plots::State,
    device_panels: device_panels::State,
}

impl Gallery {
    fn new(
        cc: &eframe::CreationContext<'_>,
        capture: Option<PathBuf>,
        result: Arc<Mutex<CaptureResult>>,
    ) -> Self {
        theme::apply(&cc.egui_ctx);
        cc.egui_ctx.set_pixels_per_point(1.0);
        Self {
            capture,
            result,
            started: Instant::now(),
            stage: 0,
            warmup: 0,
            requested: false,
            page: 0,
            appearance: theme::Appearance::Light,
            actions: Default::default(),
            inputs: Default::default(),
            navigation: Default::default(),
            timeline: Default::default(),
            mixer: Default::default(),
            curves: Default::default(),
            devices: Default::default(),
            device_details: Default::default(),
            feedback: Default::default(),
            tracks: Default::default(),
            signal_analysis: Default::default(),
            device_controls: Default::default(),
            device_plots: Default::default(),
            device_panels: Default::default(),
        }
    }

    fn capture_events(&mut self, ctx: &egui::Context) {
        if self.capture.is_none() {
            return;
        }
        if self.started.elapsed() > Duration::from_secs(90) {
            self.result.lock().unwrap().error =
                Some("Component capture exceeded 90 seconds".into());
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !self.requested {
            return;
        }
        let screenshot = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = screenshot {
            let theme = if self.stage < PAGES.len() {
                "light"
            } else {
                "dark"
            };
            let page = self.stage % PAGES.len();
            let path = self
                .capture
                .as_ref()
                .unwrap()
                .join(format!("{}-{theme}.png", PAGES[page].0));
            let bytes: Vec<_> = image
                .pixels
                .iter()
                .flat_map(|pixel| pixel.to_array())
                .collect();
            if let Err(error) = image::save_buffer(
                &path,
                &bytes,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            ) {
                self.result.lock().unwrap().error = Some(format!("{}: {error}", path.display()));
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            println!(
                "Captured {} ({}×{})",
                path.display(),
                image.size[0],
                image.size[1]
            );
            self.stage += 1;
            self.warmup = 0;
            self.requested = false;
            egui::Popup::close_all(ctx);
            if self.stage == PAGES.len() * 2 {
                self.result.lock().unwrap().complete = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

impl eframe::App for Gallery {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        if self.capture.is_some() {
            // Documentation captures must not depend on where the desktop's
            // pointer happens to sit, or include incidental hover tooltips.
            input.events.retain(|event| {
                !matches!(
                    event,
                    egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. }
                )
            });
            input.events.push(egui::Event::PointerGone);
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.capture_events(ctx);
        if self.stage >= PAGES.len() * 2 {
            return;
        }
        if self.capture.is_some() {
            self.page = self.stage % PAGES.len();
            self.appearance = if self.stage < PAGES.len() {
                theme::Appearance::Light
            } else {
                theme::Appearance::Dark
            };
        }
        theme::set_appearance(ctx, self.appearance);
        let palette = theme::palette(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.bg)
                    .inner_margin(ui::tokens::SPACE_LG),
            )
            .show(ctx, |ui| {
                if self.capture.is_none() {
                    ui.horizontal_wrapped(|ui| {
                        for (index, (label, _, _)) in PAGES.iter().enumerate() {
                            if ui::view_choice(ui, self.page == index, *label).clicked() {
                                self.page = index;
                                egui::Popup::close_all(ctx);
                            }
                        }
                        if ui::small_button(ui, "Light / Dark").clicked() {
                            self.appearance = if self.appearance == theme::Appearance::Light {
                                theme::Appearance::Dark
                            } else {
                                theme::Appearance::Light
                            };
                        }
                    });
                    ui.add_space(ui::tokens::SPACE_MD);
                }
                ui::text::title(ui, PAGES[self.page].1);
                ui::text::note(ui, PAGES[self.page].2);
                ui.add_space(ui::tokens::SPACE_MD);
                ui.separator();
                ui.add_space(ui::tokens::SPACE_MD);
                ui.push_id(PAGES[self.page].0, |ui| match self.page {
                    0 => actions::show(ui, &mut self.actions),
                    1 => inputs::show(ui, &mut self.inputs),
                    2 => {
                        self.navigation.capture = self.capture.is_some();
                        navigation::show(ui, &mut self.navigation)
                    }
                    3 => timeline::show(ui, &mut self.timeline),
                    4 => mixer::show(ui, &mut self.mixer),
                    5 => curves::show(ui, &mut self.curves),
                    6 => devices::show(ui, &mut self.devices),
                    7 => feedback::show(ui, &mut self.feedback),
                    8 => tracks::show(ui, &mut self.tracks),
                    9 => device_details::show(ui, &mut self.device_details),
                    10 => routing_choices::show(ui),
                    11 => signal_analysis::show(ui, &mut self.signal_analysis),
                    12 => device_controls::show(ui, &mut self.device_controls),
                    13 => device_plots::show(ui, &mut self.device_plots),
                    14 => device_panels::show(ui, &mut self.device_panels),
                    _ => unreachable!(),
                });
            });
        if self.capture.is_some() {
            self.warmup = self.warmup.saturating_add(1);
            if self.warmup >= 12 && !self.requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                self.requested = true;
            }
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let capture = match args.next().as_deref() {
        None => None,
        Some("--capture") => Some(PathBuf::from(
            args.next().ok_or("--capture requires a directory")?,
        )),
        Some("--help" | "-h") => {
            println!(
                "component_gallery [--capture DIRECTORY]\nInteractive shared UI catalogue; --capture writes fifteen light/dark PNG pairs and exits."
            );
            return Ok(());
        }
        Some(_) => return Err("Usage: component_gallery [--capture DIRECTORY]".into()),
    };
    if args.next().is_some() {
        return Err("Unexpected extra argument".into());
    }
    if let Some(dir) = &capture {
        std::fs::create_dir_all(dir)?;
    }
    let capturing = capture.is_some();
    let result = Arc::new(Mutex::new(CaptureResult::default()));
    let state = result.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rill shared UI components")
            // Leave room for every example after applying the shared spacing.
            .with_inner_size([1120.0, if capturing { 987.0 } else { 800.0 }])
            .with_min_inner_size([1120.0, if capturing { 987.0 } else { 800.0 }])
            .with_resizable(!capturing),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Rill shared UI components",
        options,
        Box::new(move |cc| Ok(Box::new(Gallery::new(cc, capture, state)))),
    )?;
    if capturing {
        let state = result.lock().unwrap();
        if let Some(error) = &state.error {
            return Err(error.clone().into());
        }
        if !state.complete {
            return Err("Component capture closed before every image was saved".into());
        }
    }
    Ok(())
}

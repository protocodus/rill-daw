use daw_ui::{self as controls, Icon, text, theme, tokens};
use egui::{Id, Rect, Sense, Ui, vec2};

#[derive(Clone, Copy)]
enum Kind {
    Audio,
    Midi,
    Bus,
}

struct Channel {
    name: &'static str,
    kind: Kind,
    color: [u8; 3],
    gain: f32,
    pan: f32,
    muted: bool,
    solo: bool,
    armed: bool,
    route: usize,
    source: usize,
    peaks: [f32; 2],
}

impl Channel {
    fn new(name: &'static str, kind: Kind, color: [u8; 3], pan: f32) -> Self {
        Self {
            name,
            kind,
            color,
            gain: 0.5,
            pan,
            muted: false,
            solo: false,
            armed: false,
            route: 0,
            source: 0,
            peaks: [0.42, 0.34],
        }
    }
}

pub struct State {
    channels: [Channel; 6],
}

impl Default for State {
    fn default() -> Self {
        Self {
            channels: [
                Channel::new("Circuit drums", Kind::Audio, [92, 176, 168], 0.0),
                Channel::new("Glass keys", Kind::Midi, [184, 166, 95], -0.12),
                Channel::new("Music", Kind::Bus, [145, 137, 190], 0.0),
                Channel::new("Circuit drums", Kind::Audio, [92, 176, 168], 0.0),
                Channel::new("Glass keys", Kind::Midi, [184, 166, 95], -0.12),
                Channel::new("Music", Kind::Bus, [145, 137, 190], 0.0),
            ],
        }
    }
}

pub fn show(ui: &mut Ui, state: &mut State) {
    text::subtitle(ui, "A clear order for each channel");
    text::note(
        ui,
        "Name and color → mix → routing and source. The meter has its own column.",
    );
    ui.add_space(tokens::SPACE_MD);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_LG;
        for index in 0..3 {
            header(
                ui,
                index,
                &mut state.channels[index],
                vec2(controls::TRACK_HEADER_WIDTH, controls::TRACK_HEADER_HEIGHT),
            );
        }
    });
    ui.add_space(tokens::SPACE_XL);
    text::subtitle(ui, "Adapt to the available space");
    text::note(
        ui,
        "Compact tracks keep mix controls; the channel button reveals connections and precise settings.",
    );
    ui.add_space(tokens::SPACE_MD);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_LG;
        for (index, label, size) in [
            (
                3,
                "Compact · 89 points high",
                vec2(
                    controls::TRACK_HEADER_WIDTH,
                    controls::TRACK_HEADER_COMPACT_HEIGHT,
                ),
            ),
            (
                4,
                "Narrow · 224 points wide",
                vec2(224.0, controls::TRACK_HEADER_HEIGHT),
            ),
            (
                5,
                "Short · 56 points high",
                vec2(controls::TRACK_HEADER_WIDTH, 56.0),
            ),
        ] {
            ui.vertical(|ui| {
                ui.set_width(size.x);
                text::label(ui, label);
                ui.add_space(tokens::SPACE_SM);
                header(ui, index, &mut state.channels[index], size);
            });
        }
    });
    ui.add_space(tokens::SPACE_LG);
    text::note(
        ui,
        "Drag a volume or pan rail, click its value for exact entry, or try M/S and the connection rows.",
    );
    text::note(
        ui,
        "These example meters show fixed stereo peaks; the app uses actual post-fader channel levels.",
    );
}

fn header(ui: &mut Ui, index: usize, channel: &mut Channel, size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let mut layout = controls::TrackHeaderLayout::new(rect);
    layout = match channel.kind {
        Kind::Bus => layout.with_bus_badge(),
        Kind::Audio | Kind::Midi => layout.with_record_arm(),
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0, theme::palette(ui.ctx()).panel);
    controls::paint_track_header_sections(&painter, &layout);
    controls::color_strip_at(
        ui,
        layout.color,
        ("gallery_track_color", index),
        &format!("Color of {}", channel.name),
        &mut channel.color,
    );
    let tint = theme::color(channel.color);
    controls::paint_track_header_identity(
        &painter,
        &layout,
        controls::TrackHeaderIdentity {
            name: channel.name,
            icon: match channel.kind {
                Kind::Audio => Icon::Waveform,
                Kind::Midi => Icon::Instrument,
                Kind::Bus => Icon::Routing,
            },
            tint,
            folder: false,
            collapsed: layout.compact,
        },
    );
    if let Some(rect) = layout.bus_badge {
        controls::paint_bus_badge(&painter, rect, tint);
    }
    controls::track_reorder_grip_at(
        ui,
        layout.reorder,
        Id::new(("gallery_track_grip", index)),
        index as u64,
    );
    if let Some(rect) = layout.record_arm
        && controls::record_arm_at(
            ui,
            rect,
            Id::new(("gallery_track_arm", index)),
            channel.armed,
            true,
        )
        .clicked()
    {
        channel.armed = !channel.armed;
    }
    if let Some(rect) = layout.mix {
        let response = controls::track_mix_controls_at(
            ui,
            rect,
            ("gallery_track_mix", index),
            &mut channel.gain,
            &mut channel.pan,
            channel.muted,
            channel.solo,
        );
        channel.muted ^= response.mute.clicked();
        channel.solo ^= response.solo.clicked();
    }
    if let Some(rect) = layout.routing {
        routing(ui, rect, index, channel);
    }
    if let Some(rect) = layout.device {
        source(ui, rect, index, channel);
    }
    if let Some(rect) = layout.meter {
        let mut meter = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("gallery_track_meter", index))
                .max_rect(rect),
        );
        meter.set_clip_rect(ui.clip_rect().intersect(rect));
        controls::stereo_meter(&mut meter, channel.peaks, rect.size())
            .on_hover_text("Fixed demonstration stereo peaks");
    }
    if let Some(rect) = layout.details {
        let response =
            controls::track_header_details_at(ui, rect, ("gallery_track_details", index));
        egui::Popup::menu(&response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                text::label(ui, channel.name);
                let mut db = controls::channel_gain_to_db(channel.gain);
                if ui.add(controls::channel_gain_number(&mut db)).changed() {
                    channel.gain = controls::channel_gain_from_db(db);
                }
                ui.add(controls::slider(&mut channel.pan, -1.0..=1.0).text("Pan"));
                ui.horizontal(|ui| {
                    controls::checkbox(ui, &mut channel.muted, "Mute");
                    controls::checkbox(ui, &mut channel.solo, "Solo");
                });
                // Native submenus keep the enclosing channel popup open while
                // opening choices; a second unrelated Popup would replace it.
                const OUTPUTS: [&str; 2] = ["Master", "Music bus"];
                controls::choice_menu(ui, format!("Routing · {}", OUTPUTS[channel.route]), |ui| {
                    for (value, name) in OUTPUTS.into_iter().enumerate() {
                        if controls::selectable_value(ui, &mut channel.route, value, name).clicked()
                        {
                            ui.close();
                        }
                    }
                });
                let (_, section, choices) = source_options(channel.kind);
                controls::choice_menu(
                    ui,
                    format!("{section} · {}", choices[channel.source]),
                    |ui| {
                        for (value, name) in choices.iter().enumerate() {
                            if controls::selectable_value(ui, &mut channel.source, value, *name)
                                .clicked()
                            {
                                ui.close();
                            }
                        }
                    },
                );
            });
    }
}

fn routing(ui: &mut Ui, rect: Rect, index: usize, channel: &mut Channel) {
    const OUTPUTS: [&str; 2] = ["Master", "Music bus"];
    let response = controls::track_header_link_at(
        ui,
        rect,
        ("gallery_track_output", index),
        Icon::Routing,
        "Routing",
        OUTPUTS[channel.route],
    );
    egui::Popup::menu(&response).show(|ui| {
        for (value, name) in OUTPUTS.into_iter().enumerate() {
            if controls::selectable_value(ui, &mut channel.route, value, name).clicked() {
                ui.close();
            }
        }
    });
}

fn source(ui: &mut Ui, rect: Rect, index: usize, channel: &mut Channel) {
    let (icon, section, choices) = source_options(channel.kind);
    let response = controls::track_header_link_at(
        ui,
        rect,
        ("gallery_track_source", index),
        icon,
        section,
        choices[channel.source],
    );
    egui::Popup::menu(&response).show(|ui| {
        for (value, name) in choices.iter().enumerate() {
            if controls::selectable_value(ui, &mut channel.source, value, *name).clicked() {
                ui.close();
            }
        }
    });
}

fn source_options(kind: Kind) -> (Icon, &'static str, &'static [&'static str]) {
    match kind {
        Kind::Audio => (Icon::Record, "Input", &["Stereo 1–2", "Mono 1", "None"]),
        Kind::Midi => (
            Icon::Instrument,
            "Instrument",
            &["Glass keys", "Warm pad", "Choose…"],
        ),
        Kind::Bus => (Icon::Effect, "Channel", &["Effects & sends"]),
    }
}

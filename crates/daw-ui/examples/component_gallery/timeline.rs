use daw_ui::{self as ui, TimelineViewport, WaveformColumn, text, theme, tokens};
use egui::{Align2, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

pub struct State {
    columns: Vec<WaveformColumn>,
}

impl Default for State {
    fn default() -> Self {
        let mut columns = Vec::with_capacity(1300);
        for channel in 0..2 {
            for index in 0..650 {
                let beat = index as f64 / 650.0 * 8.0;
                let phase = (beat * 2.0).fract();
                let pulse = (-phase * 4.4).exp();
                let detail = (beat * 83.0 + channel as f64 * 1.1).sin().abs();
                let maximum =
                    (pulse * (0.22 + detail * 0.62) * if channel == 0 { 1.0 } else { 0.78 }) as f32;
                columns.push(WaveformColumn {
                    beat,
                    channel,
                    minimum: -maximum * 0.88,
                    maximum,
                });
            }
        }
        Self { columns }
    }
}

pub fn show(ui: &mut Ui, state: &mut State) {
    text::subtitle(ui, "Waveform, musical time, and clip identity");
    text::note(
        ui,
        "Musical ruler · waveform · clip edges and selection · high-contrast position cursor",
    );
    ui.add_space(tokens::SPACE_SM);
    let colors = theme::palette(ui.ctx());
    let (ruler, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), tokens::TIMELINE_RULER_HEIGHT),
        Sense::hover(),
    );
    let view = TimelineViewport {
        start_beat: 0.0,
        pixels_per_beat: ruler.width() / 8.0,
        row_height: 31.0,
        ..Default::default()
    };
    ui::paint_timeline_ruler(ui.painter(), ruler, view, 120.0, 4);
    let (clip, _) = ui.allocate_exact_size(vec2(ui.available_width(), 152.0), Sense::hover());
    ui.painter().rect_filled(clip, tokens::RADIUS, colors.bg);
    let title = Rect::from_min_size(clip.min + vec2(10.0, 4.0), vec2(clip.width() - 20.0, 20.0));
    ui::clip_title(
        ui.painter(),
        title,
        FontId::proportional(tokens::FONT_TRACK),
        colors.text,
        "Glass rhythm · linked copy",
        true,
    );
    let plot = Rect::from_min_max(pos2(clip.left(), clip.top() + 28.0), clip.max);
    for step in 0..=16 {
        let x = plot.left() + step as f32 * plot.width() / 16.0;
        ui.painter()
            .vline(x, plot.y_range(), Stroke::new(1.0_f32, colors.grid_minor));
    }
    ui::paint_audio_waveform(
        ui.painter(),
        plot,
        view,
        &state.columns,
        2,
        1.0,
        colors.accent,
    );
    ui::paint_timeline_selection(ui.painter(), plot, view, 2.5, 4.0);
    ui::paint_audio_clip_edges(ui.painter(), plot, view, 0.0, 7.0, colors.accent);
    ui::paint_clip_tail(ui.painter(), plot, view.x_at(plot, 7.0));
    ui::paint_timeline_cursor(ui.painter(), ruler, clip, view.x_at(plot, 3.25));
    ui.add_space(tokens::SPACE_LG);
    ui.columns(2, |columns| {
        text::subtitle(&mut columns[0], "Piano-key states");
        text::note(
            &mut columns[0],
            "piano_key · active, scale and chord highlights",
        );
        columns[0].add_space(tokens::SPACE_SM);
        let width = columns[0].available_width();
        let (bounds, _) = columns[0].allocate_exact_size(vec2(width, 176.0), Sense::hover());
        let keys = [
            ("C5", false),
            ("B4", false),
            ("B♭4", true),
            ("A4", false),
            ("A♭4", true),
            ("G4", false),
            ("F♯4", true),
            ("F4", false),
        ];
        for (index, (name, black)) in keys.into_iter().enumerate() {
            let key = Rect::from_min_size(
                bounds.min + vec2(0.0, index as f32 * 22.0),
                vec2(128.0, 22.0),
            );
            let highlight = match index {
                0 | 3 | 5 => 2,
                1 | 7 => 1,
                _ => 0,
            };
            ui::piano_key(
                columns[0].painter(),
                key,
                name,
                black,
                highlight,
                index == 3,
            );
            if index == 5 {
                ui::paint_piano_key_selection(columns[0].painter(), key);
            }
            let caption = match index {
                0 => "Chord tone",
                1 => "Scale tone",
                2 => "Black key",
                3 => "Auditioning",
                4 => "Black key",
                5 => "Keyboard selection",
                6 => "Black key",
                _ => "Scale tone",
            };
            columns[0].painter().text(
                pos2(key.right() + 16.0, key.center().y),
                Align2::LEFT_CENTER,
                caption,
                FontId::proportional(tokens::FONT_SMALL),
                colors.muted,
            );
        }
        text::subtitle(&mut columns[1], "Velocity lane");
        text::note(
            &mut columns[1],
            "velocity_marker · ordinary and selected notes",
        );
        columns[1].add_space(tokens::SPACE_SM);
        let width = columns[1].available_width();
        let (bounds, _) = columns[1].allocate_exact_size(vec2(width, 176.0), Sense::hover());
        columns[1]
            .painter()
            .rect_filled(bounds, tokens::RADIUS, colors.bg);
        for (index, level) in [0.65, 0.84, 0.46, 0.94, 0.58, 0.78, 0.52, 0.88]
            .into_iter()
            .enumerate()
        {
            let column = Rect::from_min_max(
                bounds.min + vec2(18.0 + index as f32 * (bounds.width() - 36.0) / 8.0, 18.0),
                pos2(
                    bounds.left() + 18.0 + (index + 1) as f32 * (bounds.width() - 36.0) / 8.0,
                    bounds.bottom() - 16.0,
                ),
            );
            ui::velocity_marker(columns[1].painter(), column, level, index == 3);
        }
    });
    ui.add_space(tokens::SPACE_MD);
    text::note(
        ui,
        "Deterministic example peak columns and MIDI values; no audio device or project is opened.",
    );
}

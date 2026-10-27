//! Compare native bar tessellation with progressive envelope preparation/reuse.
//! cargo run --release -p daw-ui --example waveform_bench
use daw_ui::{TimelineViewport, WaveformColumn};
use egui::{Color32, Context, RawInput, Rect};
use std::{hint::black_box, time::Instant};

fn columns(width: usize, density: f32) -> Vec<WaveformColumn> {
    let bins = (width as f32 * density).ceil() as usize;
    (0..bins)
        .flat_map(|index| {
            (0..2).map(move |channel| {
                let phase = index as f32 / density * 0.035 + channel as f32;
                let amplitude = 0.05 + phase.sin().abs() * 0.7;
                WaveformColumn {
                    beat: index as f64 / f64::from(density) / 100.0,
                    channel,
                    minimum: -amplitude,
                    maximum: amplitude * 0.8,
                }
            })
        })
        .collect()
}

fn main() {
    let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 360.0));
    let view = TimelineViewport {
        pixels_per_beat: 100.0,
        row_height: 90.0,
        ..Default::default()
    };
    let iterations = 200;
    for density in [1.0, 2.0] {
        let ctx = Context::default();
        daw_ui::theme::apply(&ctx);
        ctx.set_pixels_per_point(density);
        let raw = columns(1440, 1.0);
        let refined = columns(1440, density);
        let prepared = daw_ui::prepare_audio_waveform(
            rect,
            view,
            &refined,
            2,
            1.0,
            Color32::WHITE,
            density,
            Color32::GRAY,
        );
        let mut elapsed = Vec::new();
        for mode in 0..3 {
            // Warm font/paint storage before measuring every path.
            for measuring in [false, true] {
                let start = Instant::now();
                for _ in 0..if measuring { iterations } else { 5 } {
                    let output = ctx.run(
                        RawInput {
                            screen_rect: Some(rect),
                            ..Default::default()
                        },
                        |ctx| {
                            let painter = ctx
                                .layer_painter(egui::LayerId::background())
                                .with_clip_rect(rect);
                            match mode {
                                0 => daw_ui::paint_audio_waveform_raw(
                                    &painter,
                                    rect,
                                    view,
                                    &raw,
                                    2,
                                    1.0,
                                    Color32::WHITE,
                                ),
                                1 => daw_ui::prepare_audio_waveform(
                                    rect,
                                    view,
                                    &refined,
                                    2,
                                    1.0,
                                    Color32::WHITE,
                                    density,
                                    Color32::GRAY,
                                )
                                .paint(&painter),
                                _ => prepared.paint(&painter),
                            }
                        },
                    );
                    black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
                }
                if measuring {
                    elapsed.push(start.elapsed().as_secs_f64() * 1000.0 / f64::from(iterations));
                }
            }
        }
        println!(
            "{density}x · bars {:.3} ms · rebuild {:.3} ms · cached {:.3} ms · {} vertices",
            elapsed[0], elapsed[1], elapsed[2], prepared.stats.vertices
        );
    }
}

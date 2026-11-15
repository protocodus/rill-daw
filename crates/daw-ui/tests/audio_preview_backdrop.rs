use daw_ui::{
    AudioPreviewBackdrop, AudioPreviewBackdropGeometry, AudioPreviewState,
    paint_audio_preview_backdrop, theme,
};
use egui::{Color32, Context, FullOutput, Id, LayerId, Order, RawInput, Rect, Shape, pos2, vec2};

fn paint(
    ctx: &Context,
    data: AudioPreviewBackdrop<'_>,
) -> (FullOutput, AudioPreviewBackdropGeometry, Rect) {
    let clip = Rect::from_min_size(pos2(20.0, 20.0), vec2(260.0, 340.0));
    let mut geometry = AudioPreviewBackdropGeometry::default();
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 720.0))),
            ..Default::default()
        },
        |ctx| {
            let painter = ctx
                .layer_painter(LayerId::new(Order::Background, Id::new("preview")))
                .with_clip_rect(clip);
            geometry = paint_audio_preview_backdrop(
                &painter,
                clip.expand(20.0),
                data,
                theme::palette(ctx).bg,
            );
        },
    );
    (output, geometry, clip)
}

fn luminance(color: Color32) -> f32 {
    let linear = |value: u8| {
        let value = f32::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}
fn contrast(a: Color32, b: Color32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn real_analysis_stays_bounded_quiet_and_legible_in_both_themes() {
    let peaks: Vec<_> = (0..100_000)
        .map(|i| {
            let amplitude = (i % 101) as f32 / 100.0;
            [-amplitude, amplitude]
        })
        .collect();
    let bands: Vec<_> = (0..48).map(|i| (i % 12) as f32 / 11.0).collect();
    for appearance in [theme::Appearance::Dark, theme::Appearance::Light] {
        let ctx = Context::default();
        theme::set_appearance(&ctx, appearance);
        let (output, geometry, clip) = paint(
            &ctx,
            AudioPreviewBackdrop {
                waveform: &peaks,
                spectrum: &bands,
                state: AudioPreviewState::Ready,
            },
        );
        let waveform = geometry.waveform.unwrap();
        let spectrum = geometry.spectrum.unwrap();
        assert!(clip.contains_rect(waveform) && clip.contains_rect(spectrum));
        assert!(waveform.bottom() < spectrum.top());
        let palette = theme::palette(&ctx);
        let check_color = |color: Color32| {
            assert_eq!(color.a(), 255, "Preblended marks must not compound opacity");
            assert!(contrast(palette.text, color) >= 4.5);
            assert!(contrast(palette.muted, color) >= 4.5);
        };
        let mut vertices = 0;
        for clipped in &output.shapes {
            assert!(clip.contains_rect(clipped.clip_rect));
            match &clipped.shape {
                Shape::Mesh(mesh) => {
                    vertices += mesh.vertices.len();
                    for vertex in &mesh.vertices {
                        assert!(clip.contains(vertex.pos));
                        check_color(vertex.color);
                    }
                }
                Shape::Path(path) => {
                    assert!(path.points.iter().all(|point| clip.contains(*point)));
                    assert!(path.points.len() <= 512);
                    if let egui::epaint::ColorMode::Solid(color) = path.stroke.color {
                        check_color(color);
                    }
                }
                Shape::Noop => (),
                shape => panic!("Backdrop must contain only analysis shapes: {shape:?}"),
            }
        }
        assert!(
            vertices > 0 && vertices <= 2048,
            "Geometry must be bounded by display resolution"
        );
    }
}

#[test]
fn pending_missing_and_invalid_data_never_draw_placeholder_audio() {
    let ctx = Context::default();
    theme::apply(&ctx);
    for state in [
        AudioPreviewState::Empty,
        AudioPreviewState::Loading,
        AudioPreviewState::Failed,
    ] {
        let (output, geometry, _) = paint(
            &ctx,
            AudioPreviewBackdrop {
                waveform: &[[-0.5, 0.5]],
                spectrum: &[0.8],
                state,
            },
        );
        assert_eq!(geometry, AudioPreviewBackdropGeometry::default());
        assert!(output.shapes.is_empty());
    }
    for data in [
        AudioPreviewBackdrop {
            state: AudioPreviewState::Ready,
            ..Default::default()
        },
        AudioPreviewBackdrop {
            waveform: &[[f32::NAN, 0.5]],
            spectrum: &[f32::INFINITY],
            state: AudioPreviewState::Ready,
        },
    ] {
        let (output, geometry, _) = paint(&ctx, data);
        assert_eq!(geometry, AudioPreviewBackdropGeometry::default());
        assert!(output.shapes.is_empty());
    }
}

#[test]
fn backdrop_never_changes_foreground_layout_widget_ids_or_focus() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let focus = Id::new("metadata-field");
    let _ = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.memory_mut(|memory| memory.request_focus(focus));
            let cursor = ui.cursor();
            let next_id = ui.next_auto_id();
            let geometry = paint_audio_preview_backdrop(
                ui.painter(),
                ui.max_rect(),
                AudioPreviewBackdrop {
                    waveform: &[[-0.5, 0.5]; 32],
                    spectrum: &[0.5; 48],
                    state: AudioPreviewState::Ready,
                },
                theme::palette(ctx).panel,
            );
            assert!(geometry.waveform.is_some() && geometry.spectrum.is_some());
            assert_eq!(ui.cursor(), cursor);
            assert_eq!(ui.next_auto_id(), next_id);
            assert!(ui.memory(|memory| memory.has_focus(focus)));
        });
    });
}

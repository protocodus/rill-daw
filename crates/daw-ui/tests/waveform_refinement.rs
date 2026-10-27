use daw_ui::{TimelineViewport, WaveformColumn, prepare_audio_waveform};
use egui::{Color32, Context, Rect, Shape, pos2};
use std::sync::Arc;

fn columns() -> Vec<WaveformColumn> {
    (0..2)
        .flat_map(|channel| {
            (0..4).map(move |index| WaveformColumn {
                beat: index as f64,
                channel,
                minimum: if index == 2 { -0.9 } else { -0.1 },
                maximum: if index == 1 { 0.8 } else { 0.2 },
            })
        })
        .collect()
}

#[test]
fn envelope_is_one_bounded_stereo_mesh_with_physical_pixel_edge_coverage() {
    for density in [1.0, 2.0, 3.0] {
        let ctx = Context::default();
        let rect = Rect::from_min_max(pos2(10.0, 20.0), pos2(210.0, 220.0));
        let view = TimelineViewport {
            start_beat: 0.0,
            pixels_per_beat: 50.0,
            row_offset: 0.0,
            row_height: 50.0,
        };
        let preview = prepare_audio_waveform(
            rect,
            view,
            &columns(),
            2,
            1.0,
            Color32::WHITE,
            density,
            Color32::GRAY,
        );
        assert!(preview.stats.vertices <= columns().len() * 4 + 8);
        let output = ctx.run(Default::default(), |ctx| {
            preview.paint(&ctx.layer_painter(egui::LayerId::background()))
        });
        let meshes: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let Shape::Mesh(mesh) = &shape.shape {
                    Some(mesh)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(meshes.len(), 1, "stereo requires only one envelope draw");
        let mesh = meshes[0];
        assert!(mesh.is_valid());
        assert_eq!(mesh.vertices.len(), preview.stats.vertices);
        assert_eq!(mesh.indices.len(), preview.stats.indices);
        assert_eq!(
            mesh.vertices
                .iter()
                .filter(|vertex| vertex.color.a() == 0)
                .count(),
            preview.stats.fringe_vertices
        );
        for strip in mesh.vertices.as_chunks::<4>().0 {
            assert!(strip.iter().all(|vertex| vertex.pos.is_finite()));
            assert!((strip[0].pos.distance(strip[1].pos) - density.recip()).abs() < 0.0001);
            assert!((strip[3].pos.distance(strip[2].pos) - density.recip()).abs() < 0.0001);
        }
        // Source extrema stay in the solid edge: AA cannot average away the
        // isolated peak, or connect the stereo channels into one envelope.
        assert!(
            mesh.vertices
                .iter()
                .any(|vertex| vertex.color.a() > 0 && (vertex.pos.y - 30.0).abs() < 0.001)
        );
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let belongs_to_first = |index: u32| (index as usize) < mesh.vertices.len() / 2;
            assert!(
                triangle
                    .iter()
                    .all(|&index| belongs_to_first(index) == belongs_to_first(triangle[0]))
            );
        }
    }
}

#[test]
fn repaint_reuses_mesh_storage_and_retains_the_parent_clip() {
    let ctx = Context::default();
    let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 200.0));
    let clip = Rect::from_min_max(pos2(20.0, 30.0), pos2(180.0, 170.0));
    let preview = prepare_audio_waveform(
        rect,
        TimelineViewport {
            pixels_per_beat: 50.0,
            row_height: 50.0,
            ..Default::default()
        },
        &columns(),
        2,
        1.0,
        Color32::WHITE,
        1.0,
        Color32::GRAY,
    );
    let output = ctx.run(Default::default(), |ctx| {
        let painter = ctx
            .layer_painter(egui::LayerId::background())
            .with_clip_rect(clip);
        preview.paint(&painter);
        preview.clone().paint(&painter);
    });
    let meshes: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| {
            assert_eq!(shape.clip_rect, clip);
            if let Shape::Mesh(mesh) = &shape.shape {
                Some(mesh)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(meshes.len(), 2);
    assert!(
        Arc::ptr_eq(meshes[0], meshes[1]),
        "cached repaint must not duplicate mesh vertices"
    );
}

#[test]
fn a_single_column_remains_visible_and_invalid_geometry_is_rejected() {
    let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 200.0));
    let point = [WaveformColumn {
        beat: 1.0,
        channel: 0,
        minimum: 0.2,
        maximum: 0.2,
    }];
    let view = TimelineViewport {
        pixels_per_beat: 50.0,
        row_height: 50.0,
        ..Default::default()
    };
    let one = prepare_audio_waveform(
        rect,
        view,
        &point,
        1,
        1.0,
        Color32::WHITE,
        2.0,
        Color32::GRAY,
    );
    assert!(one.stats.vertices > 0 && one.stats.indices > 0);
    for density in [0.0, f32::NAN, f32::INFINITY] {
        assert_eq!(
            prepare_audio_waveform(
                rect,
                view,
                &point,
                1,
                1.0,
                Color32::WHITE,
                density,
                Color32::GRAY
            )
            .stats
            .vertices,
            0
        );
    }
}

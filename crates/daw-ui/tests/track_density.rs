use daw_ui::{TrackDensity, TrackHeaderLayout};

#[test]
fn named_track_densities_keep_basic_channel_controls_and_detail_access() {
    assert_eq!(TrackDensity::default(), TrackDensity::Compact);
    for density in TrackDensity::ALL {
        let rect = egui::Rect::from_min_size(
            egui::pos2(21.0, 34.0),
            egui::vec2(288.0, density.row_height()),
        );
        let layout = TrackHeaderLayout::new(rect).with_record_arm().with_depth(2);
        for part in [layout.mix, layout.meter, layout.record_arm] {
            assert!(
                part.is_some_and(|part| rect.contains_rect(part)),
                "Missing control at {density:?}"
            );
        }
        if density == TrackDensity::Detailed {
            assert!(layout.routing.is_some() && layout.device.is_some());
        } else {
            assert!(
                layout.details.is_some(),
                "Compact layouts must keep one-click full controls"
            );
        }
    }
}

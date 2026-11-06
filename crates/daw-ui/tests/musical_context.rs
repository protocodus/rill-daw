use daw_ui::duration::{compact_musical_position, musical_position};

#[test]
fn two_decimal_positions_carry_before_decomposing_bars_and_beats() {
    for (beat, meter, expected) in [
        (0.0, 4, "Bar 1 · Beat 1.00"),
        (3.994, 4, "Bar 1 · Beat 4.99"),
        (3.999, 4, "Bar 2 · Beat 1.00"),
        (7.999, 4, "Bar 3 · Beat 1.00"),
        (2.999, 3, "Bar 2 · Beat 1.00"),
        (6.999, 7, "Bar 2 · Beat 1.00"),
        (32.125, 4, "Bar 9 · Beat 1.13"),
        (32.5, 4, "Bar 9 · Beat 1.50"),
        (1.999, 0, "Bar 3 · Beat 1.00"),
    ] {
        assert_eq!(musical_position(beat, meter), expected);
    }
}

#[test]
fn compact_song_positions_carry_rounded_beats_across_bar_boundaries() {
    for (beat, meter, expected) in [
        (0.0, 4, "1:1"),
        (3.9994, 4, "1:4.999"),
        (3.9996, 4, "2:1"),
        (7.9999, 4, "3:1"),
        (2.9996, 3, "2:1"),
        (31.9996, 4, "9:1"),
        (32.125, 4, "9:1.125"),
        (32.5, 4, "9:1.5"),
        (1.9999, 0, "3:1"),
    ] {
        assert_eq!(compact_musical_position(beat, meter), expected);
    }
}

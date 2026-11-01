use daw_ui::duration::compact_musical_position;

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

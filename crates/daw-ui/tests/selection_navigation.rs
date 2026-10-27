use daw_ui::{SelectionDirection as D, SelectionItem as Item, adjacent_selection_item as adjacent};

fn item(id: u64, start: f64, end: f64, row: f64) -> Item {
    Item {
        id,
        start,
        end,
        row,
    }
}

#[test]
fn time_navigation_and_vertical_overlap_are_deterministic_for_chords_and_tracks() {
    let items = vec![
        item(1, 0.0, 2.0, 4.0),
        item(2, 4.0, 5.0, 4.0),
        item(3, 4.0, 6.0, 2.0),
        item(4, 1.0, 2.0, 1.0),
        item(5, 7.0, 8.0, 2.0),
    ];
    assert_eq!(adjacent(&items, Some(2), D::Up), Some(3));
    assert_eq!(adjacent(&items, Some(3), D::Down), Some(2));
    assert_eq!(adjacent(&items, Some(1), D::Right), Some(4));
    assert_eq!(adjacent(&items, Some(5), D::Left), Some(3));
    let mut reversed = items.clone();
    reversed.reverse();
    for direction in [D::Left, D::Right, D::Up, D::Down] {
        for current in [None, Some(1), Some(2), Some(3), Some(4), Some(5)] {
            assert_eq!(
                adjacent(&items, current, direction),
                adjacent(&reversed, current, direction)
            );
        }
    }
}

#[test]
fn edges_do_not_wrap_and_invalid_geometry_never_captures_selection() {
    let items = [item(1, 0.0, 1.0, 0.0), item(2, f64::NAN, 2.0, 1.0)];
    for direction in [D::Left, D::Right, D::Up, D::Down] {
        assert_eq!(adjacent(&items, Some(1), direction), Some(1));
        assert_eq!(adjacent(&items, None, direction), Some(1));
        assert_eq!(adjacent(&[], None, direction), None);
    }
}

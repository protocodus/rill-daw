use daw_core::{SampleFavorite, SampleTagIcon};
use serde_json::json;

#[test]
fn favorite_metadata_defaults_and_semantic_icons_round_trip() {
    let empty: SampleFavorite = serde_json::from_value(json!({})).unwrap();
    assert!(empty.tag.is_empty());
    assert_eq!(empty.color, None);
    assert_eq!(empty.icon, SampleTagIcon::Star);
    empty.validate().unwrap();
    let serialized = serde_json::to_value(&empty).unwrap();
    assert!(serialized.get("tag").is_none());
    assert!(serialized.get("color").is_none());

    for (icon, key) in [
        (SampleTagIcon::Star, "star"),
        (SampleTagIcon::Drum, "drum"),
        (SampleTagIcon::Percussion, "percussion"),
        (SampleTagIcon::Tone, "tone"),
        (SampleTagIcon::Texture, "texture"),
        (SampleTagIcon::Voice, "voice"),
        (SampleTagIcon::Foley, "foley"),
    ] {
        assert_eq!(serde_json::to_value(icon).unwrap(), json!(key));
        let value = json!({"tag":"Soft attack", "color":[32, 128, 255], "icon":key});
        let favorite: SampleFavorite = serde_json::from_value(value).unwrap();
        favorite.validate().unwrap();
        assert_eq!(favorite.icon, icon);
        assert_eq!(favorite.color, Some([32, 128, 255]));
        assert_eq!(favorite.tag, "Soft attack");
        let restored: SampleFavorite =
            serde_json::from_value(serde_json::to_value(&favorite).unwrap()).unwrap();
        assert_eq!(restored, favorite);
    }
}

#[test]
fn favorite_tags_are_bounded_by_characters_and_reject_controls() {
    for tag in [String::new(), "x".repeat(64), "声".repeat(64)] {
        let favorite: SampleFavorite = serde_json::from_value(json!({"tag":tag})).unwrap();
        favorite.validate().unwrap();
    }
    for tag in [
        "x".repeat(65),
        "声".repeat(65),
        "Kick\nSnare".into(),
        "Kick\tSnare".into(),
        "Kick\0Snare".into(),
        "Kick\u{7f}".into(),
    ] {
        let favorite: SampleFavorite = serde_json::from_value(json!({"tag":tag})).unwrap();
        assert!(
            favorite.validate().is_err(),
            "invalid tag accepted: {favorite:?}"
        );
    }
}

#[test]
fn unknown_icons_and_malformed_rgb_colors_are_rejected_on_load() {
    for value in [
        json!({"icon":"unknown"}),
        json!({"icon":"Star"}),
        json!({"icon":3}),
        json!({"color":[1,2]}),
        json!({"color":[1,2,3,4]}),
        json!({"color":[-1,2,3]}),
        json!({"color":[256,2,3]}),
        json!({"color":[1.5,2,3]}),
        json!({"color":"#123456"}),
    ] {
        assert!(
            serde_json::from_value::<SampleFavorite>(value.clone()).is_err(),
            "invalid favorite metadata accepted: {value}"
        );
    }
}

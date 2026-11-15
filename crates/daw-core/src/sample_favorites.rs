//! Portable values for the user's sample-library catalog. Favorites belong to
//! that catalog, independently of which projects reference a recording.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const MAX_SAMPLE_TAG_CHARS: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleTagIcon {
    #[default]
    Star,
    Drum,
    Percussion,
    Tone,
    Texture,
    Voice,
    Foley,
}

impl SampleTagIcon {
    pub const ALL: [Self; 7] = [
        Self::Star,
        Self::Drum,
        Self::Percussion,
        Self::Tone,
        Self::Texture,
        Self::Voice,
        Self::Foley,
    ];
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleFavorite {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    #[serde(default)]
    pub icon: SampleTagIcon,
}

impl SampleFavorite {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.tag.chars().count() <= MAX_SAMPLE_TAG_CHARS
                && !self.tag.chars().any(char::is_control),
            "Sample tags have at most {MAX_SAMPLE_TAG_CHARS} printable characters"
        );
        Ok(())
    }
}

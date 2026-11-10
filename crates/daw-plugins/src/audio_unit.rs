//! Audio Unit identity and metadata rules. Pure and cross-platform, so projects
//! and catalogues naming Audio Units validate everywhere; only macOS hosts them
//! (`au.rs`).
//!
//! An Audio Unit has no file per plug-in: the system's component registry is its
//! "bundle". Every AU reference therefore shares one locator, the AudioToolbox
//! framework that owns that registry, and the class ID names the component by its
//! `type:subtype:manufacturer` four-character codes.
use anyhow::{Result, ensure};
use std::fmt;

/// The shared reference path of every Audio Unit. It exists on every Mac, so
/// path-existence checks keep working, and no plug-in folder contains it.
pub const AU_LOCATOR: &str = "/System/Library/Frameworks/AudioToolbox.framework";

/// Instruments.
pub const TYPE_MUSIC_DEVICE: u32 = u32::from_be_bytes(*b"aumu");
pub const TYPE_EFFECT: u32 = u32::from_be_bytes(*b"aufx");
/// Effects that also take MIDI.
pub const TYPE_MUSIC_EFFECT: u32 = u32::from_be_bytes(*b"aumf");
/// Component types Rill lists and hosts.
pub const HOSTED_TYPES: [u32; 3] = [TYPE_MUSIC_DEVICE, TYPE_EFFECT, TYPE_MUSIC_EFFECT];

/// `kAudioUnitProperty_ClassInfo` is stored as a binary property list.
const BINARY_PLIST_MAGIC: &[u8] = b"bplist00";
const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;

/// An `AudioComponentDescription` without flags, written as `aufx:dely:appl`.
/// A code whose bytes are not printable ASCII (or contain `:`) is written as
/// `0x` and eight hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioUnitId {
    pub kind: u32,
    pub subtype: u32,
    pub manufacturer: u32,
}

impl AudioUnitId {
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        const ERROR: &str = "Audio Unit ID must be type:subtype:manufacturer four-character codes, such as aufx:dely:appl";
        let mut codes = text.split(':').map(|code| parse_code(code).ok_or(ERROR));
        let id = Self {
            kind: codes.next().ok_or(ERROR)??,
            subtype: codes.next().ok_or(ERROR)??,
            manufacturer: codes.next().ok_or(ERROR)??,
        };
        if codes.next().is_some() {
            return Err(ERROR);
        }
        Ok(id)
    }

    pub fn is_hosted_type(self) -> bool {
        HOSTED_TYPES.contains(&self.kind)
    }
}

impl fmt::Display for AudioUnitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            Code(self.kind),
            Code(self.subtype),
            Code(self.manufacturer)
        )
    }
}

struct Code(u32);

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = self.0.to_be_bytes();
        if bytes.iter().all(|&b| printable(b)) {
            bytes.iter().try_for_each(|&b| write!(f, "{}", b as char))
        } else {
            write!(f, "0x{:08x}", self.0)
        }
    }
}

fn printable(byte: u8) -> bool {
    (0x20..=0x7E).contains(&byte) && byte != b':'
}

fn parse_code(code: &str) -> Option<u32> {
    if let Some(hex) = code.strip_prefix("0x") {
        return (hex.len() == 8 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| u32::from_str_radix(hex, 16).ok())
            .flatten();
    }
    let bytes: [u8; 4] = code.as_bytes().try_into().ok()?;
    bytes
        .iter()
        .all(|&b| printable(b))
        .then(|| u32::from_be_bytes(bytes))
}

/// Rill's VST3-style category list for a component type.
pub fn category(kind: u32) -> &'static str {
    match kind {
        TYPE_MUSIC_DEVICE => "Instrument",
        TYPE_MUSIC_EFFECT => "Fx|MIDI controlled",
        _ => "Fx",
    }
}

/// `AudioComponentCopyName` returns "Manufacturer: Name".
pub fn split_component_name(full: &str) -> (String, String) {
    match full.split_once(':') {
        Some((vendor, name)) if !name.trim().is_empty() => {
            (vendor.trim().to_owned(), name.trim().to_owned())
        }
        _ => (String::new(), full.trim().to_owned()),
    }
}

/// `AudioComponentGetVersion` packs `0xMMMMmmbb`.
pub fn version_text(version: u32) -> String {
    format!(
        "{}.{}.{}",
        version >> 16,
        (version >> 8) & 0xFF,
        version & 0xFF
    )
}

/// Display text for an `AudioUnitParameterUnit`; custom units carry their own name.
pub fn unit_label(unit: u32) -> &'static str {
    match unit {
        3 => "%",
        4 => "s",
        5 => "samples",
        6 => "°",
        8 => "Hz",
        9 | 20 => "cents",
        10 => "semitones",
        13 => "dB",
        15 => "°",
        21 => "octaves",
        22 => "BPM",
        23 => "beats",
        24 => "ms",
        _ => "",
    }
}

/// Indexed and boolean parameters take whole values.
pub fn unit_is_stepped(unit: u32) -> bool {
    matches!(unit, 1 | 2)
}

/// Saved state must be the binary property list Rill wrote.
pub fn validate_state(data: &[u8]) -> Result<()> {
    ensure!(
        data.len() <= MAX_STATE_BYTES,
        "Audio Unit state exceeds 64 MiB"
    );
    ensure!(
        data.starts_with(BINARY_PLIST_MAGIC),
        "Audio Unit state must be a binary property list"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_including_spaces_and_unprintable_codes() {
        let delay = AudioUnitId::parse("aufx:dely:appl").unwrap();
        assert_eq!(delay.kind, TYPE_EFFECT);
        assert_eq!(delay.subtype, u32::from_be_bytes(*b"dely"));
        assert_eq!(delay.to_string(), "aufx:dely:appl");
        let dls = AudioUnitId::parse("aumu:dls :appl").unwrap();
        assert_eq!(dls.subtype, u32::from_be_bytes(*b"dls "));
        assert_eq!(dls.to_string(), "aumu:dls :appl");
        let odd = AudioUnitId {
            kind: TYPE_MUSIC_EFFECT,
            subtype: 0x0102_0304,
            manufacturer: u32::from_be_bytes(*b"a:bc"),
        };
        let text = odd.to_string();
        assert_eq!(text, "aumf:0x01020304:0x613a6263");
        assert_eq!(AudioUnitId::parse(&text).unwrap(), odd);
        assert!(odd.is_hosted_type());
        assert!(
            !AudioUnitId::parse("auou:ahal:appl")
                .unwrap()
                .is_hosted_type()
        );
        for bad in [
            "",
            "aufx:dely",
            "aufx:dely:appl:x",
            "aufx:del:appl",
            "aufx:delay:appl",
            "aufx:0x1234:appl",
            "aufx:0xzzzzzzzz:appl",
            "aufx:dé:appl",
        ] {
            assert!(AudioUnitId::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn metadata_maps_to_rill_categories_vendor_and_version() {
        assert_eq!(category(TYPE_MUSIC_DEVICE), "Instrument");
        assert_eq!(category(TYPE_EFFECT), "Fx");
        assert_eq!(category(TYPE_MUSIC_EFFECT), "Fx|MIDI controlled");
        assert_eq!(
            split_component_name("Apple: AUDelay"),
            ("Apple".into(), "AUDelay".into())
        );
        assert_eq!(
            split_component_name("Lonely"),
            (String::new(), "Lonely".into())
        );
        assert_eq!(version_text(0x0001_0203), "1.2.3");
        assert_eq!(unit_label(13), "dB");
        assert!(unit_is_stepped(1) && unit_is_stepped(2) && !unit_is_stepped(13));
    }

    #[test]
    fn state_is_a_bounded_binary_property_list() {
        assert!(validate_state(b"bplist00\xd0\x08").is_ok());
        assert!(validate_state(b"<?xml version").is_err());
        assert!(validate_state(b"RILLCLAP").is_err());
    }
}

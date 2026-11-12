/// How a parameter's plain value maps to the normalized 0–1 value that the
/// project, automation and the engine store.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamRange {
    Linear {
        min: f32,
        max: f32,
    },
    /// Geometric mapping for frequencies and times; `min` must be positive.
    Log {
        min: f32,
        max: f32,
    },
    /// Whole numbers, such as octaves, semitones or a voice count.
    Integer {
        min: i32,
        max: i32,
    },
    /// Plain value is the option index.
    Choice(&'static [&'static str]),
    /// Plain value is 0 (off) or 1 (on).
    Toggle,
}

/// One parameter of a built-in device. `id` and `key` are persisted (projects
/// and automation refer to `id`, presets to `key`) and are never changed or
/// reused; the snapshot test in `tests/conformance.rs` enforces this.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamSpec {
    pub id: u32,
    pub key: &'static str,
    pub name: &'static str,
    pub group: &'static str,
    pub unit: &'static str,
    pub range: ParamRange,
    /// Plain value.
    pub default: f32,
    pub modulatable: bool,
}

impl ParamSpec {
    /// Number of discrete values, for choices and toggles.
    pub fn steps(&self) -> Option<usize> {
        match self.range {
            ParamRange::Integer { min, max } => Some((max - min) as usize + 1),
            ParamRange::Choice(options) => Some(options.len()),
            ParamRange::Toggle => Some(2),
            _ => None,
        }
    }

    pub fn plain_min(&self) -> f32 {
        match self.range {
            ParamRange::Linear { min, .. } | ParamRange::Log { min, .. } => min,
            ParamRange::Integer { min, .. } => min as f32,
            ParamRange::Choice(_) | ParamRange::Toggle => 0.0,
        }
    }

    pub fn plain_max(&self) -> f32 {
        match self.range {
            ParamRange::Linear { max, .. } | ParamRange::Log { max, .. } => max,
            ParamRange::Integer { max, .. } => max as f32,
            ParamRange::Choice(options) => options.len().saturating_sub(1) as f32,
            ParamRange::Toggle => 1.0,
        }
    }

    /// Clamps out-of-range input; nonfinite input takes the default. Discrete
    /// values snap to the nearest option.
    pub fn to_normalized(&self, plain: f32) -> f32 {
        let plain = if plain.is_finite() {
            plain
        } else {
            self.default
        };
        let plain = plain.clamp(self.plain_min(), self.plain_max());
        match self.range {
            ParamRange::Linear { min, max } => (plain - min) / (max - min),
            ParamRange::Log { min, max } => (plain / min).ln() / (max / min).ln(),
            ParamRange::Integer { min, max } => (plain.round() - min as f32) / (max - min) as f32,
            ParamRange::Choice(options) if options.len() > 1 => {
                plain.round() / (options.len() - 1) as f32
            }
            ParamRange::Choice(_) => 0.0,
            ParamRange::Toggle => plain.round(),
        }
    }

    pub fn to_plain(&self, normalized: f32) -> f32 {
        let normalized = if normalized.is_finite() {
            normalized.clamp(0.0, 1.0)
        } else {
            self.default_normalized()
        };
        match self.range {
            ParamRange::Linear { min, max } => min + normalized * (max - min),
            ParamRange::Log { min, max } => min * (max / min).powf(normalized),
            ParamRange::Integer { min, max } => {
                (min as f32 + normalized * (max - min) as f32).round()
            }
            ParamRange::Choice(options) => {
                (normalized * options.len().saturating_sub(1) as f32).round()
            }
            ParamRange::Toggle => normalized.round(),
        }
    }

    pub fn default_normalized(&self) -> f32 {
        self.to_normalized(self.default)
    }

    /// Option label for a choice parameter.
    pub fn choice_label(&self, normalized: f32) -> Option<&'static str> {
        match self.range {
            ParamRange::Choice(options) => options.get(self.to_plain(normalized) as usize).copied(),
            _ => None,
        }
    }

    /// Display text with unit, the same everywhere: "1.20 kHz", "−6.0 dB", "Transistor Ladder".
    pub fn format(&self, normalized: f32) -> String {
        let plain = self.to_plain(normalized);
        match self.range {
            ParamRange::Choice(_) => self.choice_label(normalized).unwrap_or("").to_string(),
            ParamRange::Toggle => if plain >= 0.5 { "On" } else { "Off" }.to_string(),
            ParamRange::Linear { .. } | ParamRange::Log { .. } => format_value(plain, self.unit),
            ParamRange::Integer { min, .. } => {
                let number = if min < 0 && plain > 0.0 {
                    format!("+{plain:.0}")
                } else {
                    format!("{plain:.0}")
                };
                if self.unit.is_empty() {
                    number
                } else {
                    format!("{number} {}", self.unit)
                }
            }
        }
    }

    /// Parses user or tool text into a normalized value. Accepts an optional
    /// unit, `k` for thousands ("1.2k", "1.2 kHz"), seconds for millisecond
    /// parameters ("1.5 s"), ratios with or without ":1", choice labels in
    /// any case, and on/off.
    pub fn parse(&self, text: &str) -> Option<f32> {
        let text = text.trim();
        match self.range {
            ParamRange::Choice(options) => options
                .iter()
                .position(|option| option.eq_ignore_ascii_case(text))
                .map(|index| self.to_normalized(index as f32)),
            ParamRange::Toggle => match text.to_ascii_lowercase().as_str() {
                "on" | "true" | "1" | "yes" => Some(1.0),
                "off" | "false" | "0" | "no" => Some(0.0),
                _ => None,
            },
            ParamRange::Linear { .. } | ParamRange::Log { .. } | ParamRange::Integer { .. } => {
                parse_value(text, self.unit).map(|plain| self.to_normalized(plain))
            }
        }
    }
}

fn format_value(plain: f32, unit: &str) -> String {
    match unit {
        "Hz" if plain.abs() >= 1000.0 => format!("{:.2} kHz", plain / 1000.0),
        "Hz" if plain.abs() < 1.0 => format!("{plain:.2} Hz"),
        // LFO rates: whole hertz would hide most of a turn of the knob.
        "Hz" if plain.abs() < 20.0 => format!("{plain:.1} Hz"),
        "Hz" => format!("{plain:.0} Hz"),
        "ms" if plain.abs() >= 1000.0 => format!("{:.2} s", plain / 1000.0),
        "ms" if plain.abs() < 0.1 => format!("{plain:.2} ms"),
        "ms" if plain.abs() < 10.0 => format!("{plain:.1} ms"),
        "ms" => format!("{plain:.0} ms"),
        "dB" | "oct" | "st" => format!("{plain:+.1} {unit}"),
        "%" | "°" => format!("{plain:.0} {unit}"),
        ":1" => format!("{plain:.1}:1"),
        "" => format!("{plain:.2}"),
        _ => format!("{plain:.2} {unit}"),
    }
}

fn parse_value(text: &str, unit: &str) -> Option<f32> {
    let lower = text.to_ascii_lowercase().replace('−', "-");
    let mut number = lower.trim();
    let mut scale = 1.0;
    if !unit.is_empty() {
        let unit = unit.to_ascii_lowercase();
        if let Some(rest) = number.strip_suffix(unit.as_str()) {
            number = rest.trim_end();
        }
    }
    // "1.5 s" for a millisecond parameter ("ms" was not found above).
    if unit == "ms"
        && let Some(rest) = number.strip_suffix('s')
    {
        number = rest.trim_end();
        scale = 1000.0;
    }
    if let Some(rest) = number.strip_suffix('k') {
        number = rest.trim_end();
        scale *= 1000.0;
    }
    let value: f32 = number.trim_start_matches('+').parse().ok()?;
    let value = value * scale;
    value.is_finite().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUTOFF: ParamSpec = ParamSpec {
        id: 1,
        key: "cutoff",
        name: "Cutoff",
        group: "Filter",
        unit: "Hz",
        range: ParamRange::Log {
            min: 20.0,
            max: 20_000.0,
        },
        default: 1000.0,
        modulatable: true,
    };
    const MODE: ParamSpec = ParamSpec {
        id: 2,
        key: "mode",
        name: "Mode",
        group: "Filter",
        unit: "",
        range: ParamRange::Choice(&["A", "B", "C"]),
        default: 1.0,
        modulatable: false,
    };

    #[test]
    fn log_mapping_round_trips_and_formats() {
        let normalized = CUTOFF.to_normalized(1000.0);
        assert!((CUTOFF.to_plain(normalized) - 1000.0).abs() < 0.01);
        assert_eq!(CUTOFF.format(normalized), "1.00 kHz");
        assert!((CUTOFF.parse("1.2k").unwrap() - CUTOFF.to_normalized(1200.0)).abs() < 1e-6);
        assert!((CUTOFF.parse("1.2 kHz").unwrap() - CUTOFF.to_normalized(1200.0)).abs() < 1e-6);
        assert_eq!(CUTOFF.parse("300 Hz"), Some(CUTOFF.to_normalized(300.0)));
        assert_eq!(CUTOFF.to_normalized(1e9), 1.0);
        assert_eq!(CUTOFF.to_normalized(f32::NAN), CUTOFF.default_normalized());
        assert_eq!(CUTOFF.parse("loud"), None);
    }

    #[test]
    fn choices_snap_and_parse_labels() {
        assert_eq!(MODE.default_normalized(), 0.5);
        assert_eq!(MODE.format(0.9), "C");
        assert_eq!(MODE.parse("b"), Some(0.5));
        assert_eq!(MODE.to_plain(0.3), 1.0);
    }

    #[test]
    fn integers_snap_and_show_their_sign() {
        let semitones = ParamSpec {
            unit: "st",
            range: ParamRange::Integer { min: -12, max: 12 },
            default: 0.0,
            ..CUTOFF
        };
        assert_eq!(semitones.steps(), Some(25));
        assert_eq!(semitones.to_plain(semitones.to_normalized(6.6)), 7.0);
        assert_eq!(semitones.format(semitones.to_normalized(7.0)), "+7 st");
        assert_eq!(semitones.format(semitones.to_normalized(-3.0)), "-3 st");
        assert_eq!(
            semitones.parse("-5 st"),
            Some(semitones.to_normalized(-5.0))
        );
        assert_eq!(semitones.default_normalized(), 0.5);
    }

    #[test]
    fn milliseconds_accept_seconds() {
        let release = ParamSpec {
            unit: "ms",
            range: ParamRange::Log {
                min: 10.0,
                max: 2000.0,
            },
            default: 150.0,
            ..CUTOFF
        };
        assert_eq!(release.parse("1.5 s"), Some(release.to_normalized(1500.0)));
        assert_eq!(release.parse("150 ms"), Some(release.to_normalized(150.0)));
        assert_eq!(release.format(release.to_normalized(1500.0)), "1.50 s");
    }

    #[test]
    fn ratios_and_short_times_keep_their_digits() {
        let ratio = ParamSpec {
            unit: ":1",
            range: ParamRange::Log {
                min: 1.0,
                max: 20.0,
            },
            default: 4.0,
            ..CUTOFF
        };
        assert_eq!(ratio.format(ratio.to_normalized(4.0)), "4.0:1");
        assert_eq!(ratio.parse("4:1"), Some(ratio.to_normalized(4.0)));
        assert_eq!(ratio.parse("2.5"), Some(ratio.to_normalized(2.5)));
        let attack = ParamSpec {
            unit: "ms",
            range: ParamRange::Log {
                min: 0.02,
                max: 250.0,
            },
            default: 10.0,
            ..CUTOFF
        };
        assert_eq!(attack.format(0.0), "0.02 ms");
        assert_eq!(attack.format(attack.to_normalized(0.5)), "0.5 ms");
        assert_eq!(attack.parse("0.05 ms"), Some(attack.to_normalized(0.05)));
        let rate = ParamSpec {
            range: ParamRange::Log {
                min: 0.01,
                max: 20.0,
            },
            default: 0.6,
            ..CUTOFF
        };
        assert_eq!(rate.format(rate.to_normalized(0.6)), "0.60 Hz");
        assert_eq!(rate.format(rate.to_normalized(5.5)), "5.5 Hz");
        assert_eq!(rate.format(1.0), "20 Hz");
        assert_eq!(CUTOFF.format(0.0), "20 Hz");
    }
}

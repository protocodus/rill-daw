//! One module per device: parameter table, `Device` implementation and the
//! pure response functions its panel draws.
use crate::{ParamRange, ParamSpec};

pub mod equalizer;
pub mod filter;
pub mod mini;

/// A parameter; continuous ones are modulatable.
pub(crate) const fn param(
    id: u32,
    key: &'static str,
    name: &'static str,
    group: &'static str,
    unit: &'static str,
    range: ParamRange,
    default: f32,
) -> ParamSpec {
    ParamSpec {
        id,
        key,
        name,
        group,
        unit,
        range,
        default,
        modulatable: matches!(range, ParamRange::Linear { .. } | ParamRange::Log { .. }),
    }
}

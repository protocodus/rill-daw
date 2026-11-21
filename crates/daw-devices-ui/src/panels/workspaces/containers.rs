//! R1 Split and R1 Chain: how the signal is divided on top, the output
//! below; the panel draws the lanes themselves after the output.
use super::*;
use daw_devices::devices::container::{self, BANDS, CROSSOVERS, MIX, OUTPUT, RULE, Rule};

pub(super) fn build(mut layout: Layout, descriptor: &DeviceDescriptor) -> Workspace {
    use Slot::Control as C;
    layout.cells = true;
    layout.sections = if descriptor.id == container::SPLIT.id {
        vec![cells(
            "Split",
            0,
            12,
            &[&[
                C(RULE, 5),
                C(BANDS, 1),
                C(CROSSOVERS[0], 2),
                C(CROSSOVERS[1], 2),
                C(CROSSOVERS[2], 2),
            ]],
        )]
    } else {
        Vec::new()
    };
    layout.captions = &LANE_CAPTIONS;
    layout.leds = &LANE_LEDS;
    // Crossovers that do not apply stay in place, disabled, so switching
    // rules never moves the card.
    layout.keep_inactive = true;
    layout.visible = |id, values| {
        let rule = Rule::from_normalized(values(RULE) as f32);
        let bands = container::bands(values(BANDS) as f32);
        match id {
            BANDS => rule == Rule::Bands,
            id => match CROSSOVERS.iter().position(|crossover| *crossover == id) {
                Some(index) => rule == Rule::Bands && index + 1 < bands,
                None => true,
            },
        }
    };
    layout.hint = |id, _| match id {
        RULE => Some(
            "All: every lane gets the whole signal. Bands: one frequency band per lane. Mid/Side and Left/Right: one part per lane.",
        ),
        BANDS => Some("How many frequency bands, one lane each."),
        _ => None,
    };
    Workspace {
        layout,
        output: vec![MIX, OUTPUT],
        toolbar: vec![],
        body_height: 0.0,
    }
}

/// Inside its lane's card a lane control needs no lane number.
static LANE_CAPTIONS: [(u32, &str); container::MAX_LANES] =
    lane_labels(container::LANE_GAIN, "Gain");
static LANE_LEDS: [(u32, &str); 2 * container::MAX_LANES] = {
    let mute = lane_labels(container::LANE_MUTE, "Mute");
    let solo = lane_labels(container::LANE_SOLO, "Solo");
    let mut all = [(0, ""); 2 * container::MAX_LANES];
    let mut lane = 0;
    while lane < container::MAX_LANES {
        all[lane] = mute[lane];
        all[container::MAX_LANES + lane] = solo[lane];
        lane += 1;
    }
    all
};

const fn lane_labels(
    base: u32,
    label: &'static str,
) -> [(u32, &'static str); container::MAX_LANES] {
    let mut labels = [(0, ""); container::MAX_LANES];
    let mut lane = 0;
    while lane < container::MAX_LANES {
        labels[lane] = (base + lane as u32, label);
        lane += 1;
    }
    labels
}

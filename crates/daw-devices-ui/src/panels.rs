//! Per-device layout rules. A device without an entry here gets the generic
//! layout: one section per parameter group, every control visible.
use daw_devices::DeviceDescriptor;
use daw_devices::devices::{filter, mini};

pub type Values<'a> = &'a dyn Fn(u32) -> f64;

/// Drawing only; at other host rates the curve differs only near Nyquist.
const RESPONSE_RATE: f32 = 48_000.0;

pub struct Section {
    pub title: &'static str,
    pub parameters: Vec<u32>,
    /// Sections sharing a row sit side by side, in signal order.
    pub row: usize,
}

pub struct Layout {
    /// One of the two panel widths of the panel grammar.
    pub width: f32,
    pub sections: Vec<Section>,
    /// The one large knob of the panel, if any.
    pub hero: Option<u32>,
    /// Controls that do nothing in the current mode are hidden, not disabled.
    pub visible: fn(u32, Values<'_>) -> bool,
    /// Choice options that apply in the current mode, when not all do.
    pub options: fn(u32, Values<'_>) -> Option<&'static [&'static str]>,
    /// The response curve drawn above the controls.
    pub response: Option<ResponseView>,
}

pub struct ResponseView {
    /// Fills dB values for the given frequencies.
    pub curve: fn(Values<'_>, &[f32], &mut [f32]),
    /// The frequency parameter whose node moves along the curve's axis.
    pub node: u32,
}

pub fn layout(descriptor: &DeviceDescriptor) -> Layout {
    let mut layout = generic(descriptor);
    if descriptor.id == filter::DESCRIPTOR.id {
        layout.hero = Some(filter::CUTOFF);
        layout.visible = |id, values| filter::applies(id, &filter_settings(values));
        layout.options = |id, values| {
            (id == filter::TYPE).then(|| filter::type_options(filter_settings(values).model))
        };
        layout.response = Some(ResponseView {
            curve: |values, frequencies, out| {
                let settings = filter_settings(values);
                // The drawn curve is static; the sounding cutoff also moves with the LFO and envelope.
                filter::response_db(&settings, settings.cutoff, RESPONSE_RATE, frequencies, out);
            },
            node: filter::CUTOFF,
        });
    }
    if descriptor.id == mini::DESCRIPTOR.id {
        layout.width = daw_ui::tokens::POPUP_WIDTH_LG;
        // Rows in signal order, each filled to fit the width without
        // wrapping: Mixer and Filter side by side would not, so voicing
        // shares the mixer's row and velocity the filter's.
        for section in &mut layout.sections {
            section.row = match section.title {
                "Osc 1" | "Osc 2" => 0,
                "Mixer" | "Voice" => 1,
                "Filter" | "Velocity" => 2,
                "Filter envelope" | "Amp envelope" => 3,
                _ => 4,
            };
        }
        layout.hero = Some(mini::CUTOFF);
        layout.visible = |id, values| mini::applies(id, &mini_settings(values));
        layout.response = Some(ResponseView {
            curve: |values, frequencies, out| {
                mini::response_db(&mini_settings(values), RESPONSE_RATE, frequencies, out);
            },
            node: mini::CUTOFF,
        });
    }
    layout
}

fn mini_settings(values: Values<'_>) -> mini::Settings {
    mini::Settings::from_normalized(|id| values(id) as f32)
}

fn filter_settings(values: Values<'_>) -> filter::Settings {
    filter::Settings::from_normalized(|id| values(id) as f32)
}

/// Groups in the order their first parameter appears in the descriptor,
/// which devices keep in signal order.
fn generic(descriptor: &DeviceDescriptor) -> Layout {
    let mut sections: Vec<Section> = Vec::new();
    for spec in descriptor.parameters {
        match sections
            .iter_mut()
            .find(|section| section.title == spec.group)
        {
            Some(section) => section.parameters.push(spec.id),
            None => sections.push(Section {
                title: spec.group,
                parameters: vec![spec.id],
                row: sections.len(),
            }),
        }
    }
    Layout {
        width: daw_ui::tokens::POPUP_WIDTH_MD,
        sections,
        hero: None,
        visible: |_, _| true,
        options: |_, _| None,
        response: None,
    }
}

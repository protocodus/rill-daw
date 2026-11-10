//! Per-device layout rules. A device without an entry here gets the generic
//! layout: one section per parameter group, every control visible.
use daw_devices::DeviceDescriptor;
use daw_devices::devices::filter;

pub type Values<'a> = &'a dyn Fn(u32) -> f64;

pub struct Section {
    pub title: &'static str,
    pub parameters: Vec<u32>,
}

pub struct Layout {
    pub sections: Vec<Section>,
    /// The one large knob of the panel, if any.
    pub hero: Option<u32>,
    /// Controls that do nothing in the current mode are hidden, not disabled.
    pub visible: fn(u32, Values<'_>) -> bool,
    /// Choice options that apply in the current mode, when not all do.
    pub options: fn(u32, Values<'_>) -> Option<&'static [&'static str]>,
}

pub fn layout(descriptor: &DeviceDescriptor) -> Layout {
    let mut layout = generic(descriptor);
    if descriptor.id == filter::DESCRIPTOR.id {
        layout.hero = Some(filter::CUTOFF);
        layout.visible = |id, values| filter::applies(id, &filter_settings(values));
        layout.options = |id, values| {
            (id == filter::TYPE).then(|| filter::type_options(filter_settings(values).model))
        };
    }
    layout
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
            }),
        }
    }
    Layout {
        sections,
        hero: None,
        visible: |_, _| true,
        options: |_, _| None,
    }
}

//! Deterministic CLAP test plug-ins, compiled by tests against Cargo's `clap-sys` rlib.
//!
//! `org.rill.test.synth`: CLAP/MIDI note input; Main output is a sine per note scaled by
//! velocity, the Level parameter and the Volume note expression; Aux output carries the
//! host transport (tempo / 1000, beat position or -1 when stopped).
//! `org.rill.test.gain`: Main plus Sidechain inputs; Main output is (main + sidechain) * Level,
//! delayed by its reported 32-sample latency.
//! Key 127 aborts and key 126 hangs, for isolation tests. An embedded "editor" reports one
//! Level gesture (to 0.75) from its timer, as a real GUI drag would.
#![allow(clippy::missing_safety_doc, non_upper_case_globals)]
use clap_sys::{
    audio_buffer::clap_audio_buffer,
    entry::clap_plugin_entry,
    events::*,
    ext::{audio_ports::*, gui::*, latency::*, note_ports::*, params::*, state::*, timer_support::*},
    factory::plugin_factory::*,
    host::clap_host,
    id::clap_id,
    plugin::*,
    process::*,
    stream::*,
    version::CLAP_VERSION,
};
use std::ffi::{CStr, c_char, c_void};
use std::ptr::null;

const LATENCY: usize = 32;

struct Features([*const c_char; 3]);
unsafe impl Sync for Features {}
static SYNTH_FEATURES: Features = Features([c"instrument".as_ptr(), c"synthesizer".as_ptr(), null()]);
static GAIN_FEATURES: Features = Features([c"audio-effect".as_ptr(), c"utility".as_ptr(), null()]);

struct Descriptor(clap_plugin_descriptor);
unsafe impl Sync for Descriptor {}
const fn descriptor(id: &'static CStr, name: &'static CStr, features: &'static Features) -> Descriptor {
    Descriptor(clap_plugin_descriptor {
        clap_version: CLAP_VERSION,
        id: id.as_ptr(),
        name: name.as_ptr(),
        vendor: c"RILL tests".as_ptr(),
        url: c"".as_ptr(),
        manual_url: c"".as_ptr(),
        support_url: c"".as_ptr(),
        version: c"1.0.0".as_ptr(),
        description: c"".as_ptr(),
        features: features.0.as_ptr(),
    })
}
static SYNTH: Descriptor = descriptor(c"org.rill.test.synth", c"Rill CLAP Synth", &SYNTH_FEATURES);
static GAIN: Descriptor = descriptor(c"org.rill.test.gain", c"Rill CLAP Gain", &GAIN_FEATURES);

struct Voice {
    key: i16,
    channel: i16,
    id: i32,
    velocity: f64,
    tuning: f64,
    volume: f64,
    phase: f64,
}

struct Probe {
    plugin: clap_plugin,
    host: *const clap_host,
    effect: bool,
    level: f64,
    mode: f64,
    sample_rate: f64,
    voices: Vec<Voice>,
    delay: Vec<[f32; 2]>,
    delay_position: usize,
    timer: Option<clap_id>,
    gui_shown: bool,
    gesture_pending: bool,
}

unsafe fn probe<'a>(plugin: *const clap_plugin) -> &'a mut Probe {
    unsafe { &mut *((*plugin).plugin_data as *mut Probe) }
}

unsafe fn host_extension<T>(probe: &Probe, id: &CStr) -> Option<&'static T> {
    unsafe {
        let get = (*probe.host).get_extension?;
        (get(probe.host, id.as_ptr()) as *const T).as_ref()
    }
}

unsafe extern "C" fn init(_plugin: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn destroy(plugin: *const clap_plugin) {
    unsafe { drop(Box::from_raw((*plugin).plugin_data as *mut Probe)) }
}
unsafe extern "C" fn activate(plugin: *const clap_plugin, rate: f64, _min: u32, _max: u32) -> bool {
    let probe = unsafe { probe(plugin) };
    probe.sample_rate = rate;
    probe.delay = vec![[0.0; 2]; LATENCY];
    probe.delay_position = 0;
    true
}
unsafe extern "C" fn deactivate(_plugin: *const clap_plugin) {}
unsafe extern "C" fn start_processing(_plugin: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn stop_processing(_plugin: *const clap_plugin) {}
unsafe extern "C" fn reset(plugin: *const clap_plugin) {
    let probe = unsafe { probe(plugin) };
    probe.voices.clear();
    probe.delay.iter_mut().for_each(|frame| *frame = [0.0; 2]);
}

fn apply_param(probe: &mut Probe, id: clap_id, value: f64) {
    match id {
        0 => probe.level = value.clamp(0.0, 2.0),
        1 => probe.mode = value.round().clamp(0.0, 3.0),
        _ => {}
    }
}

fn note_on(probe: &mut Probe, key: i16, channel: i16, id: i32, velocity: f64) {
    match key {
        127 => std::process::abort(),
        126 => loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        },
        _ => {}
    }
    probe.voices.push(Voice { key, channel, id, velocity, tuning: 0.0, volume: 1.0, phase: 0.0 });
}

unsafe fn handle_event(probe: &mut Probe, header: *const clap_event_header) {
    let header = unsafe { &*header };
    if header.space_id != CLAP_CORE_EVENT_SPACE_ID {
        return;
    }
    match header.type_ {
        CLAP_EVENT_NOTE_ON => {
            let note = unsafe { &*(header as *const _ as *const clap_event_note) };
            note_on(probe, note.key, note.channel, note.note_id, note.velocity);
        }
        CLAP_EVENT_NOTE_OFF | CLAP_EVENT_NOTE_CHOKE => {
            let note = unsafe { &*(header as *const _ as *const clap_event_note) };
            probe.voices.retain(|voice| {
                !((note.key < 0 || note.key == voice.key)
                    && (note.channel < 0 || note.channel == voice.channel)
                    && (note.note_id < 0 || note.note_id == voice.id))
            });
        }
        CLAP_EVENT_NOTE_EXPRESSION => {
            let e = unsafe { &*(header as *const _ as *const clap_event_note_expression) };
            for voice in probe.voices.iter_mut().filter(|voice| {
                (e.key < 0 || e.key == voice.key) && (e.note_id < 0 || e.note_id == voice.id)
            }) {
                match e.expression_id {
                    CLAP_NOTE_EXPRESSION_TUNING => voice.tuning = e.value,
                    CLAP_NOTE_EXPRESSION_VOLUME => voice.volume = e.value,
                    _ => {}
                }
            }
        }
        CLAP_EVENT_MIDI => {
            let midi = unsafe { &*(header as *const _ as *const clap_event_midi) };
            let (status, key, velocity) = (midi.data[0] & 0xF0, midi.data[1] as i16, midi.data[2]);
            let channel = (midi.data[0] & 0x0F) as i16;
            if status == 0x90 && velocity > 0 {
                note_on(probe, key, channel, -1, f64::from(velocity) / 127.0);
            } else if status == 0x80 || status == 0x90 {
                probe.voices.retain(|voice| voice.key != key || voice.channel != channel);
            }
        }
        CLAP_EVENT_PARAM_VALUE => {
            let param = unsafe { &*(header as *const _ as *const clap_event_param_value) };
            apply_param(probe, param.param_id, param.value);
        }
        _ => {}
    }
}

unsafe fn input_events(probe: &mut Probe, events: *const clap_input_events, until: Option<u32>, next: &mut u32) {
    unsafe {
        let count = ((*events).size.unwrap())(events);
        while *next < count {
            let header = ((*events).get.unwrap())(events, *next);
            if until.is_some_and(|frame| (*header).time > frame) {
                break;
            }
            handle_event(probe, header);
            *next += 1;
        }
    }
}

unsafe fn push_gesture(probe: &mut Probe, out: *const clap_output_events) {
    if !probe.gesture_pending {
        return;
    }
    probe.gesture_pending = false;
    probe.level = 0.75;
    let push = unsafe { (*out).try_push.unwrap() };
    let header = |type_: u16, size: usize| clap_event_header {
        size: size as u32,
        time: 0,
        space_id: CLAP_CORE_EVENT_SPACE_ID,
        type_,
        flags: 0,
    };
    let begin = clap_event_param_gesture {
        header: header(CLAP_EVENT_PARAM_GESTURE_BEGIN, size_of::<clap_event_param_gesture>()),
        param_id: 0,
    };
    let value = clap_event_param_value {
        header: header(CLAP_EVENT_PARAM_VALUE, size_of::<clap_event_param_value>()),
        param_id: 0,
        cookie: std::ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value: 0.75,
    };
    let end = clap_event_param_gesture {
        header: header(CLAP_EVENT_PARAM_GESTURE_END, size_of::<clap_event_param_gesture>()),
        param_id: 0,
    };
    unsafe {
        push(out, &begin.header);
        push(out, &value.header);
        push(out, &end.header);
    }
}

unsafe extern "C" fn process(plugin: *const clap_plugin, process: *const clap_process) -> clap_process_status {
    let probe = unsafe { probe(plugin) };
    let process = unsafe { &*process };
    let frames = process.frames_count as usize;
    let outputs = unsafe { std::slice::from_raw_parts_mut(process.audio_outputs, process.audio_outputs_count as usize) };
    let inputs = unsafe { std::slice::from_raw_parts(process.audio_inputs, process.audio_inputs_count as usize) };
    let channel = |buffer: &clap_audio_buffer, index: usize| -> &'static mut [f32] {
        unsafe { std::slice::from_raw_parts_mut(*buffer.data32.add(index), frames) }
    };
    let mut next_event = 0;
    if probe.effect {
        unsafe { input_events(probe, process.in_events, None, &mut next_event) };
        for frame in 0..frames {
            let mut sum = [0.0f32; 2];
            for input in inputs {
                for (side, value) in sum.iter_mut().enumerate() {
                    *value += channel(input, side.min(input.channel_count as usize - 1))[frame];
                }
            }
            let delayed = probe.delay[probe.delay_position];
            probe.delay[probe.delay_position] = [sum[0] * probe.level as f32, sum[1] * probe.level as f32];
            probe.delay_position = (probe.delay_position + 1) % LATENCY;
            channel(&outputs[0], 0)[frame] = delayed[0];
            channel(&outputs[0], 1)[frame] = delayed[1];
        }
    } else {
        let transport = unsafe { process.transport.as_ref() };
        let tempo = transport.map_or(0.0, |t| t.tempo) as f32 / 1000.0;
        let beats = transport.map_or(-1.0, |t| {
            if t.flags & CLAP_TRANSPORT_IS_PLAYING != 0 {
                t.song_pos_beats as f64 / (1u64 << 31) as f64
            } else {
                -1.0
            }
        }) as f32;
        for frame in 0..frames {
            unsafe { input_events(probe, process.in_events, Some(frame as u32), &mut next_event) };
            let mut sample = 0.0f64;
            for voice in &mut probe.voices {
                let frequency = 440.0 * 2f64.powf((f64::from(voice.key) - 69.0 + voice.tuning) / 12.0);
                let wave = (voice.phase * std::f64::consts::TAU).sin();
                let wave = if probe.mode >= 1.0 { wave.signum() } else { wave };
                sample += wave * voice.velocity * voice.volume * 0.25;
                voice.phase = (voice.phase + frequency / probe.sample_rate).fract();
            }
            let sample = (sample * probe.level) as f32;
            channel(&outputs[0], 0)[frame] = sample;
            channel(&outputs[0], 1)[frame] = sample;
            channel(&outputs[1], 0)[frame] = tempo;
            channel(&outputs[1], 1)[frame] = beats;
        }
        unsafe { input_events(probe, process.in_events, None, &mut next_event) };
    }
    unsafe { push_gesture(probe, process.out_events) };
    CLAP_PROCESS_CONTINUE
}

fn write_name(target: &mut [c_char], name: &str) {
    for (slot, byte) in target.iter_mut().zip(name.bytes().chain([0])) {
        *slot = byte as c_char;
    }
}

unsafe extern "C" fn audio_ports_count(plugin: *const clap_plugin, is_input: bool) -> u32 {
    let probe = unsafe { probe(plugin) };
    match (probe.effect, is_input) {
        (true, true) => 2,
        (true, false) => 1,
        (false, true) => 0,
        (false, false) => 2,
    }
}
unsafe extern "C" fn audio_ports_get(plugin: *const clap_plugin, index: u32, is_input: bool, info: *mut clap_audio_port_info) -> bool {
    if index >= unsafe { audio_ports_count(plugin, is_input) } {
        return false;
    }
    let probe = unsafe { probe(plugin) };
    let info = unsafe { &mut *info };
    info.id = index;
    let name = match (probe.effect, is_input, index) {
        (true, true, 1) => "Sidechain",
        (false, false, 1) => "Aux",
        _ => "Main",
    };
    write_name(&mut info.name, name);
    info.flags = if index == 0 { CLAP_AUDIO_PORT_IS_MAIN } else { 0 };
    info.channel_count = 2;
    info.port_type = CLAP_PORT_STEREO.as_ptr();
    info.in_place_pair = u32::MAX;
    true
}
static AUDIO_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(audio_ports_count),
    get: Some(audio_ports_get),
};

unsafe extern "C" fn note_ports_count(plugin: *const clap_plugin, is_input: bool) -> u32 {
    u32::from(is_input && !unsafe { probe(plugin) }.effect)
}
unsafe extern "C" fn note_ports_get(plugin: *const clap_plugin, index: u32, is_input: bool, info: *mut clap_note_port_info) -> bool {
    if index >= unsafe { note_ports_count(plugin, is_input) } {
        return false;
    }
    let info = unsafe { &mut *info };
    info.id = 0;
    info.supported_dialects = CLAP_NOTE_DIALECT_CLAP | CLAP_NOTE_DIALECT_MIDI;
    info.preferred_dialect = CLAP_NOTE_DIALECT_CLAP;
    write_name(&mut info.name, "Notes");
    true
}
static NOTE_PORTS: clap_plugin_note_ports = clap_plugin_note_ports {
    count: Some(note_ports_count),
    get: Some(note_ports_get),
};

unsafe extern "C" fn params_count(_plugin: *const clap_plugin) -> u32 {
    2
}
unsafe extern "C" fn params_info(_plugin: *const clap_plugin, index: u32, info: *mut clap_param_info) -> bool {
    let info = unsafe { &mut *info };
    let (name, max, default, flags) = match index {
        0 => ("Level", 2.0, 1.0, CLAP_PARAM_IS_AUTOMATABLE),
        1 => ("Mode", 3.0, 0.0, CLAP_PARAM_IS_AUTOMATABLE | CLAP_PARAM_IS_STEPPED),
        _ => return false,
    };
    info.id = index;
    info.flags = flags;
    info.cookie = std::ptr::null_mut();
    write_name(&mut info.name, name);
    write_name(&mut info.module, "");
    info.min_value = 0.0;
    info.max_value = max;
    info.default_value = default;
    true
}
unsafe extern "C" fn params_value(plugin: *const clap_plugin, id: clap_id, value: *mut f64) -> bool {
    let probe = unsafe { probe(plugin) };
    let current = match id {
        0 => probe.level,
        1 => probe.mode,
        _ => return false,
    };
    unsafe { *value = current };
    true
}
unsafe extern "C" fn params_to_text(_plugin: *const clap_plugin, id: clap_id, value: f64, buffer: *mut c_char, capacity: u32) -> bool {
    let text = match id {
        0 => format!("{value:.2} x"),
        1 => ["Sine", "Square", "Saw", "Noise"][value.round().clamp(0.0, 3.0) as usize].to_owned(),
        _ => return false,
    };
    let target = unsafe { std::slice::from_raw_parts_mut(buffer, capacity as usize) };
    write_name(target, &text);
    true
}
unsafe extern "C" fn params_flush(plugin: *const clap_plugin, input: *const clap_input_events, out: *const clap_output_events) {
    let probe = unsafe { probe(plugin) };
    let mut next = 0;
    unsafe {
        input_events(probe, input, None, &mut next);
        push_gesture(probe, out);
    }
}
static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(params_count),
    get_info: Some(params_info),
    get_value: Some(params_value),
    value_to_text: Some(params_to_text),
    text_to_value: None,
    flush: Some(params_flush),
};

const MAGIC: &[u8; 8] = b"RILLCLAP";
unsafe extern "C" fn state_save(plugin: *const clap_plugin, stream: *const clap_ostream) -> bool {
    let probe = unsafe { probe(plugin) };
    let mut bytes = MAGIC.to_vec();
    bytes.extend(probe.level.to_le_bytes());
    bytes.extend(probe.mode.to_le_bytes());
    let mut written = 0;
    while written < bytes.len() {
        let result = unsafe {
            ((*stream).write.unwrap())(stream, bytes[written..].as_ptr() as *const c_void, (bytes.len() - written) as u64)
        };
        if result <= 0 {
            return false;
        }
        written += result as usize;
    }
    true
}
unsafe extern "C" fn state_load(plugin: *const clap_plugin, stream: *const clap_istream) -> bool {
    let probe = unsafe { probe(plugin) };
    let mut bytes = [0u8; 24];
    let mut read = 0;
    while read < bytes.len() {
        let result = unsafe {
            ((*stream).read.unwrap())(stream, bytes[read..].as_mut_ptr() as *mut c_void, (bytes.len() - read) as u64)
        };
        if result <= 0 {
            return false;
        }
        read += result as usize;
    }
    if &bytes[..8] != MAGIC {
        return false;
    }
    probe.level = f64::from_le_bytes(bytes[8..16].try_into().unwrap());
    probe.mode = f64::from_le_bytes(bytes[16..24].try_into().unwrap());
    true
}
static STATE: clap_plugin_state = clap_plugin_state {
    save: Some(state_save),
    load: Some(state_load),
};

unsafe extern "C" fn latency_get(plugin: *const clap_plugin) -> u32 {
    if unsafe { probe(plugin) }.effect { LATENCY as u32 } else { 0 }
}
static LATENCY_EXT: clap_plugin_latency = clap_plugin_latency { get: Some(latency_get) };

fn platform_api() -> &'static CStr {
    if cfg!(target_os = "macos") {
        CLAP_WINDOW_API_COCOA
    } else if cfg!(windows) {
        CLAP_WINDOW_API_WIN32
    } else {
        CLAP_WINDOW_API_X11
    }
}
unsafe extern "C" fn gui_supported(_plugin: *const clap_plugin, api: *const c_char, floating: bool) -> bool {
    !floating && unsafe { CStr::from_ptr(api) } == platform_api()
}
unsafe extern "C" fn gui_create(plugin: *const clap_plugin, api: *const c_char, floating: bool) -> bool {
    if !unsafe { gui_supported(plugin, api, floating) } {
        return false;
    }
    let probe = unsafe { probe(plugin) };
    let Some(timers) = (unsafe { host_extension::<clap_host_timer_support>(probe, CLAP_EXT_TIMER_SUPPORT) }) else {
        return false;
    };
    let mut id = u32::MAX;
    if !unsafe { (timers.register_timer.unwrap())(probe.host, 50, &mut id) } {
        return false;
    }
    probe.timer = Some(id);
    true
}
unsafe extern "C" fn gui_destroy(plugin: *const clap_plugin) {
    let probe = unsafe { probe(plugin) };
    if let (Some(id), Some(timers)) = (probe.timer.take(), unsafe {
        host_extension::<clap_host_timer_support>(probe, CLAP_EXT_TIMER_SUPPORT)
    }) {
        unsafe { (timers.unregister_timer.unwrap())(probe.host, id) };
    }
    probe.gui_shown = false;
}
unsafe extern "C" fn gui_size(_plugin: *const clap_plugin, width: *mut u32, height: *mut u32) -> bool {
    unsafe {
        *width = 320;
        *height = 200;
    }
    true
}
unsafe extern "C" fn gui_false(_plugin: *const clap_plugin) -> bool {
    false
}
unsafe extern "C" fn gui_true(_plugin: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn gui_show(plugin: *const clap_plugin) -> bool {
    unsafe { probe(plugin) }.gui_shown = true;
    true
}
unsafe extern "C" fn gui_parent(_plugin: *const clap_plugin, window: *const clap_window) -> bool {
    !window.is_null()
}
unsafe extern "C" fn gui_set_size(_plugin: *const clap_plugin, _width: u32, _height: u32) -> bool {
    true
}
static GUI: clap_plugin_gui = clap_plugin_gui {
    is_api_supported: Some(gui_supported),
    get_preferred_api: None,
    create: Some(gui_create),
    destroy: Some(gui_destroy),
    set_scale: None,
    get_size: Some(gui_size),
    can_resize: Some(gui_false),
    get_resize_hints: None,
    adjust_size: None,
    set_size: Some(gui_set_size),
    set_parent: Some(gui_parent),
    set_transient: None,
    suggest_title: None,
    show: Some(gui_show),
    hide: Some(gui_true),
};

unsafe extern "C" fn on_timer(plugin: *const clap_plugin, _id: clap_id) {
    let probe = unsafe { probe(plugin) };
    if probe.gui_shown && probe.level != 0.75 && !probe.gesture_pending {
        probe.gesture_pending = true;
        if let Some(params) = unsafe { host_extension::<clap_host_params>(probe, CLAP_EXT_PARAMS) } {
            unsafe { (params.request_flush.unwrap())(probe.host) };
        }
    }
}
static TIMER: clap_plugin_timer_support = clap_plugin_timer_support { on_timer: Some(on_timer) };

unsafe extern "C" fn get_extension(plugin: *const clap_plugin, id: *const c_char) -> *const c_void {
    let id = unsafe { CStr::from_ptr(id) };
    let effect = unsafe { probe(plugin) }.effect;
    if id == CLAP_EXT_AUDIO_PORTS {
        &AUDIO_PORTS as *const _ as *const c_void
    } else if id == CLAP_EXT_NOTE_PORTS && !effect {
        &NOTE_PORTS as *const _ as *const c_void
    } else if id == CLAP_EXT_PARAMS {
        &PARAMS as *const _ as *const c_void
    } else if id == CLAP_EXT_STATE {
        &STATE as *const _ as *const c_void
    } else if id == CLAP_EXT_LATENCY {
        &LATENCY_EXT as *const _ as *const c_void
    } else if id == CLAP_EXT_GUI {
        &GUI as *const _ as *const c_void
    } else if id == CLAP_EXT_TIMER_SUPPORT {
        &TIMER as *const _ as *const c_void
    } else {
        null()
    }
}
unsafe extern "C" fn on_main_thread(_plugin: *const clap_plugin) {}

unsafe extern "C" fn factory_count(_factory: *const clap_plugin_factory) -> u32 {
    2
}
unsafe extern "C" fn factory_descriptor(_factory: *const clap_plugin_factory, index: u32) -> *const clap_plugin_descriptor {
    match index {
        0 => &SYNTH.0,
        1 => &GAIN.0,
        _ => null(),
    }
}
unsafe extern "C" fn factory_create(_factory: *const clap_plugin_factory, host: *const clap_host, id: *const c_char) -> *const clap_plugin {
    let id = unsafe { CStr::from_ptr(id) };
    let (descriptor, effect) = if id == c"org.rill.test.synth" {
        (&SYNTH.0, false)
    } else if id == c"org.rill.test.gain" {
        (&GAIN.0, true)
    } else {
        return null();
    };
    let probe = Box::into_raw(Box::new(Probe {
        plugin: clap_plugin {
            desc: descriptor,
            plugin_data: std::ptr::null_mut(),
            init: Some(init),
            destroy: Some(destroy),
            activate: Some(activate),
            deactivate: Some(deactivate),
            start_processing: Some(start_processing),
            stop_processing: Some(stop_processing),
            reset: Some(reset),
            process: Some(process),
            get_extension: Some(get_extension),
            on_main_thread: Some(on_main_thread),
        },
        host,
        effect,
        level: 1.0,
        mode: 0.0,
        sample_rate: 48_000.0,
        voices: Vec::new(),
        delay: vec![[0.0; 2]; LATENCY],
        delay_position: 0,
        timer: None,
        gui_shown: false,
        gesture_pending: false,
    }));
    unsafe {
        (*probe).plugin.plugin_data = probe as *mut c_void;
        &(*probe).plugin
    }
}
static FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(factory_count),
    get_plugin_descriptor: Some(factory_descriptor),
    create_plugin: Some(factory_create),
};

unsafe extern "C" fn entry_init(_path: *const c_char) -> bool {
    true
}
unsafe extern "C" fn entry_deinit() {}
unsafe extern "C" fn entry_factory(id: *const c_char) -> *const c_void {
    if unsafe { CStr::from_ptr(id) } == CLAP_PLUGIN_FACTORY_ID {
        &FACTORY as *const _ as *const c_void
    } else {
        null()
    }
}

#[unsafe(no_mangle)]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry {
    clap_version: CLAP_VERSION,
    init: Some(entry_init),
    deinit: Some(entry_deinit),
    get_factory: Some(entry_factory),
};

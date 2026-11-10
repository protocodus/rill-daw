//! Minimal LV2 synth and gain, compiled by tests into `RillProbe.lv2`. Plain C ABI,
//! no dependencies. A Mode change is applied through the worker extension, so the
//! new waveform starts once its response has arrived (the next block); notes are
//! sample accurate. Keys 127 and 126 crash and hang the synth, for isolation tests.
#![allow(clippy::missing_safety_doc)]
use std::ffi::{CStr, c_char, c_void};
use std::ptr;

#[repr(C)]
pub struct Descriptor {
    uri: *const c_char,
    instantiate: unsafe extern "C" fn(*const Descriptor, f64, *const c_char, *const *const Feature) -> *mut c_void,
    connect_port: unsafe extern "C" fn(*mut c_void, u32, *mut c_void),
    activate: Option<unsafe extern "C" fn(*mut c_void)>,
    run: unsafe extern "C" fn(*mut c_void, u32),
    deactivate: Option<unsafe extern "C" fn(*mut c_void)>,
    cleanup: unsafe extern "C" fn(*mut c_void),
    extension_data: Option<unsafe extern "C" fn(*const c_char) -> *const c_void>,
}
unsafe impl Sync for Descriptor {}

#[repr(C)]
pub struct Feature {
    uri: *const c_char,
    data: *mut c_void,
}

#[repr(C)]
struct UridMap {
    handle: *mut c_void,
    map: unsafe extern "C" fn(*mut c_void, *const c_char) -> u32,
}

#[repr(C)]
struct Schedule {
    handle: *mut c_void,
    schedule_work: unsafe extern "C" fn(*mut c_void, u32, *const c_void) -> u32,
}

type Respond = unsafe extern "C" fn(*mut c_void, u32, *const c_void) -> u32;

#[repr(C)]
struct WorkerInterface {
    work: unsafe extern "C" fn(*mut c_void, Respond, *mut c_void, u32, *const c_void) -> u32,
    work_response: unsafe extern "C" fn(*mut c_void, u32, *const c_void) -> u32,
    end_run: Option<unsafe extern "C" fn(*mut c_void) -> u32>,
}
unsafe impl Sync for WorkerInterface {}

const LATENCY: usize = 32;

struct Synth {
    ports: [*mut f32; 5],
    events: *const u8,
    midi: u32,
    schedule: *const Schedule,
    rate: f64,
    phase: f64,
    frequency: f64,
    velocity: f32,
    sounding: bool,
    requested_mode: f32,
    active_mode: f32,
}

struct Gain {
    ports: [*mut f32; 8],
    delay: Vec<[f32; 2]>,
    cursor: usize,
}

unsafe fn feature(features: *const *const Feature, uri: &str) -> *mut c_void {
    let mut cursor = features;
    while !cursor.is_null() && unsafe { !(*cursor).is_null() } {
        let feature = unsafe { &**cursor };
        if unsafe { CStr::from_ptr(feature.uri) }.to_bytes() == uri.as_bytes() {
            return feature.data;
        }
        cursor = unsafe { cursor.add(1) };
    }
    ptr::null_mut()
}

unsafe extern "C" fn synth_new(
    _: *const Descriptor,
    rate: f64,
    _: *const c_char,
    features: *const *const Feature,
) -> *mut c_void {
    let map = unsafe { feature(features, "http://lv2plug.in/ns/ext/urid#map") } as *const UridMap;
    let schedule =
        unsafe { feature(features, "http://lv2plug.in/ns/ext/worker#schedule") } as *const Schedule;
    if map.is_null() || schedule.is_null() {
        return ptr::null_mut();
    }
    let midi = unsafe {
        ((*map).map)(
            (*map).handle,
            c"http://lv2plug.in/ns/ext/midi#MidiEvent".as_ptr(),
        )
    };
    Box::into_raw(Box::new(Synth {
        ports: [ptr::null_mut(); 5],
        events: ptr::null(),
        midi,
        schedule,
        rate,
        phase: 0.0,
        frequency: 0.0,
        velocity: 0.0,
        sounding: false,
        requested_mode: 0.0,
        active_mode: 0.0,
    }))
    .cast()
}

unsafe extern "C" fn synth_connect(handle: *mut c_void, port: u32, data: *mut c_void) {
    let synth = unsafe { &mut *(handle as *mut Synth) };
    if port == 0 {
        synth.events = data.cast();
    } else if let Some(slot) = synth.ports.get_mut(port as usize) {
        *slot = data.cast();
    }
}

unsafe extern "C" fn synth_run(handle: *mut c_void, frames: u32) {
    let synth = unsafe { &mut *(handle as *mut Synth) };
    if synth.events.is_null() {
        return;
    }
    let size = unsafe { *(synth.events as *const u32) } as usize;
    let mut offset = 16;
    let mut notes = Vec::new();
    while offset + 16 <= size + 8 {
        let event = unsafe { synth.events.add(offset) };
        let time = unsafe { *(event as *const i64) } as usize;
        let length = unsafe { *(event.add(8) as *const u32) } as usize;
        let kind = unsafe { *(event.add(12) as *const u32) };
        if kind == synth.midi && length >= 3 {
            let data = unsafe { std::slice::from_raw_parts(event.add(16), 3) };
            notes.push((time, data[0] & 0xF0, data[1], data[2]));
        }
        offset += 16 + length.div_ceil(8) * 8;
    }
    let level = unsafe { *synth.ports[3] };
    let mode = unsafe { *synth.ports[4] };
    if mode != synth.requested_mode {
        synth.requested_mode = mode;
        unsafe {
            ((*synth.schedule).schedule_work)(
                (*synth.schedule).handle,
                4,
                (&mode as *const f32).cast(),
            )
        };
    }
    let (left, right) = (synth.ports[1], synth.ports[2]);
    let mut next = 0;
    for frame in 0..frames as usize {
        while next < notes.len() && notes[next].0 <= frame {
            let (_, status, key, velocity) = notes[next];
            if status == 0x90 && velocity > 0 {
                match key {
                    127 => std::process::abort(),
                    126 => loop {
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    },
                    _ => {}
                }
                synth.frequency = 440.0 * 2f64.powf((f64::from(key) - 69.0) / 12.0);
                synth.velocity = f32::from(velocity) / 127.0;
                synth.sounding = true;
            } else if status == 0x80 || status == 0x90 || (status == 0xB0 && key == 123) {
                synth.sounding = false;
            }
            next += 1;
        }
        let sample = if synth.sounding {
            synth.phase = (synth.phase + synth.frequency / synth.rate).fract();
            let wave = if synth.active_mode >= 0.5 {
                if synth.phase < 0.5 { 1.0 } else { -1.0 }
            } else {
                (synth.phase * std::f64::consts::TAU).sin() as f32
            };
            wave * synth.velocity * level * 0.25
        } else {
            0.0
        };
        unsafe {
            *left.add(frame) = sample;
            *right.add(frame) = sample;
        }
    }
}

unsafe extern "C" fn synth_work(
    _: *mut c_void,
    respond: Respond,
    target: *mut c_void,
    size: u32,
    data: *const c_void,
) -> u32 {
    if size != 4 {
        return 1;
    }
    unsafe { respond(target, 4, data) }
}

unsafe extern "C" fn synth_work_response(handle: *mut c_void, size: u32, data: *const c_void) -> u32 {
    let synth = unsafe { &mut *(handle as *mut Synth) };
    if size == 4 {
        synth.active_mode = unsafe { *(data as *const f32) };
    }
    0
}

static WORKER: WorkerInterface = WorkerInterface {
    work: synth_work,
    work_response: synth_work_response,
    end_run: None,
};

unsafe extern "C" fn synth_extension(uri: *const c_char) -> *const c_void {
    if unsafe { CStr::from_ptr(uri) }.to_bytes() == b"http://lv2plug.in/ns/ext/worker#interface" {
        (&WORKER as *const WorkerInterface).cast()
    } else {
        ptr::null()
    }
}

unsafe extern "C" fn synth_free(handle: *mut c_void) {
    drop(unsafe { Box::from_raw(handle as *mut Synth) });
}

unsafe extern "C" fn gain_new(
    _: *const Descriptor,
    _: f64,
    _: *const c_char,
    _: *const *const Feature,
) -> *mut c_void {
    Box::into_raw(Box::new(Gain {
        ports: [ptr::null_mut(); 8],
        delay: vec![[0.0; 2]; LATENCY],
        cursor: 0,
    }))
    .cast()
}

unsafe extern "C" fn gain_connect(handle: *mut c_void, port: u32, data: *mut c_void) {
    let gain = unsafe { &mut *(handle as *mut Gain) };
    if let Some(slot) = gain.ports.get_mut(port as usize) {
        *slot = data.cast();
    }
}

unsafe extern "C" fn gain_run(handle: *mut c_void, frames: u32) {
    let gain = unsafe { &mut *(handle as *mut Gain) };
    unsafe { *gain.ports[7] = LATENCY as f32 };
    let level = unsafe { *gain.ports[6] };
    for frame in 0..frames as usize {
        for channel in 0..2 {
            let input = unsafe { *gain.ports[channel].add(frame) + *gain.ports[2 + channel].add(frame) };
            let delayed = std::mem::replace(&mut gain.delay[gain.cursor][channel], input * level);
            unsafe { *gain.ports[4 + channel].add(frame) = delayed };
        }
        gain.cursor = (gain.cursor + 1) % LATENCY;
    }
}

unsafe extern "C" fn gain_free(handle: *mut c_void) {
    drop(unsafe { Box::from_raw(handle as *mut Gain) });
}

static SYNTH: Descriptor = Descriptor {
    uri: c"urn:rill:test:lv2-synth".as_ptr(),
    instantiate: synth_new,
    connect_port: synth_connect,
    activate: None,
    run: synth_run,
    deactivate: None,
    cleanup: synth_free,
    extension_data: Some(synth_extension),
};

static GAIN: Descriptor = Descriptor {
    uri: c"urn:rill:test:lv2-gain".as_ptr(),
    instantiate: gain_new,
    connect_port: gain_connect,
    activate: None,
    run: gain_run,
    deactivate: None,
    cleanup: gain_free,
    extension_data: None,
};

#[unsafe(no_mangle)]
pub extern "C" fn lv2_descriptor(index: u32) -> *const Descriptor {
    match index {
        0 => &SYNTH,
        1 => &GAIN,
        _ => ptr::null(),
    }
}

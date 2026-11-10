//! Audio Unit hosting through the AUv2 C API, used only inside the isolated helpers.
//! AUv3 extensions load through the system's v2 bridge.
//!
//! Every call runs on the helper's main thread. Instantiation pumps that thread's run
//! loop while it waits, so components that complete asynchronously can finish.
//! References: Apple's AudioToolbox headers `AudioComponent.h`, `AUComponent.h`,
//! `AudioUnitProperties.h`, `MusicDevice.h`, `AudioUnitUtilities.h` and
//! `AUCocoaUIView.h`, through <https://docs.rs/objc2-audio-toolbox/0.3.2>.
use crate::audio_unit::{self, AU_LOCATOR, AudioUnitId, HOSTED_TYPES, TYPE_EFFECT};
use crate::native::{
    LoadRequest, LoadedInfo, NativeProcessor, ParameterEdit, RESTART_LATENCY_CHANGED,
    RESTART_PARAM_TITLES_CHANGED, RESTART_PARAM_VALUES_CHANGED, Transport, normalized_from_plain,
    plain_from_normalized, plain_parameter,
};
use crate::{PluginFormat, PluginInfo};
use anyhow::{Context, Result, ensure};
use block2::RcBlock;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{MainThreadMarker, msg_send, sel};
use objc2_app_kit::NSView;
use objc2_audio_toolbox::{
    AUListenerDispose, AUParameterListenerNotify, AUParameterSet, AURenderCallbackStruct,
    AudioComponent, AudioComponentCopyName, AudioComponentDescription, AudioComponentFindNext,
    AudioComponentGetDescription, AudioComponentGetVersion, AudioComponentInstance,
    AudioComponentInstanceDispose, AudioComponentInstantiate, AudioComponentInstantiationOptions,
    AudioUnit, AudioUnitAddPropertyListener, AudioUnitElement, AudioUnitGetParameter,
    AudioUnitGetProperty, AudioUnitGetPropertyInfo, AudioUnitInitialize, AudioUnitParameter,
    AudioUnitParameterInfo, AudioUnitParameterOptions, AudioUnitParameterStringFromValue,
    AudioUnitPropertyID, AudioUnitRender, AudioUnitRenderActionFlags, AudioUnitReset,
    AudioUnitScope, AudioUnitSetParameter, AudioUnitSetProperty, AudioUnitUninitialize,
    HostCallbackInfo, MusicDeviceMIDIEvent, kAUParameterListener_AnyParameter,
    kAudioUnitErr_CannotDoInCurrentContext, kAudioUnitProperty_ClassInfo,
    kAudioUnitProperty_CocoaUI, kAudioUnitProperty_ElementCount, kAudioUnitProperty_ElementName,
    kAudioUnitProperty_HostCallbacks, kAudioUnitProperty_Latency,
    kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitProperty_OfflineRender,
    kAudioUnitProperty_ParameterInfo, kAudioUnitProperty_ParameterList,
    kAudioUnitProperty_ParameterStringFromValue, kAudioUnitProperty_PresentPreset,
    kAudioUnitProperty_SetRenderCallback, kAudioUnitProperty_StreamFormat,
    kAudioUnitProperty_TailTime, kAudioUnitScope_Global, kAudioUnitScope_Input,
    kAudioUnitScope_Output,
};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp, AudioTimeStampFlags,
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved, kAudioFormatFlagIsPacked,
    kAudioFormatLinearPCM,
};
use objc2_core_foundation::{
    CFData, CFPropertyListCreateData, CFPropertyListCreateWithData, CFPropertyListFormat,
    CFRetained, CFRunLoop, CFRunLoopMode, CFString, CFType, kCFRunLoopDefaultMode,
};
use objc2_foundation::{NSBundle, NSPoint, NSSize, NSString, NSURL};
use std::ffi::c_void;
use std::mem::{offset_of, size_of};
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use vst3_host::{
    AudioBusConfig, AudioBusLayout, BusAudioBuffers, MidiEvent,
    audio::{AudioBusDetails, AudioBusDetailsLayout, BusDirection, MediaType},
    midi::{PluginEvent, PluginEventData},
    parameters::Parameter,
    plugin::{ProcessMode, TransportPosition},
};

type OSStatus = i32;

const MAX_COMPONENTS: usize = 4096;
const MAX_BUSES: u32 = 64;
const MAX_BUS_CHANNELS: u32 = 32;
const MAX_STAGED: usize = 16_384;
const INSTANTIATE_DEADLINE: Duration = Duration::from_secs(15);
/// Private restart bit: the tail time changed.
const TAIL_CHANGED: i32 = 1 << 30;
/// `kAudioUnitEvent_ParameterValueChange` and `_BeginParameterChangeGesture`.
const EVENT_VALUE: u32 = 0;
const EVENT_BEGIN: u32 = 1;

/// `AudioUnitEvent` from `AudioUnitUtilities.h`; both union members share this layout.
#[repr(C)]
struct AudioUnitEvent {
    event_type: u32,
    argument: AudioUnitParameter,
}

type EventListener = *mut c_void;
type EventProc =
    unsafe extern "C-unwind" fn(*mut c_void, *mut c_void, *const AudioUnitEvent, u64, f32);

// The binding crate omits the AUEventListener family, whose events are a union.
#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C-unwind" {
    fn AUEventListenerCreate(
        proc_: EventProc,
        user_data: *mut c_void,
        run_loop: *const CFRunLoop,
        run_loop_mode: *const CFRunLoopMode,
        notification_interval: f32,
        value_change_granularity: f32,
        out_listener: *mut EventListener,
    ) -> OSStatus;
    fn AUEventListenerAddEventType(
        listener: EventListener,
        object: *mut c_void,
        event: *const AudioUnitEvent,
    ) -> OSStatus;
}

fn check(status: OSStatus, action: &str) -> Result<()> {
    ensure!(status == 0, "{action} failed (OSStatus {status})");
    Ok(())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Take ownership of a +1 `CFString` returned by an AudioToolbox call.
///
/// # Safety
/// `value` must be null or a string the caller owns.
unsafe fn take_string(value: *const CFString) -> Option<String> {
    let value = NonNull::new(value.cast_mut())?;
    // SAFETY: the caller transfers its reference.
    let owned = unsafe { CFRetained::from_raw(value) };
    Some(owned.to_string())
}

/// Read a fixed-size property.
fn get<T: Copy>(
    unit: AudioUnit,
    id: AudioUnitPropertyID,
    scope: AudioUnitScope,
    element: u32,
) -> Option<T> {
    let mut value = std::mem::MaybeUninit::<T>::zeroed();
    let mut size = size_of::<T>() as u32;
    // SAFETY: `value` has room for `size` bytes; callers only read plain-data types.
    let status = unsafe {
        AudioUnitGetProperty(
            unit,
            id,
            scope,
            element,
            NonNull::new_unchecked(value.as_mut_ptr()).cast(),
            NonNull::from(&mut size),
        )
    };
    // SAFETY: the unit filled the value (or left it zeroed, a valid bit pattern here).
    (status == 0).then(|| unsafe { value.assume_init() })
}

fn set<T>(
    unit: AudioUnit,
    id: AudioUnitPropertyID,
    scope: AudioUnitScope,
    element: u32,
    value: &T,
) -> OSStatus {
    // SAFETY: `value` is a live `T` of the size given.
    unsafe {
        AudioUnitSetProperty(
            unit,
            id,
            scope,
            element,
            (value as *const T).cast(),
            size_of::<T>() as u32,
        )
    }
}

fn description(id: AudioUnitId) -> AudioComponentDescription {
    AudioComponentDescription {
        componentType: id.kind,
        componentSubType: id.subtype,
        componentManufacturer: id.manufacturer,
        componentFlags: 0,
        componentFlagsMask: 0,
    }
}

fn component_identity(component: AudioComponent) -> (String, String, String) {
    let mut name: *const CFString = ptr::null();
    // SAFETY: `component` came from the registry; the name is returned at +1.
    let full = unsafe {
        if AudioComponentCopyName(component, NonNull::from(&mut name)) == 0 {
            take_string(name)
        } else {
            None
        }
    }
    .unwrap_or_default();
    let (vendor, name) = audio_unit::split_component_name(&full);
    let mut version = 0u32;
    // SAFETY: as above.
    let version =
        if unsafe { AudioComponentGetVersion(component, NonNull::from(&mut version)) } == 0 {
            audio_unit::version_text(version)
        } else {
            String::new()
        };
    (vendor, name, version)
}

/// Run on the helper only. Reading the registry never executes plug-in code.
pub(crate) fn scan() -> Result<Vec<PluginInfo>> {
    let mut found = Vec::new();
    for kind in HOSTED_TYPES {
        let mut query = AudioComponentDescription {
            componentType: kind,
            componentSubType: 0,
            componentManufacturer: 0,
            componentFlags: 0,
            componentFlagsMask: 0,
        };
        let mut component: AudioComponent = ptr::null_mut();
        while found.len() < MAX_COMPONENTS {
            // SAFETY: `query` is a valid description; a null result ends the search.
            component = unsafe { AudioComponentFindNext(component, NonNull::from(&mut query)) };
            if component.is_null() {
                break;
            }
            let mut registered = description(AudioUnitId {
                kind: 0,
                subtype: 0,
                manufacturer: 0,
            });
            // SAFETY: `component` is a live registry entry.
            if unsafe { AudioComponentGetDescription(component, NonNull::from(&mut registered)) }
                != 0
            {
                continue;
            }
            let id = AudioUnitId {
                kind: registered.componentType,
                subtype: registered.componentSubType,
                manufacturer: registered.componentManufacturer,
            };
            let (vendor, name, version) = component_identity(component);
            found.push(PluginInfo {
                name: if name.is_empty() {
                    id.to_string()
                } else {
                    name
                },
                path: AU_LOCATOR.into(),
                vendor,
                category: audio_unit::category(id.kind).to_owned(),
                class_id: Some(id.to_string()),
                version,
                architectures: Vec::new(),
                format: PluginFormat::Au,
            });
        }
    }
    found.sort_by(|a, b| a.class_id.cmp(&b.class_id));
    found.dedup_by(|a, b| a.class_id == b.class_id);
    Ok(found)
}

/// Instantiate asynchronously, which AUv3 extensions require, and pump this
/// thread's run loop until the completion handler reports.
fn instantiate(component: AudioComponent) -> Result<AudioUnit> {
    let result: Arc<Mutex<Option<(usize, OSStatus)>>> = Arc::default();
    let slot = Arc::clone(&result);
    let completion = RcBlock::new(move |instance: AudioComponentInstance, status: OSStatus| {
        *lock(&slot) = Some((instance as usize, status));
    });
    // SAFETY: the system copies the block; `component` is a live registry entry.
    unsafe {
        AudioComponentInstantiate(
            component,
            AudioComponentInstantiationOptions(0),
            &completion,
        )
    };
    let deadline = Instant::now() + INSTANTIATE_DEADLINE;
    loop {
        if let Some((instance, status)) = lock(&result).take() {
            check(status, "Instantiating the Audio Unit")?;
            ensure!(instance != 0, "The Audio Unit returned no instance");
            return Ok(instance as AudioUnit);
        }
        ensure!(
            Instant::now() < deadline,
            "The Audio Unit did not finish instantiating within {} seconds",
            INSTANTIATE_DEADLINE.as_secs()
        );
        // SAFETY: reading an immutable framework constant.
        let mode = unsafe { kCFRunLoopDefaultMode };
        CFRunLoop::run_in_mode(mode, 0.005, true);
    }
}

fn float_format(sample_rate: f64, channels: u32) -> AudioStreamBasicDescription {
    AudioStreamBasicDescription {
        mSampleRate: sample_rate,
        mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kAudioFormatFlagIsFloat
            | kAudioFormatFlagIsPacked
            | kAudioFormatFlagIsNonInterleaved,
        mBytesPerPacket: 4,
        mFramesPerPacket: 1,
        mBytesPerFrame: 4,
        mChannelsPerFrame: channels,
        mBitsPerChannel: 32,
        mReserved: 0,
    }
}

/// Transport as the host callbacks report it during one render segment.
#[derive(Clone, Copy, Default)]
struct Timeline {
    playing: bool,
    changed: bool,
    tempo: f64,
    beat: f64,
    sample: f64,
    numerator: i32,
    denominator: i32,
    sample_rate: f64,
}

/// Input channel pointers for the segment being rendered, indexed by element.
#[derive(Default)]
struct Feed {
    buses: Vec<Vec<*mut f32>>,
    frames: usize,
    silence: Vec<f32>,
}
// SAFETY: the pointers are only dereferenced inside `AudioUnitRender`, while the
// owning block buffers are borrowed by the rendering call.
unsafe impl Send for Feed {}

#[derive(Clone, Copy)]
enum RawEdit {
    Begin(u32),
    Value(u32, f32),
}

/// State the unit's callbacks reach through their reference constant.
#[derive(Default)]
struct Shared {
    restart: AtomicI32,
    dirty: AtomicBool,
    timeline: Mutex<Timeline>,
    feed: Mutex<Feed>,
    edits: Mutex<Vec<RawEdit>>,
}

impl Shared {
    fn refcon(self: &Arc<Self>) -> *mut c_void {
        Arc::as_ptr(self).cast_mut().cast()
    }
}

unsafe extern "C-unwind" fn render_input(
    refcon: NonNull<c_void>,
    _flags: NonNull<AudioUnitRenderActionFlags>,
    _time: NonNull<AudioTimeStamp>,
    bus: u32,
    frames: u32,
    data: *mut AudioBufferList,
) -> OSStatus {
    // SAFETY: `refcon` is the `Shared` that outlives the unit.
    let shared = unsafe { refcon.cast::<Shared>().as_ref() };
    let Some(list) = (unsafe { data.as_mut() }) else {
        return kAudioUnitErr_CannotDoInCurrentContext;
    };
    let mut feed = lock(&shared.feed);
    let frames = (frames as usize).min(feed.frames);
    let silence = feed.silence.as_mut_ptr();
    let count = list.mNumberBuffers as usize;
    // SAFETY: the list holds `mNumberBuffers` buffers.
    let buffers = unsafe {
        std::slice::from_raw_parts_mut(
            ptr::addr_of_mut!(list.mBuffers).cast::<AudioBuffer>(),
            count,
        )
    };
    let sources = feed.buses.get(bus as usize);
    for (index, buffer) in buffers.iter_mut().enumerate() {
        let source = sources.and_then(|channels| channels.get(index)).copied();
        if buffer.mData.is_null() {
            // The unit asked the host to provide the buffer.
            buffer.mData = source.unwrap_or(silence).cast();
            buffer.mDataByteSize = (frames * 4) as u32;
            continue;
        }
        let length = frames.min(buffer.mDataByteSize as usize / 4);
        // SAFETY: the unit's buffer holds `mDataByteSize` bytes; sources hold `frames`.
        unsafe {
            let target = std::slice::from_raw_parts_mut(buffer.mData.cast::<f32>(), length);
            match source {
                Some(source) => target.copy_from_slice(std::slice::from_raw_parts(source, length)),
                None => target.fill(0.0),
            }
        }
        buffer.mDataByteSize = (length * 4) as u32;
    }
    0
}

#[allow(non_upper_case_globals)]
unsafe extern "C-unwind" fn property_changed(
    refcon: NonNull<c_void>,
    _unit: AudioUnit,
    id: AudioUnitPropertyID,
    _scope: AudioUnitScope,
    _element: AudioUnitElement,
) {
    // SAFETY: `refcon` is the `Shared` that outlives the unit.
    let shared = unsafe { refcon.cast::<Shared>().as_ref() };
    let bits = match id {
        kAudioUnitProperty_ParameterList | kAudioUnitProperty_ParameterInfo => {
            RESTART_PARAM_TITLES_CHANGED | RESTART_PARAM_VALUES_CHANGED
        }
        kAudioUnitProperty_Latency => RESTART_LATENCY_CHANGED,
        kAudioUnitProperty_TailTime => TAIL_CHANGED,
        kAudioUnitProperty_PresentPreset | kAudioUnitProperty_ClassInfo => {
            shared.dirty.store(true, Ordering::Release);
            RESTART_PARAM_VALUES_CHANGED
        }
        _ => 0,
    };
    shared.restart.fetch_or(bits, Ordering::AcqRel);
}

unsafe extern "C-unwind" fn parameter_event(
    refcon: *mut c_void,
    _object: *mut c_void,
    event: *const AudioUnitEvent,
    _host_time: u64,
    value: f32,
) {
    // SAFETY: `refcon` is the `Shared` that outlives the listener.
    let (Some(shared), Some(event)) = (unsafe { refcon.cast::<Shared>().as_ref() }, unsafe {
        event.as_ref()
    }) else {
        return;
    };
    let id = event.argument.mParameterID;
    let edit = match event.event_type {
        EVENT_VALUE => RawEdit::Value(id, value),
        EVENT_BEGIN => RawEdit::Begin(id),
        _ => return,
    };
    let mut edits = lock(&shared.edits);
    if edits.len() < MAX_STAGED {
        edits.push(edit);
    }
}

/// # Safety
/// `user` is the `Shared` passed as `hostUserData`.
unsafe fn timeline(user: *mut c_void) -> Option<Timeline> {
    // SAFETY: see above.
    let shared = unsafe { user.cast::<Shared>().as_ref() }?;
    Some(*lock(&shared.timeline))
}

/// # Safety
/// `target` is null or writable.
unsafe fn put<T>(target: *mut T, value: T) {
    if !target.is_null() {
        // SAFETY: see above.
        unsafe { target.write(value) };
    }
}

unsafe extern "C-unwind" fn beat_and_tempo(
    user: *mut c_void,
    beat: *mut f64,
    tempo: *mut f64,
) -> OSStatus {
    // SAFETY: host callback contract.
    let Some(t) = (unsafe { timeline(user) }) else {
        return kAudioUnitErr_CannotDoInCurrentContext;
    };
    unsafe {
        put(beat, t.beat);
        put(tempo, t.tempo);
    }
    0
}

unsafe extern "C-unwind" fn musical_time(
    user: *mut c_void,
    to_next_beat: *mut u32,
    numerator: *mut f32,
    denominator: *mut u32,
    downbeat: *mut f64,
) -> OSStatus {
    // SAFETY: host callback contract.
    let Some(t) = (unsafe { timeline(user) }) else {
        return kAudioUnitErr_CannotDoInCurrentContext;
    };
    let bar = f64::from(t.numerator.max(1)) * 4.0 / f64::from(t.denominator.max(1));
    let samples_per_beat = t.sample_rate * 60.0 / t.tempo.max(1e-6);
    let to_next = ((t.beat.ceil() - t.beat) * samples_per_beat).round();
    unsafe {
        put(to_next_beat, to_next.clamp(0.0, f64::from(u32::MAX)) as u32);
        put(numerator, t.numerator as f32);
        put(denominator, t.denominator.max(1) as u32);
        put(downbeat, (t.beat / bar).floor() * bar);
    }
    0
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C-unwind" fn transport_state2(
    user: *mut c_void,
    playing: *mut u8,
    recording: *mut u8,
    changed: *mut u8,
    sample: *mut f64,
    cycling: *mut u8,
    cycle_start: *mut f64,
    cycle_end: *mut f64,
) -> OSStatus {
    // SAFETY: host callback contract.
    let Some(t) = (unsafe { timeline(user) }) else {
        return kAudioUnitErr_CannotDoInCurrentContext;
    };
    unsafe {
        put(playing, u8::from(t.playing));
        put(recording, 0);
        put(changed, u8::from(t.changed));
        put(sample, t.sample);
        put(cycling, 0);
        put(cycle_start, 0.0);
        put(cycle_end, 0.0);
    }
    0
}

unsafe extern "C-unwind" fn transport_state(
    user: *mut c_void,
    playing: *mut u8,
    changed: *mut u8,
    sample: *mut f64,
    cycling: *mut u8,
    cycle_start: *mut f64,
    cycle_end: *mut f64,
) -> OSStatus {
    // SAFETY: forwarded unchanged.
    unsafe {
        transport_state2(
            user,
            playing,
            ptr::null_mut(),
            changed,
            sample,
            cycling,
            cycle_start,
            cycle_end,
        )
    }
}

struct Bus {
    element: u32,
    name: String,
    channels: usize,
    active: bool,
}

struct ParamMeta {
    id: u32,
    name: String,
    unit: String,
    min: f64,
    max: f64,
    default: f64,
    stepped: bool,
    writable: bool,
}

#[derive(Clone, Copy)]
enum Staged {
    Midi([u8; 3], usize),
    Param(u32, f32, usize),
}

/// Storage for an `AudioBufferList` with one non-interleaved buffer per channel.
struct BufferList {
    words: Vec<u64>,
    channels: usize,
}

impl BufferList {
    fn new(channels: usize) -> Self {
        let bytes =
            offset_of!(AudioBufferList, mBuffers) + channels.max(1) * size_of::<AudioBuffer>();
        Self {
            words: vec![0; bytes.div_ceil(8)],
            channels,
        }
    }

    fn list(&mut self) -> *mut AudioBufferList {
        self.words.as_mut_ptr().cast()
    }

    fn buffers(&mut self) -> &mut [AudioBuffer] {
        let list = self.list();
        // SAFETY: `words` holds the header and `channels` buffers, 8-byte aligned.
        unsafe {
            (*list).mNumberBuffers = self.channels as u32;
            std::slice::from_raw_parts_mut(
                ptr::addr_of_mut!((*list).mBuffers).cast(),
                self.channels,
            )
        }
    }
}

pub(crate) struct AuProcessor {
    unit: AudioUnit,
    kind: u32,
    info: LoadedInfo,
    sample_rate: f64,
    max_block: usize,
    inputs: Vec<Bus>,
    outputs: Vec<Bus>,
    input_elements: usize,
    lists: Vec<BufferList>,
    params: Vec<ParamMeta>,
    shared: Arc<Shared>,
    transport: Transport,
    transport_changed: bool,
    staged: Vec<Staged>,
    sample_time: f64,
    initialized: bool,
    processing: bool,
    latency: u32,
    tail: u32,
    view: Option<Retained<NSView>>,
    view_size: (u32, u32),
    listener: EventListener,
}

impl AuProcessor {
    pub(crate) fn load(request: &LoadRequest<'_>) -> Result<Self> {
        ensure!(
            request.sample_rate.is_finite() && request.sample_rate > 0.0,
            "Invalid sample rate"
        );
        ensure!(
            (1..=65_536).contains(&request.block_size),
            "Invalid Audio Unit block size"
        );
        let text = request
            .class_id
            .context("An Audio Unit reference needs its type:subtype:manufacturer ID")?;
        let id = AudioUnitId::parse(text).map_err(anyhow::Error::msg)?;
        ensure!(
            id.is_hosted_type(),
            "{text} is not an instrument or effect Audio Unit"
        );
        let mut query = description(id);
        // SAFETY: `query` is a valid description.
        let component =
            unsafe { AudioComponentFindNext(ptr::null_mut(), NonNull::from(&mut query)) };
        ensure!(!component.is_null(), "Audio Unit {text} is not installed");
        let (vendor, name, version) = component_identity(component);
        let unit = instantiate(component).with_context(|| format!("Loading Audio Unit {text}"))?;
        let mut processor = Self {
            unit,
            kind: id.kind,
            info: LoadedInfo {
                name: if name.is_empty() {
                    text.to_owned()
                } else {
                    name
                },
                vendor,
                version,
                category: audio_unit::category(id.kind).to_owned(),
                uid: id.to_string(),
                has_gui: true,
                has_midi_input: id.kind != TYPE_EFFECT,
                has_midi_output: false,
            },
            sample_rate: request.sample_rate,
            max_block: request.block_size,
            inputs: Vec::new(),
            outputs: Vec::new(),
            input_elements: 0,
            lists: Vec::new(),
            params: Vec::new(),
            shared: Arc::default(),
            transport: Transport::new(request.tempo, request.time_signature),
            transport_changed: false,
            staged: Vec::with_capacity(4096),
            sample_time: 0.0,
            initialized: false,
            processing: false,
            latency: 0,
            tail: 0,
            view: None,
            view_size: (0, 0),
            listener: ptr::null_mut(),
        };
        processor.install_callbacks();
        processor.configure()?;
        processor.initialize()?;
        processor.scan_params();
        Ok(processor)
    }

    fn install_callbacks(&mut self) {
        let refcon = self.shared.refcon();
        let callbacks = HostCallbackInfo {
            hostUserData: refcon,
            beatAndTempoProc: Some(beat_and_tempo),
            musicalTimeLocationProc: Some(musical_time),
            transportStateProc: Some(transport_state),
            transportStateProc2: Some(transport_state2),
        };
        // Optional: units that ignore host time reject the property.
        let _ = set(
            self.unit,
            kAudioUnitProperty_HostCallbacks,
            kAudioUnitScope_Global,
            0,
            &callbacks,
        );
        for property in [
            kAudioUnitProperty_ParameterList,
            kAudioUnitProperty_ParameterInfo,
            kAudioUnitProperty_Latency,
            kAudioUnitProperty_TailTime,
            kAudioUnitProperty_PresentPreset,
            kAudioUnitProperty_ClassInfo,
        ] {
            // SAFETY: `refcon` outlives the unit, which is disposed first in `Drop`.
            let _ = unsafe {
                AudioUnitAddPropertyListener(self.unit, property, Some(property_changed), refcon)
            };
        }
    }

    fn element_count(&self, scope: AudioUnitScope) -> u32 {
        get::<u32>(self.unit, kAudioUnitProperty_ElementCount, scope, 0)
            .unwrap_or(0)
            .min(MAX_BUSES)
    }

    fn element_name(&self, scope: AudioUnitScope, element: u32) -> Option<String> {
        let name =
            get::<*const CFString>(self.unit, kAudioUnitProperty_ElementName, scope, element)?;
        // SAFETY: the property returns the name at +1.
        unsafe { take_string(name) }.filter(|name| !name.trim().is_empty())
    }

    /// Ask every element for 32-bit float non-interleaved audio at the session rate,
    /// keeping its channel count. Auxiliary elements that refuse are left unconnected.
    fn configure_scope(&self, scope: AudioUnitScope) -> Result<Vec<Bus>> {
        let direction = if scope == kAudioUnitScope_Input {
            "input"
        } else {
            "output"
        };
        let mut buses = Vec::new();
        for element in 0..self.element_count(scope) {
            let channels = get::<AudioStreamBasicDescription>(
                self.unit,
                kAudioUnitProperty_StreamFormat,
                scope,
                element,
            )
            .map_or(2, |format| format.mChannelsPerFrame);
            let channels = if channels == 0 {
                2
            } else {
                channels.min(MAX_BUS_CHANNELS)
            };
            let status = set(
                self.unit,
                kAudioUnitProperty_StreamFormat,
                scope,
                element,
                &float_format(self.sample_rate, channels),
            );
            if status != 0 {
                ensure!(
                    element != 0,
                    "The Audio Unit rejected 32-bit float audio on its main {direction} (OSStatus {status})"
                );
                eprintln!("Audio Unit {direction} {element} left unconnected (OSStatus {status})");
                continue;
            }
            let name = self
                .element_name(scope, element)
                .unwrap_or_else(|| match element {
                    0 => "Main".to_owned(),
                    1 if scope == kAudioUnitScope_Input => "Sidechain".to_owned(),
                    _ => format!(
                        "{} {}",
                        if scope == kAudioUnitScope_Input {
                            "Input"
                        } else {
                            "Output"
                        },
                        element + 1
                    ),
                });
            buses.push(Bus {
                element,
                name,
                channels: channels as usize,
                active: true,
            });
        }
        Ok(buses)
    }

    fn configure(&mut self) -> Result<()> {
        let frames = self.max_block as u32;
        check(
            set(
                self.unit,
                kAudioUnitProperty_MaximumFramesPerSlice,
                kAudioUnitScope_Global,
                0,
                &frames,
            ),
            "Setting the Audio Unit block size",
        )?;
        let keep = |old: &[Bus], new: &mut [Bus]| {
            if old.len() == new.len() {
                for (bus, previous) in new.iter_mut().zip(old) {
                    bus.active = previous.active;
                }
            }
        };
        let mut inputs = self.configure_scope(kAudioUnitScope_Input)?;
        let mut outputs = self.configure_scope(kAudioUnitScope_Output)?;
        keep(&self.inputs, &mut inputs);
        keep(&self.outputs, &mut outputs);
        ensure!(!outputs.is_empty(), "The Audio Unit has no audio output");
        self.input_elements = self.element_count(kAudioUnitScope_Input) as usize;
        let callback = AURenderCallbackStruct {
            inputProc: Some(render_input),
            inputProcRefCon: self.shared.refcon(),
        };
        for element in 0..self.input_elements as u32 {
            let status = set(
                self.unit,
                kAudioUnitProperty_SetRenderCallback,
                kAudioUnitScope_Input,
                element,
                &callback,
            );
            if inputs.iter().any(|bus| bus.element == element) {
                check(status, "Connecting the Audio Unit input")?;
            }
        }
        {
            let mut feed = lock(&self.shared.feed);
            feed.buses = vec![Vec::new(); self.input_elements];
            feed.silence = vec![0.0; self.max_block];
        }
        self.lists = outputs
            .iter()
            .map(|bus| BufferList::new(bus.channels))
            .collect();
        self.inputs = inputs;
        self.outputs = outputs;
        Ok(())
    }

    fn initialize(&mut self) -> Result<()> {
        // SAFETY: the unit is configured and uninitialized.
        check(
            unsafe { AudioUnitInitialize(self.unit) },
            "Initializing the Audio Unit",
        )?;
        self.initialized = true;
        self.update_latency();
        self.update_tail();
        Ok(())
    }

    fn uninitialize(&mut self) {
        if std::mem::take(&mut self.initialized) {
            // SAFETY: the unit is initialized.
            unsafe { AudioUnitUninitialize(self.unit) };
        }
    }

    fn seconds_to_samples(&self, seconds: f64) -> u32 {
        if seconds.is_finite() && seconds > 0.0 {
            (seconds * self.sample_rate)
                .round()
                .min(f64::from(u32::MAX)) as u32
        } else if seconds.is_infinite() && seconds > 0.0 {
            u32::MAX
        } else {
            0
        }
    }

    fn update_latency(&mut self) {
        let seconds = get::<f64>(
            self.unit,
            kAudioUnitProperty_Latency,
            kAudioUnitScope_Global,
            0,
        );
        self.latency = self.seconds_to_samples(seconds.unwrap_or(0.0));
    }

    fn update_tail(&mut self) {
        let seconds = get::<f64>(
            self.unit,
            kAudioUnitProperty_TailTime,
            kAudioUnitScope_Global,
            0,
        );
        self.tail = self.seconds_to_samples(seconds.unwrap_or(0.0));
    }

    fn scan_params(&mut self) {
        let mut size = 0u32;
        // SAFETY: querying the size of a variable-length property.
        let status = unsafe {
            AudioUnitGetPropertyInfo(
                self.unit,
                kAudioUnitProperty_ParameterList,
                kAudioUnitScope_Global,
                0,
                &mut size,
                ptr::null_mut(),
            )
        };
        let count = if status == 0 {
            (size as usize / 4).min(65_536)
        } else {
            0
        };
        let mut ids = vec![0u32; count];
        let mut size = (count * 4) as u32;
        if count > 0 {
            // SAFETY: `ids` holds `size` bytes.
            let status = unsafe {
                AudioUnitGetProperty(
                    self.unit,
                    kAudioUnitProperty_ParameterList,
                    kAudioUnitScope_Global,
                    0,
                    NonNull::new_unchecked(ids.as_mut_ptr()).cast(),
                    NonNull::from(&mut size),
                )
            };
            ids.truncate(if status == 0 { size as usize / 4 } else { 0 });
        }
        self.params = ids
            .into_iter()
            .filter_map(|id| self.param_info(id))
            .collect();
    }

    fn param_info(&self, id: u32) -> Option<ParamMeta> {
        let info = get::<AudioUnitParameterInfo>(
            self.unit,
            kAudioUnitProperty_ParameterInfo,
            kAudioUnitScope_Global,
            id,
        )?;
        let flags = info.flags;
        let release = flags.contains(AudioUnitParameterOptions::Flag_CFNameRelease);
        let custom = info.unit.0 == 26;
        let cf_name = if flags.contains(AudioUnitParameterOptions::Flag_HasCFNameString) {
            info.cfNameString
        } else {
            ptr::null()
        };
        // SAFETY: names are borrowed unless the unit set `CFNameRelease`.
        let borrowed = |value: *const CFString| -> Option<String> {
            NonNull::new(value.cast_mut()).map(|value| unsafe { value.as_ref() }.to_string())
        };
        let name = if release {
            unsafe { take_string(cf_name) }
        } else {
            borrowed(cf_name)
        };
        let unit_name = if custom && release {
            unsafe { take_string(info.unitName) }
        } else if custom {
            borrowed(info.unitName)
        } else {
            None
        };
        let name = name.filter(|n| !n.is_empty()).unwrap_or_else(|| {
            // SAFETY: `name` is a fixed array; stop at its terminator or end.
            let bytes: Vec<u8> = info
                .name
                .iter()
                .map(|&c| c as u8)
                .take_while(|&b| b != 0)
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        });
        let (min, max) = (f64::from(info.minValue), f64::from(info.maxValue));
        (min.is_finite() && max.is_finite() && max >= min).then(|| ParamMeta {
            id,
            name: if name.is_empty() {
                format!("Parameter {id}")
            } else {
                name
            },
            unit: unit_name.unwrap_or_else(|| audio_unit::unit_label(info.unit.0).to_owned()),
            min,
            max,
            default: f64::from(info.defaultValue).clamp(min, max),
            stepped: audio_unit::unit_is_stepped(info.unit.0),
            writable: flags.contains(AudioUnitParameterOptions::Flag_IsWritable)
                && !flags.contains(AudioUnitParameterOptions::Flag_MeterReadOnly),
        })
    }

    fn param(&self, id: u32) -> Result<&ParamMeta> {
        self.params
            .iter()
            .find(|p| p.id == id)
            .with_context(|| format!("Unknown Audio Unit parameter {id}"))
    }

    fn plain_value(&self, id: u32) -> Option<f64> {
        let mut value = 0f32;
        // SAFETY: writes one `f32`.
        let status = unsafe {
            AudioUnitGetParameter(
                self.unit,
                id,
                kAudioUnitScope_Global,
                0,
                NonNull::from(&mut value),
            )
        };
        (status == 0 && value.is_finite()).then_some(f64::from(value))
    }

    fn apply_parameter(&self, id: u32, plain: f32) -> Result<()> {
        let status = if self.listener.is_null() {
            // SAFETY: plain call on a live unit.
            unsafe { AudioUnitSetParameter(self.unit, id, kAudioUnitScope_Global, 0, plain, 0) }
        } else {
            // An open editor's view follows host changes only through AUParameterSet;
            // naming our listener keeps the change from echoing back as a gesture.
            let mut parameter = AudioUnitParameter {
                mAudioUnit: self.unit,
                mParameterID: id,
                mScope: kAudioUnitScope_Global,
                mElement: 0,
            };
            // SAFETY: `parameter` names a live unit; the listener is ours.
            unsafe {
                AUParameterSet(
                    self.listener.cast(),
                    self.shared.refcon(),
                    NonNull::from(&mut parameter),
                    plain,
                    0,
                )
            }
        };
        check(status, "Setting the Audio Unit parameter")
    }

    fn stage(&mut self, event: Staged) -> Result<()> {
        ensure!(
            self.staged.len() < MAX_STAGED,
            "Too many Audio Unit events in one block"
        );
        self.staged.push(event);
        Ok(())
    }

    fn stage_midi(&mut self, data: [u8; 3], offset: i32) -> Result<()> {
        if self.kind == TYPE_EFFECT {
            return Ok(());
        }
        let offset = usize::try_from(offset).context("Negative Audio Unit event offset")?;
        self.stage(Staged::Midi(data, offset))
    }

    fn publish_timeline(&self, start: usize) {
        let t = &self.transport;
        let elapsed = if t.playing { start as f64 } else { 0.0 };
        *lock(&self.shared.timeline) = Timeline {
            playing: t.playing,
            changed: self.transport_changed,
            tempo: t.tempo,
            beat: t.position.quarter_notes + elapsed / self.sample_rate * t.tempo / 60.0,
            sample: t.position.samples as f64 + elapsed,
            numerator: t.numerator,
            denominator: t.denominator,
            sample_rate: self.sample_rate,
        };
    }

    fn render_segment(
        &mut self,
        buffers: &mut BusAudioBuffers,
        start: usize,
        end: usize,
    ) -> Result<()> {
        let frames = end - start;
        {
            let mut feed = lock(&self.shared.feed);
            feed.frames = frames;
            for channels in &mut feed.buses {
                channels.clear();
            }
            for (bus, ours) in buffers.inputs.iter_mut().zip(&self.inputs) {
                if let Some(target) = feed.buses.get_mut(ours.element as usize) {
                    target.extend(bus.channels.iter_mut().map(|c| c[start..].as_mut_ptr()));
                }
            }
        }
        self.publish_timeline(start);
        // SAFETY: an all-zero timestamp is valid; only the sample time is declared.
        let mut stamp: AudioTimeStamp = unsafe { std::mem::zeroed() };
        stamp.mSampleTime = self.sample_time;
        stamp.mFlags = AudioTimeStampFlags::SampleTimeValid;
        let unit = self.unit;
        for ((bus, ours), list) in buffers
            .outputs
            .iter_mut()
            .zip(&self.outputs)
            .zip(&mut self.lists)
        {
            for (buffer, channel) in list.buffers().iter_mut().zip(bus.channels.iter_mut()) {
                buffer.mNumberChannels = 1;
                buffer.mDataByteSize = (frames * 4) as u32;
                buffer.mData = channel[start..].as_mut_ptr().cast();
            }
            let mut flags = AudioUnitRenderActionFlags(0);
            // SAFETY: the list describes `frames` writable samples per channel, and the
            // input feed points into `buffers` for the duration of the call.
            let status = unsafe {
                AudioUnitRender(
                    unit,
                    &mut flags,
                    NonNull::from(&stamp),
                    ours.element,
                    frames as u32,
                    NonNull::new_unchecked(list.list()),
                )
            };
            check(status, "Audio Unit rendering")?;
            // A unit may answer with its own buffers instead of ours.
            for (buffer, channel) in list.buffers().iter().zip(bus.channels.iter_mut()) {
                let source = buffer.mData.cast::<f32>().cast_const();
                let target = &mut channel[start..end];
                if !source.is_null() && source != target.as_ptr() {
                    let length = frames.min(buffer.mDataByteSize as usize / 4);
                    // SAFETY: the unit's buffer is valid until its next render.
                    target[..length]
                        .copy_from_slice(unsafe { std::slice::from_raw_parts(source, length) });
                }
            }
        }
        lock(&self.shared.feed)
            .buses
            .iter_mut()
            .for_each(Vec::clear);
        self.sample_time += frames as f64;
        Ok(())
    }

    pub(crate) fn name(&self) -> &str {
        &self.info.name
    }

    pub(crate) fn take_dirty(&mut self) -> bool {
        self.shared.dirty.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn take_edits(&mut self) -> Vec<ParameterEdit> {
        let raw = std::mem::take(&mut *lock(&self.shared.edits));
        raw.into_iter()
            .filter_map(|edit| match edit {
                RawEdit::Begin(id) => self.param(id).ok().map(|_| ParameterEdit::Begin(id)),
                RawEdit::Value(id, plain) => self.param(id).ok().map(|meta| {
                    ParameterEdit::Value(
                        id,
                        normalized_from_plain(meta.min, meta.max, f64::from(plain)),
                    )
                }),
            })
            .collect()
    }

    /// Embed the unit's Cocoa view, or Apple's generic parameter view when it has
    /// none (AUv3 extensions included), into the host window's container view.
    pub(crate) fn open_gui(&mut self, container: *mut c_void) -> Result<(u32, u32)> {
        MainThreadMarker::new().context("Audio Unit editors open on the helper main thread")?;
        ensure!(!container.is_null(), "Missing editor window");
        let unit = self.unit;
        let view = autoreleasepool(|_| {
            // SAFETY: AppKit is initialized on this thread and `unit` is live.
            unsafe { cocoa_view(unit).or_else(|| generic_view(unit)) }
        })
        .context("This Audio Unit has no editor view")?;
        let mut size = view.frame().size;
        if !(size.width >= 1.0 && size.height >= 1.0) {
            size = NSSize::new(480.0, 320.0);
            view.setFrameSize(size);
        }
        view.setFrameOrigin(NSPoint::new(0.0, 0.0));
        // SAFETY: the container is the host window's live NSView.
        let parent = unsafe { &*container.cast::<NSView>() };
        parent.addSubview(&view);
        self.view_size = view_size(&view);
        self.view = Some(view);
        if let Err(error) = self.listen_to_editor() {
            eprintln!("Audio Unit editor gestures are unavailable: {error:#}");
        }
        Ok(self.view_size)
    }

    /// Value and gesture events from the view, delivered by this thread's run loop.
    fn listen_to_editor(&mut self) -> Result<()> {
        let refcon = self.shared.refcon();
        let mut listener: EventListener = ptr::null_mut();
        let run_loop = CFRunLoop::current().context("No run loop on the editor thread")?;
        // SAFETY: reading an immutable framework constant.
        let mode = unsafe { kCFRunLoopDefaultMode }.map_or(ptr::null(), |mode| mode as *const _);
        // SAFETY: `refcon` outlives the listener, which `close_gui` disposes first.
        check(
            unsafe {
                AUEventListenerCreate(
                    parameter_event,
                    refcon,
                    CFRetained::as_ptr(&run_loop).as_ptr(),
                    mode,
                    0.01,
                    0.01,
                    &mut listener,
                )
            },
            "Creating the Audio Unit event listener",
        )?;
        self.listener = listener;
        for meta in self.params.iter().filter(|meta| meta.writable) {
            for event_type in [EVENT_VALUE, EVENT_BEGIN] {
                let event = AudioUnitEvent {
                    event_type,
                    argument: AudioUnitParameter {
                        mAudioUnit: self.unit,
                        mParameterID: meta.id,
                        mScope: kAudioUnitScope_Global,
                        mElement: 0,
                    },
                };
                // SAFETY: `listener` is live and `event` names this unit.
                unsafe { AUEventListenerAddEventType(listener, refcon, &event) };
            }
        }
        Ok(())
    }

    /// The unit's view resizes itself; follow its frame.
    pub(crate) fn take_gui_resize(&mut self) -> Option<(u32, u32)> {
        let size = view_size(self.view.as_ref()?);
        (size != self.view_size).then(|| {
            self.view_size = size;
            size
        })
    }

    pub(crate) fn resize_gui(&mut self, width: u32, height: u32) {
        if let Some(view) = &self.view {
            view.setFrameSize(NSSize::new(f64::from(width), f64::from(height)));
            self.view_size = (width, height);
        }
    }

    pub(crate) fn close_gui(&mut self) {
        if !self.listener.is_null() {
            // SAFETY: the listener is ours and still live.
            unsafe {
                AUListenerDispose(std::mem::replace(&mut self.listener, ptr::null_mut()).cast())
            };
        }
        if let Some(view) = self.view.take() {
            view.removeFromSuperview();
        }
    }

    /// Tell an open view that every parameter may have changed.
    fn notify_editor(&self) {
        if self.listener.is_null() {
            return;
        }
        let mut parameter = AudioUnitParameter {
            mAudioUnit: self.unit,
            mParameterID: kAUParameterListener_AnyParameter,
            mScope: kAudioUnitScope_Global,
            mElement: 0,
        };
        // SAFETY: `parameter` names a live unit; the listener is ours.
        unsafe {
            AUParameterListenerNotify(
                self.listener.cast(),
                self.shared.refcon(),
                NonNull::from(&mut parameter),
            )
        };
    }
}

fn view_size(view: &NSView) -> (u32, u32) {
    let size = view.frame().size;
    let clamp = |value: f64| value.round().clamp(64.0, 8192.0) as u32;
    (clamp(size.width), clamp(size.height))
}

/// `objc_msgSend` with exact prototypes: the AU view entry points take an
/// `AudioUnit`, whose runtime type encoding objc2's checked `msg_send!` cannot name.
fn msg_send_fn<F>() -> F {
    // SAFETY: only instantiated below with `extern "C-unwind"` function pointer types.
    unsafe {
        std::mem::transmute_copy(&(objc2::ffi::objc_msgSend as unsafe extern "C-unwind" fn()))
    }
}

/// # Safety
/// `receiver` responds to the argument-less `selector` returning an object.
unsafe fn send0(receiver: *mut AnyObject, selector: Sel) -> *mut AnyObject {
    let imp: unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> *mut AnyObject = msg_send_fn();
    // SAFETY: see above.
    unsafe { imp(receiver, selector) }
}

/// # Safety
/// `receiver` responds to `selector` taking an `AudioUnit` and returning an object.
unsafe fn send_unit(receiver: *mut AnyObject, selector: Sel, unit: AudioUnit) -> *mut AnyObject {
    let imp: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, AudioUnit) -> *mut AnyObject =
        msg_send_fn();
    // SAFETY: see above.
    unsafe { imp(receiver, selector, unit) }
}

/// # Safety
/// `receiver` implements `AUCocoaUIBase`'s `uiViewForAudioUnit:withSize:`.
unsafe fn send_unit_size(
    receiver: *mut AnyObject,
    selector: Sel,
    unit: AudioUnit,
    size: NSSize,
) -> *mut AnyObject {
    let imp: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, AudioUnit, NSSize) -> *mut AnyObject =
        msg_send_fn();
    // SAFETY: see above.
    unsafe { imp(receiver, selector, unit, size) }
}

fn class_object(class: &AnyClass) -> *mut AnyObject {
    (class as *const AnyClass).cast_mut().cast()
}

/// The view from the unit's `AUCocoaUIBase` factory (`kAudioUnitProperty_CocoaUI`).
///
/// # Safety
/// Main thread, inside an autorelease pool, with a live `unit`.
unsafe fn cocoa_view(unit: AudioUnit) -> Option<Retained<NSView>> {
    let mut size = 0u32;
    // SAFETY: querying a variable-length property.
    let status = unsafe {
        AudioUnitGetPropertyInfo(
            unit,
            kAudioUnitProperty_CocoaUI,
            kAudioUnitScope_Global,
            0,
            &mut size,
            ptr::null_mut(),
        )
    };
    let words = size as usize / size_of::<*const c_void>();
    if status != 0 || words < 2 {
        return None;
    }
    let mut raw = vec![ptr::null_mut::<c_void>(); words];
    let mut io = (words * size_of::<*const c_void>()) as u32;
    // SAFETY: `raw` holds `io` bytes.
    let status = unsafe {
        AudioUnitGetProperty(
            unit,
            kAudioUnitProperty_CocoaUI,
            kAudioUnitScope_Global,
            0,
            NonNull::new_unchecked(raw.as_mut_ptr()).cast(),
            NonNull::from(&mut io),
        )
    };
    if status != 0 {
        return None;
    }
    // The host owns the returned bundle URL and class names.
    // SAFETY: each non-null entry is a +1 CoreFoundation object.
    let owned: Vec<CFRetained<CFType>> = raw
        .iter()
        .filter_map(|&value| NonNull::new(value.cast::<CFType>()))
        .map(|value| unsafe { CFRetained::from_raw(value) })
        .collect();
    if owned.len() != words {
        return None;
    }
    // SAFETY: CFURL and CFString are toll-free bridged with NSURL and NSString.
    let url = unsafe { &*raw[0].cast::<NSURL>() };
    let bundle = NSBundle::bundleWithURL(url)?;
    let selector = sel!(uiViewForAudioUnit:withSize:);
    for &name in &raw[1..] {
        let name = unsafe { &*name.cast::<NSString>() };
        let Some(class) = bundle.classNamed(name) else {
            continue;
        };
        // SAFETY: `new` returns a +1 instance or nil.
        let factory = unsafe { send0(class_object(class), sel!(new)) };
        let Some(factory) = (unsafe { Retained::from_raw(factory) }) else {
            continue;
        };
        let responds: bool = unsafe { msg_send![&*factory, respondsToSelector: selector] };
        if !responds {
            continue;
        }
        // SAFETY: the factory implements `AUCocoaUIBase`; its view is autoreleased.
        let view = unsafe {
            send_unit_size(
                Retained::as_ptr(&factory).cast_mut(),
                selector,
                unit,
                NSSize::new(480.0, 320.0),
            )
        };
        if let Some(view) = unsafe { Retained::retain(view.cast::<NSView>()) } {
            return Some(view);
        }
    }
    None
}

/// CoreAudioKit's `AUGenericView`: sliders for every parameter of any unit.
///
/// # Safety
/// Main thread with a live `unit`.
unsafe fn generic_view(unit: AudioUnit) -> Option<Retained<NSView>> {
    let bundle = NSBundle::bundleWithPath(&NSString::from_str(
        "/System/Library/Frameworks/CoreAudioKit.framework",
    ))?;
    // SAFETY: loading a system framework.
    if !unsafe { bundle.load() } {
        return None;
    }
    let class = AnyClass::get(c"AUGenericView")?;
    // SAFETY: `alloc` then the designated `initWithAudioUnit:`, which returns +1.
    unsafe {
        let allocated = send0(class_object(class), sel!(alloc));
        if allocated.is_null() {
            return None;
        }
        let view = send_unit(allocated, sel!(initWithAudioUnit:), unit);
        Retained::from_raw(view.cast::<NSView>())
    }
}

impl Drop for AuProcessor {
    fn drop(&mut self) {
        self.close_gui();
        self.uninitialize();
        // SAFETY: nothing uses the unit afterwards; `shared` outlives this call.
        unsafe { AudioComponentInstanceDispose(self.unit) };
    }
}

impl NativeProcessor for AuProcessor {
    fn loaded_info(&self) -> LoadedInfo {
        LoadedInfo {
            name: self.info.name.clone(),
            vendor: self.info.vendor.clone(),
            version: self.info.version.clone(),
            category: self.info.category.clone(),
            uid: self.info.uid.clone(),
            has_gui: self.info.has_gui,
            has_midi_input: self.info.has_midi_input,
            has_midi_output: false,
        }
    }

    fn start_processing(&mut self) -> Result<()> {
        if !self.initialized {
            self.initialize()?;
        }
        // SAFETY: plain call on a live unit.
        unsafe { AudioUnitReset(self.unit, kAudioUnitScope_Global, 0) };
        self.processing = true;
        Ok(())
    }

    fn stop_processing(&mut self) -> Result<()> {
        self.processing = false;
        Ok(())
    }

    fn is_processing(&self) -> bool {
        self.processing
    }

    fn reconfigure(&mut self, sample_rate: f64, block_size: usize) -> Result<()> {
        ensure!(
            sample_rate.is_finite() && sample_rate > 0.0 && (1..=65_536).contains(&block_size),
            "Invalid Audio Unit processing configuration"
        );
        self.uninitialize();
        self.sample_rate = sample_rate;
        self.max_block = block_size;
        self.configure()?;
        self.initialize()
    }

    fn set_process_mode(&mut self, mode: ProcessMode) -> Result<()> {
        let offline = u32::from(mode == ProcessMode::Offline);
        // Optional: most units render identically either way.
        let _ = set(
            self.unit,
            kAudioUnitProperty_OfflineRender,
            kAudioUnitScope_Global,
            0,
            &offline,
        );
        Ok(())
    }

    fn service_host_requests(&mut self) -> Result<i32> {
        let bits = self.shared.restart.swap(0, Ordering::AcqRel);
        if bits & RESTART_PARAM_TITLES_CHANGED != 0 {
            self.scan_params();
        }
        if bits & RESTART_LATENCY_CHANGED != 0 {
            self.update_latency();
        }
        if bits & TAIL_CHANGED != 0 {
            self.update_tail();
        }
        Ok(bits & !TAIL_CHANGED)
    }

    fn audio_bus_layout(&self) -> Result<AudioBusLayout> {
        let config = |buses: &[Bus]| {
            buses
                .iter()
                .map(|bus| AudioBusConfig {
                    channel_count: bus.channels,
                    active: bus.active,
                })
                .collect()
        };
        Ok(AudioBusLayout {
            inputs: config(&self.inputs),
            outputs: config(&self.outputs),
        })
    }

    fn audio_bus_details(&self) -> Result<AudioBusDetailsLayout> {
        let details = |buses: &[Bus]| {
            buses
                .iter()
                .map(|bus| AudioBusDetails {
                    name: bus.name.clone(),
                    channels: bus.channels,
                    active: bus.active,
                    main: bus.element == 0,
                })
                .collect()
        };
        Ok(AudioBusDetailsLayout {
            inputs: details(&self.inputs),
            outputs: details(&self.outputs),
        })
    }

    /// Elements stay connected; inactive Rill buses read and produce silence.
    fn set_bus_active(
        &mut self,
        media: MediaType,
        direction: BusDirection,
        index: i32,
        active: bool,
    ) -> Result<()> {
        if media == MediaType::Event {
            return Ok(());
        }
        let buses = match direction {
            BusDirection::Input => &mut self.inputs,
            BusDirection::Output => &mut self.outputs,
        };
        usize::try_from(index)
            .ok()
            .and_then(|index| buses.get_mut(index))
            .context("Invalid Audio Unit bus index")?
            .active = active;
        Ok(())
    }

    fn set_transport_position(&mut self, position: TransportPosition) -> Result<()> {
        ensure!(
            position.quarter_notes.is_finite(),
            "Invalid transport position"
        );
        self.transport.position = position;
        Ok(())
    }
    fn transport_position(&self) -> Result<TransportPosition> {
        Ok(self.transport.position)
    }
    fn set_tempo(&mut self, bpm: f64) -> Result<()> {
        Transport::validate_tempo(bpm)?;
        self.transport.tempo = bpm;
        Ok(())
    }
    fn set_playing(&mut self, playing: bool) -> Result<()> {
        self.transport_changed |= self.transport.playing != playing;
        self.transport.playing = playing;
        Ok(())
    }
    fn set_time_signature(&mut self, numerator: i32, denominator: i32) -> Result<()> {
        ensure!(numerator > 0 && denominator > 0, "Invalid time signature");
        self.transport.numerator = numerator;
        self.transport.denominator = denominator;
        Ok(())
    }

    fn get_parameters(&mut self) -> Result<Vec<Parameter>> {
        Ok(self
            .params
            .iter()
            .map(|p| {
                plain_parameter(
                    p.id,
                    p.name.clone(),
                    p.unit.clone(),
                    p.min,
                    p.max,
                    p.default,
                    self.plain_value(p.id).unwrap_or(p.default),
                    p.stepped,
                    p.writable,
                    !p.writable,
                    false,
                )
            })
            .collect())
    }

    fn get_parameter(&mut self, id: u32) -> Result<f64> {
        let meta = self.param(id)?;
        let plain = self
            .plain_value(id)
            .context("The Audio Unit did not report a parameter value")?;
        Ok(normalized_from_plain(meta.min, meta.max, plain))
    }

    fn set_parameter(&mut self, id: u32, value: f64) -> Result<()> {
        ensure!(value.is_finite(), "Invalid parameter value");
        let meta = self.param(id)?;
        let plain = plain_from_normalized(meta.min, meta.max, meta.stepped, value) as f32;
        self.apply_parameter(id, plain)
    }

    fn set_parameter_at(&mut self, id: u32, value: f64, offset: i32) -> Result<()> {
        ensure!(value.is_finite(), "Invalid parameter value");
        let meta = self.param(id)?;
        let plain = plain_from_normalized(meta.min, meta.max, meta.stepped, value) as f32;
        let offset = usize::try_from(offset).context("Negative Audio Unit event offset")?;
        self.stage(Staged::Param(id, plain, offset))
    }

    fn format_parameter(&mut self, id: u32, value: f64) -> Result<String> {
        let meta = self.param(id)?;
        let plain = plain_from_normalized(meta.min, meta.max, meta.stepped, value);
        let mut plain_f32 = plain as f32;
        let mut query = AudioUnitParameterStringFromValue {
            inParamID: id,
            inValue: NonNull::from(&mut plain_f32),
            outString: ptr::null(),
        };
        let mut size = size_of::<AudioUnitParameterStringFromValue>() as u32;
        // SAFETY: `query` is the property's in/out structure; its string is returned at +1.
        let text = unsafe {
            let status = AudioUnitGetProperty(
                self.unit,
                kAudioUnitProperty_ParameterStringFromValue,
                kAudioUnitScope_Global,
                0,
                NonNull::from(&mut query).cast(),
                NonNull::from(&mut size),
            );
            if status == 0 {
                take_string(query.outString)
            } else {
                None
            }
        };
        let number = if meta.stepped {
            format!("{plain:.0}")
        } else {
            format!("{plain:.3}")
        };
        Ok(match (text, meta.unit.is_empty()) {
            (Some(text), _) if !text.is_empty() => text,
            (_, true) => number,
            (_, false) => format!("{number} {}", meta.unit),
        })
    }

    fn send_midi_event_at(&mut self, event: MidiEvent, offset: i32) -> Result<()> {
        match crate::clap::midi_bytes(event) {
            Some(data) => self.stage_midi(data, offset),
            None => Ok(()),
        }
    }

    fn send_plugin_event(&mut self, event: PluginEvent) -> Result<()> {
        let channel = |channel: i16| channel.clamp(0, 15) as u8;
        let key = |pitch: i16| pitch.clamp(0, 127) as u8;
        let data = match event.data {
            PluginEventData::NoteOn {
                channel: c,
                pitch,
                velocity,
                ..
            } => [
                0x90 | channel(c),
                key(pitch),
                ((velocity * 127.0).round() as u8).clamp(1, 127),
            ],
            PluginEventData::NoteOff {
                channel: c,
                pitch,
                velocity,
                ..
            } => [
                0x80 | channel(c),
                key(pitch),
                ((velocity * 127.0).round() as u8).min(127),
            ],
            PluginEventData::PolyPressure {
                channel: c,
                pitch,
                pressure,
                ..
            } => [
                0xA0 | channel(c),
                key(pitch),
                ((pressure.clamp(0.0, 1.0) * 127.0).round() as u8).min(127),
            ],
            _ => return Ok(()),
        };
        self.stage_midi(data, event.sample_offset)
    }

    fn midi_panic(&mut self) -> Result<()> {
        for channel in 0..16u8 {
            self.stage_midi([0xB0 | channel, 123, 0], 0)?;
        }
        Ok(())
    }

    fn process_bus_audio(&mut self, buffers: &mut BusAudioBuffers) -> Result<()> {
        let frames = buffers.block_size;
        ensure!(
            frames > 0 && frames <= self.max_block,
            "Invalid Audio Unit block size"
        );
        ensure!(self.processing, "Audio Unit is not processing");
        ensure!(
            buffers.inputs.len() == self.inputs.len()
                && buffers.outputs.len() == self.outputs.len(),
            "Audio Unit bus layout changed"
        );
        for (bus, ours) in buffers.inputs.iter_mut().zip(&self.inputs) {
            ensure!(
                bus.channels.len() == ours.channels
                    && bus.channels.iter().all(|c| c.len() >= frames),
                "Audio Unit input buffer shape differs from its bus"
            );
            if !ours.active {
                bus.channels.iter_mut().for_each(|c| c.fill(0.0));
            }
        }
        for (bus, ours) in buffers.outputs.iter().zip(&self.outputs) {
            ensure!(
                bus.channels.len() == ours.channels
                    && bus.channels.iter().all(|c| c.len() >= frames),
                "Audio Unit output buffer shape differs from its bus"
            );
        }
        // Parameter changes split the block so each lands at its offset; MIDI keeps
        // its offset within the segment it falls in. At equal offsets a parameter
        // precedes MIDI, so a segment starting there applies it first.
        let last = frames - 1;
        let offset = |event: &Staged| match *event {
            Staged::Midi(_, offset) | Staged::Param(_, _, offset) => offset.min(last),
        };
        let mut staged = std::mem::take(&mut self.staged);
        staged.sort_by_key(|event| (offset(event), matches!(event, Staged::Midi(..))));
        let mut boundaries: Vec<usize> = staged
            .iter()
            .filter(|event| matches!(event, Staged::Param(..)))
            .map(offset)
            .collect();
        boundaries.push(frames);
        boundaries.dedup();
        let mut next = 0;
        let mut start = 0;
        let mut result = Ok(());
        for end in boundaries {
            while let Some(&event) = staged.get(next) {
                match event {
                    Staged::Param(id, value, _) if offset(&event) <= start => {
                        // A rejected automation point leaves the previous value.
                        let _ = self.apply_parameter(id, value);
                    }
                    _ => break,
                }
                next += 1;
            }
            if end <= start {
                continue;
            }
            while let Some(&event) = staged.get(next) {
                match event {
                    Staged::Midi(data, _) if offset(&event) < end => {
                        let at = (offset(&event) - start) as u32;
                        // SAFETY: plain call on a live music device; a rejected event is dropped.
                        unsafe {
                            MusicDeviceMIDIEvent(
                                self.unit,
                                u32::from(data[0]),
                                u32::from(data[1]),
                                u32::from(data[2]),
                                at,
                            )
                        };
                    }
                    _ => break,
                }
                next += 1;
            }
            if result.is_ok() {
                result = self.render_segment(buffers, start, end);
            }
            start = end;
        }
        staged.clear();
        self.staged = staged;
        result?;
        for (bus, ours) in buffers.outputs.iter_mut().zip(&self.outputs) {
            if !ours.active {
                bus.channels.iter_mut().for_each(|c| c[..frames].fill(0.0));
            }
        }
        self.transport.advance(frames, self.sample_rate);
        self.transport_changed = false;
        Ok(())
    }

    fn latency_samples(&self) -> u32 {
        self.latency
    }

    fn tail_samples(&self) -> u32 {
        self.tail
    }

    /// `kAudioUnitProperty_ClassInfo` as a binary property list.
    fn save_state(&mut self) -> Result<Vec<u8>> {
        let plist = get::<*const CFType>(
            self.unit,
            kAudioUnitProperty_ClassInfo,
            kAudioUnitScope_Global,
            0,
        )
        .and_then(|plist| NonNull::new(plist.cast_mut()))
        .context("The Audio Unit did not provide its state")?;
        // SAFETY: the property returns the dictionary at +1.
        let plist = unsafe { CFRetained::from_raw(plist) };
        // SAFETY: `plist` is a property list; no error object is requested.
        let data = unsafe {
            CFPropertyListCreateData(
                None,
                Some(&plist),
                CFPropertyListFormat::BinaryFormat_v1_0,
                0,
                ptr::null_mut(),
            )
        }
        .context("Serializing the Audio Unit state failed")?;
        let bytes = data.to_vec();
        audio_unit::validate_state(&bytes)?;
        Ok(bytes)
    }

    fn load_state(&mut self, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        audio_unit::validate_state(data)?;
        let data = CFData::from_bytes(data);
        // SAFETY: `data` is a live CFData; no format or error output is requested.
        let plist = unsafe {
            CFPropertyListCreateWithData(None, Some(&data), 0, ptr::null_mut(), ptr::null_mut())
        }
        .context("Invalid Audio Unit state")?;
        let raw: *const CFType = CFRetained::as_ptr(&plist).as_ptr();
        check(
            set(
                self.unit,
                kAudioUnitProperty_ClassInfo,
                kAudioUnitScope_Global,
                0,
                &raw,
            ),
            "Restoring the Audio Unit state",
        )?;
        self.shared
            .restart
            .fetch_or(RESTART_PARAM_VALUES_CHANGED, Ordering::AcqRel);
        self.notify_editor();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_lists_hold_one_buffer_per_channel() {
        let mut list = BufferList::new(3);
        assert_eq!(list.buffers().len(), 3);
        // SAFETY: `buffers` initialized the header.
        assert_eq!(unsafe { (*list.list()).mNumberBuffers }, 3);
        assert!(list.words.len() * 8 >= offset_of!(AudioBufferList, mBuffers) + 3 * 16);
    }

    #[test]
    fn apple_delay_is_registered_with_its_vendor() {
        let mut query = description(AudioUnitId::parse("aufx:dely:appl").unwrap());
        // SAFETY: `query` is a valid description.
        let component =
            unsafe { AudioComponentFindNext(ptr::null_mut(), NonNull::from(&mut query)) };
        assert!(!component.is_null());
        let (vendor, name, version) = component_identity(component);
        assert_eq!(vendor, "Apple");
        assert!(name.contains("Delay"), "{name}");
        assert!(!version.is_empty());
    }
}

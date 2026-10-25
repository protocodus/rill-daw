//! Test-only native VST3. Any attempted editor construction terminates its process.
//! Implements the generated SDK interfaces, and is never distributed with Rill.
#![allow(non_snake_case, unsafe_op_in_unsafe_fn)]

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use vst3::{Class, ComRef, ComWrapper, Steinberg::Vst::*, Steinberg::*};

const CLASS_ID: TUID = [0x11; 16];
const GAIN: u32 = 7;
const LIFECYCLE_DELAY_MS: u64 = 0;
const PROCESS_DELAY_MS: u64 = 0;

fn lifecycle_delay() {
    std::thread::sleep(std::time::Duration::from_millis(LIFECYCLE_DELAY_MS));
}

fn text8<const N: usize>(value: &str) -> [char8; N] {
    let mut result = [0; N];
    for (dest, source) in result.iter_mut().take(N - 1).zip(value.bytes()) {
        // SDK char8 follows the platform C char signedness (u8 on ARM Linux).
        *dest = source as char8;
    }
    result
}

fn text16(value: &str) -> String128 {
    let mut result = [0; 128];
    for (dest, source) in result.iter_mut().take(127).zip(value.encode_utf16()) {
        *dest = source as TChar;
    }
    result
}

struct Factory;
impl Class for Factory {
    type Interfaces = (IPluginFactory2,);
}
impl IPluginFactoryTrait for Factory {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        *info = PFactoryInfo {
            vendor: text8("RILL tests"),
            url: text8(""),
            email: text8(""),
            flags: 0,
        };
        kResultOk
    }
    unsafe fn countClasses(&self) -> int32 {
        1
    }
    unsafe fn getClassInfo(&self, index: int32, info: *mut PClassInfo) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }
        *info = PClassInfo {
            cid: CLASS_ID,
            cardinality: i32::MAX,
            category: text8("Audio Module Class"),
            name: text8("No Editor Probe"),
        };
        kResultOk
    }
    unsafe fn createInstance(
        &self,
        cid: FIDString,
        iid: FIDString,
        obj: *mut *mut c_void,
    ) -> tresult {
        *obj = ptr::null_mut();
        if ptr::read_unaligned(cid.cast::<TUID>()) != CLASS_ID {
            return kInvalidArgument;
        }
        let plugin = ComWrapper::new(Instrument {
            gain: AtomicU64::new(0.5_f64.to_bits()),
            playing: AtomicBool::new(false),
        })
        .to_com_ptr::<IComponent>()
        .unwrap();
        let unknown = plugin.as_ptr().cast::<FUnknown>();
        ((*(*unknown).vtbl).queryInterface)(unknown, iid.cast(), obj)
    }
}

impl IPluginFactory2Trait for Factory {
    unsafe fn getClassInfo2(&self, index: int32, info: *mut PClassInfo2) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }
        *info = PClassInfo2 {
            cid: CLASS_ID,
            cardinality: i32::MAX,
            category: text8("Audio Module Class"),
            name: text8("No Editor Probe"),
            classFlags: 0,
            subCategories: text8("Instrument|Synth"),
            vendor: text8("RILL tests"),
            version: text8("1.0"),
            sdkVersion: text8("VST 3.7"),
        };
        kResultOk
    }
}

struct Instrument {
    gain: AtomicU64,
    playing: AtomicBool,
}
impl Class for Instrument {
    type Interfaces = (IComponent, IAudioProcessor, IEditController);
}
impl Instrument {
    unsafe fn restore(&self, state: *mut IBStream) -> tresult {
        let Some(state) = ComRef::from_raw(state) else {
            return kInvalidArgument;
        };
        let mut bytes = [0u8; 8];
        let mut count = 0;
        if state.read(bytes.as_mut_ptr().cast(), 8, &mut count) != kResultOk || count != 8 {
            return kResultFalse;
        }
        self.gain.store(u64::from_le_bytes(bytes), Relaxed);
        kResultOk
    }
    unsafe fn save(&self, state: *mut IBStream) -> tresult {
        let Some(state) = ComRef::from_raw(state) else {
            return kInvalidArgument;
        };
        let mut bytes = self.gain.load(Relaxed).to_le_bytes();
        state.write(bytes.as_mut_ptr().cast(), 8, ptr::null_mut())
    }
}
impl IPluginBaseTrait for Instrument {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }
    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}
impl IComponentTrait for Instrument {
    unsafe fn getControllerClassId(&self, _classId: *mut TUID) -> tresult {
        kResultFalse
    }
    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }
    unsafe fn getBusCount(&self, media: MediaType, direction: BusDirection) -> int32 {
        i32::from(
            (media == MediaTypes_::kAudio as i32 && direction == BusDirections_::kOutput as i32)
                || (media == MediaTypes_::kEvent as i32
                    && direction == BusDirections_::kInput as i32),
        )
    }
    unsafe fn getBusInfo(
        &self,
        media: MediaType,
        direction: BusDirection,
        index: int32,
        bus: *mut BusInfo,
    ) -> tresult {
        if index != 0 || self.getBusCount(media, direction) == 0 {
            return kInvalidArgument;
        }
        *bus = BusInfo {
            mediaType: media,
            direction,
            channelCount: if media == MediaTypes_::kAudio as i32 {
                2
            } else {
                16
            },
            name: text16("Main"),
            busType: BusTypes_::kMain as i32,
            flags: BusInfo_::BusFlags_::kDefaultActive as u32,
        };
        kResultOk
    }
    unsafe fn getRoutingInfo(
        &self,
        _input: *mut RoutingInfo,
        _output: *mut RoutingInfo,
    ) -> tresult {
        kResultFalse
    }
    unsafe fn activateBus(
        &self,
        _media: MediaType,
        _direction: BusDirection,
        _index: int32,
        _state: TBool,
    ) -> tresult {
        kResultOk
    }
    unsafe fn setActive(&self, _state: TBool) -> tresult {
        lifecycle_delay();
        kResultOk
    }
    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        self.restore(state)
    }
    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        self.save(state)
    }
}
impl IAudioProcessorTrait for Instrument {
    unsafe fn setBusArrangements(
        &self,
        _inputs: *mut SpeakerArrangement,
        numIns: int32,
        _outputs: *mut SpeakerArrangement,
        numOuts: int32,
    ) -> tresult {
        if numIns == 0 && numOuts == 1 {
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn getBusArrangement(
        &self,
        direction: BusDirection,
        index: int32,
        arrangement: *mut SpeakerArrangement,
    ) -> tresult {
        if direction != BusDirections_::kOutput as i32 || index != 0 {
            return kInvalidArgument;
        }
        *arrangement = SpeakerArr::kStereo;
        kResultOk
    }
    unsafe fn canProcessSampleSize(&self, size: int32) -> tresult {
        if size == SymbolicSampleSizes_::kSample32 as i32 {
            kResultOk
        } else {
            kResultFalse
        }
    }
    unsafe fn getLatencySamples(&self) -> uint32 {
        0
    }
    unsafe fn setupProcessing(&self, _setup: *mut ProcessSetup) -> tresult {
        lifecycle_delay();
        kResultOk
    }
    unsafe fn setProcessing(&self, _state: TBool) -> tresult {
        lifecycle_delay();
        kResultOk
    }
    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        std::thread::sleep(std::time::Duration::from_millis(PROCESS_DELAY_MS));
        let data = &mut *data;
        let events = ComRef::from_raw(data.inputEvents);
        let mut playing = self.playing.load(Relaxed);
        for frame in 0..data.numSamples {
            if let Some(events) = events.as_ref() {
                for index in 0..events.getEventCount() {
                    let mut event: Event = std::mem::zeroed();
                    if events.getEvent(index, &mut event) == kResultOk
                        && event.sampleOffset == frame
                    {
                        if event.r#type == Event_::EventTypes_::kNoteOnEvent as u16 {
                            playing = true;
                        }
                        if event.r#type == Event_::EventTypes_::kNoteOffEvent as u16 {
                            playing = false;
                        }
                    }
                }
            }
            let value = if playing {
                f64::from_bits(self.gain.load(Relaxed)) as f32 * 0.25
            } else {
                0.0
            };
            for bus_index in 0..data.numOutputs {
                let bus = &mut *data.outputs.offset(bus_index as isize);
                bus.silenceFlags = 0;
                for channel in 0..bus.numChannels {
                    let samples = *bus.__field0.channelBuffers32.offset(channel as isize);
                    *samples.offset(frame as isize) = value;
                }
            }
        }
        self.playing.store(playing, Relaxed);
        kResultOk
    }
    unsafe fn getTailSamples(&self) -> uint32 {
        0
    }
}
impl IEditControllerTrait for Instrument {
    unsafe fn setComponentState(&self, state: *mut IBStream) -> tresult {
        self.restore(state)
    }
    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        self.restore(state)
    }
    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        self.save(state)
    }
    unsafe fn getParameterCount(&self) -> int32 {
        lifecycle_delay();
        1
    }
    unsafe fn getParameterInfo(&self, index: int32, info: *mut ParameterInfo) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }
        *info = ParameterInfo {
            id: GAIN,
            title: text16("Level"),
            shortTitle: text16("Level"),
            units: text16("%"),
            stepCount: 0,
            defaultNormalizedValue: 0.5,
            unitId: 0,
            flags: ParameterInfo_::ParameterFlags_::kCanAutomate,
        };
        kResultOk
    }
    unsafe fn getParamStringByValue(
        &self,
        _id: ParamID,
        value: ParamValue,
        output: *mut String128,
    ) -> tresult {
        *output = text16(&format!("{:.0}", value * 100.0));
        kResultOk
    }
    unsafe fn getParamValueByString(
        &self,
        _id: ParamID,
        _string: *mut TChar,
        _value: *mut ParamValue,
    ) -> tresult {
        kResultFalse
    }
    unsafe fn normalizedParamToPlain(&self, _id: ParamID, value: ParamValue) -> ParamValue {
        value * 100.0
    }
    unsafe fn plainParamToNormalized(&self, _id: ParamID, value: ParamValue) -> ParamValue {
        value / 100.0
    }
    unsafe fn getParamNormalized(&self, _id: ParamID) -> ParamValue {
        f64::from_bits(self.gain.load(Relaxed))
    }
    unsafe fn setParamNormalized(&self, id: ParamID, value: ParamValue) -> tresult {
        if id != GAIN {
            return kInvalidArgument;
        }
        self.gain.store(value.to_bits(), Relaxed);
        kResultOk
    }
    unsafe fn setComponentHandler(&self, _handler: *mut IComponentHandler) -> tresult {
        kResultOk
    }
    unsafe fn createView(&self, _name: FIDString) -> *mut IPlugView {
        // A load/capability query reaching this kills only its test-owned process.
        eprintln!("REGRESSION: DSP-only hosting called IEditController::createView");
        std::process::exit(86)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn GetPluginFactory() -> *mut IPluginFactory {
    ComWrapper::new(Factory)
        .to_com_ptr::<IPluginFactory>()
        .unwrap()
        .into_raw()
}
#[unsafe(no_mangle)]
pub extern "C" fn bundleEntry(_bundle: *mut c_void) -> bool {
    true
}
#[unsafe(no_mangle)]
pub extern "C" fn bundleExit() -> bool {
    true
}
#[unsafe(no_mangle)]
pub extern "C" fn ModuleEntry(_module: *mut c_void) -> bool {
    true
}
#[unsafe(no_mangle)]
pub extern "C" fn ModuleExit() -> bool {
    true
}
#[unsafe(no_mangle)]
pub extern "C" fn InitDll() -> bool {
    true
}
#[unsafe(no_mangle)]
pub extern "C" fn ExitDll() -> bool {
    true
}

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
const FIRST_PROCESS_DELAY_ONLY: bool = false;
const REQUIRE_OFFLINE: bool = false;
const MULTI_BUS: bool = false;
const MULTI_EFFECT: bool = false;
const NONZERO_MAIN: bool = false;
const EXPRESSIVE: bool = false;

fn logical_bus(index:i32)->i32 {if NONZERO_MAIN && index<2 {1-index} else {index}}

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
            processed: AtomicBool::new(false),
            active_inputs: AtomicU64::new(if NONZERO_MAIN {2} else {1}),
            active_outputs: AtomicU64::new(if NONZERO_MAIN {2} else {1}),
            voices: std::sync::Mutex::new(Vec::new()),
            last_velocity: AtomicU64::new(0),
            last_expression: AtomicU64::new(0),
            last_release: AtomicU64::new(0),
            last_tuning: AtomicU64::new(0),
            expressive_gain: AtomicU64::new(0.5_f64.to_bits()),
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
            subCategories: text8(if MULTI_EFFECT {"Fx"} else {"Instrument|Synth"}),
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
    processed: AtomicBool,
    active_inputs: AtomicU64,
    active_outputs: AtomicU64,
    voices: std::sync::Mutex<Vec<ExpressiveVoice>>,
    last_velocity: AtomicU64,
    last_expression: AtomicU64,
    last_release: AtomicU64,
    last_tuning: AtomicU64,
    expressive_gain: AtomicU64,
}
struct ExpressiveVoice { channel: i16, pitch: i16, id: i32, velocity: f64, expression: f64 }
impl Class for Instrument {
    type Interfaces = (IComponent, IAudioProcessor, IEditController, INoteExpressionController, IMidiMapping);
}
impl Instrument {
    unsafe fn expressive_process(&self,data:&mut ProcessData)->tresult {
        let mut voices=self.voices.lock().unwrap();
        let events=ComRef::from_raw(data.inputEvents);
        let parameters=ComRef::from_raw(data.inputParameterChanges);
        for frame in 0..data.numSamples {
            if let Some(parameters)=parameters.as_ref() {
                for index in 0..parameters.getParameterCount() {
                    if let Some(queue)=ComRef::from_raw(parameters.getParameterData(index)) {
                        if queue.getParameterId()!=GAIN {continue;}
                        for point in 0..queue.getPointCount() {
                            let(mut offset,mut value)=(0,0.0);
                            if queue.getPoint(point,&mut offset,&mut value)==kResultOk && offset==frame {self.expressive_gain.store(value.to_bits(),Relaxed);}
                        }
                    }
                }
            }
            if let Some(events)=events.as_ref() {
                for index in 0..events.getEventCount() {
                    let mut event:Event=std::mem::zeroed();
                    if events.getEvent(index,&mut event)!=kResultOk || event.sampleOffset!=frame {continue;}
                    match event.r#type as u32 {
                        Event_::EventTypes_::kNoteOnEvent => {
                            let note=event.__field0.noteOn;
                            voices.retain(|voice| if note.noteId>=0 {voice.id!=note.noteId} else {voice.channel!=note.channel || voice.pitch!=note.pitch});
                            voices.push(ExpressiveVoice{channel:note.channel,pitch:note.pitch,id:note.noteId,velocity:f64::from(note.velocity),expression:1.0});
                            self.last_velocity.store(f64::from(note.velocity).to_bits(),Relaxed);
                            self.last_tuning.store(f64::from(note.tuning).to_bits(),Relaxed);
                        }
                        Event_::EventTypes_::kNoteOffEvent => {
                            let note=event.__field0.noteOff;
                            voices.retain(|voice|if note.noteId>=0 {voice.id!=note.noteId} else {voice.channel!=note.channel || voice.pitch!=note.pitch});
                            self.last_release.store(f64::from(note.velocity).to_bits(),Relaxed);
                        }
                        Event_::EventTypes_::kNoteExpressionValueEvent => {
                            let expression=event.__field0.noteExpressionValue;
                            for voice in voices.iter_mut().filter(|voice|voice.id==expression.noteId) {voice.expression=expression.value;}
                            self.last_expression.store(expression.value.to_bits(),Relaxed);
                        }
                        Event_::EventTypes_::kPolyPressureEvent => {
                            let expression=event.__field0.polyPressure;
                            for voice in voices.iter_mut().filter(|voice|if expression.noteId>=0 {voice.id==expression.noteId} else {voice.channel==expression.channel && voice.pitch==expression.pitch}) {voice.expression=f64::from(expression.pressure);}
                            self.last_expression.store(f64::from(expression.pressure).to_bits(),Relaxed);
                        }
                        _=>{}
                    }
                }
            }
            let mut stereo=[0.0_f64;2];
            for voice in voices.iter() {stereo[(voice.channel as usize)&1]+=voice.velocity*voice.expression*f64::from_bits(self.expressive_gain.load(Relaxed));}
            for index in 0..data.numOutputs {
                let output=&mut *data.outputs.offset(index as isize);output.silenceFlags=0;
                for channel in 0..output.numChannels {
                    let samples=*output.__field0.channelBuffers32.offset(channel as isize);
                    if !samples.is_null() {*samples.offset(frame as isize)=stereo[(channel as usize)&1] as f32;}
                }
            }
        }
        kResultOk
    }
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
        if EXPRESSIVE {self.voices.lock().unwrap().clear();self.expressive_gain.store(u64::from_le_bytes(bytes),Relaxed);}
        if MULTI_BUS {
            self.active_inputs.store(if NONZERO_MAIN {2} else {1},Relaxed);
            self.active_outputs.store(if NONZERO_MAIN {2} else {1},Relaxed);
        }
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
        if MULTI_BUS && media == MediaTypes_::kAudio as i32 {
            return if direction==BusDirections_::kInput as i32 {2} else {3};
        }
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
        if index < 0 || index >= self.getBusCount(media, direction) {
            return kInvalidArgument;
        }
        let index=if media==MediaTypes_::kAudio as i32 {logical_bus(index)} else {index};
        *bus = BusInfo {
            mediaType: media,
            direction,
            channelCount: if media == MediaTypes_::kAudio as i32 {
                if MULTI_BUS {
                    if direction==BusDirections_::kInput as i32 {[2,3][index as usize]} else {[2,5,1][index as usize]}
                } else {2}
            } else {
                16
            },
            name: text16(if index==0 {"Main"} else if direction==BusDirections_::kInput as i32 {
                if f64::from_bits(self.gain.load(Relaxed)) >= 0.75 {"Restored sidechain"} else {"Sidechain"}
            } else if index==1 {"Wide outputs"} else {"Mono output"}),
            busType: if index==0 {BusTypes_::kMain as i32} else {BusTypes_::kAux as i32},
            flags: if index==0 || (NONZERO_MAIN && MULTI_EFFECT && index==1) {BusInfo_::BusFlags_::kDefaultActive as u32} else {0},
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
        media: MediaType,
        direction: BusDirection,
        index: int32,
        state: TBool,
    ) -> tresult {
        if index < 0 || index >= self.getBusCount(media,direction) {return kInvalidArgument;}
        if MULTI_BUS && media==MediaTypes_::kAudio as i32 {
            let bits=if direction==BusDirections_::kInput as i32 {&self.active_inputs} else {&self.active_outputs};
            if state!=0 {bits.fetch_or(1<<index,Relaxed);} else {bits.fetch_and(!(1<<index),Relaxed);}
        }
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
        if (MULTI_BUS && numIns==2 && numOuts==3) || (!MULTI_BUS && numIns == 0 && numOuts == 1) {
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
        if index < 0 || index >= self.getBusCount(MediaTypes_::kAudio as i32,direction) {
            return kInvalidArgument;
        }
        let index=logical_bus(index);
        let channels=if MULTI_BUS {
            if direction==BusDirections_::kInput as i32 {[2,3][index as usize]} else {[2,5,1][index as usize]}
        } else {2};
        *arrangement=match channels {1=>SpeakerArr::kMono,2=>SpeakerArr::kStereo,_=>(1u64<<channels)-1};
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
        if !FIRST_PROCESS_DELAY_ONLY || !self.processed.swap(true, Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(PROCESS_DELAY_MS));
        }
        let data = &mut *data;
        if REQUIRE_OFFLINE && data.processMode != ProcessModes_::kOffline as i32 {
            return kResultFalse;
        }
        if EXPRESSIVE {return self.expressive_process(data);}
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
                if MULTI_BUS && self.active_outputs.load(Relaxed)&(1<<bus_index)==0 {
                    bus.silenceFlags=u64::MAX;
                    continue;
                }
                bus.silenceFlags = 0;
                let output_index=logical_bus(bus_index);
                for channel in 0..bus.numChannels {
                    let samples = *bus.__field0.channelBuffers32.offset(channel as isize);
                    if samples.is_null() {continue;}
                    let sample = if MULTI_BUS {
                        let input = |index:i32,channel:i32| {
                            let index=logical_bus(index);
                            if index>=data.numInputs || self.active_inputs.load(Relaxed)&(1<<index)==0 {return 0.0;}
                            let input=&*data.inputs.offset(index as isize);
                            if channel>=input.numChannels {return 0.0;}
                            let samples=*input.__field0.channelBuffers32.offset(channel as isize);
                            if samples.is_null() {return 0.0;}
                            *samples.offset(frame as isize)
                        };
                        if self.active_outputs.load(Relaxed)&(1<<bus_index)==0 {0.0}
                        else if output_index==0 {
                            let mix=value+input(0,channel)+0.5*input(1,channel)+0.25*input(1,2);
                            if MULTI_EFFECT {mix*f64::from_bits(self.gain.load(Relaxed)) as f32} else {mix}
                        }
                        else if output_index==1 {value*(channel+2) as f32+input(0,0)*(channel+1) as f32}
                        else {value*8.0+input(1,2)}
                    } else {value};
                    *samples.offset(frame as isize) = sample;
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
        if EXPRESSIVE {5} else {1}
    }
    unsafe fn getParameterInfo(&self, index: int32, info: *mut ParameterInfo) -> tresult {
        if EXPRESSIVE && (1..=4).contains(&index) {
            *info=ParameterInfo{id:7+index as u32,title:text16(["Last velocity","Last expression","Last release","Last tuning"][(index-1) as usize]),shortTitle:text16("Telemetry"),units:text16(""),stepCount:0,defaultNormalizedValue:0.0,unitId:0,flags:ParameterInfo_::ParameterFlags_::kIsReadOnly};
            return kResultOk;
        }
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
        if EXPRESSIVE {
            let value=match _id {8=>Some(&self.last_velocity),9=>Some(&self.last_expression),10=>Some(&self.last_release),11=>Some(&self.last_tuning),_=>None};
            if let Some(value)=value {return f64::from_bits(value.load(Relaxed));}
        }
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

impl INoteExpressionControllerTrait for Instrument {
    unsafe fn getNoteExpressionCount(&self, bus: i32, channel: i16) -> i32 {
        if EXPRESSIVE && bus == 0 && (0..16).contains(&channel) { 4 } else { 0 }
    }
    unsafe fn getNoteExpressionInfo(&self, bus: i32, channel: i16, index: i32, info: *mut NoteExpressionTypeInfo) -> tresult {
        if index < 0 || index >= self.getNoteExpressionCount(bus, channel) || info.is_null() { return kInvalidArgument; }
        let (id, title) = [(2,"Tuning"),(4,"Pressure"),(201074,"Assignable 74"),(202000,"Registered 0")][index as usize];
        *info = NoteExpressionTypeInfo {
            typeId: id, title: text16(title), shortTitle: text16(title), units: text16(""), unitId: 0,
            valueDesc: NoteExpressionValueDescription { defaultValue: if id == 2 {0.5} else {1.0}, minimum: 0.0, maximum: 1.0, stepCount: 0 },
            associatedParameterId: 0, flags: NoteExpressionTypeInfo_::NoteExpressionTypeFlags_::kIsAbsolute as i32,
        };
        kResultOk
    }
    unsafe fn getNoteExpressionStringByValue(&self, bus: i32, channel: i16, id: u32, value: f64, string: *mut String128) -> tresult {
        if self.getNoteExpressionCount(bus, channel) == 0 || ![2,4,201074,202000].contains(&id) || string.is_null() || !value.is_finite() { return kInvalidArgument; }
        *string = text16(&value.to_string());
        kResultOk
    }
    unsafe fn getNoteExpressionValueByString(&self, _bus: i32, _channel: i16, _id: u32, _string: *const TChar, _value: *mut f64) -> tresult {
        kResultFalse
    }
}
impl IMidiMappingTrait for Instrument {
    unsafe fn getMidiControllerAssignment(&self, bus: i32, channel: i16, controller: CtrlNumber, id: *mut ParamID) -> tresult {
        if !EXPRESSIVE || bus != 0 || !(0..16).contains(&channel) || ![7,129].contains(&controller) || id.is_null() { return kResultFalse; }
        *id = GAIN;
        kResultOk
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

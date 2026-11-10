//! LV2 hosting (Linux only). lilv is opened at run time, so Rill has no link-time
//! dependency on it: without liblilv, LV2 bundles fail to scan or load with a clear
//! message and every other format keeps working. Only the isolated helper calls this.
//!
//! Supported: audio ports (main and `lv2:isSideChain` inputs), control ports as
//! parameters, atom MIDI input, URID map/unmap, options, the worker extension,
//! state (port values plus the state extension, through lilv), latency reporting
//! and freewheeling for offline renders. LV2 has no Rill-embedded native UI; the
//! generic parameter browser edits LV2 parameters.
use crate::PluginInfo;
use crate::native::{
    LoadRequest, LoadedInfo, NativeProcessor, RESTART_LATENCY_CHANGED,
    RESTART_PARAM_VALUES_CHANGED, Transport, normalized_from_plain, plain_from_normalized,
    plain_parameter,
};
use anyhow::{Context, Result, bail, ensure};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::ptr;
use std::sync::{Mutex, OnceLock};
use vst3_host::{
    AudioBusConfig, AudioBusLayout, BusAudioBuffers, MidiEvent,
    audio::{AudioBusDetails, AudioBusDetailsLayout, BusDirection, MediaType},
    midi::{PluginEvent, PluginEventData},
    parameters::Parameter,
    plugin::{ProcessMode, TransportPosition},
};

const LV2: &str = "http://lv2plug.in/ns/lv2core#";
const ATOM: &str = "http://lv2plug.in/ns/ext/atom#";
const URID_MAP: &str = "http://lv2plug.in/ns/ext/urid#map";
const URID_UNMAP: &str = "http://lv2plug.in/ns/ext/urid#unmap";
const WORKER_SCHEDULE: &str = "http://lv2plug.in/ns/ext/worker#schedule";
const WORKER_INTERFACE: &str = "http://lv2plug.in/ns/ext/worker#interface";
const OPTIONS: &str = "http://lv2plug.in/ns/ext/options#options";
const BOUNDED_BLOCK: &str = "http://lv2plug.in/ns/ext/buf-size#boundedBlockLength";
const MIDI_EVENT: &str = "http://lv2plug.in/ns/ext/midi#MidiEvent";
const STATE_IS_POD: u32 = 1;
const STATE_IS_PORTABLE: u32 = 2;
const ATOM_CAPACITY: usize = 64 * 1024;

/// Opaque lilv types.
type World = c_void;
type Plugin = c_void;
type Plugins = c_void;
type Port = c_void;
type Node = c_void;
type Nodes = c_void;
type State = c_void;
type Iter = c_void;

#[repr(C)]
struct Feature {
    uri: *const c_char,
    data: *mut c_void,
}

#[repr(C)]
struct Descriptor {
    uri: *const c_char,
    instantiate: *const c_void,
    connect_port: unsafe extern "C" fn(*mut c_void, u32, *mut c_void),
    activate: Option<unsafe extern "C" fn(*mut c_void)>,
    run: unsafe extern "C" fn(*mut c_void, u32),
    deactivate: Option<unsafe extern "C" fn(*mut c_void)>,
    cleanup: unsafe extern "C" fn(*mut c_void),
    extension_data: Option<unsafe extern "C" fn(*const c_char) -> *const c_void>,
}

/// `LilvInstance`, whose layout lilv documents for its inline accessors.
#[repr(C)]
struct Instance {
    descriptor: *const Descriptor,
    handle: *mut c_void,
    pimpl: *mut c_void,
}

#[repr(C)]
struct UridMap {
    handle: *mut c_void,
    map: unsafe extern "C" fn(*mut c_void, *const c_char) -> u32,
}

#[repr(C)]
struct UridUnmap {
    handle: *mut c_void,
    unmap: unsafe extern "C" fn(*mut c_void, u32) -> *const c_char,
}

type Respond = unsafe extern "C" fn(*mut c_void, u32, *const c_void) -> u32;

#[repr(C)]
struct WorkerSchedule {
    handle: *mut c_void,
    schedule_work: unsafe extern "C" fn(*mut c_void, u32, *const c_void) -> u32,
}

#[repr(C)]
struct WorkerInterface {
    work: unsafe extern "C" fn(*mut c_void, Respond, *mut c_void, u32, *const c_void) -> u32,
    work_response: unsafe extern "C" fn(*mut c_void, u32, *const c_void) -> u32,
    end_run: Option<unsafe extern "C" fn(*mut c_void) -> u32>,
}

#[repr(C)]
struct OptionsOption {
    context: u32,
    subject: u32,
    key: u32,
    size: u32,
    kind: u32,
    value: *const c_void,
}

type GetValue =
    unsafe extern "C" fn(*const c_char, *mut c_void, *mut u32, *mut u32) -> *const c_void;
type SetValue = unsafe extern "C" fn(*const c_char, *mut c_void, *const c_void, u32, u32);

macro_rules! lilv_api {
    ($($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        /// The lilv functions Rill calls, resolved from `liblilv-0.so.0` at run time.
        #[allow(non_snake_case)]
        struct Lilv {
            _library: libloading::Library,
            $($name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }

        impl Lilv {
            fn open() -> Result<Self> {
                // SAFETY: lilv's initializers have no preconditions.
                let library = unsafe { libloading::Library::new("liblilv-0.so.0") }
                    .or_else(|_| unsafe { libloading::Library::new("liblilv-0.so") })
                    .context("LV2 hosting needs lilv; install liblilv-0-0 (or lilv)")?;
                // SAFETY: each symbol is declared with lilv 0.24's C signature.
                unsafe {
                    Ok(Self {
                        $($name: *library
                            .get(concat!(stringify!($name), "\0").as_bytes())
                            .with_context(|| format!("liblilv lacks {}", stringify!($name)))?,)*
                        _library: library,
                    })
                }
            }
        }
    };
}

lilv_api! {
    lilv_world_new: fn() -> *mut World;
    lilv_world_free: fn(*mut World);
    lilv_world_load_bundle: fn(*mut World, *const Node);
    lilv_world_get_all_plugins: fn(*const World) -> *const Plugins;
    lilv_plugins_begin: fn(*const Plugins) -> *mut Iter;
    lilv_plugins_next: fn(*const Plugins, *mut Iter) -> *mut Iter;
    lilv_plugins_is_end: fn(*const Plugins, *mut Iter) -> bool;
    lilv_plugins_get: fn(*const Plugins, *mut Iter) -> *const Plugin;
    lilv_new_uri: fn(*mut World, *const c_char) -> *mut Node;
    lilv_new_file_uri: fn(*mut World, *const c_char, *const c_char) -> *mut Node;
    lilv_node_free: fn(*mut Node);
    lilv_node_as_uri: fn(*const Node) -> *const c_char;
    lilv_node_as_string: fn(*const Node) -> *const c_char;
    lilv_node_as_float: fn(*const Node) -> f32;
    lilv_node_equals: fn(*const Node, *const Node) -> bool;
    lilv_nodes_begin: fn(*const Nodes) -> *mut Iter;
    lilv_nodes_next: fn(*const Nodes, *mut Iter) -> *mut Iter;
    lilv_nodes_is_end: fn(*const Nodes, *mut Iter) -> bool;
    lilv_nodes_get: fn(*const Nodes, *mut Iter) -> *const Node;
    lilv_nodes_free: fn(*mut Nodes);
    lilv_plugin_get_uri: fn(*const Plugin) -> *const Node;
    lilv_plugin_get_name: fn(*const Plugin) -> *mut Node;
    lilv_plugin_get_author_name: fn(*const Plugin) -> *mut Node;
    lilv_plugin_get_value: fn(*const Plugin, *const Node) -> *mut Nodes;
    lilv_plugin_get_required_features: fn(*const Plugin) -> *mut Nodes;
    lilv_plugin_get_num_ports: fn(*const Plugin) -> u32;
    lilv_plugin_get_port_ranges_float: fn(*const Plugin, *mut f32, *mut f32, *mut f32);
    lilv_plugin_get_port_by_index: fn(*const Plugin, u32) -> *const Port;
    lilv_port_is_a: fn(*const Plugin, *const Port, *const Node) -> bool;
    lilv_port_has_property: fn(*const Plugin, *const Port, *const Node) -> bool;
    lilv_port_supports_event: fn(*const Plugin, *const Port, *const Node) -> bool;
    lilv_port_get: fn(*const Plugin, *const Port, *const Node) -> *mut Node;
    lilv_port_get_symbol: fn(*const Plugin, *const Port) -> *const Node;
    lilv_port_get_name: fn(*const Plugin, *const Port) -> *mut Node;
    lilv_port_get_scale_points: fn(*const Plugin, *const Port) -> *mut Nodes;
    lilv_scale_points_free: fn(*mut Nodes);
    lilv_scale_points_begin: fn(*const Nodes) -> *mut Iter;
    lilv_scale_points_next: fn(*const Nodes, *mut Iter) -> *mut Iter;
    lilv_scale_points_is_end: fn(*const Nodes, *mut Iter) -> bool;
    lilv_scale_points_get: fn(*const Nodes, *mut Iter) -> *const c_void;
    lilv_scale_point_get_label: fn(*const c_void) -> *const Node;
    lilv_scale_point_get_value: fn(*const c_void) -> *const Node;
    lilv_plugin_instantiate: fn(*const Plugin, f64, *const *const Feature) -> *mut Instance;
    lilv_instance_free: fn(*mut Instance);
    lilv_state_new_from_instance: fn(*const Plugin, *mut Instance, *mut UridMap, *const c_char, *const c_char, *const c_char, *const c_char, GetValue, *mut c_void, u32, *const *const Feature) -> *mut State;
    lilv_state_to_string: fn(*mut World, *mut UridMap, *mut UridUnmap, *const State, *const c_char, *const c_char) -> *mut c_char;
    lilv_state_new_from_string: fn(*mut World, *mut UridMap, *const c_char) -> *mut State;
    lilv_state_restore: fn(*const State, *mut Instance, SetValue, *mut c_void, u32, *const *const Feature);
    lilv_state_free: fn(*mut State);
    lilv_free: fn(*mut c_void);
}

fn lilv() -> Result<&'static Lilv> {
    static LILV: OnceLock<std::result::Result<Lilv, String>> = OnceLock::new();
    LILV.get_or_init(|| Lilv::open().map_err(|error| format!("{error:#}")))
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error}"))
}

/// Interned URIs, shared with the plug-in through the URID features.
#[derive(Default)]
struct Urids {
    ids: HashMap<CString, u32>,
    uris: Vec<CString>,
}

impl Urids {
    fn map(&mut self, uri: &CStr) -> u32 {
        if let Some(id) = self.ids.get(uri) {
            return *id;
        }
        self.uris.push(uri.to_owned());
        let id = self.uris.len() as u32;
        self.ids.insert(uri.to_owned(), id);
        id
    }
}

unsafe extern "C" fn map_uri(handle: *mut c_void, uri: *const c_char) -> u32 {
    if uri.is_null() {
        return 0;
    }
    // SAFETY: `handle` is the processor's boxed `Mutex<Urids>`; `uri` is a C string.
    let urids = unsafe { &*(handle as *const Mutex<Urids>) };
    let uri = unsafe { CStr::from_ptr(uri) };
    urids.lock().unwrap_or_else(|p| p.into_inner()).map(uri)
}

unsafe extern "C" fn unmap_uri(handle: *mut c_void, id: u32) -> *const c_char {
    // SAFETY: as in `map_uri`; interned strings live as long as the processor.
    let urids = unsafe { &*(handle as *const Mutex<Urids>) };
    let urids = urids.lock().unwrap_or_else(|p| p.into_inner());
    id.checked_sub(1)
        .and_then(|index| urids.uris.get(index as usize))
        .map_or(ptr::null(), |uri| uri.as_ptr())
}

/// Work requests queued by `schedule_work` during `run`, and their responses.
#[derive(Default)]
struct Worker {
    requests: Vec<Vec<u8>>,
    responses: Vec<Vec<u8>>,
}

const WORKER_SUCCESS: u32 = 0;
const WORKER_ERR_NO_SPACE: u32 = 2;

unsafe extern "C" fn schedule_work(handle: *mut c_void, size: u32, data: *const c_void) -> u32 {
    // SAFETY: `handle` is the processor's boxed `Worker`; calls arrive on the helper's
    // processing thread, inside `run`.
    let worker = unsafe { &mut *(handle as *mut Worker) };
    if worker.requests.len() >= 4096 || size as usize > ATOM_CAPACITY {
        return WORKER_ERR_NO_SPACE;
    }
    worker
        .requests
        .push(unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size as usize) }.to_vec());
    WORKER_SUCCESS
}

unsafe extern "C" fn respond(handle: *mut c_void, size: u32, data: *const c_void) -> u32 {
    // SAFETY: as in `schedule_work`, while the host is calling `work`.
    let worker = unsafe { &mut *(handle as *mut Worker) };
    if worker.responses.len() >= 4096 || size as usize > ATOM_CAPACITY {
        return WORKER_ERR_NO_SPACE;
    }
    worker
        .responses
        .push(unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size as usize) }.to_vec());
    WORKER_SUCCESS
}

/// Nodes for the vocabulary Rill queries, owned by one world.
struct Vocabulary {
    audio: *mut Node,
    control: *mut Node,
    atom_port: *mut Node,
    input: *mut Node,
    output: *mut Node,
    midi_event: *mut Node,
    integer: *mut Node,
    toggled: *mut Node,
    enumeration: *mut Node,
    reports_latency: *mut Node,
    side_chain: *mut Node,
    designation: *mut Node,
    latency: *mut Node,
    free_wheeling: *mut Node,
    rdf_type: *mut Node,
}

impl Vocabulary {
    fn new(lilv: &Lilv, world: *mut World) -> Self {
        let uri = |text: &str| {
            let text = CString::new(text).unwrap();
            // SAFETY: `world` is live; the node is freed in `free`.
            unsafe { (lilv.lilv_new_uri)(world, text.as_ptr()) }
        };
        Self {
            audio: uri(&format!("{LV2}AudioPort")),
            control: uri(&format!("{LV2}ControlPort")),
            atom_port: uri(&format!("{ATOM}AtomPort")),
            input: uri(&format!("{LV2}InputPort")),
            output: uri(&format!("{LV2}OutputPort")),
            midi_event: uri(MIDI_EVENT),
            integer: uri(&format!("{LV2}integer")),
            toggled: uri(&format!("{LV2}toggled")),
            enumeration: uri(&format!("{LV2}enumeration")),
            reports_latency: uri(&format!("{LV2}reportsLatency")),
            side_chain: uri(&format!("{LV2}isSideChain")),
            designation: uri(&format!("{LV2}designation")),
            latency: uri(&format!("{LV2}latency")),
            free_wheeling: uri(&format!("{LV2}freeWheeling")),
            rdf_type: uri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        }
    }

    fn free(&self, lilv: &Lilv) {
        for node in [
            self.audio,
            self.control,
            self.atom_port,
            self.input,
            self.output,
            self.midi_event,
            self.integer,
            self.toggled,
            self.enumeration,
            self.reports_latency,
            self.side_chain,
            self.designation,
            self.latency,
            self.free_wheeling,
            self.rdf_type,
        ] {
            // SAFETY: each node was created by `new` and is freed once.
            unsafe { (lilv.lilv_node_free)(node) };
        }
    }
}

/// A lilv world holding one bundle.
struct Bundle {
    lilv: &'static Lilv,
    world: *mut World,
    vocabulary: Vocabulary,
}

impl Bundle {
    fn load(path: &Path) -> Result<Self> {
        let lilv = lilv()?;
        let mut directory = path
            .canonicalize()
            .with_context(|| format!("Reading LV2 bundle {}", path.display()))?
            .to_string_lossy()
            .into_owned();
        ensure!(
            Path::new(&directory).is_dir(),
            "LV2 bundles are folders: {}",
            path.display()
        );
        // lilv requires bundle URIs to end with a slash.
        directory.push('/');
        let directory = CString::new(directory).context("Invalid LV2 bundle path")?;
        // SAFETY: lilv calls on a world this function creates and `Drop` frees.
        unsafe {
            let world = (lilv.lilv_world_new)();
            ensure!(!world.is_null(), "lilv could not create a world");
            let bundle = Self {
                lilv,
                world,
                vocabulary: Vocabulary::new(lilv, world),
            };
            let uri = (lilv.lilv_new_file_uri)(world, ptr::null(), directory.as_ptr());
            ensure!(!uri.is_null(), "Invalid LV2 bundle path");
            (lilv.lilv_world_load_bundle)(world, uri);
            (lilv.lilv_node_free)(uri);
            Ok(bundle)
        }
    }

    fn plugins(&self) -> Vec<*const Plugin> {
        let lilv = self.lilv;
        let mut found = Vec::new();
        // SAFETY: iteration over the world's live plug-in collection.
        unsafe {
            let plugins = (lilv.lilv_world_get_all_plugins)(self.world);
            let mut iter = (lilv.lilv_plugins_begin)(plugins);
            while !(lilv.lilv_plugins_is_end)(plugins, iter) {
                found.push((lilv.lilv_plugins_get)(plugins, iter));
                iter = (lilv.lilv_plugins_next)(plugins, iter);
            }
        }
        found
    }

    fn uri(&self, plugin: *const Plugin) -> String {
        // SAFETY: a plug-in of this world always has a URI.
        unsafe {
            text((self.lilv.lilv_node_as_uri)((self
                .lilv
                .lilv_plugin_get_uri)(
                plugin
            )))
        }
    }

    fn owned_text(&self, node: *mut Node) -> String {
        if node.is_null() {
            return String::new();
        }
        // SAFETY: `node` is an owned lilv node, freed here.
        unsafe {
            let value = text((self.lilv.lilv_node_as_string)(node));
            (self.lilv.lilv_node_free)(node);
            value
        }
    }

    fn info(&self, plugin: *const Plugin, path: &Path) -> PluginInfo {
        let lilv = self.lilv;
        let ports = self.ports(plugin);
        // Read rdf:type directly: classes resolve without lv2core's class hierarchy.
        let types = self.types(plugin);
        let instrument = types.iter().any(|t| t == "InstrumentPlugin")
            || (ports.iter().any(|p| p.kind == Kind::MidiIn)
                && !ports.iter().any(|p| p.kind == Kind::AudioIn));
        let has_output = ports.iter().any(|p| p.kind == Kind::AudioOut);
        let mut category = Vec::new();
        if instrument {
            category.push("Instrument".to_owned());
        } else if has_output {
            category.push("Fx".to_owned());
        }
        category.extend(
            types
                .iter()
                .filter_map(|t| t.strip_suffix("Plugin"))
                .filter(|name| !name.is_empty() && *name != "Instrument")
                .map(str::to_owned),
        );
        PluginInfo {
            // SAFETY: owned nodes returned by lilv getters.
            name: self.owned_text(unsafe { (lilv.lilv_plugin_get_name)(plugin) }),
            path: path.to_owned(),
            vendor: self.owned_text(unsafe { (lilv.lilv_plugin_get_author_name)(plugin) }),
            category: category.join("|"),
            class_id: Some(self.uri(plugin)),
            version: String::new(),
            architectures: vec![std::env::consts::ARCH.to_owned()],
            format: crate::PluginFormat::Lv2,
        }
    }

    /// Local names of the plug-in's lv2core classes, such as `InstrumentPlugin`.
    fn types(&self, plugin: *const Plugin) -> Vec<String> {
        let lilv = self.lilv;
        let mut found = Vec::new();
        // SAFETY: an owned node collection, freed here.
        unsafe {
            let nodes = (lilv.lilv_plugin_get_value)(plugin, self.vocabulary.rdf_type);
            if nodes.is_null() {
                return found;
            }
            let mut iter = (lilv.lilv_nodes_begin)(nodes);
            while !(lilv.lilv_nodes_is_end)(nodes, iter) {
                let uri = text((lilv.lilv_node_as_uri)((lilv.lilv_nodes_get)(nodes, iter)));
                if let Some(name) = uri.strip_prefix(LV2) {
                    found.push(name.to_owned());
                }
                iter = (lilv.lilv_nodes_next)(nodes, iter);
            }
            (lilv.lilv_nodes_free)(nodes);
        }
        found.sort();
        found
    }

    fn ports(&self, plugin: *const Plugin) -> Vec<PortInfo> {
        let lilv = self.lilv;
        let v = &self.vocabulary;
        // SAFETY: port queries on a live plug-in of this world; owned results are freed.
        unsafe {
            let count = (lilv.lilv_plugin_get_num_ports)(plugin) as usize;
            let (mut min, mut max, mut def) = (
                vec![f32::NAN; count],
                vec![f32::NAN; count],
                vec![f32::NAN; count],
            );
            (lilv.lilv_plugin_get_port_ranges_float)(
                plugin,
                min.as_mut_ptr(),
                max.as_mut_ptr(),
                def.as_mut_ptr(),
            );
            (0..count)
                .map(|index| {
                    let port = (lilv.lilv_plugin_get_port_by_index)(plugin, index as u32);
                    let is = |class| (lilv.lilv_port_is_a)(plugin, port, class);
                    let has = |property| (lilv.lilv_port_has_property)(plugin, port, property);
                    let input = is(v.input);
                    let designation = (lilv.lilv_port_get)(plugin, port, v.designation);
                    let designated =
                        |node| !designation.is_null() && (lilv.lilv_node_equals)(designation, node);
                    let latency = designated(v.latency) || has(v.reports_latency);
                    let free_wheeling = designated(v.free_wheeling);
                    if !designation.is_null() {
                        (lilv.lilv_node_free)(designation);
                    }
                    let kind = if is(v.audio) {
                        match (input, has(v.side_chain)) {
                            (true, false) => Kind::AudioIn,
                            (true, true) => Kind::SidechainIn,
                            (false, _) => Kind::AudioOut,
                        }
                    } else if is(v.control) {
                        if input {
                            Kind::ControlIn
                        } else {
                            Kind::ControlOut
                        }
                    } else if is(v.atom_port) {
                        if input && (lilv.lilv_port_supports_event)(plugin, port, v.midi_event) {
                            Kind::MidiIn
                        } else if input {
                            Kind::AtomIn
                        } else {
                            Kind::AtomOut
                        }
                    } else {
                        Kind::Other
                    };
                    let (mut lo, mut hi) = (min[index], max[index]);
                    if !lo.is_finite() {
                        lo = 0.0;
                    }
                    if !hi.is_finite() || hi <= lo {
                        hi = if has(v.toggled) {
                            lo + 1.0
                        } else {
                            lo.max(0.0) + 1.0
                        };
                    }
                    let default = if def[index].is_finite() {
                        def[index].clamp(lo, hi)
                    } else {
                        lo
                    };
                    let mut labels = Vec::new();
                    let points = (lilv.lilv_port_get_scale_points)(plugin, port);
                    if !points.is_null() {
                        let mut iter = (lilv.lilv_scale_points_begin)(points);
                        while !(lilv.lilv_scale_points_is_end)(points, iter) {
                            let point = (lilv.lilv_scale_points_get)(points, iter);
                            labels.push((
                                (lilv.lilv_node_as_float)((lilv.lilv_scale_point_get_value)(point)),
                                text((lilv.lilv_node_as_string)((lilv
                                    .lilv_scale_point_get_label)(
                                    point
                                ))),
                            ));
                            iter = (lilv.lilv_scale_points_next)(points, iter);
                        }
                        (lilv.lilv_scale_points_free)(points);
                    }
                    PortInfo {
                        index: index as u32,
                        kind,
                        symbol: text((lilv.lilv_node_as_string)((lilv.lilv_port_get_symbol)(
                            plugin, port,
                        ))),
                        name: self.owned_text((lilv.lilv_port_get_name)(plugin, port)),
                        min: lo,
                        max: hi,
                        default,
                        stepped: has(v.integer) || has(v.toggled) || has(v.enumeration),
                        latency,
                        free_wheeling,
                        labels,
                    }
                })
                .collect()
        }
    }

    fn required_features(&self, plugin: *const Plugin) -> Vec<String> {
        let lilv = self.lilv;
        let mut found = Vec::new();
        // SAFETY: an owned node collection, freed here.
        unsafe {
            let nodes = (lilv.lilv_plugin_get_required_features)(plugin);
            if nodes.is_null() {
                return found;
            }
            let mut iter = (lilv.lilv_nodes_begin)(nodes);
            while !(lilv.lilv_nodes_is_end)(nodes, iter) {
                found.push(text((lilv.lilv_node_as_uri)((lilv.lilv_nodes_get)(
                    nodes, iter,
                ))));
                iter = (lilv.lilv_nodes_next)(nodes, iter);
            }
            (lilv.lilv_nodes_free)(nodes);
        }
        found
    }
}

impl Drop for Bundle {
    fn drop(&mut self) {
        self.vocabulary.free(self.lilv);
        // SAFETY: the world is freed once, after its nodes.
        unsafe { (self.lilv.lilv_world_free)(self.world) };
    }
}

/// # Safety
/// `value` must be null or a NUL-terminated string.
unsafe fn text(value: *const c_char) -> String {
    if value.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    AudioIn,
    SidechainIn,
    AudioOut,
    ControlIn,
    ControlOut,
    MidiIn,
    AtomIn,
    AtomOut,
    Other,
}

struct PortInfo {
    index: u32,
    kind: Kind,
    symbol: String,
    name: String,
    min: f32,
    max: f32,
    default: f32,
    stepped: bool,
    latency: bool,
    free_wheeling: bool,
    labels: Vec<(f32, String)>,
}

/// Run on the helper only: lilv reads the bundle's Turtle, never its binary.
pub(crate) fn scan(path: &Path) -> Result<Vec<PluginInfo>> {
    let bundle = Bundle::load(path)?;
    let plugins = bundle.plugins();
    ensure!(!plugins.is_empty(), "No LV2 plug-ins in {}", path.display());
    let mut found: Vec<_> = plugins
        .into_iter()
        .map(|plugin| bundle.info(plugin, path))
        .collect();
    found.sort_by(|a, b| a.class_id.cmp(&b.class_id));
    Ok(found)
}

pub(crate) fn load(request: &LoadRequest<'_>) -> Result<Box<dyn NativeProcessor>> {
    Ok(Box::new(Lv2Processor::load(request)?))
}

const SUPPORTED_FEATURES: [&str; 5] = [
    URID_MAP,
    URID_UNMAP,
    WORKER_SCHEDULE,
    OPTIONS,
    BOUNDED_BLOCK,
];

/// Staged block events: MIDI bytes or a parameter change, at a frame offset.
#[derive(Clone, Copy)]
enum Staged {
    Midi([u8; 3], usize),
    Param(u32, f32, usize),
}

/// Heap state whose addresses the plug-in keeps: features, URIDs and options.
struct Host {
    urids: Mutex<Urids>,
    worker: Worker,
    map: UridMap,
    unmap: UridUnmap,
    schedule: WorkerSchedule,
    options: Vec<OptionsOption>,
    option_values: [i32; 3],
    sample_rate: f32,
    feature_uris: Vec<CString>,
    features: Vec<Feature>,
    feature_ptrs: Vec<*const Feature>,
}

impl Host {
    fn new(sample_rate: f64, max_block: usize) -> Box<Self> {
        let mut host = Box::new(Self {
            urids: Mutex::default(),
            worker: Worker::default(),
            map: UridMap {
                handle: ptr::null_mut(),
                map: map_uri,
            },
            unmap: UridUnmap {
                handle: ptr::null_mut(),
                unmap: unmap_uri,
            },
            schedule: WorkerSchedule {
                handle: ptr::null_mut(),
                schedule_work,
            },
            options: Vec::new(),
            option_values: [1, max_block as i32, max_block as i32],
            sample_rate: sample_rate as f32,
            feature_uris: Vec::new(),
            features: Vec::new(),
            feature_ptrs: Vec::new(),
        });
        let urids = &host.urids as *const Mutex<Urids> as *mut c_void;
        host.map.handle = urids;
        host.unmap.handle = urids;
        host.schedule.handle = &mut host.worker as *mut Worker as *mut c_void;
        let int = host.urid(&format!("{ATOM}Int"));
        let float = host.urid(&format!("{ATOM}Float"));
        let keys = [
            "http://lv2plug.in/ns/ext/buf-size#minBlockLength",
            "http://lv2plug.in/ns/ext/buf-size#maxBlockLength",
            "http://lv2plug.in/ns/ext/buf-size#nominalBlockLength",
        ]
        .map(|key| host.urid(key));
        let rate_key = host.urid("http://lv2plug.in/ns/ext/parameters#sampleRate");
        let values = host.option_values.as_ptr();
        host.options = keys
            .iter()
            .enumerate()
            .map(|(index, key)| OptionsOption {
                context: 0,
                subject: 0,
                key: *key,
                size: 4,
                kind: int,
                // SAFETY: points into the boxed host's own array.
                value: unsafe { values.add(index) }.cast(),
            })
            .collect();
        host.options.push(OptionsOption {
            context: 0,
            subject: 0,
            key: rate_key,
            size: 4,
            kind: float,
            value: (&host.sample_rate as *const f32).cast(),
        });
        host.options.push(OptionsOption {
            context: 0,
            subject: 0,
            key: 0,
            size: 0,
            kind: 0,
            value: ptr::null(),
        });
        let data = [
            (URID_MAP, &mut host.map as *mut UridMap as *mut c_void),
            (URID_UNMAP, &mut host.unmap as *mut UridUnmap as *mut c_void),
            (
                WORKER_SCHEDULE,
                &mut host.schedule as *mut WorkerSchedule as *mut c_void,
            ),
            (OPTIONS, host.options.as_mut_ptr().cast()),
            (BOUNDED_BLOCK, ptr::null_mut()),
        ];
        host.feature_uris = data
            .iter()
            .map(|(uri, _)| CString::new(*uri).unwrap())
            .collect();
        host.features = data
            .iter()
            .zip(&host.feature_uris)
            .map(|((_, data), uri)| Feature {
                uri: uri.as_ptr(),
                data: *data,
            })
            .collect();
        host.feature_ptrs = host
            .features
            .iter()
            .map(|feature| feature as *const Feature)
            .chain([ptr::null()])
            .collect();
        host
    }

    fn urid(&self, uri: &str) -> u32 {
        self.urids
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .map(&CString::new(uri).unwrap())
    }
}

/// Values lilv reads and writes by port symbol while saving/restoring state.
struct PortValues<'a> {
    symbols: &'a HashMap<String, u32>,
    values: &'a mut [f32],
    float: u32,
    double: u32,
    int: u32,
    changed: bool,
}

unsafe extern "C" fn get_port_value(
    symbol: *const c_char,
    user: *mut c_void,
    size: *mut u32,
    kind: *mut u32,
) -> *const c_void {
    // SAFETY: `user` is the `PortValues` passed to lilv for this call.
    let values = unsafe { &mut *(user as *mut PortValues<'_>) };
    let symbol = unsafe { text(symbol) };
    match values.symbols.get(&symbol) {
        Some(&index) => unsafe {
            *size = 4;
            *kind = values.float;
            (&values.values[index as usize] as *const f32).cast()
        },
        None => unsafe {
            *size = 0;
            *kind = 0;
            ptr::null()
        },
    }
}

unsafe extern "C" fn set_port_value(
    symbol: *const c_char,
    user: *mut c_void,
    value: *const c_void,
    size: u32,
    kind: u32,
) {
    // SAFETY: as in `get_port_value`; `value` holds `size` bytes of type `kind`.
    let values = unsafe { &mut *(user as *mut PortValues<'_>) };
    let symbol = unsafe { text(symbol) };
    let Some(&index) = values.symbols.get(&symbol) else {
        return;
    };
    let number = unsafe {
        match (kind, size) {
            (k, 4) if k == values.float => Some(*(value as *const f32)),
            (k, 8) if k == values.double => Some(*(value as *const f64) as f32),
            (k, 4) if k == values.int => Some(*(value as *const i32) as f32),
            _ => None,
        }
    };
    if let Some(number) = number.filter(|n| n.is_finite()) {
        values.values[index as usize] = number;
        values.changed = true;
    }
}

pub(crate) struct Lv2Processor {
    bundle: Bundle,
    plugin: *const Plugin,
    instance: *mut Instance,
    host: Box<Host>,
    info: LoadedInfo,
    ports: Vec<PortInfo>,
    symbols: HashMap<String, u32>,
    /// One value per port index; control ports are connected here once.
    values: Vec<f32>,
    audio: Vec<Vec<f32>>,
    atom_in: Vec<u64>,
    atom_out: Vec<Vec<u64>>,
    inputs: Vec<Bus>,
    outputs: Vec<Bus>,
    staged: Vec<Staged>,
    midi_event: u32,
    sequence: u32,
    chunk: u32,
    sample_rate: f64,
    max_block: usize,
    active: bool,
    latency: u32,
    restart: i32,
    transport: Transport,
}

struct Bus {
    name: &'static str,
    ports: Vec<u32>,
    active: bool,
}

impl Lv2Processor {
    fn load(request: &LoadRequest<'_>) -> Result<Self> {
        ensure!(
            request.sample_rate.is_finite() && request.sample_rate > 0.0,
            "Invalid LV2 sample rate"
        );
        ensure!(
            (1..=65_536).contains(&request.block_size),
            "Invalid LV2 block size"
        );
        let bundle = Bundle::load(request.path)?;
        let plugins = bundle.plugins();
        let plugin = match request.class_id {
            Some(uri) => plugins
                .iter()
                .copied()
                .find(|plugin| bundle.uri(*plugin) == uri)
                .with_context(|| format!("LV2 bundle has no plug-in {uri}"))?,
            None => {
                ensure!(
                    plugins.len() == 1,
                    "Choose one of this bundle's {} LV2 plug-ins",
                    plugins.len()
                );
                plugins[0]
            }
        };
        if let Some(missing) = bundle
            .required_features(plugin)
            .into_iter()
            .find(|feature| !SUPPORTED_FEATURES.contains(&feature.as_str()))
        {
            bail!("This LV2 plug-in requires an unsupported host feature: {missing}");
        }
        let catalog = bundle.info(plugin, request.path);
        let ports = bundle.ports(plugin);
        ensure!(
            !ports.iter().any(|p| p.kind == Kind::Other),
            "This LV2 plug-in has a port type Rill cannot connect"
        );
        let host = Host::new(request.sample_rate, request.block_size);
        let bus = |name, kind| Bus {
            name,
            ports: ports
                .iter()
                .filter(|p| p.kind == kind)
                .map(|p| p.index)
                .collect(),
            active: true,
        };
        let inputs: Vec<_> = [
            bus("Input", Kind::AudioIn),
            bus("Sidechain", Kind::SidechainIn),
        ]
        .into_iter()
        .filter(|bus| !bus.ports.is_empty())
        .collect();
        let outputs: Vec<_> = [bus("Output", Kind::AudioOut)]
            .into_iter()
            .filter(|bus| !bus.ports.is_empty())
            .collect();
        let has_midi = ports.iter().any(|p| p.kind == Kind::MidiIn);
        let mut processor = Self {
            info: LoadedInfo {
                name: catalog.name,
                vendor: catalog.vendor,
                version: String::new(),
                category: catalog.category,
                uid: catalog.class_id.unwrap_or_default(),
                has_gui: false,
                has_midi_input: has_midi,
                has_midi_output: false,
            },
            symbols: ports.iter().map(|p| (p.symbol.clone(), p.index)).collect(),
            values: ports.iter().map(|p| p.default).collect(),
            audio: Vec::new(),
            atom_in: vec![0; ATOM_CAPACITY / 8],
            atom_out: Vec::new(),
            inputs,
            outputs,
            staged: Vec::new(),
            midi_event: host.urid(MIDI_EVENT),
            sequence: host.urid(&format!("{ATOM}Sequence")),
            chunk: host.urid(&format!("{ATOM}Chunk")),
            sample_rate: request.sample_rate,
            max_block: request.block_size,
            active: false,
            latency: 0,
            restart: 0,
            transport: Transport::new(request.tempo, request.time_signature),
            ports,
            host,
            bundle,
            plugin,
            instance: ptr::null_mut(),
        };
        processor.instantiate()?;
        // Hosts read latency before processing starts; LV2 reports it from `run`.
        processor.start_processing()?;
        processor.stop_processing()?;
        processor.restart = 0;
        Ok(processor)
    }

    fn instantiate(&mut self) -> Result<()> {
        let lilv = self.bundle.lilv;
        self.host.option_values = [1, self.max_block as i32, self.max_block as i32];
        self.host.sample_rate = self.sample_rate as f32;
        // SAFETY: the feature array and everything it points to live in `self.host`,
        // which outlives the instance.
        let instance = unsafe {
            (lilv.lilv_plugin_instantiate)(
                self.plugin,
                self.sample_rate,
                self.host.feature_ptrs.as_ptr(),
            )
        };
        ensure!(!instance.is_null(), "The LV2 plug-in failed to instantiate");
        self.instance = instance;
        self.audio = self
            .ports
            .iter()
            .map(|p| {
                if matches!(p.kind, Kind::AudioIn | Kind::SidechainIn | Kind::AudioOut) {
                    vec![0.0; self.max_block]
                } else {
                    Vec::new()
                }
            })
            .collect();
        self.atom_out = self
            .ports
            .iter()
            .map(|p| {
                if p.kind == Kind::AtomOut {
                    vec![0; ATOM_CAPACITY / 8]
                } else {
                    Vec::new()
                }
            })
            .collect();
        for port in &self.ports {
            let index = port.index as usize;
            let data: *mut c_void = match port.kind {
                Kind::ControlIn | Kind::ControlOut => (&mut self.values[index] as *mut f32).cast(),
                Kind::AudioIn | Kind::SidechainIn | Kind::AudioOut => {
                    self.audio[index].as_mut_ptr().cast()
                }
                Kind::MidiIn | Kind::AtomIn => self.atom_in.as_mut_ptr().cast(),
                Kind::AtomOut => self.atom_out[index].as_mut_ptr().cast(),
                Kind::Other => ptr::null_mut(),
            };
            // SAFETY: each buffer lives in `self` and is never reallocated while the
            // instance exists; `instantiate` reconnects after `reconfigure` reallocates.
            unsafe { self.connect(port.index, data) };
        }
        Ok(())
    }

    fn descriptor(&self) -> &Descriptor {
        // SAFETY: lilv instances always carry their descriptor.
        unsafe { &*(*self.instance).descriptor }
    }

    fn handle(&self) -> *mut c_void {
        // SAFETY: `instance` is live for `self`'s lifetime.
        unsafe { (*self.instance).handle }
    }

    unsafe fn connect(&self, port: u32, data: *mut c_void) {
        unsafe { (self.descriptor().connect_port)(self.handle(), port, data) };
    }

    fn worker(&self) -> Option<&WorkerInterface> {
        let extension = self.descriptor().extension_data?;
        let uri = CString::new(WORKER_INTERFACE).unwrap();
        // SAFETY: the plug-in returns null or its static worker interface.
        unsafe { extension(uri.as_ptr()).cast::<WorkerInterface>().as_ref() }
    }

    fn activate(&mut self) {
        if !self.active {
            if let Some(activate) = self.descriptor().activate {
                // SAFETY: lifecycle call on the live instance, outside `run`.
                unsafe { activate(self.handle()) };
            }
            self.active = true;
        }
    }

    fn deactivate(&mut self) {
        if self.active {
            if let Some(deactivate) = self.descriptor().deactivate {
                // SAFETY: lifecycle call on the live instance, outside `run`.
                unsafe { deactivate(self.handle()) };
            }
            self.active = false;
        }
    }

    fn free_instance(&mut self) {
        self.deactivate();
        if !self.instance.is_null() {
            // SAFETY: freed once; nothing references it afterwards.
            unsafe { (self.bundle.lilv.lilv_instance_free)(self.instance) };
            self.instance = ptr::null_mut();
        }
    }

    fn control(&self, id: u32) -> Result<&PortInfo> {
        self.ports
            .get(id as usize)
            .filter(|p| p.kind == Kind::ControlIn && !p.latency && !p.free_wheeling)
            .with_context(|| format!("Unknown LV2 parameter {id}"))
    }

    fn plain(&self, id: u32, normalized: f64) -> Result<f32> {
        ensure!(normalized.is_finite(), "Invalid parameter value");
        let port = self.control(id)?;
        Ok(plain_from_normalized(
            f64::from(port.min),
            f64::from(port.max),
            port.stepped,
            normalized,
        ) as f32)
    }

    /// Fill one atom sequence with the MIDI events in `frames`, relative to its start.
    fn write_sequence(&mut self, events: &[([u8; 3], usize)], start: usize) {
        let words = &mut self.atom_in;
        words.fill(0);
        let bytes: &mut [u8] = bytemuck_bytes(words);
        let mut offset = 16;
        for (data, frame) in events {
            let length = if matches!(data[0] & 0xF0, 0xC0 | 0xD0) {
                2
            } else {
                3
            };
            if offset + 16 + 8 > bytes.len() {
                break;
            }
            bytes[offset..offset + 8].copy_from_slice(&((frame - start) as i64).to_ne_bytes());
            bytes[offset + 8..offset + 12].copy_from_slice(&(length as u32).to_ne_bytes());
            bytes[offset + 12..offset + 16].copy_from_slice(&self.midi_event.to_ne_bytes());
            bytes[offset + 16..offset + 16 + length].copy_from_slice(&data[..length]);
            offset += 16 + 8;
        }
        // LV2_Atom_Sequence: { size, type } then body { unit, pad }.
        bytes[0..4].copy_from_slice(&((offset - 8) as u32).to_ne_bytes());
        bytes[4..8].copy_from_slice(&self.sequence.to_ne_bytes());
    }

    fn run(&mut self, frames: usize) {
        for (port, buffer) in self.ports.iter().zip(&mut self.atom_out) {
            if port.kind == Kind::AtomOut {
                let bytes = bytemuck_bytes(buffer);
                bytes[0..4].copy_from_slice(&((ATOM_CAPACITY - 8) as u32).to_ne_bytes());
                bytes[4..8].copy_from_slice(&self.chunk.to_ne_bytes());
            }
        }
        // SAFETY: every port is connected to a buffer of at least `max_block` frames.
        unsafe { (self.descriptor().run)(self.handle(), frames as u32) };
        self.service_worker();
    }

    /// Requests from this block's `run` are handled now, and their responses
    /// reach the plug-in before its next block, as the worker extension requires.
    fn service_worker(&mut self) {
        if self.host.worker.requests.is_empty() {
            return;
        }
        let Some(worker) = self.worker().map(|w| (w.work, w.work_response, w.end_run)) else {
            self.host.worker.requests.clear();
            return;
        };
        let (work, work_response, end_run) = worker;
        let handle = self.handle();
        let worker_handle = &mut self.host.worker as *mut Worker as *mut c_void;
        for request in std::mem::take(&mut self.host.worker.requests) {
            // SAFETY: `work` runs outside `run`, with a respond handle that lives here.
            unsafe {
                work(
                    handle,
                    respond,
                    worker_handle,
                    request.len() as u32,
                    request.as_ptr().cast(),
                )
            };
        }
        for response in std::mem::take(&mut self.host.worker.responses) {
            // SAFETY: responses are delivered on the processing thread between runs.
            unsafe { work_response(handle, response.len() as u32, response.as_ptr().cast()) };
        }
        if let Some(end_run) = end_run {
            // SAFETY: as above.
            unsafe { end_run(handle) };
        }
    }

    fn update_latency(&mut self) {
        if let Some(port) = self
            .ports
            .iter()
            .find(|p| p.kind == Kind::ControlOut && p.latency)
        {
            let value = self.values[port.index as usize];
            let latency = if value.is_finite() {
                value.clamp(0.0, 1_048_576.0) as u32
            } else {
                0
            };
            if latency != self.latency {
                self.latency = latency;
                self.restart |= RESTART_LATENCY_CHANGED;
            }
        }
    }

    fn port_values(&mut self) -> PortValues<'_> {
        PortValues {
            float: self.host.urid(&format!("{ATOM}Float")),
            double: self.host.urid(&format!("{ATOM}Double")),
            int: self.host.urid(&format!("{ATOM}Int")),
            symbols: &self.symbols,
            values: &mut self.values,
            changed: false,
        }
    }
}

/// View a word buffer as bytes; atom buffers are u64-aligned as LV2 requires.
fn bytemuck_bytes(words: &mut [u64]) -> &mut [u8] {
    // SAFETY: u8 has no alignment or validity requirements.
    unsafe { std::slice::from_raw_parts_mut(words.as_mut_ptr().cast(), words.len() * 8) }
}

impl Drop for Lv2Processor {
    fn drop(&mut self) {
        self.free_instance();
    }
}

impl NativeProcessor for Lv2Processor {
    fn loaded_info(&self) -> LoadedInfo {
        LoadedInfo {
            name: self.info.name.clone(),
            vendor: self.info.vendor.clone(),
            version: self.info.version.clone(),
            category: self.info.category.clone(),
            uid: self.info.uid.clone(),
            has_gui: false,
            has_midi_input: self.info.has_midi_input,
            has_midi_output: false,
        }
    }

    fn start_processing(&mut self) -> Result<()> {
        self.activate();
        // A zero-length run publishes output controls such as reported latency.
        self.run(0);
        self.update_latency();
        Ok(())
    }

    fn stop_processing(&mut self) -> Result<()> {
        self.deactivate();
        Ok(())
    }

    fn is_processing(&self) -> bool {
        self.active
    }

    fn reconfigure(&mut self, sample_rate: f64, block_size: usize) -> Result<()> {
        ensure!(
            sample_rate.is_finite() && sample_rate > 0.0 && (1..=65_536).contains(&block_size),
            "Invalid LV2 processing configuration"
        );
        // The sample rate is fixed at instantiation; carry state across a new instance.
        let state = self.save_state()?;
        let active = self.active;
        self.free_instance();
        self.sample_rate = sample_rate;
        self.max_block = block_size;
        self.instantiate()?;
        self.load_state(&state)?;
        if active {
            self.start_processing()?;
        }
        Ok(())
    }

    fn set_process_mode(&mut self, mode: ProcessMode) -> Result<()> {
        if let Some(port) = self.ports.iter().find(|p| p.free_wheeling) {
            self.values[port.index as usize] = if mode == ProcessMode::Offline {
                1.0
            } else {
                0.0
            };
        }
        Ok(())
    }

    fn service_host_requests(&mut self) -> Result<i32> {
        Ok(std::mem::take(&mut self.restart))
    }

    fn audio_bus_layout(&self) -> Result<AudioBusLayout> {
        let config = |buses: &[Bus]| {
            buses
                .iter()
                .map(|bus| AudioBusConfig {
                    channel_count: bus.ports.len(),
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
                .enumerate()
                .map(|(index, bus)| AudioBusDetails {
                    name: bus.name.to_owned(),
                    channels: bus.ports.len(),
                    active: bus.active,
                    main: index == 0,
                })
                .collect()
        };
        Ok(AudioBusDetailsLayout {
            inputs: details(&self.inputs),
            outputs: details(&self.outputs),
        })
    }

    /// LV2 ports are always connected; inactive Rill buses read and produce silence.
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
            .context("Invalid LV2 audio bus index")?
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
            .ports
            .iter()
            .filter(|p| p.kind == Kind::ControlIn && !p.latency && !p.free_wheeling)
            .map(|p| {
                plain_parameter(
                    p.index,
                    if p.name.is_empty() {
                        p.symbol.clone()
                    } else {
                        p.name.clone()
                    },
                    String::new(),
                    f64::from(p.min),
                    f64::from(p.max),
                    f64::from(p.default),
                    f64::from(self.values[p.index as usize]),
                    p.stepped,
                    true,
                    false,
                    false,
                )
            })
            .collect())
    }

    fn get_parameter(&mut self, id: u32) -> Result<f64> {
        let port = self.control(id)?;
        Ok(normalized_from_plain(
            f64::from(port.min),
            f64::from(port.max),
            f64::from(self.values[id as usize]),
        ))
    }

    fn set_parameter(&mut self, id: u32, value: f64) -> Result<()> {
        self.values[id as usize] = self.plain(id, value)?;
        Ok(())
    }

    fn set_parameter_at(&mut self, id: u32, value: f64, offset: i32) -> Result<()> {
        let plain = self.plain(id, value)?;
        let offset = usize::try_from(offset).context("Negative LV2 event offset")?;
        ensure!(
            self.staged.len() < 16_384,
            "Too many LV2 events in one block"
        );
        self.staged.push(Staged::Param(id, plain, offset));
        Ok(())
    }

    fn format_parameter(&mut self, id: u32, value: f64) -> Result<String> {
        let plain = self.plain(id, value)?;
        let port = self.control(id)?;
        Ok(port
            .labels
            .iter()
            .find(|(point, _)| (point - plain).abs() < 1e-4)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| {
                if port.stepped {
                    format!("{plain:.0}")
                } else {
                    format!("{plain:.3}")
                }
            }))
    }

    fn send_midi_event_at(&mut self, event: MidiEvent, offset: i32) -> Result<()> {
        let offset = usize::try_from(offset).context("Negative LV2 event offset")?;
        if let Some(data) = crate::clap::midi_bytes(event) {
            ensure!(
                self.staged.len() < 16_384,
                "Too many LV2 events in one block"
            );
            self.staged.push(Staged::Midi(data, offset));
        }
        Ok(())
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
        let offset = usize::try_from(event.sample_offset).context("Negative LV2 event offset")?;
        self.staged.push(Staged::Midi(data, offset));
        Ok(())
    }

    fn midi_panic(&mut self) -> Result<()> {
        for channel in 0..16u8 {
            self.staged.push(Staged::Midi([0xB0 | channel, 123, 0], 0));
        }
        Ok(())
    }

    fn process_bus_audio(&mut self, buffers: &mut BusAudioBuffers) -> Result<()> {
        let frames = buffers.block_size;
        ensure!(
            frames > 0 && frames <= self.max_block,
            "Invalid LV2 block size"
        );
        ensure!(self.active, "LV2 plug-in is not processing");
        ensure!(
            buffers.inputs.len() == self.inputs.len()
                && buffers.outputs.len() == self.outputs.len(),
            "LV2 audio bus layout changed"
        );
        for (bus, ours) in buffers.inputs.iter().zip(&self.inputs) {
            ensure!(
                bus.channels.len() == ours.ports.len()
                    && bus.channels.iter().all(|c| c.len() >= frames),
                "LV2 input buffer shape differs from its bus"
            );
            for (channel, port) in bus.channels.iter().zip(&ours.ports) {
                let target = &mut self.audio[*port as usize][..frames];
                if ours.active {
                    target.copy_from_slice(&channel[..frames]);
                } else {
                    target.fill(0.0);
                }
            }
        }
        for (bus, ours) in buffers.outputs.iter().zip(&self.outputs) {
            ensure!(
                bus.channels.len() == ours.ports.len()
                    && bus.channels.iter().all(|c| c.len() >= frames),
                "LV2 output buffer shape differs from its bus"
            );
        }
        // Control ports are block-rate, so split the block at each staged parameter
        // change; MIDI keeps its frame offsets within each segment.
        let staged = std::mem::take(&mut self.staged);
        let mut changes = Vec::new();
        let mut midi = Vec::new();
        for event in staged {
            match event {
                Staged::Param(id, value, offset) => {
                    changes.push((offset.min(frames - 1), id, value))
                }
                Staged::Midi(data, offset) => midi.push((data, offset.min(frames - 1))),
            }
        }
        changes.sort_by_key(|(offset, ..)| *offset);
        midi.sort_by_key(|(_, offset)| *offset);
        let mut boundaries: Vec<usize> = changes.iter().map(|(offset, ..)| *offset).collect();
        boundaries.push(frames);
        boundaries.dedup();
        let mut start = 0;
        let mut split = false;
        let mut pending = changes.into_iter().peekable();
        let output_ports: Vec<u32> = self.outputs.iter().flat_map(|b| b.ports.clone()).collect();
        let input_ports: Vec<u32> = self.inputs.iter().flat_map(|b| b.ports.clone()).collect();
        for end in boundaries {
            while let Some((_, id, value)) = pending.next_if(|(offset, ..)| *offset <= start) {
                self.values[id as usize] = value;
            }
            if end <= start {
                continue;
            }
            let segment: Vec<_> = midi
                .iter()
                .filter(|(_, offset)| (start..end).contains(offset))
                .copied()
                .collect();
            self.write_sequence(&segment, start);
            if start > 0 {
                split = true;
                for port in input_ports.iter().chain(&output_ports) {
                    let data = self.audio[*port as usize][start..].as_mut_ptr().cast();
                    // SAFETY: the slice lives in `self.audio` for this block.
                    unsafe { self.connect(*port, data) };
                }
            }
            self.run(end - start);
            start = end;
        }
        if split {
            for port in input_ports.iter().chain(&output_ports) {
                let data = self.audio[*port as usize].as_mut_ptr().cast();
                // SAFETY: restores the block-start connection.
                unsafe { self.connect(*port, data) };
            }
        }
        for (bus, ours) in buffers.outputs.iter_mut().zip(&self.outputs) {
            for (channel, port) in bus.channels.iter_mut().zip(&ours.ports) {
                if ours.active {
                    channel[..frames].copy_from_slice(&self.audio[*port as usize][..frames]);
                } else {
                    channel[..frames].fill(0.0);
                }
            }
        }
        self.update_latency();
        self.transport.advance(frames, self.sample_rate);
        Ok(())
    }

    fn latency_samples(&self) -> u32 {
        self.latency
    }

    fn tail_samples(&self) -> u32 {
        0
    }

    fn save_state(&mut self) -> Result<Vec<u8>> {
        let lilv = self.bundle.lilv;
        let (plugin, instance, world) = (self.plugin, self.instance, self.bundle.world);
        let map = &mut self.host.map as *mut UridMap;
        let unmap = &mut self.host.unmap as *mut UridUnmap;
        let features = self.host.feature_ptrs.as_ptr();
        let mut values = self.port_values();
        let uri = CString::new("urn:rill:lv2-state").unwrap();
        // SAFETY: lilv reads port values through `values` during this call only, and the
        // instance is not running.
        unsafe {
            let state = (lilv.lilv_state_new_from_instance)(
                plugin,
                instance,
                map,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                get_port_value,
                (&mut values as *mut PortValues<'_>).cast(),
                STATE_IS_POD | STATE_IS_PORTABLE,
                features,
            );
            ensure!(!state.is_null(), "Saving LV2 state failed");
            let text =
                (lilv.lilv_state_to_string)(world, map, unmap, state, uri.as_ptr(), ptr::null());
            (lilv.lilv_state_free)(state);
            ensure!(!text.is_null(), "Serializing LV2 state failed");
            let bytes = CStr::from_ptr(text).to_bytes().to_vec();
            (lilv.lilv_free)(text.cast());
            Ok(bytes)
        }
    }

    fn load_state(&mut self, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let text = CString::new(data).context("LV2 state must be Turtle text")?;
        let lilv = self.bundle.lilv;
        let (instance, world) = (self.instance, self.bundle.world);
        let map = &mut self.host.map as *mut UridMap;
        let features = self.host.feature_ptrs.as_ptr();
        let mut values = self.port_values();
        // SAFETY: as in `save_state`; restore runs outside `run`.
        let changed = unsafe {
            let state = (lilv.lilv_state_new_from_string)(world, map, text.as_ptr());
            ensure!(!state.is_null(), "Invalid LV2 state");
            (lilv.lilv_state_restore)(
                state,
                instance,
                set_port_value,
                (&mut values as *mut PortValues<'_>).cast(),
                STATE_IS_POD | STATE_IS_PORTABLE,
                features,
            );
            (lilv.lilv_state_free)(state);
            values.changed
        };
        if changed {
            self.restart |= RESTART_PARAM_VALUES_CHANGED;
        }
        Ok(())
    }
}

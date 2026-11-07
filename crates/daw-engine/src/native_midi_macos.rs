//! Native CoreMIDI UMP input. The receive callback only borrows OS packet memory;
//! the engine copies bounded complete UMP messages into its preallocated queue.
//! A MIDI-2 protocol port may receive OS-translated MIDI-1 devices, so the source's
//! advertised protocol is reported separately from this transport's capabilities.
use anyhow::{Context, Result};
use coremidi::{Client, InputPortWithContext, Properties, PropertyGetter, Protocol, Sources};

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioConvertHostTimeToNanos(host_time: u64) -> u64;
    fn AudioGetCurrentHostTime() -> u64;
}

pub(super) struct NativePort {
    pub id: String,
    pub name: String,
    pub source_protocol: Option<u8>,
}

pub(super) fn inputs() -> Result<Vec<NativePort>> {
    let _client = Client::new("RILL UMP discovery")
        .map_err(|status| anyhow::anyhow!("Initialize CoreMIDI discovery: {status}"))?;
    Ok(Sources
        .into_iter()
        .filter_map(|source| {
            let id = source.unique_id()?.to_string();
            let name = source
                .display_name()
                .or_else(|| source.name())
                .unwrap_or_else(|| "MIDI input".into());
            let protocol: Result<i32, _> = Properties::protocol_id().value_from(source.as_ref());
            let source_protocol = protocol.ok().and_then(|value| match value {
                1 => Some(1),
                2 => Some(2),
                _ => None,
            });
            Some(NativePort {
                id,
                name,
                source_protocol,
            })
        })
        .collect())
}

pub(super) struct NativeInput {
    // Dispose the port before its client. Port disposal releases the receive block
    // and its callback captures before the owning CoreMIDI client is retired.
    _port: InputPortWithContext<()>,
    _client: Client,
    id: String,
}

impl NativeInput {
    pub(super) fn open(
        id: &str,
        mut receive: impl FnMut(u64, &[u32]) + Send + 'static,
    ) -> Result<Self> {
        let source = Sources
            .into_iter()
            .find(|source| {
                source
                    .unique_id()
                    .is_some_and(|unique| unique.to_string() == id)
            })
            .with_context(|| format!("MIDI input is unavailable: {id}"))?;
        let client = Client::new("RILL MIDI 2 input")
            .map_err(|status| anyhow::anyhow!("Initialize CoreMIDI input: {status}"))?;
        let source_protocol: Result<i32, _> = Properties::protocol_id().value_from(source.as_ref());
        let protocol = if source_protocol.ok() == Some(1) {
            Protocol::Midi10
        } else {
            Protocol::Midi20
        };
        let mut port = client
            .input_port_with_protocol("RILL UMP input", protocol, move |list, _: &mut ()| {
                for packet in list {
                    let timestamp = packet.timestamp();
                    // CoreMIDI timestamps are host ticks, not microseconds. Match
                    // the existing engine clock bridge used for MIDI-1 callbacks.
                    let host_time = if timestamp == 0 {
                        unsafe { AudioGetCurrentHostTime() }
                    } else {
                        timestamp
                    };
                    let micros = unsafe { AudioConvertHostTimeToNanos(host_time) } / 1000;
                    receive(micros, packet.data());
                }
            })
            .map_err(|status| anyhow::anyhow!("Create CoreMIDI UMP input: {status}"))?;
        port.connect_source(&source, ())
            .map_err(|status| anyhow::anyhow!("Connect CoreMIDI input {id}: {status}"))?;
        Ok(Self {
            _port: port,
            _client: client,
            id: id.to_owned(),
        })
    }

    pub(super) fn available(&self) -> bool {
        Sources.into_iter().any(|source| {
            source
                .unique_id()
                .is_some_and(|unique| unique.to_string() == self.id)
        })
    }
}

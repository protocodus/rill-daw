//! Native LV2 fixture (Linux) shared by plug-in and running-app tests.
use std::path::{Path, PathBuf};
use std::process::Command;

#[allow(dead_code)]
pub const SYNTH_URI: &str = "urn:rill:test:lv2-synth";
#[allow(dead_code)]
pub const GAIN_URI: &str = "urn:rill:test:lv2-gain";
#[allow(dead_code)]
pub const GAIN_LATENCY: u32 = 32;

pub struct Lv2ProbeFixture {
    pub path: PathBuf,
    _directory: tempfile::TempDir,
}

const MANIFEST: &str = r#"@prefix lv2: <http://lv2plug.in/ns/lv2core#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
<urn:rill:test:lv2-synth> a lv2:Plugin ; lv2:binary <rill_probe.so> ; rdfs:seeAlso <probe.ttl> .
<urn:rill:test:lv2-gain> a lv2:Plugin ; lv2:binary <rill_probe.so> ; rdfs:seeAlso <probe.ttl> .
"#;

const PLUGINS: &str = r#"@prefix lv2: <http://lv2plug.in/ns/lv2core#> .
@prefix atom: <http://lv2plug.in/ns/ext/atom#> .
@prefix doap: <http://usefulinc.com/ns/doap#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix midi: <http://lv2plug.in/ns/ext/midi#> .
@prefix urid: <http://lv2plug.in/ns/ext/urid#> .
@prefix work: <http://lv2plug.in/ns/ext/worker#> .

<urn:rill:test:lv2-synth> a lv2:Plugin, lv2:InstrumentPlugin ;
  doap:name "Rill LV2 Synth" ;
  doap:maintainer [ foaf:name "RILL tests" ] ;
  lv2:requiredFeature urid:map, work:schedule ;
  lv2:extensionData work:interface ;
  lv2:port [
    a lv2:InputPort, atom:AtomPort ; atom:bufferType atom:Sequence ; atom:supports midi:MidiEvent ;
    lv2:index 0 ; lv2:symbol "events" ; lv2:name "Events"
  ], [
    a lv2:OutputPort, lv2:AudioPort ; lv2:index 1 ; lv2:symbol "out_l" ; lv2:name "Left"
  ], [
    a lv2:OutputPort, lv2:AudioPort ; lv2:index 2 ; lv2:symbol "out_r" ; lv2:name "Right"
  ], [
    a lv2:InputPort, lv2:ControlPort ; lv2:index 3 ; lv2:symbol "level" ; lv2:name "Level" ;
    lv2:default 1.0 ; lv2:minimum 0.0 ; lv2:maximum 2.0
  ], [
    a lv2:InputPort, lv2:ControlPort ; lv2:index 4 ; lv2:symbol "mode" ; lv2:name "Mode" ;
    lv2:default 0 ; lv2:minimum 0 ; lv2:maximum 3 ;
    lv2:portProperty lv2:integer, lv2:enumeration ;
    lv2:scalePoint [ rdfs:label "Sine" ; rdf:value 0 ], [ rdfs:label "Square" ; rdf:value 1 ],
      [ rdfs:label "Saw" ; rdf:value 2 ], [ rdfs:label "Noise" ; rdf:value 3 ]
  ] .

<urn:rill:test:lv2-gain> a lv2:Plugin, lv2:AmplifierPlugin ;
  doap:name "Rill LV2 Gain" ;
  doap:maintainer [ foaf:name "RILL tests" ] ;
  lv2:port [
    a lv2:InputPort, lv2:AudioPort ; lv2:index 0 ; lv2:symbol "in_l" ; lv2:name "In L"
  ], [
    a lv2:InputPort, lv2:AudioPort ; lv2:index 1 ; lv2:symbol "in_r" ; lv2:name "In R"
  ], [
    a lv2:InputPort, lv2:AudioPort ; lv2:index 2 ; lv2:symbol "sc_l" ; lv2:name "Sidechain L" ;
    lv2:portProperty lv2:isSideChain
  ], [
    a lv2:InputPort, lv2:AudioPort ; lv2:index 3 ; lv2:symbol "sc_r" ; lv2:name "Sidechain R" ;
    lv2:portProperty lv2:isSideChain
  ], [
    a lv2:OutputPort, lv2:AudioPort ; lv2:index 4 ; lv2:symbol "out_l" ; lv2:name "Out L"
  ], [
    a lv2:OutputPort, lv2:AudioPort ; lv2:index 5 ; lv2:symbol "out_r" ; lv2:name "Out R"
  ], [
    a lv2:InputPort, lv2:ControlPort ; lv2:index 6 ; lv2:symbol "level" ; lv2:name "Level" ;
    lv2:default 1.0 ; lv2:minimum 0.0 ; lv2:maximum 2.0
  ], [
    a lv2:OutputPort, lv2:ControlPort ; lv2:index 7 ; lv2:symbol "latency" ; lv2:name "Latency" ;
    lv2:designation lv2:latency ; lv2:portProperty lv2:reportsLatency
  ] .
"#;

impl Lv2ProbeFixture {
    /// Compile `lv2_probe_plugin.rs` into `RillProbe.lv2` in a fresh directory.
    pub fn build() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = Self::build_in(directory.path());
        Self {
            path,
            _directory: directory,
        }
    }

    pub fn build_in(folder: &Path) -> PathBuf {
        let bundle = folder.join("RillProbe.lv2");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("manifest.ttl"), MANIFEST).unwrap();
        std::fs::write(
            bundle.join("probe.ttl"),
            format!("@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n{PLUGINS}"),
        )
        .unwrap();
        let source = folder.join("lv2_probe_plugin.rs");
        std::fs::write(&source, include_str!("lv2_probe_plugin.rs")).unwrap();
        let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .args([
                "--edition=2024",
                "--crate-type=cdylib",
                "--crate-name=rill_probe",
                "-C",
                "opt-level=1",
            ])
            .arg(&source)
            .arg("-o")
            .arg(bundle.join("rill_probe.so"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Native LV2 fixture did not compile:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        bundle
    }
}

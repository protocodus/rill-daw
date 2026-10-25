# Signalsmith Stretch source provenance

Unmodified Signalsmith Stretch 1.3.1 and its Signalsmith Linear FFT/STFT headers,
extracted from the published `signalsmith-stretch` Rust crate version 0.1.3.
Both upstream libraries use the MIT license; retain both adjacent license files.

- Upstream: https://github.com/Signalsmith-Audio/signalsmith-stretch
- Linear: https://github.com/Signalsmith-Audio/linear
- Crate: https://crates.io/crates/signalsmith-stretch/0.1.3
- Crate archive SHA-256: `51dae6f10b5532510f65c309c4d868babe3aecf6ce0782678081338311f176fd`
- Wrapper repository revision recorded in the crate: `c4d0cbd41351966f7fc0b18b9d83d9e381aa4e26`

Rill compiles its own narrow `src/stretch_bridge.cpp` via `cc`, avoiding the
published wrapper's build-time libclang/bindgen dependency. The bridge catches
C++ exceptions, uses deterministic random state, and exposes no plugin or GUI
functionality. DSP preparation happens away from the audio render/callback path.
No upstream algorithm source is changed.

| File | SHA-256 |
| --- | --- |
| `signalsmith-stretch.h` | `a1ad98eaaf81380723b5f39ef5389fa5aa27fa62c11c6173701431f96510a786` |
| `signalsmith-linear/fft.h` | `fb3409a88cdd4354bafe3286ef38c673f26edebd36bf0d4d002a2acecc1e6037` |
| `signalsmith-linear/stft.h` | `727e19c35ee1792bf72863f07d96d78a4b3e9669e35374ade5aa523c667988cf` |
| `LICENSE-stretch.txt` | `ee2ef82481ffb445ecdd4b3a4c1f82c0cddb2da8fe39d8e8dc384fffb3e7f06f` |
| `LICENSE-linear.txt` | `072b5e9eb5b22880bdf6256324654cf3ce53dba828edb911921121e1be09d9c3` |

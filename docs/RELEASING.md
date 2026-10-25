# Distributions and releases

The CI workflow builds native distributions after formatting, workspace unit/integration/doc tests, application UI tests, subprocess E2E scenarios, and strict Clippy pass. Each archive is then extracted and the subprocess scenarios run again against that extracted executable before upload. The app E2E suite needs no audio device, display server, or installed VST. Separate native capture tests exercise real Win32 pixels and Linux X11 pixels under Xvfb. Hardware audio, native file dialogs, and third-party plug-in compatibility need separate testing.

| Platform | Native CI runner | Archive |
| --- | --- | --- |
| macOS Apple silicon | `macos-15` | `rill-VERSION-aarch64-apple-darwin.zip` |
| macOS Intel | `macos-15-intel` | `rill-VERSION-x86_64-apple-darwin.zip` |
| Linux x86-64 | `ubuntu-22.04` | `rill-VERSION-x86_64-unknown-linux-gnu.tar.gz` |
| Linux arm64 | `ubuntu-22.04-arm` | `rill-VERSION-aarch64-unknown-linux-gnu.tar.gz` |
| Windows x86-64 | `windows-2022` | `rill-VERSION-x86_64-pc-windows-msvc.zip` |

Runner labels and CPU architectures follow [GitHub's hosted-runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners). Archives are separate native builds; macOS packages are not universal binaries. The app and any installed VST3 plug-ins must use matching architectures.

Each archive contains the application, its adjacent `daw-plugin-host` helper, `LICENSE`, `README.md`, documentation, platform installation instructions, `THIRD_PARTY_NOTICES.md`, and dependency license texts where supplied by the dependency packages. The generated inventory records the entire locked Cargo dependency graph, including platform-specific and development dependencies that may not be linked into a given binary. It records package licenses and source URLs, plus checksums for copied license files. Plug-in binaries are not included.

## Build locally

Audio stretching and transposition compile pinned, vendored Signalsmith Stretch/Linear (C++14) and Bungee Basic (C++20) sources directly through `cc`. Use a current Apple Clang on macOS, GCC 11 or newer/Clang with a C++20 standard library on Linux (`g++` in the container/CI), or the Visual Studio 2022 MSVC C++ build tools on Windows. Bungee's pinned Eigen headers and PFFFT C sources are included. No CMake, libclang, bindgen, build-time source download, or external stretch runtime is required.

Signalsmith's MIT licenses and source provenance ship in `licenses/signalsmith-stretch/`. Bungee Basic and Eigen retain MPL-2.0, with PFFFT's UCAR/NCAR license and Eigen's accompanying notices. Rill's separate source remains MIT. `scripts/native_dsp_licenses.py` includes exact covered source, notices and pinned revision/checksum records in a reproducible `licenses/bungee-basic/source.zip`, alongside readable notices. Every release archive includes these files; standalone macOS bundles include them under `Contents/Resources/licenses/` as well. Preserve these materials when redistributing binaries. Vendored source has `-text` attributes so Windows checkout does not alter the recorded source bytes.

Install Rust 1.92 or newer, Python 3, and the platform's normal Rust native build tools. Linux additionally needs `pkg-config`, `libasound2-dev`, `libudev-dev`, `libxkbcommon-dev`, and `libwayland-dev`. Run from the repository:

```sh
# Native macOS or Linux archive and checksum:
./scripts/package-release.sh

# Native Windows PowerShell archive and checksum:
./scripts/package-windows.ps1

# macOS app bundle only:
./scripts/package-macos.sh
```

The scripts build the full locked workspace in release mode. Both executable and helper are required. Packaging validates the native target, generates notices, and renders a demo WAV from the packaged executable before creating an archive in `dist/`. CI additionally tests the extracted archive. `RILL_TARGET` can assert the expected Rust target; cross-compilation is deliberately rejected by these packaging entry points.

`scripts/package-macos.sh` writes `dist/Rill.app`; the release ZIP includes that bundle and its surrounding documentation. Move the app to Applications and keep its contents intact. These builds have no Developer ID signature or notarization, so macOS may require first-launch approval in Privacy & Security.

For Linux, extract the archive and run `./rill`, keeping `daw-plugin-host` beside it. CI uses Ubuntu 22.04, giving the packaged binaries a glibc 2.35 baseline. Desktop use requires a graphical session, an OpenGL-capable driver, the C++ standard library, and ALSA, udev, xkbcommon, and Wayland client runtime libraries. On Ubuntu 22.04 install `libstdc++6 libasound2 libudev1 libxkbcommon0 libwayland-client0 libgl1`; Ubuntu 24.04 names the ALSA package `libasound2t64`. Locally built archives inherit their build machine's runtime baseline.

For Windows, extract the complete archive and launch `rill.exe`, keeping `daw-plugin-host.exe` beside it. Windows 10/11 x64 and an OpenGL-capable driver are required. CI statically links the application's MSVC C runtime; installed plug-ins may have their own runtime requirements. The application is not Authenticode signed. No installer, registry writes, or administrator access are needed to unpack the distribution.

Native VST3 editors on Linux require X11 or XWayland. Thumbnail capture also needs the XRes and Composite extensions on that display.

Verify an archive against its adjacent checksum file:

```sh
# Linux
sha256sum --check rill-VERSION-TARGET.tar.gz.sha256
# macOS
shasum -a 256 --check rill-VERSION-TARGET.zip.sha256
```

In PowerShell, compare `(Get-FileHash .\rill-VERSION-TARGET.zip -Algorithm SHA256).Hash` with the digest in the `.sha256` file. Checksums detect damaged downloads; they are not a code-signing certificate.

## CI artifacts and tagged releases

Pull requests, pushes to `main`, and manual CI runs upload one artifact per target after that target's checks pass. These artifacts expire after 14 days. Download them from the workflow run's Artifacts section. A failed target does not create a public release.

The release workflow reuses the same CI workflow. A manual release-workflow run builds and verifies artifacts only. A pushed `vVERSION` tag triggers verification across all five targets and then publication. The tag must exactly match `[workspace.package].version` in `Cargo.toml`; for example, version `0.1.0` requires tag `v0.1.0`. Version changes must also update the workspace package entries in `Cargo.lock`.

After all target jobs succeed, the release job downloads all five archives and checksum files, rejects missing or unexpected assets, verifies every checksum, and publishes through [`gh release create`](https://cli.github.com/manual/gh_release_create) with `--verify-tag` and generated notes. Tags containing a hyphen become prereleases. The workflow refuses to overwrite an existing release. Only that publishing job receives `contents: write`; ordinary CI and packaging use read-only repository access. No signing secrets or external publication service are configured.

Do not push a version tag until the intended commit and its checks have been reviewed. Creating a tag triggers publication; it does not merge a pull request.

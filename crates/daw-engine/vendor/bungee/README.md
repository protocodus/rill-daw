# Bungee Basic source provenance and distribution

Unmodified Bungee Basic sources are built directly by `daw-engine/build.rs` using
`cc`; no CMake, package-manager installation, or build-time download is needed.
Rill's separate bridge is `daw-engine/src/bungee_bridge.cpp` (MIT). Bungee and Eigen
retain MPL-2.0; their inclusion does not change the license of separate Rill files.
Eigen internal parallelism is disabled so Rill controls worker scheduling.

| Component | Exact upstream revision | Source archive SHA-256 |
| --- | --- | --- |
| Bungee Basic | `052451eff35165dae16221092447be0ce521dd24` | `9106212a5b5d3b878b663bbfd3a7af059c7f090adc6842a2ba03665f4de9e647` |
| Eigen (Bungee's pinned submodule) | `c29c800126982c561e8d0b9255dc65474cd98de3` | `b7e0c7f1291c5c551dc596d3e8c1c5bf5f997ee886a6cb17037e33d3392dc007` |
| PFFFT (Bungee's pinned submodule) | `02fe7715a5bf8bfd914681c53429600f94e0f536` | `9adeb18ac7bb52e9fb921c31c0c6a4e9ae150cc6fcb20a899d4b3a2275176ded` |

Original archives:

- https://codeload.github.com/bungee-audio-stretch/bungee/tar.gz/052451eff35165dae16221092447be0ce521dd24
- https://gitlab.com/libeigen/eigen/-/archive/c29c800126982c561e8d0b9255dc65474cd98de3/eigen-c29c800126982c561e8d0b9255dc65474cd98de3.tar.gz
- https://bitbucket.org/jpommier/pffft/get/02fe7715a5bf8bfd914681c53429600f94e0f536.tar.gz

This directory contains Bungee's `src/`, public `bungee/` headers and license;
Eigen's `Eigen/` headers and COPYING notices; and PFFFT C sources/headers.
Unneeded command-line applications, examples, and tests are not built. Cxxopts
is only a dependency of Bungee's command-line application and is not included.
`SHA256SUMS` records the exact bytes of every retained upstream file.

Licenses:

- Bungee: `LICENSE` (Mozilla Public License 2.0).
- Eigen: `eigen/COPYING.MPL2`, plus its accompanying third-party notices.
  `EIGEN_MPL2_ONLY` is set during compilation.
- PFFFT/FFTPACK: BSD-like UCAR/NCAR terms in the original source file headers;
  reproduced verbatim in `pffft/LICENSE.txt` for binary distribution.

Every Rill release and standalone macOS bundle includes this complete source
directory as `licenses/bungee-basic/source.zip`, including the covered source,
license notices, exact revisions and hashes. Recipients may use, modify and
redistribute the MPL-covered files under MPL-2.0. No proprietary Bungee Pro code
or Rubber Band implementation is included.

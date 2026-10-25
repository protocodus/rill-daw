fn main() {
    println!("cargo:rerun-if-changed=src/stretch_bridge.cpp");
    println!("cargo:rerun-if-changed=vendor/signalsmith");
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++14")
        .opt_level(2)
        .warnings(false)
        .include("vendor/signalsmith")
        .file("src/stretch_bridge.cpp");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // C++ exceptions must unwind into the checked C ABI wrappers on MSVC.
        build.flag("/EHsc");
    }
    build.compile("rill_stretch");

    println!("cargo:rerun-if-changed=src/bungee_bridge.cpp");
    println!("cargo:rerun-if-changed=vendor/bungee");
    let mut bungee = cc::Build::new();
    bungee
        .cpp(true)
        .std("c++20")
        .opt_level(2)
        .warnings(false)
        .include("vendor/bungee")
        .include("vendor/bungee/eigen")
        .include("vendor/bungee/pffft")
        .define(
            "BUNGEE_VERSION",
            "\"052451eff35165dae16221092447be0ce521dd24\"",
        )
        .define("BUNGEE_SELF_TEST", "0")
        .define("EIGEN_MPL2_ONLY", "1")
        .define("EIGEN_DONT_PARALLELIZE", "1")
        .define("eigen_assert", "BUNGEE_ASSERT1")
        .flag_if_supported("-fwrapv")
        .file("src/bungee_bridge.cpp");
    let mut sources: Vec<_> = std::fs::read_dir("vendor/bungee/src")
        .expect("Vendored Bungee sources")
        .map(|entry| entry.expect("Bungee source entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "cpp"))
        .collect();
    sources.sort();
    bungee.files(sources);
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        bungee.flag("/EHsc");
    }
    bungee.compile("rill_bungee");
    // PFFFT is C, not C++. Build it independently so the same checked-in
    // sources work with MSVC, Clang and GCC without CMake or downloaded tools.
    cc::Build::new()
        // MSVC exposes M_PI only when this is set before <math.h>. PFFFT
        // defines it internally, but FFTPACK is a separate translation unit.
        .define("_USE_MATH_DEFINES", None)
        .opt_level(2)
        .warnings(false)
        .file("vendor/bungee/pffft/pffft.c")
        .file("vendor/bungee/pffft/fftpack.c")
        .compile("rill_pffft");
}

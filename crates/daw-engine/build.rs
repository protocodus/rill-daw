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
}

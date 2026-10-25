fn main() {
    println!("cargo:rerun-if-changed=../daw-ui/assets/brand/rill.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../daw-ui/assets/brand/rill.ico")
            .set("ProductName", "RILL")
            .set("FileDescription", "RILL digital audio workstation")
            .set("InternalName", "rill")
            .set("OriginalFilename", "rill.exe")
            .compile_for("rill")
            .expect("Compile RILL's Windows application icon and version resource");
    }
}

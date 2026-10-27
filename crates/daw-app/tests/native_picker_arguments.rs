//! Compile the exact production Zenity argument helper without RFD's upstream
//! desktop-example dev dependencies. The native picker also receives UI checks.
#[cfg(target_os = "linux")]
#[path = "../../../vendor/rfd/src/backend/linux/zenity_filename.rs"]
mod zenity_filename;

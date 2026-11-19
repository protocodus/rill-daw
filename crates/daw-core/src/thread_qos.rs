//! Cooperative OS thread Quality-of-Service (QoS) hints separating real-time
//! audio workers from background UI and analysis threads.

/// Marks the calling thread as urgent interactive/real-time work so OS
/// schedulers prioritize audio rendering over background tasks.
pub fn set_realtime_audio_qos() {
    #[cfg(target_os = "macos")]
    unsafe {
        let _ =
            libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0);
    }
}

/// Marks the calling thread as background utility work so OS schedulers keep
/// heavy analysis and waveform decoding off real-time audio cores.
pub fn set_background_utility_qos() {
    #[cfg(target_os = "macos")]
    unsafe {
        let _ = libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qos_hints_succeed_on_spawned_threads() {
        std::thread::spawn(set_realtime_audio_qos)
            .join()
            .expect("realtime qos thread");
        std::thread::spawn(set_background_utility_qos)
            .join()
            .expect("utility qos thread");
    }
}

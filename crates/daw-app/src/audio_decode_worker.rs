//! One-shot codec process. Never creates a window or an audio device.
use anyhow::{Context, Result, ensure};
use std::{ffi::OsString, io::Write, path::PathBuf};

pub fn run(args: impl Iterator<Item = OsString>) -> Result<()> {
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<_> {
        let args: Vec<_> = args.take(4).collect();
        ensure!(
            args.len() == 3,
            "Usage: --decode-audio-worker input output.wav max-decoded-bytes"
        );
        let limit = args[2]
            .to_str()
            .context("Invalid decoder budget")?
            .parse::<usize>()
            .context("Invalid decoder budget")?;
        daw_core::decode_audio_to_wav(&PathBuf::from(&args[0]), &PathBuf::from(&args[1]), limit)
    }))
    .unwrap_or_else(|_| Err(anyhow::anyhow!("Audio decoder failed unexpectedly")));
    match result {
        Ok(info) => {
            serde_json::to_writer(std::io::stdout().lock(), &info)?;
            println!();
            Ok(())
        }
        Err(error) => {
            let text = format!("{error:#}");
            let bounded: String = text.chars().take(1_000).collect();
            let _ = writeln!(std::io::stderr().lock(), "{bounded}");
            std::process::exit(1);
        }
    }
}

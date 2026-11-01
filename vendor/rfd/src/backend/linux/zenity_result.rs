//! File-selection exit codes differ from Zenity message/extra-button results.
use std::process::Output;

pub(super) fn file_selection(output: Output) -> Result<Option<String>, String> {
    match output.status.code() {
        // Zenity documents 1 as Cancel or closing the dialog. Stderr may also
        // contain harmless GTK warnings, so it cannot turn Cancel into a fault.
        Some(1) => Ok(None),
        Some(0) => {
            let selected = String::from_utf8(output.stdout)
                .map_err(|error| format!("zenity returned an invalid filename: {error}"))?;
            if selected.trim().is_empty() {
                Err("zenity completed without returning a filename".into())
            } else {
                Ok(Some(selected))
            }
        }
        _ => {
            let details: String = String::from_utf8_lossy(&output.stderr)
                .trim()
                .chars()
                .take(2048)
                .collect();
            Err(if details.is_empty() {
                format!("zenity failed with {}", output.status)
            } else {
                format!("zenity failed with {}: {details}", output.status)
            })
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    fn output(code: i32, stdout: &[u8], stderr: &[u8]) -> Output {
        Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
        }
    }

    #[test]
    fn cancel_remains_cancel_even_with_gtk_warnings_or_stdout() {
        assert_eq!(
            file_selection(output(1, b"ignored", b"GTK warning")).unwrap(),
            None
        );
    }

    #[test]
    fn failure_never_becomes_a_selection_or_cancellation() {
        let error =
            file_selection(output(255, b"/tmp/not-selected.rill", b"helper failed")).unwrap_err();
        assert!(error.contains("helper failed"));
        assert!(error.contains("255"));
        assert!(file_selection(output(5, b"", b"timeout")).is_err());
        assert!(
            file_selection(Output {
                status: std::process::ExitStatus::from_raw(9),
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
            .is_err()
        );
    }

    #[test]
    fn successful_selection_requires_a_valid_nonempty_filename() {
        let path = "/tmp/音楽 with spaces.rill\n";
        assert_eq!(
            file_selection(output(0, path.as_bytes(), b"GTK warning")).unwrap(),
            Some(path.into())
        );
        assert!(file_selection(output(0, b"\n", b"")).is_err());
        assert!(file_selection(output(0, &[255], b"")).is_err());
    }
}

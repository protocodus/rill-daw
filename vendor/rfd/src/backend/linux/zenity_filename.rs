//! Zenity combines its starting directory and suggested name in one argument.
use std::{ffi::OsString, path::Path};

pub(crate) fn filename_argument(directory: Option<&Path>, name: Option<&str>) -> Option<OsString> {
    Some(match (directory, name) {
        (Some(directory), Some(name)) => directory.join(name).into_os_string(),
        (Some(directory), None) => {
            let mut directory = directory.as_os_str().to_os_string();
            // A separator distinguishes an initial folder from a filename.
            // Keep non-UTF8 paths intact.
            if !directory.as_encoded_bytes().ends_with(b"/") {
                directory.push("/");
            }
            directory
        }
        (None, Some(name)) => name.into(),
        (None, None) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_initial_directory_and_basename() {
        for (directory, name, expected) in [
            (None, None, None),
            (None, Some("Song.wav"), Some("Song.wav")),
            (
                Some("/tmp/Saved Songs"),
                Some("Song.wav"),
                Some("/tmp/Saved Songs/Song.wav"),
            ),
            (Some("/tmp/Saved Songs"), None, Some("/tmp/Saved Songs/")),
            (Some("/"), None, Some("/")),
        ] {
            assert_eq!(
                filename_argument(directory.map(Path::new), name),
                expected.map(OsString::from)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_directories() {
        use std::{os::unix::ffi::OsStringExt, path::PathBuf};
        let directory = PathBuf::from(OsString::from_vec(b"/tmp/non-utf8-\xff".to_vec()));
        assert_eq!(
            filename_argument(Some(&directory), Some("Song.mid")),
            Some(OsString::from_vec(b"/tmp/non-utf8-\xff/Song.mid".to_vec()))
        );
    }
}

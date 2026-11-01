# Rill patch to rfd 0.15.4

Source: the published crates.io `rfd` 0.15.4 package, upstream commit
`bac4bd3364f2befaabafa9f58ac77fd8925b2abf` (see `.cargo_vcs_info.json`).
Upstream project: https://github.com/PolyMeilex/rfd. Its MIT license remains in
`LICENSE`; unmodified source files retain their published contents.

The Linux Zenity fallback ignored `FileDialog::set_directory`, so save dialogs
opened in the process working directory even when Rill supplied the saved
project's directory. `src/backend/linux/zenity.rs` now uses the small
`zenity_filename.rs` helper to join the directory and basename only when
constructing Zenity's `--filename` argument. A directory
without a filename retains its trailing separator, including non-UTF8 paths.
The XDG portal still receives separate `current_folder` and `current_name`
values; macOS and Windows backends are unchanged.

The focused executable regression compiles the actual production helper and
checks basename-only, directory+basename,
directory-only, root-directory, and non-UTF8 argument construction:

```sh
cargo test --locked -p daw-app --test native_picker_arguments
```

Native validation additionally opens Rill's Save as and WAV/MIDI destination
pickers in a saved project's directory with spaces and accepts a basename-only
edit, verifying the resulting files remain beside the project. Headless picker
injection alone does not verify native backend behavior.

Rill also uses the additive `FileDialog::pick_file_result` and
`FileDialog::save_file_result` APIs. On the Linux/BSD portal backend they retain
`Result<Option<PathBuf>, FileDialogError>`: explicit portal cancellation or
Zenity exit 1 is `Ok(None)`; a missing or failing helper, invalid response, or
other portal failure is an error. When the portal cannot open and the fallback
also fails, the error retains both causes and tells the user how to restore the
desktop picker. The legacy `Option` APIs remain available. Other native
backends keep their existing behavior; this patch does not claim new error
classification for GTK, macOS, or Windows.

Zenity file-selection exits are parsed independently of its message dialogs:
exit 0 requires a valid nonempty filename, exit 1 is cancellation, and other
statuses or signals are failures, even if stdout contains a path. Stderr alone
never turns Cancel into an error because GTK can emit warnings during normal
use. A backend that exits with the documented cancellation code for a different
reason cannot be distinguished reliably by this API.

The app reports these failures in its existing status/details UI and retains
them in the unsaved-document guard without changing the document. Regression
tests exercise the production app menus and native adapter with only the child
process's PATH and session-bus address isolated. They cover missing helpers,
failed helpers with misleading stdout, cancellation, successful saves/exports
after a helper is restored while the app stays open, and guarded Save retries.
The scripted helper tests do not claim visual validation of a desktop chooser.

```sh
cargo test --locked -p daw-app --test native_picker_results --test native_picker_failure_e2e
```

The portal cancellation classifier also has a vendor unit test in
`backend::xdg_desktop_portal::result_tests`.

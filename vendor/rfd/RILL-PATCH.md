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

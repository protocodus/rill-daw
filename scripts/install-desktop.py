#!/usr/bin/env python3
"""Install an optional launcher for an extracted Linux RILL distribution."""
import argparse
import os
from pathlib import Path
import shutil


def quoted_exec(path):
    # Desktop Entry string unescaping precedes Exec argument unquoting. Literal
    # percent signs need escaping independently of shell-style quoting.
    value = str(path).replace("%", "%%")
    if any(character in value for character in "\n\r\0="):
        raise ValueError("The application path cannot contain newline, NUL, or '='")
    value = "".join("\\" + c if c in '\\"`$' else c for c in value)
    return '"' + value.replace("\\", "\\\\") + '"'


def install(package, data_home, remove=False):
    desktop = data_home / "applications/dev.rill.daw.desktop"
    icon = data_home / "icons/hicolor/256x256/apps/dev.rill.daw.png"
    if remove:
        desktop.unlink(missing_ok=True)
        icon.unlink(missing_ok=True)
        return desktop
    executable = package / "rill"
    source_icon = package / "share/icons/hicolor/256x256/apps/dev.rill.daw.png"
    template = package / "share/applications/dev.rill.daw.desktop"
    if not executable.is_file() or not os.access(executable, os.X_OK):
        raise ValueError("Run this script from a complete, extracted RILL distribution")
    content = template.read_text(encoding="utf-8")
    content = content.replace("Exec=rill %f", f"Exec={quoted_exec(executable)} %f")
    if not source_icon.is_file():
        raise ValueError("The distribution is missing its application icon")
    desktop.parent.mkdir(parents=True, exist_ok=True)
    icon.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source_icon, icon)
    desktop.write_text(content, encoding="utf-8")
    desktop.chmod(0o644)
    return desktop


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--remove", action="store_true", help="Remove the user launcher and icon")
    args = parser.parse_args()
    data_home = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
    if not data_home.is_absolute():
        parser.error("XDG_DATA_HOME must be an absolute path")
    try:
        desktop = install(Path(__file__).resolve().parent, data_home, args.remove)
    except (OSError, ValueError) as error:
        parser.exit(1, f"RILL launcher: {error}\n")
    print(f"{'Removed' if args.remove else 'Installed'} {desktop}")


if __name__ == "__main__":
    main()

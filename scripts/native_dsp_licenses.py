#!/usr/bin/env python3
"""Include MPL-covered native DSP source and notices with distributed binaries."""

import hashlib
from pathlib import Path
import shutil
import sys
import zipfile


def collect_bungee(destination):
    source = Path(__file__).resolve().parents[1] / "crates/daw-engine/vendor/bungee"
    target = Path(destination) / "licenses/bungee-basic"
    target.mkdir(parents=True, exist_ok=True)
    notices = [source / name for name in ("LICENSE", "README.md", "SHA256SUMS", "pffft/LICENSE.txt")]
    notices.extend(sorted((source / "eigen").glob("COPYING*")))
    for path in notices:
        notice = target / path.relative_to(source)
        notice.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, notice)
    # Bundle exact covered source instead of depending on future availability of
    # a third-party repository. Fixed timestamps keep this archive reproducible.
    archive = target / "source.zip"
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as output:
        for path in sorted(source.rglob("*")):
            if not path.is_file():
                continue
            info = zipfile.ZipInfo(path.relative_to(source).as_posix(), (2020, 1, 1, 0, 0, 0))
            info.create_system = 3  # Identical Unix metadata, including on Windows.
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            output.writestr(info, path.read_bytes())
    return [
        {"file": str(path.relative_to(destination)), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
        for path in sorted(target.rglob("*")) if path.is_file()
    ]


if __name__ == "__main__":
    collect_bungee(Path(sys.argv[1]))

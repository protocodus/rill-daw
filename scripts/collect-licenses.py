#!/usr/bin/env python3
"""Bundle resolved Cargo package license metadata and available license notices."""

import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
from native_dsp_licenses import collect_bungee


def main():
    destination = Path(sys.argv[1])
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1"], text=True
    ))
    workspace = set(metadata["workspace_members"])
    notices = destination / "licenses"
    notices.mkdir(parents=True, exist_ok=True)
    rows = []
    inventory = []
    # Fonts embedded by the workspace are not Cargo dependencies. Include their
    # source/attribution and OFL notices in every platform's release archive.
    font_source = Path(__file__).resolve().parents[1] / "crates/daw-ui/assets/fonts"
    font_notices = notices / "rill-fonts"
    font_notices.mkdir(parents=True, exist_ok=True)
    for path in sorted(font_source.iterdir()):
        if path.suffix.lower() in {".txt", ".md"}:
            shutil.copyfile(path, font_notices / path.name)
    stretch_source = Path(__file__).resolve().parents[1] / "crates/daw-engine/vendor/signalsmith"
    stretch_notices = notices / "signalsmith-stretch"
    stretch_notices.mkdir(parents=True, exist_ok=True)
    copied_stretch = []
    for name in ("LICENSE-stretch.txt", "LICENSE-linear.txt", "README.md"):
        target = stretch_notices / name
        shutil.copyfile(stretch_source / name, target)
        copied_stretch.append({"file": str(target.relative_to(destination)), "sha256": hashlib.sha256(target.read_bytes()).hexdigest()})
    stretch_repository = "https://github.com/Signalsmith-Audio/signalsmith-stretch"
    rows.append(f'| Signalsmith Stretch 1.3.1 + Linear (vendored) | MIT | {stretch_repository} |')
    inventory.append({"name": "signalsmith-stretch-and-linear", "version": "1.3.1-vendored", "license": "MIT", "repository": stretch_repository, "notices": copied_stretch})
    bungee_notices = collect_bungee(destination)
    for name, version, license_name, repository in (
        ("Bungee Basic", "052451eff35165dae16221092447be0ce521dd24", "MPL-2.0", "https://github.com/bungee-audio-stretch/bungee"),
        ("Eigen", "c29c800126982c561e8d0b9255dc65474cd98de3", "MPL-2.0 and accompanying notices", "https://gitlab.com/libeigen/eigen"),
        ("PFFFT", "02fe7715a5bf8bfd914681c53429600f94e0f536", "BSD-like UCAR/NCAR", "https://bitbucket.org/jpommier/pffft"),
    ):
        rows.append(f'| {name} {version} (vendored) | {license_name} | {repository} |')
        inventory.append({"name": name, "version": version, "license": license_name, "repository": repository, "notices": bungee_notices})
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["id"] in workspace:
            continue
        source = Path(package["manifest_path"]).parent
        package_name = f'{package["name"]}-{package["version"]}'
        files = set()
        if package.get("license_file"):
            files.add(source / package["license_file"])
        # Some dependencies embed fonts or vendored code with their own notices.
        # Keep relative paths so nested files with the same basename do not collide.
        for path in source.rglob("*"):
            name = path.name.lower()
            if (name.startswith(("license", "licence", "copying", "notice"))
                    or "license" in name or "licence" in name
                    or name in {"ofl.txt", "ufl.txt", "hack-regular.txt"}):
                files.add(path)
        copied = []
        for path in sorted(files):
            if not path.is_file():
                continue
            try:
                relative = path.relative_to(source)
            except ValueError:
                relative = Path(path.name)
            target = notices / package_name / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
            copied.append({"file": str(target.relative_to(destination)), "sha256": hashlib.sha256(target.read_bytes()).hexdigest()})
        license_name = package.get("license") or "See package license file/source"
        repository = package.get("repository") or f'https://crates.io/crates/{package["name"]}/{package["version"]}'
        rows.append(f'| {package_name} | {license_name} | {repository} |')
        inventory.append({"name": package["name"], "version": package["version"], "license": package.get("license"), "repository": repository, "notices": copied})
    (destination / "THIRD_PARTY_NOTICES.md").write_text(
        "# Third-party dependencies\n\n"
        "This inventory records the locked Cargo dependency graph, including platform and development dependencies that may not be linked into this archive. "
        "Available license and notice files from downloaded package sources are included under `licenses/`. "
        "Embedded Inter and Noto Sans CJK fonts use SIL OFL 1.1; their licenses and source attribution are in `licenses/rill-fonts/`. "
        "Vendored Signalsmith Stretch and Linear use MIT; both licenses and source provenance are in `licenses/signalsmith-stretch/`. "
        "Bungee Basic and Eigen retain MPL-2.0, PFFFT retains its UCAR/NCAR license, and their exact source and notices are bundled in `licenses/bungee-basic/source.zip`. "
        "For packages without bundled license text, consult the linked package source. VST3 plug-ins installed separately are not included.\n\n"
        "| Package | Declared license | Source |\n| --- | --- | --- |\n" + "\n".join(rows) + "\n",
        encoding="utf-8",
    )
    (destination / "third-party-licenses.json").write_text(json.dumps(inventory, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()

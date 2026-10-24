#!/usr/bin/env python3
"""Bundle resolved Cargo package license metadata and available license notices."""

import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys


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
        "# Third-party Rust dependencies\n\n"
        "This inventory records the locked Cargo dependency graph, including platform and development dependencies that may not be linked into this archive. "
        "Available license and notice files from downloaded package sources are included under `licenses/`. "
        "For packages without bundled license text, consult the linked package source. VST3 plug-ins installed separately are not included.\n\n"
        "| Package | Declared license | Source |\n| --- | --- | --- |\n" + "\n".join(rows) + "\n",
        encoding="utf-8",
    )
    (destination / "third-party-licenses.json").write_text(json.dumps(inventory, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()

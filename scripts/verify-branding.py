#!/usr/bin/env python3
"""Verify that distribution metadata and native icon resources match RILL artwork."""
import argparse
from pathlib import Path
import plistlib
import struct


BRAND = Path(__file__).resolve().parents[1] / "crates/daw-ui/assets/brand"


def png_size(data):
    assert data[:8] == b"\x89PNG\r\n\x1a\n" and data[12:16] == b"IHDR", "Invalid PNG icon"
    return struct.unpack_from(">II", data, 16)


def ico_images(data):
    reserved, kind, count = struct.unpack_from("<HHH", data)
    assert reserved == 0 and kind == 1 and 1 <= count <= 32, "Invalid ICO header"
    images = []
    sizes = set()
    for index in range(count):
        width, height, _, _, _, _, length, offset = struct.unpack_from("<BBBBHHII", data, 6 + 16 * index)
        assert offset >= 6 + 16 * count and offset + length <= len(data), "Invalid ICO image bounds"
        sizes.add((width or 256, height or 256))
        images.append(data[offset:offset + length])
    assert {(16, 16), (32, 32), (48, 48), (256, 256)} <= sizes, "Missing small or high-DPI ICO sizes"
    return images


def icns_images(data):
    assert data[:4] == b"icns" and struct.unpack_from(">I", data, 4)[0] == len(data), "Invalid ICNS header"
    offset = 8
    images = {}
    while offset < len(data):
        kind = data[offset:offset + 4]
        size = struct.unpack_from(">I", data, offset + 4)[0]
        assert size >= 8 and offset + size <= len(data), "Invalid ICNS image bounds"
        images[kind] = data[offset + 8:offset + size]
        offset += size
    assert b"ic10" in images, "Missing 1024-pixel Retina icon"
    assert png_size(images[b"ic10"]) == (1024, 1024), "Wrong Retina icon dimensions"
    return images


def pe_icon_images(data):
    """Read RT_ICON/RT_GROUP_ICON from a PE, without executing a Windows binary."""
    assert data[:2] == b"MZ", "Not a Windows executable"
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    assert data[pe:pe + 4] == b"PE\0\0", "Missing PE header"
    sections, optional_size = struct.unpack_from("<H", data, pe + 6)[0], struct.unpack_from("<H", data, pe + 20)[0]
    optional = pe + 24
    magic = struct.unpack_from("<H", data, optional)[0]
    assert magic in (0x10B, 0x20B), "Unsupported PE optional header"
    directories = optional + (112 if magic == 0x20B else 96)
    resource_rva, resource_size = struct.unpack_from("<II", data, directories + 16)
    assert resource_rva and resource_size, "Windows executable has no resources"
    table = optional + optional_size

    def file_offset(rva, length):
        for index in range(sections):
            _, address, raw_size, raw_pointer = struct.unpack_from("<IIII", data, table + 40 * index + 8)
            if address <= rva and rva + length <= address + raw_size:
                offset = raw_pointer + rva - address
                assert offset + length <= len(data), "Resource exceeds executable size"
                return offset
        raise AssertionError("Resource RVA is outside PE sections")

    root = file_offset(resource_rva, resource_size)

    def entries(relative):
        assert relative + 16 <= resource_size, "Invalid resource directory"
        named, numbered = struct.unpack_from("<HH", data, root + relative + 12)
        assert relative + 16 + (named + numbered) * 8 <= resource_size, "Invalid resource entries"
        return [struct.unpack_from("<II", data, root + relative + 16 + 8 * index) for index in range(named + numbered)]

    def payloads(entry, depth=0):
        offset = entry & 0x7FFFFFFF
        assert depth <= 3, "Resource directory is too deep"
        if entry & 0x80000000:
            return [blob for _, child in entries(offset) for blob in payloads(child, depth + 1)]
        assert offset + 16 <= resource_size, "Invalid resource data entry"
        rva, size = struct.unpack_from("<II", data, root + offset)
        start = file_offset(rva, size)
        return [data[start:start + size]]

    kinds = dict(entries(0))
    assert 3 in kinds and 14 in kinds, "Missing embedded application icon/group"
    groups = dict(entries(kinds[14] & 0x7FFFFFFF))
    icons = dict(entries(kinds[3] & 0x7FFFFFFF))
    assert 1 in groups, "Missing the default application icon (resource 1)"
    result = []
    for group in payloads(groups[1]):
        reserved, kind, count = struct.unpack_from("<HHH", group)
        assert reserved == 0 and kind == 1 and 1 <= count <= 32, "Invalid icon group"
        assert len(group) >= 6 + 14 * count, "Truncated icon group"
        for index in range(count):
            resource = struct.unpack_from("<H", group, 6 + 14 * index + 12)[0]
            assert resource in icons, "Icon group references missing pixels"
            result.extend(payloads(icons[resource]))
    return result


def verify_macos(bundle):
    content = bundle / "Contents"
    with (content / "Info.plist").open("rb") as source:
        info = plistlib.load(source)
    assert info["CFBundleIconFile"] == "rill.icns", "Bundle does not select the RILL icon"
    assert info["CFBundleIdentifier"] == "dev.rill.daw", "Incorrect application identity"
    assert info.get("NSMicrophoneUsageDescription", "").strip(), "Missing microphone permission explanation"
    icon = (content / "Resources/rill.icns").read_bytes()
    icns_images(icon)
    assert icon == (BRAND / "rill.icns").read_bytes(), "Packaged ICNS differs from the canonical artwork"


def verify_linux(package):
    icon = package / "share/icons/hicolor/256x256/apps/dev.rill.daw.png"
    assert png_size(icon.read_bytes()) == (256, 256), "Wrong Linux theme icon dimensions"
    assert icon.read_bytes() == (BRAND / "rill-icon-256.png").read_bytes(), "Linux icon differs from canonical artwork"
    desktop = (package / "share/applications/dev.rill.daw.desktop").read_text(encoding="utf-8")
    for line in ("Name=RILL", "Exec=rill %f", "Icon=dev.rill.daw", "StartupWMClass=dev.rill.daw"):
        assert line in desktop.splitlines(), f"Missing desktop entry key: {line}"
    assert (package / "install-desktop.py").is_file(), "Missing portable launcher installer"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--macos", type=Path)
    group.add_argument("--windows", type=Path)
    group.add_argument("--linux", type=Path)
    args = parser.parse_args()
    if args.macos:
        verify_macos(args.macos)
    elif args.windows:
        expected = ico_images((BRAND / "rill.ico").read_bytes())
        actual = pe_icon_images(args.windows.read_bytes())
        assert set(expected) <= set(actual), "Executable icon pixels differ from the RILL ICO"
    else:
        verify_linux(args.linux)
    print("RILL native branding verified")


if __name__ == "__main__":
    main()

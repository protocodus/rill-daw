#!/usr/bin/env python3
"""Exercise launcher installation in an isolated directory, without host changes."""
import importlib.util
from pathlib import Path
import tempfile
import unittest


spec = importlib.util.spec_from_file_location("installer", Path(__file__).with_name("install-desktop.py"))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class DesktopInstallTests(unittest.TestCase):
    def test_quoted_portable_location_install_update_and_remove(self):
        with tempfile.TemporaryDirectory() as directory:
            package = Path(directory) / 'RILL music \\"$`% á'
            package.mkdir()
            executable = package / "rill"
            executable.write_bytes(b"test executable")
            executable.chmod(0o755)
            icon = package / "share/icons/hicolor/256x256/apps/dev.rill.daw.png"
            icon.parent.mkdir(parents=True)
            icon.write_bytes(b"test icon")
            template = package / "share/applications/dev.rill.daw.desktop"
            template.parent.mkdir(parents=True)
            template.write_text(Path(__file__).with_name("dev.rill.daw.desktop").read_text(), encoding="utf-8")
            target = Path(directory) / "user data"
            desktop = installer.install(package, target)
            content = desktop.read_text(encoding="utf-8")
            self.assertIn(f"Exec={installer.quoted_exec(executable)} %f\n", content)
            self.assertIn("Icon=dev.rill.daw\n", content)
            installed_icon = target / "icons/hicolor/256x256/apps/dev.rill.daw.png"
            self.assertEqual(installed_icon.read_bytes(), icon.read_bytes())
            icon.write_bytes(b"updated icon")
            installer.install(package, target)
            self.assertEqual(installed_icon.read_bytes(), b"updated icon")
            unrelated = target / "keep.txt"
            unrelated.write_text("retain")
            installer.install(package, target, remove=True)
            self.assertFalse(desktop.exists())
            self.assertFalse(installed_icon.exists())
            self.assertEqual(unrelated.read_text(), "retain")

    def test_exec_escaping_and_invalid_paths(self):
        self.assertEqual(installer.quoted_exec('/opt/RILL music/rill'), '"/opt/RILL music/rill"')
        self.assertEqual(installer.quoted_exec('/opt/100%/rill'), '"/opt/100%%/rill"')
        self.assertEqual(installer.quoted_exec('/opt/$cash/rill'), '"/opt/\\\\$cash/rill"')
        self.assertEqual(installer.quoted_exec('/opt/\\folder/rill'), '"/opt/\\\\\\\\folder/rill"')
        for path in ('/opt/a=b/rill', '/opt/line\nbreak/rill'):
            with self.assertRaises(ValueError):
                installer.quoted_exec(path)


if __name__ == "__main__":
    unittest.main()

import tempfile
from pathlib import Path
import unittest

from update_manifest import build_manifest


class UpdateManifestTests(unittest.TestCase):
    def test_linux_downloads_stay_in_their_format(self):
        with tempfile.TemporaryDirectory() as directory:
            assets = Path(directory)
            filenames = {
                "appimage": "LowNotes-0.2.7-linux-x64.AppImage",
                "deb": "LowNotes-0.2.7-linux-x64.deb",
                "rpm": "LowNotes-0.2.7-linux-x64.rpm",
                "arch": "LowNotes-0.2.7-linux-x86_64.pkg.tar.zst",
                "portable": "LowNotes-0.2.7-linux-x64-portable.tar.gz",
            }
            for filename in filenames.values():
                (assets / filename).touch()
            for signed_format in ["appimage", "deb", "rpm"]:
                (assets / f"{filenames[signed_format]}.sig").write_text("signature")
            doc = build_manifest("0.2.7", "LowBloat/LowNotes", assets, "Notes")
            self.assertEqual(sorted(doc["platforms"]),
                ["linux-x86_64-appimage", "linux-x86_64-deb", "linux-x86_64-rpm"])
            self.assertNotIn("linux-x86_64", doc["platforms"])
            for signed_format in ["appimage", "deb", "rpm"]:
                self.assertEqual(doc["platforms"][f"linux-x86_64-{signed_format}"]["url"],
                    f"https://github.com/LowBloat/LowNotes/releases/download/v0.2.7/{filenames[signed_format]}")
            for package_format, filename in filenames.items():
                self.assertEqual(doc["downloads"][f"linux-x86_64-{package_format}"],
                    f"https://github.com/LowBloat/LowNotes/releases/download/v0.2.7/{filename}")

    def test_missing_or_unsigned_assets_are_not_advertised_as_installable(self):
        with tempfile.TemporaryDirectory() as directory:
            assets = Path(directory)
            (assets / "LowNotes-0.2.7-linux-x64.AppImage").touch()
            (assets / "LowNotes-0.2.7-windows-x64-setup.exe.sig").write_text("signature")
            doc = build_manifest("0.2.7", "LowBloat/LowNotes", assets, "Notes")
            self.assertEqual(doc["platforms"], {})
            self.assertEqual(list(doc["downloads"]), ["linux-x86_64-appimage"])

    def test_windows_and_macos_updater_targets_are_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            assets = Path(directory)
            for filename in ["LowNotes-0.2.7-windows-x64-setup.exe",
                "LowNotes-0.2.7-macos-universal.app.tar.gz"]:
                (assets / filename).touch()
                (assets / f"{filename}.sig").write_text("signature")
            doc = build_manifest("0.2.7", "LowBloat/LowNotes", assets, "Notes")
            self.assertEqual(set(doc["platforms"]),
                {"windows-x86_64", "darwin-aarch64", "darwin-x86_64"})


if __name__ == "__main__":
    unittest.main()

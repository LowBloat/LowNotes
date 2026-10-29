import hashlib
import tempfile
import unittest
from pathlib import Path

from render_pkgbuild import render

ROOT = Path(__file__).resolve().parents[2]


class RenderPkgbuildTests(unittest.TestCase):
    def test_renders_version_and_real_checksums(self):
        with tempfile.TemporaryDirectory() as directory:
            assets = Path(directory)
            portable = assets / "LowNotes-1.2.3-linux-x64-portable.tar.gz"
            portable.write_bytes(b"portable archive")
            text = render("1.2.3", assets, ROOT)
            self.assertIn("pkgver=1.2.3", text)
            self.assertIn(hashlib.sha256(b"portable archive").hexdigest(), text)
            self.assertNotIn("SKIP", text)
            self.assertNotIn("__", text.split("package()")[0])
            self.assertIn('"${pkgdir}/usr/bin/lownotes-mcp"', text)
            self.assertNotIn("\r", text)

    def test_missing_portable_archive_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(FileNotFoundError):
                render("1.2.3", Path(directory), ROOT)


if __name__ == "__main__":
    unittest.main()

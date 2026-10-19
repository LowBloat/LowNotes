"""Render the AUR PKGBUILD template with the release version and real checksums.

Usage: python render_pkgbuild.py VERSION ASSETS_DIR [REPO_ROOT]
Writes ASSETS_DIR/PKGBUILD so the release publishes a verifiable recipe.
"""

import hashlib
import sys
from pathlib import Path


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def render(version, assets, root):
    template = (root / "packaging/arch/PKGBUILD").read_text(encoding="utf-8").replace("\r\n", "\n")
    portable = assets / f"LowNotes-{version}-linux-x64-portable.tar.gz"
    values = {
        "__VERSION__": version,
        "__SHA256_PORTABLE__": sha256(portable),
        "__SHA256_DESKTOP__": sha256(root / "packaging/arch/lownotes.desktop"),
        "__SHA256_ICON__": sha256(root / "src-tauri/icons/128x128.png"),
    }
    for placeholder, value in values.items():
        if placeholder not in template:
            raise ValueError(f"PKGBUILD template lacks {placeholder}")
        template = template.replace(placeholder, value)
    if "SKIP" in template or "__" in template.split("package()")[0]:
        raise ValueError("PKGBUILD still contains unchecked sources")
    return template


if __name__ == "__main__":
    version, assets = sys.argv[1], Path(sys.argv[2])
    root = Path(sys.argv[3]) if len(sys.argv) > 3 else Path(__file__).resolve().parents[2]
    output = assets / "PKGBUILD"
    output.write_text(render(version, assets, root), encoding="utf-8", newline="\n")
    print(f"Rendered {output}")

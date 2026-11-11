"""Generate updater metadata without mixing Linux installation formats."""

import datetime
import json
import os
from pathlib import Path
import subprocess


def build_manifest(version, repo, assets, notes):
    base = f"https://github.com/{repo}/releases/download/v{version}"
    platforms = {}
    downloads = {}

    def signed(key, asset):
        signature = assets / f"{asset}.sig"
        if (assets / asset).is_file() and signature.is_file():
            platforms[key] = {"signature": signature.read_text().strip(), "url": f"{base}/{asset}"}

    signed("windows-x86_64", f"LowNotes-{version}-windows-x64-setup.exe")
    signed("darwin-aarch64", f"LowNotes-{version}-macos-universal.app.tar.gz")
    signed("darwin-x86_64", f"LowNotes-{version}-macos-universal.app.tar.gz")
    # One key per Linux format, never a generic one: a .deb install must get a
    # .deb, an .rpm install an .rpm, and the Arch package nothing (pacman owns it).
    signed("linux-x86_64-appimage", f"LowNotes-{version}-linux-x64.AppImage")
    signed("linux-x86_64-deb", f"LowNotes-{version}-linux-x64.deb")
    signed("linux-x86_64-rpm", f"LowNotes-{version}-linux-x64.rpm")
    for package_format, asset in {
        "appimage": f"LowNotes-{version}-linux-x64.AppImage",
        "deb": f"LowNotes-{version}-linux-x64.deb",
        "rpm": f"LowNotes-{version}-linux-x64.rpm",
        "arch": f"LowNotes-{version}-linux-x86_64.pkg.tar.zst",
        "portable": f"LowNotes-{version}-linux-x64-portable.tar.gz",
    }.items():
        if (assets / asset).is_file():
            downloads[f"linux-x86_64-{package_format}"] = f"{base}/{asset}"

    return {
        "version": version,
        "notes": notes,
        "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": platforms,
        "downloads": downloads,
    }


if __name__ == "__main__":
    version = os.environ["VERSION"]
    repo = os.environ["GITHUB_REPOSITORY"]
    tag = os.environ["RELEASE_TAG"]
    if tag != f"v{version}":
        raise ValueError("Release tag must match the application version")
    notes = f"LowNotes {version}"
    try:
        result = subprocess.run(
            ["gh", "release", "view", tag, "--repo", repo, "--json", "body", "-q", ".body"],
            capture_output=True, text=True, check=False,
        )
        if result.returncode == 0 and result.stdout.strip():
            notes = result.stdout.strip()
    except OSError:
        pass
    assets = Path("release-assets")
    doc = build_manifest(version, repo, assets, notes)
    (assets / "latest.json").write_text(json.dumps(doc, indent=2), encoding="utf-8")
    print("latest.json platforms:", sorted(doc["platforms"]))
    print("latest.json downloads:", sorted(doc["downloads"]))

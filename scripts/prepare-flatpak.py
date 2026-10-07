#!/usr/bin/env python3
"""Stage only release source and lockfile-pinned crates for offline Flatpak builds."""
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parent.parent
destination = root / "dist" / "flatpak-source"
if destination.exists():
    shutil.rmtree(destination)
destination.mkdir(parents=True)
for name in ("Cargo.toml", "Cargo.lock", "build.rs", "LICENSE", "src", "data", "icons", "po", "scripts"):
    source = root / name
    if source.is_dir():
        shutil.copytree(source, destination / name)
    else:
        shutil.copy2(source, destination / name)
configuration = subprocess.check_output(
    ["cargo", "vendor", "--locked", "--versioned-dirs", str(destination / "vendor")],
    cwd=root, text=True,
)
configuration = configuration.replace(str(destination / "vendor"), "vendor")
(destination / ".cargo").mkdir()
(destination / ".cargo" / "config.toml").write_text(configuration)
print(f"Offline sources prepared in {destination}")

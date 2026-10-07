#!/usr/bin/env python3
"""Include dependency license notices in binary packages, without host paths."""
import json
from pathlib import Path
import shutil
import subprocess
import sys

destination = Path(sys.argv[1]) / "share/licenses/ankunft/dependencies"
destination.mkdir(parents=True, exist_ok=True)
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version=1", "--locked", "--offline"], text=True,
))
notices = []
for package in metadata["packages"]:
    if package["name"] == "ankunft":
        continue
    notices.append({key: package.get(key) for key in ("name", "version", "license", "repository")})
    source = Path(package["manifest_path"]).parent.resolve()
    files = {path for path in source.iterdir() if path.is_file()
             and path.name.lower().startswith(("license", "copying", "notice", "copyright"))}
    if package.get("license_file"):
        candidate = (source / package["license_file"]).resolve()
        if candidate.is_relative_to(source) and candidate.is_file():
            files.add(candidate)
    for path in files:
        if path.resolve().is_relative_to(source):
            target = destination / f'{package["name"]}-{package["version"]}' / path.name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
(destination / "DEPENDENCIES.json").write_text(json.dumps(notices, indent=2) + "\n")

#!/usr/bin/env bash
set -Eeuo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
python3 scripts/prepare-flatpak.py
manifest=packaging/io.github.dandiccf.Ankunft.json
arch="$(flatpak --default-arch)"
if command -v flatpak-builder >/dev/null; then
    builder=(flatpak-builder)
else
    builder=(flatpak run org.flatpak.Builder)
fi
"${builder[@]}" --user --disable-rofiles-fuse --force-clean --install-deps-from=flathub \
    --repo=dist/flatpak-repo "dist/flatpak-build-$arch" "$manifest"
flatpak build-bundle --arch="$arch" --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
    dist/flatpak-repo "dist/Ankunft-$arch.flatpak" io.github.dandiccf.Ankunft
(cd dist && sha256sum "Ankunft-$arch.flatpak" > "Ankunft-$arch.flatpak.sha256")

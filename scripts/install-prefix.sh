#!/usr/bin/env bash
# Package staging only; use install-user.sh for desktop user installations.
set -Eeuo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
prefix="${1:?Usage: install-prefix.sh ABSOLUTE_PREFIX}"
[[ "$prefix" == /* && "$prefix" != / ]] || { echo "Prefix must be an absolute directory other than /." >&2; exit 1; }
app_id=io.github.dandiccf.Ankunft
install -Dm755 target/release/ankunft "$prefix/bin/ankunft"
install -Dm644 "data/$app_id.desktop" "$prefix/share/applications/$app_id.desktop"
install -Dm644 "data/$app_id.metainfo.xml" "$prefix/share/metainfo/$app_id.metainfo.xml"
install -Dm644 "icons/$app_id.svg" "$prefix/share/icons/hicolor/scalable/apps/$app_id.svg"
install -Dm644 LICENSE "$prefix/share/licenses/ankunft/LICENSE"
python3 scripts/collect-licenses.py "$prefix"
mkdir -p "$prefix/share/dbus-1/services"
sed "s|@ANKUNFT_BINARY@|$prefix/bin/ankunft|" "data/$app_id.service.in" > "$prefix/share/dbus-1/services/$app_id.service"
while read -r locale; do
    [[ "$locale" =~ ^[a-zA-Z_]+$ ]] || continue
    mkdir -p "$prefix/share/locale/$locale/LC_MESSAGES"
    msgfmt --check "po/$locale.po" -o "$prefix/share/locale/$locale/LC_MESSAGES/ankunft.mo"
done < po/LINGUAS

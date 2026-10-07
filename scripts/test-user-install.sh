#!/usr/bin/env bash
# Integration check for an ephemeral, unprivileged CI account only.
set -Eeuo pipefail
[[ "${CI:-}" == true ]] || { echo "Run this only in an ephemeral CI account." >&2; exit 1; }
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
app_id=io.github.dandiccf.Ankunft
data_directory="${XDG_DATA_HOME:-$HOME/.local/share}"
state_directory="${XDG_STATE_HOME:-$HOME/.local/state}/$app_id"
[[ ! -e "$HOME/.local/bin/ankunft" ]] || { echo "CI account already has Ankunft installed." >&2; exit 1; }
# The installer operates on the tested debug binary; optimization is tested by
# Flatpak builds. This fixture avoids an unnecessary second native compilation.
mkdir -p target/release "$state_directory"
cp target/debug/ankunft target/release/ankunft
printf 'Synthetic retained state\n' > "$state_directory/install-test-marker"
bash scripts/install-user.sh
test -x "$HOME/.local/bin/ankunft"
desktop-file-validate "$data_directory/applications/$app_id.desktop"
test -f "$data_directory/dbus-1/services/$app_id.service"
test -f "$data_directory/icons/hicolor/scalable/apps/$app_id.svg"
test -f "$data_directory/metainfo/$app_id.metainfo.xml"
while read -r locale; do
    test -f "$data_directory/locale/$locale/LC_MESSAGES/ankunft.mo"
done < po/LINGUAS
"$HOME/.local/bin/ankunft" --version
bash scripts/uninstall-user.sh
test ! -e "$HOME/.local/bin/ankunft"
test ! -e "$data_directory/applications/$app_id.desktop"
test ! -e "$data_directory/metainfo/$app_id.metainfo.xml"
test -f "$state_directory/install-test-marker"
echo 'User install/uninstall and retained state passed.'

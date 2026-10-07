#!/usr/bin/env bash
# openSUSE supplies Xvfb without the xvfb-run wrapper used by other distros.
set -Eeuo pipefail
if command -v xvfb-run >/dev/null; then
    exec xvfb-run -a -s '-screen 0 1280x900x24' "$@"
fi
headless_directory="$(mktemp -d)"
Xvfb -displayfd 3 -screen 0 1280x900x24 -nolisten tcp -ac 3>"$headless_directory/display" &
xvfb_pid=$!
cleanup() {
    kill "$xvfb_pid" 2>/dev/null || true
    wait "$xvfb_pid" 2>/dev/null || true
    python3 -c 'import shutil,sys; shutil.rmtree(sys.argv[1])' "$headless_directory"
}
trap cleanup EXIT
for ((attempt=0; attempt<100; attempt++)); do
    [[ -s "$headless_directory/display" ]] && break
    kill -0 "$xvfb_pid"
    sleep 0.05
done
read -r display_number < "$headless_directory/display"
DISPLAY=":$display_number" "$@"

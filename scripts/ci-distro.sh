#!/usr/bin/env bash
# Run in an ephemeral root-owned CI container, never on the user's desktop.
set -Eeuo pipefail
case "${1:?Expected distro name}" in
    ubuntu|debian)
        apt-get update
        DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
            build-essential pkg-config libgtk-4-dev libadwaita-1-dev libsecret-1-dev \
            gettext curl ca-certificates python3 git xvfb xauth dbus-x11
        ;;
    fedora)
        dnf install -y --allowerasing gcc pkgconf-pkg-config gtk4-devel libadwaita-devel libsecret-devel \
            gettext curl ca-certificates python3 git xorg-x11-server-Xvfb xorg-x11-xauth dbus-daemon
        ;;
    arch)
        pacman -Syu --noconfirm --needed base-devel gtk4 libadwaita libsecret gettext curl \
            ca-certificates python git xorg-server-xvfb xorg-xauth dbus
        ;;
    opensuse)
        zypper --non-interactive install gcc pkg-config gtk4-devel libadwaita-devel \
            libsecret-devel gettext-tools curl ca-certificates python3 git xorg-x11-server-Xvfb xauth dbus-1
        ;;
    *) echo "Unknown CI distro" >&2; exit 1 ;;
esac
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/ankunft-rustup.sh
sh /tmp/ankunft-rustup.sh -y --profile minimal --default-toolchain stable --component clippy
export PATH="/root/.cargo/bin:$PATH"
cd /workspace
pkg-config --modversion gtk4 libadwaita-1 libsecret-1
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked
dbus-run-session -- bash scripts/run-headless.sh python3 scripts/smoke-test.py "${CARGO_TARGET_DIR:-target}/debug/ankunft"

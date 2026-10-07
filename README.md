# Ankunft

[![Linux checks](https://github.com/Dandiccf/Ankunft/actions/workflows/ci.yml/badge.svg)](https://github.com/Dandiccf/Ankunft/actions/workflows/ci.yml)
[![Flatpak packages](https://github.com/Dandiccf/Ankunft/actions/workflows/flatpak.yml/badge.svg)](https://github.com/Dandiccf/Ankunft/actions/workflows/flatpak.yml)

A native Linux desktop client for **Parcel Premium**, built with Rust, GTK4 and
Libadwaita. Track packages, add deliveries, and receive private status-change
notifications from one desktop window.

**Status: 0.2.0 preview.** Source, tests and packaging are public. Preview releases
are intended for early adopters; real-account and desktop acceptance checks are
tracked in [the release checklist](docs/RELEASE-CHECKLIST.md). Ankunft is an
independent project and is not affiliated with Parcel.

![Ankunft's delivery overview using synthetic sample data](docs/screenshots/overview.png)

## Features

- Three-column delivery overview, search, status filters and event timelines.
- Add-delivery dialog with carrier search, local tracking-number suggestions,
  and optional postcode/email fields.
- Private local history of deliveries observed by Ankunft. Older deliveries
  outside Parcel's recent API view cannot be imported retroactively.
- Reversible, device-local delivered marks; Parcel's original status is retained.
- Automatic refresh every 15 minutes while running, plus manual refresh.
- Optional background operation after closing the window, enabled from the
  account menu for the current session. Flatpak asks the desktop for permission.
- Native status-change notifications with no tracking numbers or delivery
  descriptions in notification text. The first successful sync is silent.
- Offline cache, atomic writes and persistent protection of Parcel API limits.
- API-key storage through Secret Service, normally GNOME Keyring.
- German, English, French, Spanish, Italian and Brazilian Portuguese interfaces.
  The system language is selected automatically; unsupported languages use English.
- Application launcher, icon and D-Bus activation; compatible desktop docks may
  display unread notification badges.
- No telemetry.

## Install the Flatpak (recommended)

Flatpak supplies the GTK/Libadwaita runtime independently of the distribution.
This is the installation path for Ubuntu, Debian, Linux Mint, Fedora, Arch Linux,
openSUSE and other Linux distributions with Flatpak support, including hosts
whose native GTK libraries are too old. Packages are built for **x86_64** and
**aarch64**. Windows and macOS are not supported.

1. Install Flatpak using [your distribution's setup instructions](https://flatpak.org/setup/).
2. Download the matching architecture's `.flatpak` and `.sha256` files from
   [GitHub Releases](https://github.com/Dandiccf/Ankunft/releases). Choose x86_64
   for most Intel/AMD PCs; aarch64 is for 64-bit ARM.
3. From the download directory, verify and install it:

```bash
sha256sum -c Ankunft-x86_64.flatpak.sha256
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user ./Ankunft-x86_64.flatpak
flatpak run io.github.dandiccf.Ankunft
```

Replace `x86_64` with `aarch64` for ARM. The first installation downloads the
GNOME runtime. These are direct GitHub bundles; **Ankunft is not currently listed
on Flathub**. Flathub supplies its runtime. To upgrade, download and install the
new bundle with the same command; Flatpak preserves account data.

To uninstall while retaining local data:

```bash
flatpak uninstall --user io.github.dandiccf.Ankunft
```

Use **Remove connection** inside Ankunft first if you also want to remove its API
key and cached deliveries. Flatpak's `--delete-data` option removes sandbox data,
but does not delete an API key held separately in the desktop keyring.

## Connect your Parcel account

A Parcel Premium subscription and a personal API key are required for live data.
Generate a key at [Parcel Web](https://web.parcelapp.net/), then click **Connect API**
in Ankunft. The setup dialog also links to Parcel Web. Enter the key locally;
never include it in issues, screenshots, source files, or logs.

Without a connection, Ankunft displays clearly labelled synthetic sample data.
Once connected, it loads Parcel's recent view and preserves completed deliveries
it observes locally. **Remove connection** deletes Ankunft's keyring entry and
attempts to clear its local delivery cache; storage errors are shown explicitly.

### Refresh and background behavior

Deliveries refresh on connection/startup when the cached snapshot is older than
15 minutes, then every 15 minutes while the app runs. Manual refresh is available
from the header. Refreshes do not overlap active requests or account dialogs.
Parcel's API returns server-cached data; polling does not force a carrier update.

Closing the window normally exits. Enable **Keep running in the background** in
the account menu to keep refreshing and receiving notifications after closing
it. Reopen from the launcher or a notification. Use **Quit** or Ctrl+Q to stop it.
The background choice resets when the process exits; there is no login autostart
or tray icon. A desktop portal may deny background operation in a sandbox.

The [Parcel read API](https://parcelapp.net/help/api-view-deliveries.html) permits
20 requests per hour. The [add API](https://parcelapp.net/help/api-add-delivery.html)
permits 20 attempts per day, including rejected tracking numbers. Ankunft records
attempts before sending requests and never automatically retries delivery
creation. Other clients using the same account can also consume server limits.

## Build and install natively

Native builds require **Rust 1.92+**, **GTK 4.12+**, **Libadwaita 1.5+**,
**Libsecret 0.20+**, GNU gettext, a C compiler and pkg-config. Install Rust with
[rustup](https://rustup.rs/) if your distribution's Rust package is older.

The CI matrix builds and starts the demo on Ubuntu 24.04, Debian 13, Fedora 44,
Arch Linux and openSUSE Tumbleweed. This checks compilation and desktop startup;
it does not certify every desktop session or every version of each distribution.
Debian 12 and Ubuntu 22.04 should use the Flatpak because their native toolkit
versions are below the minimum.

### Ubuntu 24.04+ / Debian 13+

```bash
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev libsecret-1-dev gettext
```

### Fedora

```bash
sudo dnf install gcc pkgconf-pkg-config gtk4-devel libadwaita-devel libsecret-devel gettext
```

### Arch Linux

```bash
sudo pacman -S --needed base-devel gtk4 libadwaita libsecret gettext
```

### openSUSE Tumbleweed

```bash
sudo zypper install gcc pkg-config gtk4-devel libadwaita-devel libsecret-devel gettext-tools
```

Then clone, build and install without administrator privileges:

```bash
git clone https://github.com/Dandiccf/Ankunft.git
cd Ankunft
cargo build --release --locked
./scripts/install-user.sh
```

The executable goes in `~/.local/bin`; desktop integration and translations go
in `${XDG_DATA_HOME:-~/.local/share}`. Launch from the application menu, or run
`~/.local/bin/ankunft`. For development, use `cargo run --locked`.

```bash
./scripts/uninstall-user.sh
```

Uninstalling application files preserves the keyring entry and offline state.
The install/uninstall scripts intentionally refuse to run as root.

## Languages and demo mode

The interface follows `LC_ALL`, `LC_MESSAGES`, `LANG`, and gettext's `LANGUAGE`
preferences. For example, from a UTF-8 desktop session:

```bash
LANGUAGE=fr ~/.local/bin/ankunft
flatpak run --env=LANGUAGE=pt_BR io.github.dandiccf.Ankunft
```

The C/POSIX locale and unsupported languages use English. For a different
language, quit the existing process before relaunching. Delivery descriptions,
carrier-provided events and ambiguous date strings are displayed as supplied by
Parcel; the UI cannot translate external account data. Native carrier search
uses prefixes on Libadwaita 1.5 and substrings on 1.6+.

A separate demo instance never reads or writes the account's cache, keyring or
API counters and never makes API requests:

```bash
cargo run --locked -- --demo
flatpak run io.github.dandiccf.Ankunft --demo
```

## Security and local data

The API key is stored through the desktop's Secret Service. GNOME Keyring is the
usual provider; another compatible provider can work on other desktops. A
running, unlocked service is required for connection. Keys are not saved in
plain-text configuration files. HTTPS uses Rustls and redirects are disabled.

Normalized delivery data is stored with private directory/file permissions and
atomic replacement. It is not encrypted at rest. Native state lives under
`${XDG_STATE_HOME:-~/.local/state}/io.github.dandiccf.Ankunft/`:

- `deliveries-v1.json`: versioned delivery cache and local delivered marks.
- `rate-limits-v1.json`: persistent API-attempt counters.

In Flatpak, those paths are inside `~/.var/app/io.github.dandiccf.Ankunft/`.
The API counter remains when disconnecting to avoid accidentally bypassing
limits. There is no telemetry. Flatpak permissions include network access,
graphics and access to Secret Service; it does not receive general home-directory
access. See [SECURITY.md](SECURITY.md) for vulnerability reporting.

## Troubleshooting

| Symptom | What to check |
| --- | --- |
| Missing GTK/Libadwaita or pkg-config build error | Check the minimum versions above and install development packages, or use Flatpak. |
| Rust version is too old | Update using rustup; the locked GTK bindings need Rust 1.92+. |
| `msgfmt` not found | Install GNU gettext (`gettext-tools` on openSUSE). |
| Keyring unavailable or locked | Unlock GNOME Keyring or enable a compatible Secret Service provider in the desktop session. |
| Live data unavailable | Check network access, Premium status, the API key and the error banner. Cached deliveries remain available. |
| API limit reached | Wait for the rolling limit to expire; do not erase the counters to retry. |
| New delivery has no events yet | Parcel validates creation first; carrier data appears after its next server update. |
| Old delivery missing | Parcel exposes a limited recent view. Local history only contains deliveries Ankunft has observed. |
| No notifications after closing | Enable background operation and check desktop notification/background permissions. Quit stops monitoring. |
| Launcher missing after native install | Try `~/.local/bin/ankunft`, then log out/in to refresh the application menu. |
| Wrong language | Quit the current process, check the locale environment, and relaunch. |

Report reproducible problems in [GitHub Issues](https://github.com/Dandiccf/Ankunft/issues).
Include the version, distro, desktop, installation method and reproduction steps;
remove credentials, tracking numbers and personal delivery data.

## Development and packaging

See [CONTRIBUTING.md](CONTRIBUTING.md), [architecture](docs/ARCHITECTURE.md) and
[packaging instructions](docs/PACKAGING.md). CI checks formatting, Clippy, tests,
translation coverage/placeholders, desktop and AppStream metadata, five distro
builds, and Flatpak installation/startup for x86_64 and aarch64.

## License

Ankunft is public open-source software under the [MIT license](LICENSE).
Application metadata is CC0-1.0 as declared in its AppStream file.

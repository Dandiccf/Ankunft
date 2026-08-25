# Ankunft

Ankunft is a modern native GNOME client for Parcel Premium. It is in early
development, but already supports secure access to the official Parcel Premium
API.

## Current features

- native GTK4 and Libadwaita interface
- GNOME application icon for the app grid, dock, and notifications
- unread-update badges on compatible GNOME docks, powered by native
  notifications
- three-column delivery overview with search and status filters
- dedicated delivered-shipment view with a private local history of completed
  deliveries observed by Ankunft
- reversible, device-local “delivered” marks without changing Parcel account
  data or generating artificial status notifications
- detailed event timeline and delivery estimates
- add-delivery assistant with live carrier filtering, private local carrier
  suggestions, and optional postcode or email details
- secure setup dialog for the personal Parcel API key
- live synchronization outside the GTK main thread
- API key storage exclusively in the GNOME Keyring
- private, atomically written offline cache
- persistent protection of Parcel API limits across application restarts
- privacy-preserving GNOME notifications for important status changes
- localized interface in German, English, French, Spanish, Italian, and
  Brazilian Portuguese
- clear loading, offline, empty, and error states
- no credentials in source code, configuration files, or logs

## Run from source

```bash
cargo run
```

The build requires Rust, GNU gettext, GTK 4, Libadwaita, and Libsecret,
including their development packages.

After launch, select **Connect API** to open the secure setup dialog. Enter the
key locally and never paste it into chats, issues, source code, or logs.

## Install for the current user

Build the optimized application and install its desktop integration without
administrator privileges:

```bash
cargo build --release
./scripts/install-user.sh
```

This installs the executable, application launcher, icon, D-Bus activation
service, and translation catalogs below `~/.local`. Ankunft then appears in the
GNOME app grid and can be pinned to the dock. The interface follows the current
GNOME system language automatically.

To remove the application files while keeping the API key, preferences, and
offline data:

```bash
./scripts/uninstall-user.sh
```

## Security and privacy

The Parcel API key is stored only in the GNOME Keyring. The offline cache holds
normalized delivery data rather than raw API responses and is readable only by
the signed-in user. The application does not include telemetry.

## Project status

The next milestones are scheduled background refresh, optional tray
integration, local-history management, and reproducible packaging. Ankunft is
an independent project and is not affiliated with Parcel.

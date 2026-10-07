# Release acceptance

Automated checks establish build/startup compatibility, not full acceptance on
every desktop. Keep preview releases labelled until the manual checks below have
been completed with a consenting tester's own account.

## Automated gates

- [ ] Linux checks workflow passes on the release commit: formatting, Clippy,
  unit/mock HTTP tests, catalogs and application metadata.
- [ ] Native build and demo startup pass on Ubuntu 24.04, Debian 13, Fedora 44,
  Arch Linux and openSUSE Tumbleweed.
- [ ] Both x86_64 and aarch64 Flatpaks build, install and start on all six supported
  locales and an unsupported locale.
- [ ] Native install/uninstall restores desktop files and retains private data.
- [ ] Downloaded release bundles match their published SHA-256 files.

## Real desktop and Parcel account (not performed by CI)

- [ ] GNOME Wayland: connect, restart, restore key, sync and disconnect.
- [ ] GNOME X11: launcher, clipboard, reopen window and notification activation.
- [ ] KDE Plasma: compatible Secret Service provider, notifications and launcher.
- [ ] Confirm the 15-minute refresh with an active delivery, without extra requests.
- [ ] Enable background mode, close/reopen, receive a real status update, then Quit.
- [ ] Deny the Flatpak background request; the window stays open and the toggle resets.
- [ ] Locked/unavailable keyring: clear error, no credential persistence fallback.
- [ ] Offline/reconnect: cache displays, errors recover and requests do not overlap.
- [ ] Add a valid delivery once; check optional carrier fields and eventual events.
- [ ] Invalid key, rejected tracking ID and server rate limit: clear errors, no POST retry.
- [ ] Delivered history and local marks survive restart; Parcel status remains unchanged.
- [ ] Test sandbox removal and native uninstall; key/cache retention matches README.

Use synthetic data for public evidence. Never commit an API key, account cache or
personal delivery screenshots. Record the distro, desktop, architecture, package
version and outcome when a manual check is completed.

An explicit optional check can read recent deliveries using Ankunft's existing
keyring entry without printing credentials or delivery data:

```bash
ANKUNFT_LIVE_SMOKE=1 cargo test --locked secrets::tests::live_saved_connection_read_only -- --ignored --exact
```

This consumes one read attempt when a saved key is available. It never adds or
modifies a Parcel delivery and must not be enabled in CI.

## Verified locally on 2026-10-07

- 68 automated tests, strict Clippy, formatting, translation/placeholder checks,
  desktop metadata, AppStream metadata and shell script checks passed.
- Ubuntu 24.04 native build/startup passed with GTK 4.14.5 and Libadwaita 1.5.0.
- Native install/uninstall passed in a fresh unprivileged Ubuntu account: six
  catalogs, desktop integration and retention of synthetic local state.
- The x86_64 Flatpak built offline, installed, and mapped its window in each of
  the six interface languages; an unsupported locale selected English.
- The explicitly enabled read-only live check successfully loaded Ankunft's
  saved key and fetched recent deliveries. No credentials or delivery details
  were printed, and no Parcel deliveries were added or modified.

These results do not replace the physical-desktop, account-change, background
permission and notification acceptance checks above.

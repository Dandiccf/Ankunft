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

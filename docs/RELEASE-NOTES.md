Ankunft 0.2.0 is an open-source preview release for Linux.

- Installable Flatpak bundles for x86_64 and aarch64, built against GNOME 50.
- Native minimums lowered to GTK 4.12, Libadwaita 1.5 and Libsecret 0.20;
  source builds now support Ubuntu 24.04's toolkit versions.
- Automatic refresh every 15 minutes, with optional session-local background
  operation after closing the window and Flatpak desktop permission handling.
- Six validated interface translations and English fallback for other locales.
- Isolated demo mode for screenshots and desktop startup checks.
- New HTTP integration tests, distro CI, metadata checks and offline dependency
  packaging from Cargo.lock.
- Expanded installation, privacy, troubleshooting and contributor documentation.

A Parcel Premium subscription, personal API key and unlocked Secret Service
provider are required for live tracking. There is no tray icon or login autostart.
Flatpak runtime downloads are supplied by Flathub; the app is distributed through
these GitHub bundles and is not listed on Flathub.

This is a preview, pending live-account and physical-desktop acceptance checks.
See the repository's release checklist for the exact remaining validation.

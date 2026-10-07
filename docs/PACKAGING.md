# Flatpak packaging

The manifest in `packaging/io.github.dandiccf.Ankunft.json` uses GNOME 50 and the
Freedesktop Rust SDK extension. It supports x86_64 and aarch64, uses no general
host filesystem permission, and permits Secret Service access for the API key.

## Build

Install Flatpak, Python 3, Cargo and flatpak-builder. Alternatively install the
`org.flatpak.Builder` Flatpak; the build script detects it. Add the user remote:

```bash
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
bash scripts/build-flatpak.sh
```

The script vendors the exact dependency versions/checksums from `Cargo.lock`
into `dist/flatpak-source`, then compiles with `--frozen --offline` in the SDK.
Dependency acquisition occurs before the sandbox build. Build output and sources
are excluded from Git; only the manifest, lockfile and build scripts are tracked.
Do not enable network access in the build sandbox to resolve missing crates.

The result is `dist/Ankunft-ARCH.flatpak` plus its SHA-256 file. Install it with
`flatpak install --user ./dist/Ankunft-ARCH.flatpak`. Install prerequisites and
build/install it on a matching-architecture host. Cross-compilation is not used.

The SDK and Rust extension receive upstream updates. The source dependencies
are locked, but this pipeline does not claim bit-identical builds across SDK
updates. Flatpak's resolved manifest and OSTree metadata record the build
inputs; GitHub Actions records the source revision and build logs.

## Release

1. Update the Cargo version, AppStream release and `docs/RELEASE-NOTES.md`.
2. Run local checks and the CI matrix; inspect all failures.
3. Commit and push `main` and wait for both workflows to pass.
4. Tag that tested commit (`git tag vVERSION`) and push the tag.
5. The packaging workflow builds, installs and starts both architecture bundles,
   then publishes a GitHub **prerelease** containing them and their checksums.
6. Finish the real-desktop/account checklist before advertising a stable release.

Direct bundles are distributed through GitHub, not a Flathub listing. Installing
a newer bundle upgrades the same application and retains sandbox data. Native
binaries compiled against one distro are not advertised as portable Linux
binaries; use the Flatpak runtime for portability.

Reference: [Flatpak's Rust packaging guide](https://github.com/flatpak/flatpak-builder-tools/blob/master/cargo/README.md)
and [manifest documentation](https://docs.flatpak.org/en/latest/manifests.html).

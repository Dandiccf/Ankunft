# Contributing to Ankunft

Issues and pull requests are welcome. Describe the problem, reproduction steps
and expected behavior. Never upload API keys, real tracking numbers, postcodes,
email addresses, cached account data or unredacted screenshots.

## Local checks

Install the dependencies listed in the README, plus Python 3, desktop-file-utils
and AppStream. The catalog checker uses Python's standard library and GNU gettext.

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
python3 scripts/check-translations.py
desktop-file-validate data/io.github.dandiccf.Ankunft.desktop
appstreamcli validate --no-net data/io.github.dandiccf.Ankunft.metainfo.xml
```

For desktop startup checks, install Xvfb, xauth and dbus-run-session:

```bash
cargo build --locked
dbus-run-session -- xvfb-run -a python3 scripts/smoke-test.py target/debug/ankunft
```

These tests use `--demo`, synthetic data, an X11 virtual display and the Cairo
software renderer, with no account access. GPU/Wayland acceptance needs a real
desktop. HTTP tests
use a loopback mock server, dummy credentials and isolated temporary storage.
Do not make live API requests in CI.

## Translations

German strings are message identifiers; runtime fallback is English. Wrap new
UI text in `tr`, or `trn` for plurals. Add corresponding entries to all six files
in `po/` and retain placeholder names/counts. Keep `po/POTFILES.in` up to date for
new source files. Do not translate tracking numbers or carrier-supplied data.

## Changes and releases

Keep UI work on the GTK main thread; network work belongs in a blocking worker.
Preserve atomic storage, private permissions, request reservations and the rule
against automatic POST retries. Add behavior tests where a change affects these
guarantees. Update the README and release notes when user-visible behavior changes.

Release packaging and its acceptance checklist live in `docs/`. Preview tags
publish only after both architecture packages build and start successfully.

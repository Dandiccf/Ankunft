#!/usr/bin/env python3
"""Launch each locale with isolated demo data under an existing display."""
import os
import selectors
import signal
from pathlib import Path
import subprocess
import sys
import time
import tomllib

command = sys.argv[1:] or ["target/debug/ankunft"]
root = Path(__file__).resolve().parent.parent
version = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
assert subprocess.check_output([*command, "--version"], text=True).strip() == f"Ankunft {version}"
for locale in ("en", "de", "fr", "es", "it", "pt_BR", "ja"):
    environment = dict(os.environ, LC_ALL="C.UTF-8", LANG="C.UTF-8", LANGUAGE=locale,
                       GDK_BACKEND="x11", GSK_RENDERER="cairo", GTK_A11Y="none")
    # C.UTF-8 deliberately follows gettext's untranslated C convention.
    # Use a non-C effective locale to exercise LANGUAGE without requiring
    # locale generation: Ankunft's catalog loader normalizes this directly.
    environment["LC_MESSAGES"] = "en_US.UTF-8"
    environment.pop("LC_ALL", None)
    with subprocess.Popen([*command, "--demo"], env=environment, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True, start_new_session=True) as application:
        failure = None
        try:
            deadline = time.monotonic() + 45
            expected = "en" if locale == "ja" else locale
            with selectors.DefaultSelector() as selector:
                selector.register(application.stdout, selectors.EVENT_READ)
                while True:
                    assert time.monotonic() < deadline, f"Window did not appear for {locale}"
                    if selector.select(timeout=1):
                        line = application.stdout.readline().strip()
                        assert line or application.poll() is None, f"Application exited during {locale} startup"
                        if line.startswith("Ankunft demo ready: "):
                            assert line == f"Ankunft demo ready: {expected}", f"Incorrect locale: {line}"
                            break
            time.sleep(1)
            if locale == "en" and os.environ.get("ANKUNFT_SCREENSHOT"):
                destination = Path(os.environ["ANKUNFT_SCREENSHOT"]).resolve()
                destination.parent.mkdir(parents=True, exist_ok=True)
                subprocess.run(["import", "-window", "root", str(destination)], check=True)
        except Exception as error:
            failure = error
        finally:
            if application.poll() is None:
                os.killpg(application.pid, signal.SIGTERM)
            try:
                stdout, stderr = application.communicate(timeout=10)
        if failure:
            raise RuntimeError(f"Desktop startup failed for {locale}: {failure}\n{stderr}") from failure
            except subprocess.TimeoutExpired:
                os.killpg(application.pid, signal.SIGKILL)
                stdout, stderr = application.communicate(timeout=10)
        assert not any(marker in stderr for marker in ("CRITICAL", "ERROR", "Segmentation fault")), stderr
        print(f"{locale}: desktop startup passed")

#!/usr/bin/env python3
"""Check catalogs, placeholders and statically extractable Rust messages."""
import ast
import json
from pathlib import Path
import re
import subprocess

root = Path(__file__).resolve().parent.parent

def messages(path):
    entries = {}
    for block in path.read_text().split("\n\n"):
        fields, current = {}, None
        for line in block.splitlines():
            match = re.match(r'(msgid_plural|msgid|msgstr(?:\[\d+\])?) (".*")$', line)
            if match:
                current = match[1]
                fields[current] = ast.literal_eval(match[2])
            elif line.startswith('"') and current:
                fields[current] += ast.literal_eval(line)
        if fields.get("msgid"):
            if "#, fuzzy" in block:
                raise ValueError(f"Fuzzy translation in {path}")
            entries[fields["msgid"]] = fields
    return entries

baseline = messages(root / "po/de.po")
for locale in (root / "po/LINGUAS").read_text().split():
    catalog = root / f"po/{locale}.po"
    subprocess.run(["msgfmt", "--check", "-o", "/dev/null", str(catalog)], check=True)
    translated = messages(catalog)
    assert translated.keys() == baseline.keys(), f"Message set differs: {locale}"
    for source, fields in translated.items():
        placeholders = re.findall(r'\{[^{}]*\}', source)
        forms = [value for name, value in fields.items() if name.startswith("msgstr")]
        assert forms and all(forms), f"Empty translation: {locale}: {source}"
        for form in forms:
            assert sorted(re.findall(r'\{[^{}]*\}', form)) == sorted(placeholders), f"Placeholder mismatch: {locale}: {source}"
    print(f"{locale}: {len(translated)} complete messages")

# Extract direct string-literal tr/trn calls without requiring gettext 0.26's
# Rust support. Enum/match-based messages are covered by the common catalog set
# and behavioral tests; this deliberately does not claim a full Rust parser.
literal = r'"(?:\\.|[^"\\])*"'
calls = re.compile(r'\btr(?:n)?\(\s*(' + literal + r')(?:\s*,\s*(' + literal + r')\s*,)?')
for source_file in (root / "src").rglob("*.rs"):
    for match in calls.finditer(source_file.read_text()):
        source = json.loads(match[1], strict=False)
        assert source in baseline, f"Untranslated Rust message in {source_file}: {source}"
        if match[2]:
            plural = json.loads(match[2], strict=False)
            assert baseline[source].get("msgid_plural") == plural, f"Plural differs: {source}"

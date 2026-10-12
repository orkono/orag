#!/usr/bin/env bash
# The PowerShell pack builder (Windows) must pin exactly what the bash one
# pins: the same presets, repositories, revisions, files, SHA-256 hashes,
# licence sources and manifest sections. Diffs the two preset tables.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
import re, sys
KEYS = ("repo", "revision", "file", "sha", "license_url", "license_sha",
        "card_url", "card_sha", "role", "role_section", "license_name", "mit_holder")

def unescape_bash(v):
    if v.startswith("$'"):
        return v[2:-1].encode().decode("unicode_escape")
    return v[1:-1]

def bash_presets(text):
    body = text[text.index('case "$preset" in'):text.index("\nesac")]
    out = {}
    for m in re.finditer(r"^  ([a-z0-9.\-_]+)\)\n(.*?)\n    ;;", body, re.S | re.M):
        fields = {"license_name": "Apache-2.0", "mit_holder": ""}
        for k, v in re.findall(r"^\s+(\w+)=(\$'(?:[^'\\]|\\.)*'|\"[^\"]*\")$", m.group(2), re.M):
            fields[k] = unescape_bash(v)
        out[m.group(1)] = fields
    return out

def ps_presets(text):
    out = {}
    for m in re.finditer(r"^  '([a-z0-9.\-_]+)' = @\{\n(.*?)\n  \}", text, re.S | re.M):
        fields = {"license_name": "Apache-2.0", "mit_holder": ""}
        for k, v in re.findall(r"^\s+(\w+) = '((?:[^']|'')*)'$", m.group(2), re.M):
            fields[k] = v.replace("''", "'").replace("`n", "\n")
        out[m.group(1)] = fields
    return out

try:
    ps = open("scripts/fetch-model-pack.ps1", encoding="utf-8").read()
except FileNotFoundError:
    sys.exit("scripts/fetch-model-pack.ps1 is missing")
a = bash_presets(open("scripts/fetch-model-pack.sh", encoding="utf-8").read())
b = ps_presets(ps)
if not a:
    sys.exit("no presets parsed from fetch-model-pack.sh")
bad = []
if set(a) != set(b):
    bad.append(f"preset ids differ: only sh {sorted(set(a)-set(b))}, only ps1 {sorted(set(b)-set(a))}")
for pid in sorted(set(a) & set(b)):
    for k in KEYS:
        if a[pid].get(k, "") != b[pid].get(k, ""):
            bad.append(f"{pid}.{k}: sh {a[pid].get(k, '')!r} != ps1 {b[pid].get(k, '')!r}")
if bad:
    sys.exit("\n".join(bad))
print(f"pack presets match ({len(a)} presets)")
PY

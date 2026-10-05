#!/usr/bin/env bash
# Writes the license notices of the third-party code built into the `orag`
# binary: every package on its normal dependency graph for this host, without
# proc-macro crates and their compile-time-only dependencies (`cargo tree -e
# normal,no-proc-macro`), plus the C/C++ code vendored by llama.cpp. A package
# without a license file gets its SPDX expression, its authors as the copyright
# holders and the standard license text. Identical texts are printed once.
# Usage: scripts/third-party-licenses.sh <output-file>
set -euo pipefail
out="${1:?usage: $0 <output-file>}"
out="$(cd "$(dirname "$out")" && pwd)/$(basename "$out")"   # relative to the caller
cd "$(dirname "$0")/.."
tree=$(mktemp)
meta=$(mktemp)
trap 'rm -f "$tree" "$meta"' EXIT
cargo tree --locked -p orag -e normal,no-proc-macro --prefix none -f '{p}' > "$tree"
cargo metadata --locked --format-version 1 > "$meta"
python3 - "$tree" "$meta" "$out" <<'EOF'
import hashlib, json, os, re, sys

tree_file, meta_file, out_file = sys.argv[1:]
meta = json.load(open(meta_file))
workspace = set(meta["workspace_members"])
by_key = {(p["name"], p["version"]): p for p in meta["packages"]}
wanted = set()
for line in open(tree_file):
    m = re.match(r"(\S+) v(\S+)", line.strip())
    if m:
        wanted.add((m.group(1), m.group(2)))

NAMES = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE|COPYRIGHT)", re.I)
# License texts, not code or build files that happen to share the name.
CODE = re.compile(r"\.(cmake|rs|py|c|cc|cpp|h|hpp|sh|toml|json|in)$", re.I)
# Vendored trees that are present in a package but not built.
NOT_BUILT = {"libsqlite3-sys": ("sqlcipher",)}
# Standard texts for packages that ship no license file. The copyright line is
# the package's authors; the MIT text is the template of this repository's own.
mit = open("LICENSE-MIT").read().split("\n")
MIT_BODY = "\n".join(l for l in mit if not l.startswith("Copyright")).strip()
STANDARD = {
    "MIT": MIT_BODY,
    "Apache-2.0": open("LICENSE-APACHE").read().strip(),
    "BSD-3-Clause": """Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.
3. Neither the name of the copyright holder nor the names of its contributors
   may be used to endorse or promote products derived from this software
   without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.""",
}

texts = {}
def add_text(label, text):
    digest = hashlib.sha256(text.encode()).hexdigest()[:12]
    texts.setdefault(digest, (label, text))
    return digest

def license_files(root, depth, skip):
    found = []
    for dirpath, dirnames, filenames in os.walk(root):
        rel = os.path.relpath(dirpath, root)
        level = 0 if rel == "." else rel.count(os.sep) + 1
        dirnames[:] = sorted(d for d in dirnames
                             if level < depth and not d.startswith(".") and d not in skip)
        for name in sorted(filenames):
            if NAMES.match(name) and not CODE.search(name):
                found.append(os.path.join(dirpath, name))
    return found

def header_notice(path):
    """Copyright and license lines from the top of a vendored source file."""
    lines = open(path, encoding="utf-8", errors="replace").read().split("\n")[:80]
    return "\n".join(l.strip() for l in lines if re.search(r"copyright|spdx|license", l, re.I))

rows, missing = [], []
for key in sorted(wanted):
    pkg = by_key.get(key)
    if pkg is None or pkg["id"] in workspace:
        continue
    root = os.path.dirname(pkg["manifest_path"])
    is_sys = pkg["name"].endswith("-sys") or "-sys-" in pkg["name"]
    files = license_files(root, 3 if is_sys else 0, NOT_BUILT.get(pkg["name"], ()))
    refs = [(os.path.relpath(f, root),
             add_text(f'{pkg["name"]}/{os.path.relpath(f, root)}',
                      open(f, encoding="utf-8", errors="replace").read().strip()))
            for f in files]
    expr = pkg.get("license") or ""
    if not refs:
        holders = ", ".join(a.split(" <")[0] for a in pkg["authors"]) or f'the {pkg["name"]} authors'
        for spdx in re.findall(r"MIT|Apache-2\.0|BSD-3-Clause", expr):
            body = STANDARD[spdx]
            text = body if spdx == "Apache-2.0" else f"Copyright (c) {holders}\n\n{body}"
            refs.append((f"{spdx} (standard text)", add_text(f'{pkg["name"]} ({spdx})', text)))
        if not refs:
            missing.append(f'{pkg["name"]} {pkg["version"]} ({expr or "no license expression"})')
    rows.append((pkg["name"], pkg["version"], expr or "see files", refs))
    # llama.cpp vendors C/C++ libraries; those without a LICENSE file carry
    # their notice in the source headers.
    vendor = os.path.join(root, "llama.cpp", "vendor")
    if is_sys and os.path.isdir(vendor):
        for lib in sorted(os.listdir(vendor)):
            lib_dir = os.path.join(vendor, lib)
            if not os.path.isdir(lib_dir) or license_files(lib_dir, 0, ()):
                continue
            notices = []
            for dirpath, _, filenames in sorted(os.walk(lib_dir)):
                for name in sorted(filenames):
                    if re.search(r"\.(h|hpp|c|cpp)$", name):
                        path = os.path.join(dirpath, name)
                        notice = header_notice(path)
                        if notice:
                            notices.append(f"{os.path.relpath(path, root)}:\n{notice}")
            if notices:
                label = f"llama.cpp/vendor/{lib}"
                digest = add_text(f'{pkg["name"]}/{label}', "\n\n".join(notices))
                rows.append((f"{label} (vendored by {pkg['name']})", pkg["version"],
                             "see notice", [("source headers", digest)]))

with open(out_file, "w") as out:
    out.write("# Third-party licenses\n\n")
    out.write("Third-party code built into the `orag` binary. Each package lists its license\n")
    out.write("files by text id; the texts follow, each printed once. A package without a\n")
    out.write("license file gets its SPDX expression, its authors as copyright holders and\n")
    out.write("the standard text. llama.cpp vendors C/C++ libraries; their notices are taken\n")
    out.write("from the source headers, and some are only used by features ORAG does not\n")
    out.write("enable. SQLite (bundled by libsqlite3-sys) is in the public domain.\n\n")
    out.write("| package | version | license | license files (text id) |\n|---|---|---|---|\n")
    for name, version, expr, refs in rows:
        files = ", ".join(f"`{f}` ({d})" for f, d in refs) or "none"
        out.write(f"| {name} | {version} | {expr} | {files} |\n")
    out.write("\n## License texts\n")
    for digest, (label, text) in sorted(texts.items()):
        # Four backticks: a license text may contain a three-backtick fence.
        out.write(f"\n### {digest} ({label})\n\n````text\n{text}\n````\n")
print(f"{out_file}: {len(rows)} entries, {len(texts)} distinct texts", file=sys.stderr)
if missing:
    print("no license text for: " + "; ".join(missing), file=sys.stderr)
    sys.exit(1)
EOF

#!/usr/bin/env bash
# D-001: the release binary must not depend on non-system dynamic libraries.
set -euo pipefail
usage="usage: $0 <binary> [--release-floor]"
case $# in
  1) bin="$1"; release_floor=0 ;;
  2) [ "$2" = "--release-floor" ] || { echo "unknown option: $2 ($usage)" >&2; exit 2; }
     bin="$1"; release_floor=1 ;;
  *) echo "$usage" >&2; exit 2 ;;
esac
[ -f "$bin" ] || { echo "no such binary: $bin ($usage)" >&2; exit 2; }

. "$(dirname "$0")/platform-floors.sh"

# PREFIX version needed by the binary must not exceed the floor. With
# "required", finding no PREFIX version at all is an error: every Linux
# binary needs glibc, so none found means the symbol listing was not read.
check_symbol_floor() {
  local symbols="$1" prefix="$2" floor="$3" required="${4:-}" highest
  highest=$(highest_version "$prefix" <<<"$symbols")
  if [ -z "$highest" ] && [ "$required" = required ]; then
    echo "no ${prefix}_ symbol versions found in the binary; cannot check the floor" >&2
    return 1
  fi
  if [ -n "$highest" ] && ! version_le "$highest" "$floor"; then
    echo "binary needs ${prefix}_${highest}, newer than the supported floor ${prefix}_${floor}" >&2
    return 1
  fi
}

case "$(uname -s)" in
  Darwin)
    deps=$(otool -L "$bin" | tail -n +2 | awk '{print $1}')
    bad=$(echo "$deps" | grep -vE '^(/usr/lib/|/System/Library/)' || true)
    ;;
  Linux)
    # One ldd run, read from a variable: no `ldd | grep -q` (SIGPIPE under pipefail).
    linked=$(ldd "$bin")
    unresolved=$(grep "not found" <<<"$linked" || true)
    if [ -n "$unresolved" ]; then
      echo "unresolved dynamic libraries:" >&2
      echo "$unresolved" >&2
      exit 1
    fi
    # Resolved path of every dependency; only the base system loader/libc family may appear.
    deps=$(awk '/=>/ {print $3} !/=>/ {print $1}' <<<"$linked" | grep -v '^$')
    bad=$(echo "$deps" | grep -vE '^(linux-vdso\.so[.0-9]*|/(usr/)?lib(64)?/([^/]+/)?(ld-linux[^/]*|lib(c|m|dl|rt|pthread|gcc_s|stdc\+\+))\.so[.0-9]*)$' || true)
    ;;
  MINGW*|MSYS*|CYGWIN*)
    # Imported DLLs from the PE import table (llvm-readobj ships with the
    # llvm-tools rustup component). Static CRT: no VC++ runtime may appear.
    readobj=$(command -v llvm-readobj || ls "$(rustc --print sysroot)"/lib/rustlib/*/bin/llvm-readobj* 2>/dev/null | head -1)
    [ -n "$readobj" ] || { echo "llvm-readobj not found (rustup component add llvm-tools)" >&2; exit 2; }
    imports=$("$readobj" --coff-imports "$bin")
    deps=$(sed -nE 's/^ *Name: ([^ ]+\.[dD][lL][lL])$/\1/p' <<<"$imports" | tr 'A-Z' 'a-z' | sort -u)
    if [ -z "$deps" ]; then
      echo "no DLL imports read from $bin; cannot check them" >&2
      exit 1
    fi
    bad=$(grep -vE "$WINDOWS_SYSTEM_DLLS" <<<"$deps" || true)
    ;;
  *)
    echo "unsupported OS" >&2
    exit 1
    ;;
esac
if [ -n "$bad" ]; then
  echo "non-system dynamic dependencies:" >&2
  echo "$bad" >&2
  exit 1
fi
if [ "$(uname -s)" = "Linux" ] && [ "$release_floor" = 1 ]; then
  # Only on the release image (CI and release.yml pass --release-floor): a newer
  # local distro links newer symbols (e.g. glibc 2.38 C23 functions) legitimately.
  # Floors come from platform-floors.sh (README: Ubuntu 22.04 and newer).
  symbols=$(objdump -T "$bin")
  check_symbol_floor "$symbols" GLIBC "$GLIBC_MIN" required
  check_symbol_floor "$symbols" GLIBCXX "$GLIBCXX_MIN"
  check_symbol_floor "$symbols" CXXABI "$CXXABI_MIN"
  check_symbol_floor "$symbols" GCC "$GCC_MIN"
fi
if [ "$(uname -s)" = "Darwin" ]; then
  # The final binary (Rust, SQLite, sqlite-vec, llama.cpp) must not need a newer macOS.
  load_commands=$(otool -l "$bin")
  minos=$(highest_minos <<<"$load_commands")
  if [ -z "$minos" ] || ! version_le "$minos" "$MACOS_MIN"; then
    echo "binary requires macOS ${minos:-unknown}, above the supported $MACOS_MIN" >&2
    exit 1
  fi
fi
echo "binary dependencies ok: $bin"

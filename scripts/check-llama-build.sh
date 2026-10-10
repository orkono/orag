#!/usr/bin/env bash
# Checks what the llama.cpp build actually produced (not cargo feature flags):
# the Metal backend on Apple Silicon (D-001) and no OpenMP on any platform
# (single binary). Inspects exactly the build cargo uses now, never a stale one.
# Extra arguments go to `cargo build` (e.g. --release).
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/platform-floors.sh
if [ "$(uname -s)" = "Darwin" ]; then
  # .cargo/config.toml pins the build target; it must match the documented floor.
  configured=$(configured_macos_target)
  if ! version_le "${configured:-0}" "$MACOS_MIN" || ! version_le "$MACOS_MIN" "${configured:-0}"; then
    echo ".cargo/config.toml targets macOS ${configured:-none}, expected { value = \"$MACOS_MIN.0\", force = true }" >&2
    exit 1
  fi
fi
# Compiler errors stay visible on stderr; only the JSON stream is captured.
if ! json=$(cargo build --locked --message-format=json-render-diagnostics "$@"); then
  echo "cargo build failed (see the errors above)" >&2
  exit 1
fi
message=$(printf '%s\n' "$json" | grep '"reason":"build-script-executed"' \
  | grep -E '"package_id":"[^"]*#llama-cpp-sys-2@' | tail -1 || true)
if [ -z "$message" ]; then
  echo "cargo build did not report a llama-cpp-sys-2 build" >&2
  exit 1
fi
if ! grep -q '"linked_libs":\[' <<<"$message"; then
  echo "cargo's build-script message has no linked_libs field (format changed?)" >&2
  exit 1
fi
# Linked library names, one per line, from cargo's documented JSON (`linked_libs`).
# An entry is `[kind[:modifiers]=]name`; only the name is kept. Captured into a
# variable first: `grep -q` at the end of a pipe can exit early, and under
# pipefail the writer's SIGPIPE would turn a match into a failure.
names=$(printf '%s\n' "$message" | sed -E 's/.*"linked_libs":\[([^]]*)\].*/\1/' \
  | tr ',' '\n' | tr -d '"' | sed -E 's/^.*=//')
out_dir=$(printf '%s\n' "$message" | sed -E 's/.*"out_dir":"([^"]*)".*/\1/')
# JSON escapes backslashes: a Windows path arrives as `D:\\a\\...`.
out_dir=$(sed 's/\\\\/\\/g' <<<"$out_dir")
if command -v cygpath >/dev/null 2>&1; then
  out_dir=$(cygpath -u "$out_dir")
fi
cache="$out_dir/build/CMakeCache.txt"
if [ ! -f "$cache" ]; then
  echo "expected CMake cache is missing (llama-cpp-sys-2 layout changed?): $cache" >&2
  exit 1
fi
if ! grep -qx 'GGML_OPENMP:BOOL=OFF' "$cache" || grep -qxE '(gomp|omp|iomp5)' <<<"$names"; then
  echo "OpenMP is enabled or linked: $cache" >&2
  exit 1
fi
if [ "$(uname -s)-$(uname -m)" = "Darwin-arm64" ] && ! grep -qx 'ggml-metal' <<<"$names"; then
  echo "the Metal backend was not built" >&2
  exit 1
fi
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    # ggml must be built for the documented CPU floor, never for the build
    # machine. The floor's Rust target features must be in .cargo/config.toml
    # (an environment RUSTFLAGS would replace them, so it is refused too).
    if [ -n "${RUSTFLAGS:-}" ] || [ -n "${CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS:-}" ]; then
      echo "RUSTFLAGS is set; it would replace the CPU floor and static CRT of .cargo/config.toml" >&2
      exit 1
    fi
    for feature in $WINDOWS_CPU_FLOOR crt-static; do
      if ! grep -qE "target-feature=[^\"]*\+$feature([,\"]|$)" .cargo/config.toml; then
        echo ".cargo/config.toml does not enable +$feature for Windows (floor: $WINDOWS_CPU_FLOOR)" >&2
        exit 1
      fi
    done
    # MSVC: GGML_FMA and GGML_F16C are not CMake options (implied by AVX2),
    # so they stay untyped in the cache; accept any type.
    for flag in GGML_AVX2 GGML_BMI2 GGML_FMA GGML_F16C; do
      if ! grep -qxE "$flag(:[A-Z]+)?=ON" "$cache"; then
        echo "llama.cpp was not built with $flag (floor: $WINDOWS_CPU_FLOOR; see .cargo/config.toml): $cache" >&2
        exit 1
      fi
    done
    # The static runtime comes from packaging/windows/static-crt.cmake; a
    # build directory configured before it (or with another toolchain file)
    # still has the DLL runtime and fails to link with +crt-static.
    if ! grep -qE '^CMAKE_TOOLCHAIN_FILE:[A-Z]*=.*packaging[/\\]windows[/\\]static-crt\.cmake$' "$cache"; then
      echo "llama.cpp was not configured with packaging/windows/static-crt.cmake (static C runtime);" \
           "run: cargo clean -p llama-cpp-sys-2" >&2
      exit 1
    fi
    if ! grep -qx 'GGML_NATIVE:BOOL=OFF' "$cache"; then
      echo "llama.cpp was built for the build machine (GGML_NATIVE): $cache" >&2
      exit 1
    fi
    ;;
esac
if [ "$(uname -s)" = "Darwin" ]; then
  # Every archive under OUT_DIR: CMake's lib/ and the cc-built wrappers.
  objects=$(find "$out_dir" -name '*.a' -exec otool -l {} +)
  highest=$(highest_minos <<<"$objects")
  if [ -z "$highest" ] || ! version_le "$highest" "$MACOS_MIN"; then
    echo "llama.cpp objects target macOS ${highest:-unknown}, above the supported $MACOS_MIN" >&2
    exit 1
  fi
fi
echo "llama build ok: $out_dir"

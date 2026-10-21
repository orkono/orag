#!/usr/bin/env bash
# Tests the dependency allow-lists of check-binary-deps.sh (platform-floors.sh):
# the CPU `orag` may import only system libraries; `orag-vulkan` may add the
# system Vulkan loader and nothing else (D-001, v0.6 plan Task 1).
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/platform-floors.sh
fail() { echo "FAIL: $*" >&2; exit 1; }

# disallowed_deps OS FLAVOR <deps: the libraries that are not allowed, one per line.
expect_none() { local out; out=$(disallowed_deps "$1" "$2" <<<"$3"); [ -z "$out" ] || fail "$1/$2 refused: $out"; }
expect_only() { local out; out=$(disallowed_deps "$1" "$2" <<<"$3"); [ "$out" = "$4" ] || fail "$1/$2 expected '$4', got '$out'"; }

windows_base=$'kernel32.dll\nntdll.dll\nbcrypt.dll'
expect_none windows cpu "$windows_base"
expect_only windows cpu "$windows_base"$'\nvulkan-1.dll' "vulkan-1.dll"
expect_none windows vulkan "$windows_base"$'\nvulkan-1.dll'
expect_only windows vulkan "$windows_base"$'\nvulkan-1.dll\nvcruntime140.dll' "vcruntime140.dll"
expect_only windows vulkan "$windows_base"$'\nmsvcp140.dll' "msvcp140.dll"

linux_base=$'linux-vdso.so.1\n/lib/x86_64-linux-gnu/libc.so.6\n/lib64/ld-linux-x86-64.so.2\n/lib/x86_64-linux-gnu/libstdc++.so.6'
linux_vulkan=/lib/x86_64-linux-gnu/libvulkan.so.1
expect_none linux cpu "$linux_base"
expect_only linux cpu "$linux_base"$'\n'"$linux_vulkan" "$linux_vulkan"
expect_none linux vulkan "$linux_base"$'\n'"$linux_vulkan"
expect_only linux vulkan "$linux_base"$'\n'"$linux_vulkan"$'\n/opt/sdk/lib/libvulkan.so.1' "/opt/sdk/lib/libvulkan.so.1"
expect_only linux vulkan "$linux_base"$'\n/usr/lib/x86_64-linux-gnu/libgomp.so.1' "/usr/lib/x86_64-linux-gnu/libgomp.so.1"

expect_none macos cpu $'/usr/lib/libc++.1.dylib\n/System/Library/Frameworks/Metal.framework/Versions/A/Metal'
expect_only macos cpu $'/opt/homebrew/lib/libvulkan.1.dylib' "/opt/homebrew/lib/libvulkan.1.dylib"
disallowed_deps macos vulkan <<<"" >/dev/null 2>&1 && fail "a macOS Vulkan flavour was accepted"
disallowed_deps windows metal <<<"" >/dev/null 2>&1 && fail "an unknown flavour was accepted"
echo "binary dependency allow-lists ok"

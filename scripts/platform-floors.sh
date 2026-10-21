# Supported platform floors (README "system requirements", D-001). Sourced by
# check-llama-build.sh and check-binary-deps.sh; .cargo/config.toml sets
# MACOSX_DEPLOYMENT_TARGET to MACOS_MIN and check-llama-build.sh verifies that.
MACOS_MIN=14
GLIBC_MIN=2.35
GLIBCXX_MIN=3.4.30
CXXABI_MIN=1.3.13
GCC_MIN=7.0.0   # libgcc_s: Rust needs only old GCC_ versions
# Windows x86-64 CPU floor (Haswell 2013+, AMD Zen): .cargo/config.toml turns
# these target features on, and check-llama-build.sh checks ggml was built for them.
WINDOWS_CPU_FLOOR="avx2 fma f16c bmi2"
# DLLs a Windows binary may import: system libraries present on every
# Windows 10/11. Static CRT, so no VCRUNTIME*/MSVCP*/api-ms-win-crt-*.
WINDOWS_SYSTEM_DLLS='^(kernel32|kernelbase|advapi32|ntdll|ws2_32|bcrypt|bcryptprimitives|userenv|shell32|ole32|oleaut32|crypt32|secur32|user32|gdi32|shlwapi|iphlpapi|dbghelp|psapi|powrprof|synchronization|api-ms-win-core-[a-z0-9-]+)\.dll$'
# The Vulkan loader, which a GPU driver installs: allowed only in `orag-vulkan`
# (v0.6), from the system library directories on Linux.
WINDOWS_VULKAN_DLL='^vulkan-1\.dll$'
LINUX_SYSTEM_LIBS='^(linux-vdso\.so[.0-9]*|/(usr/)?lib(64)?/([^/]+/)?(ld-linux[^/]*|lib(c|m|dl|rt|pthread|gcc_s|stdc\+\+))\.so[.0-9]*)$'
LINUX_VULKAN_LIB='^/(usr/)?lib(64)?/([^/]+/)?libvulkan\.so\.1$'
MACOS_SYSTEM_LIBS='^(/usr/lib/|/System/Library/)'

# disallowed_deps OS FLAVOR: prints the dependencies read from stdin (one per
# line: a lower-case DLL name on Windows, a resolved path on Linux, an install
# name on macOS) that FLAVOR may not have. OS: windows, linux, macos; FLAVOR:
# cpu (the `orag` build) or vulkan (`orag-vulkan`; not on macOS).
disallowed_deps() {
  local os="$1" flavor="$2" allowed
  case "$os/$flavor" in
    windows/cpu) allowed="$WINDOWS_SYSTEM_DLLS" ;;
    windows/vulkan) allowed="$WINDOWS_SYSTEM_DLLS|$WINDOWS_VULKAN_DLL" ;;
    linux/cpu) allowed="$LINUX_SYSTEM_LIBS" ;;
    linux/vulkan) allowed="$LINUX_SYSTEM_LIBS|$LINUX_VULKAN_LIB" ;;
    macos/cpu) allowed="$MACOS_SYSTEM_LIBS" ;;
    *) echo "no dependency allow-list for $os/$flavor" >&2; return 2 ;;
  esac
  grep -vE "$allowed" || true
}

# version_le A B: true if version A <= B ("14" and "14.0" are equal).
version_le() {
  local a b
  a=$(sed -E 's/(\.0)+$//' <<<"$1")
  b=$(sed -E 's/(\.0)+$//' <<<"$2")
  [ "$(printf '%s\n%s\n' "$a" "$b" | sort -V | tail -1)" = "$b" ]
}

# highest_version PREFIX: highest PREFIX_x.y version in stdin, or empty.
highest_version() {
  grep -oE "\\b$1_[0-9]+(\\.[0-9]+)*\\b" | sed "s/^$1_//" | sort -uV | tail -1 || true
}

# highest_minos: highest LC_BUILD_VERSION minos in `otool -l` output (stdin).
highest_minos() {
  awk '/LC_BUILD_VERSION/ {b=1} b && $1 == "minos" {print "MINOS_" $2; b=0}' | highest_version MINOS
}

# configured_macos_target: the value .cargo/config.toml pins (run from the repo root).
configured_macos_target() {
  sed -nE 's/^MACOSX_DEPLOYMENT_TARGET *= *\{ *value *= *"([^"]+)" *, *force *= *true *\}.*/\1/p' .cargo/config.toml
}

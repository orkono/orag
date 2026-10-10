# CMake toolchain file for the llama.cpp build on x86_64-pc-windows-msvc only:
# .cargo/config.toml points cmake-rs at it through the target-specific
# CMAKE_TOOLCHAIN_FILE_x86_64_pc_windows_msvc. llama.cpp declares policies up
# to CMake 3.28, so CMP0091 is NEW and the runtime library comes from this
# variable (default: the DLL runtime, /MD), not from the /MT flag cmake-rs
# adds for LLAMA_STATIC_CRT. Rust links the static runtime (+crt-static), so
# both must use it, or link.exe misses every __imp_ CRT symbol (D-001).
# The release runtime also in Rust debug builds: llama-cpp-sys-2 builds
# llama.cpp as RelWithDebInfo, and Rust always links the release CRT.
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded")

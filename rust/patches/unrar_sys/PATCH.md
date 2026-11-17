# unrar_sys 0.5.8, patched

Upstream: https://crates.io/crates/unrar_sys (MIT OR Apache-2.0; the UnRAR source in
`vendor/unrar` is under RARLAB's licence, `vendor/unrar/license.txt`).

One change, in `build.rs`: the Windows branch (link `powrprof` and `shell32`, compile
`isnt.cpp`, no `pthread`) is chosen by `CARGO_CFG_TARGET_OS` instead of `cfg!(windows)`, which
reads the machine running the build. Without it a Windows build cross-compiled on macOS
(`scripts/package-windows.sh`, cargo-xwin) asks for `pthread.lib` and misses `isnt.cpp`.
Drop this patch when upstream checks the target.

#!/usr/bin/env bash
# Source this before using cargo/rustc. The Rust toolchain lives on the
# persistent /aux mount (NOT ~/.cargo, which is wiped on container rebuild).
export CARGO_HOME=/aux/koutecky/logseq/.toolchain/cargo
export RUSTUP_HOME=/aux/koutecky/logseq/.toolchain/rustup
export PATH="$CARGO_HOME/bin:$PATH"

# Playwright browser + the few shared libs we extracted locally (no root), so
# headless Chromium screenshots work in this sandbox.
export PLAYWRIGHT_BROWSERS_PATH=/aux/koutecky/logseq/.toolchain/ms-playwright
export LD_LIBRARY_PATH="/aux/koutecky/logseq/.toolchain/extralibs/root/usr/lib/x86_64-linux-gnu:$LD_LIBRARY_PATH"

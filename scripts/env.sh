#!/usr/bin/env bash
# Source this before using cargo/rustc. The Rust toolchain lives on the
# persistent /aux mount (NOT ~/.cargo, which is wiped on container rebuild).
export CARGO_HOME=/aux/koutecky/logseq/.toolchain/cargo
export RUSTUP_HOME=/aux/koutecky/logseq/.toolchain/rustup
export PATH="$CARGO_HOME/bin:$PATH"

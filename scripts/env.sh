#!/usr/bin/env bash
# Source this before using cargo/rustc. The Rust toolchain + headless-browser deps
# live OUTSIDE the repo, on a persistent mount (NOT ~/.cargo, which is wiped on
# container rebuild) — by default a `.toolchain/` dir alongside the repo.
#
# Override the location by exporting TINE_TOOLCHAIN before sourcing, e.g.
#   TINE_TOOLCHAIN=$HOME/.tine-toolchain source scripts/env.sh

# Resolve the repo root from this script's own path (works when sourced), then
# default the toolchain to a sibling `.toolchain/` of the repo.
_env_sh_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
_repo_root="$(cd "$_env_sh_dir/.." && pwd)"
: "${TINE_TOOLCHAIN:=$(cd "$_repo_root/.." && pwd)/.toolchain}"

export CARGO_HOME="$TINE_TOOLCHAIN/cargo"
export RUSTUP_HOME="$TINE_TOOLCHAIN/rustup"
export PATH="$CARGO_HOME/bin:$PATH"

# Playwright browser + the few shared libs we extracted locally (no root), so
# headless Chromium screenshots work in this sandbox.
export PLAYWRIGHT_BROWSERS_PATH="$TINE_TOOLCHAIN/ms-playwright"
export LD_LIBRARY_PATH="$TINE_TOOLCHAIN/extralibs/root/usr/lib/x86_64-linux-gnu:$LD_LIBRARY_PATH"

unset _env_sh_dir _repo_root

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

# The extracted WebKitGTK/GTK development bundle carries both libraries and its
# pkg-config metadata. Point native builds at that sysroot explicitly; relying on
# a caller's PKG_CONFIG_PATH made clean release builds fail even though the
# bundled .pc files were present alongside the libraries.
export PKG_CONFIG_SYSROOT_DIR="$TINE_TOOLCHAIN/extralibs/root"
export PKG_CONFIG_PATH="$TINE_TOOLCHAIN/extralibs/root/usr/lib/x86_64-linux-gnu/pkgconfig:$TINE_TOOLCHAIN/extralibs/root/usr/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"

# Some host sessions inherit a half-active Conda toolchain (CC points at a
# compiler that is no longer on PATH, while flags still point into Conda's
# sysroot). Native Rust crates such as ring obey those variables and then fail
# before rustc runs. Drop only stale/Conda-specific overrides; valid caller
# toolchain choices remain intact.
_drop_missing_tool_override() {
  local var="$1"
  local value="${!var:-}"
  if [ -n "$value" ] && ! command -v "${value%% *}" >/dev/null 2>&1; then
    unset "$var"
  fi
}
for _tool_var in CC CXX AR RANLIB NM LD STRIP; do
  _drop_missing_tool_override "$_tool_var"
done
unset -f _drop_missing_tool_override
unset _tool_var
case "${CFLAGS:-}" in *conda*) unset CFLAGS ;; esac
case "${CXXFLAGS:-}" in *conda*) unset CXXFLAGS ;; esac
case "${CPPFLAGS:-}" in *conda*) unset CPPFLAGS ;; esac
case "${LDFLAGS:-}" in *conda*) unset LDFLAGS ;; esac

unset _env_sh_dir _repo_root

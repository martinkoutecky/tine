#!/usr/bin/env bash
# Source scripts/env.sh first; run this inside xvfb-run + dbus-run-session.
# Debian bookworm packages are extracted locally; never install into the host.
set -eu
export SPIKE_IME_OUT="${SPIKE_IME_OUT:-$PWD/artifacts/spike-cm6-stage1/${SPIKE_IME_RUN:-ime}}"
export IMROOT="${IMROOT:-$PWD/artifacts/spike-cm6-stage1/ibus-root}"
mkdir -p "$SPIKE_IME_OUT"/{data,config,cache,runtime,components}
export XDG_DATA_HOME="$SPIKE_IME_OUT/data"
export XDG_CONFIG_HOME="$SPIKE_IME_OUT/config"
export XDG_CACHE_HOME="$SPIKE_IME_OUT/cache"
export XDG_RUNTIME_DIR="$SPIKE_IME_OUT/runtime"
chmod 700 "$XDG_RUNTIME_DIR"
export LD_LIBRARY_PATH="$IMROOT/usr/lib/x86_64-linux-gnu:${LD_LIBRARY_PATH:-}"
# SPIKE_XDOTOOL_DIR: a directory holding xdotool when it is not on PATH (default: none).
# SPIKE_OPENBOX: the openbox binary used when SPIKE_IME_WM=1 (default: openbox on PATH).
export PATH="$IMROOT/usr/bin${SPIKE_XDOTOOL_DIR:+:$SPIKE_XDOTOOL_DIR}:$PATH"
export IBUS_COMPONENT_PATH="$SPIKE_IME_OUT/components"
sed "s|/usr/libexec/|$IMROOT/usr/libexec/|g" "$IMROOT/usr/share/ibus/component/hangul.xml" > "$IBUS_COMPONENT_PATH/hangul.xml"
sed "s|/usr/libexec/|$IMROOT/usr/libexec/|g" "$IMROOT/usr/share/ibus/component/simple.xml" > "$IBUS_COMPONENT_PATH/simple.xml"
export GSETTINGS_SCHEMA_DIR="$IMROOT/usr/share/glib-2.0/schemas"
glib-compile-schemas "$GSETTINGS_SCHEMA_DIR"
export GSETTINGS_BACKEND=keyfile
gsettings set org.freedesktop.ibus.engine.hangul initial-input-mode hangul
gsettings set org.freedesktop.ibus.engine.hangul on-keys F6
export GTK_IM_MODULE=ibus
unset IBUS_USE_PORTAL
export XMODIFIERS=@im=ibus
export GTK_IM_MODULE_FILE="$SPIKE_IME_OUT/immodules.cache"
/usr/lib/x86_64-linux-gnu/libgtk-3-0/gtk-query-immodules-3.0 "$IMROOT/usr/lib/x86_64-linux-gnu/gtk-3.0/3.0.0/immodules/im-ibus.so" > "$GTK_IM_MODULE_FILE"
# Unix socket paths must fit sockaddr_un; the worktree path is too long.
export IBUS_ADDRESS="unix:abstract=tine-cm6-ime-$$"
"$IMROOT/usr/bin/ibus-daemon" --config="$IMROOT/usr/libexec/ibus-dconf" --panel=disable --emoji-extension=disable --address "$IBUS_ADDRESS" --cache=none > "$SPIKE_IME_OUT/ibus.log" 2>&1 &
ibus_pid=$!
trap 'kill "$ibus_pid" 2>/dev/null || true' EXIT
sleep 1
"$IMROOT/usr/libexec/ibus-engine-hangul" --ibus > "$SPIKE_IME_OUT/hangul.log" 2>&1 &
hangul_pid=$!
trap 'kill "$hangul_pid" "$ibus_pid" 2>/dev/null || true' EXIT
if [ "${SPIKE_IME_PROBE:-0}" = 1 ]; then
  sleep 1
  timeout 8 ibus engine hangul
  ibus engine
  exit
fi
if [ "${SPIKE_IME_GTK_PROBE:-0}" = 1 ]; then
  "$PWD/artifacts/spike-cm6-stage1/gtk-probe" > "$SPIKE_IME_OUT/gtk-probe.log" 2>&1 &
  gtk_pid=$!
  sleep 1
  xdotool search --name "Tine IME control" windowfocus --sync
  ibus engine hangul
  xdotool key --delay 200 g k s
  sleep 2
  import -window root "$SPIKE_IME_OUT/gtk-probe.png"
  kill "$gtk_pid"
  exit
fi
if [ "${SPIKE_IME_WM:-0}" = 1 ]; then
  "${SPIKE_OPENBOX:-openbox}" > "$SPIKE_IME_OUT/openbox.log" 2>&1 &
  wm_pid=$!
  trap 'kill "$wm_pid" "$hangul_pid" "$ibus_pid" 2>/dev/null || true' EXIT
  sleep 1
fi
if [ -n "${SPIKE_IME_RUNNER:-}" ]; then
  bash "$SPIKE_IME_RUNNER"
else
  node "${SPIKE_IME_SCRIPT:-scripts/spike-cm6-ime.mjs}"
fi

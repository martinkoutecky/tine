// The ONE X11 window helper for native Linux journeys that drive the window
// manager's own frame (e2e-native-titlebar, e2e-pdf-ownership).
//
// Two facts both journeys depend on:
//  - xdotool's --shell Y coordinate double-counts Openbox's reparented titlebar
//    in this environment. `xwininfo` reports the actual client origin, which is
//    the coordinate _NET_FRAME_EXTENTS is defined around.
//  - Tauri/Openbox can also expose a tiny same-title helper surface, so the
//    graph window is the LARGEST visible match and owns the real frame.
//
// A journey chooses its own policy for a window with no frame-extents property
// (`missingAsZero`): an undecorated window commonly has none, which is the
// expected pre-toggle state for the titlebar journey, while the PDF journey
// requires a decorated frame and wants the malformed answer to throw.
import { execFileSync } from "node:child_process";

/** Read the client origin and size out of `xwininfo -id` output. */
export function parseXwininfo(raw, id) {
  const read = (label) => {
    const value = raw.match(new RegExp(`^\\s*${label}:\\s*(-?\\d+)`, "m"))?.[1];
    if (value === undefined) throw new Error(`xwininfo omitted ${label}: ${raw.trim()}`);
    return Number(value);
  };
  return {
    WINDOW: Number(id),
    X: read("Absolute upper-left X"),
    Y: read("Absolute upper-left Y"),
    WIDTH: read("Width"),
    HEIGHT: read("Height"),
  };
}

/** Read `{left,right,top,bottom}` out of `xprop _NET_FRAME_EXTENTS` output. */
export function parseFrameExtents(raw, { missingAsZero = false } = {}) {
  const values = raw.match(/=\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+)/)?.slice(1).map(Number);
  if (!values && missingAsZero && /not found/i.test(raw)) {
    return { left: 0, right: 0, top: 0, bottom: 0 };
  }
  if (!values) throw new Error(`window manager exposed malformed frame extents: ${raw.trim()}`);
  const [left, right, top, bottom] = values;
  return { left, right, top, bottom };
}

/** Window ids in descending area, so the first is the graph window. */
export function largestFirst(ids, geometryOf) {
  return [...ids].sort((a, b) => {
    try {
      const ga = geometryOf(a);
      const gb = geometryOf(b);
      return gb.WIDTH * gb.HEIGHT - ga.WIDTH * ga.HEIGHT;
    } catch {
      return 0;
    }
  });
}

/** Bind the X11 helpers to a journey's process environment. */
export function x11Tools(env, { xdotool = process.env.E2E_XDOTOOL || "xdotool" } = {}) {
  const xdoEnv = process.env.E2E_XDOTOOL_LIB
    ? { ...env, LD_LIBRARY_PATH: process.env.E2E_XDOTOOL_LIB }
    : env;
  const xdo = (...args) => execFileSync(xdotool, args, { encoding: "utf8", env: xdoEnv }).trim();
  const geometry = (id) => parseXwininfo(execFileSync("xwininfo", ["-id", id], { encoding: "utf8", env }), id);
  const windowIds = () => {
    try {
      // xdotool uses POSIX extended regular expressions (no `(?:...)`).
      const ids = xdo("search", "--onlyvisible", "--name", "^Tine( — .*)?$").split(/\s+/).filter(Boolean);
      return largestFirst(ids, geometry);
    } catch {
      return [];
    }
  };
  const frameExtents = (id, options) => parseFrameExtents(
    execFileSync("xprop", ["-id", id, "_NET_FRAME_EXTENTS", "_GTK_FRAME_EXTENTS"], { encoding: "utf8", env }),
    options,
  );
  return { xdo, geometry, windowIds, frameExtents };
}

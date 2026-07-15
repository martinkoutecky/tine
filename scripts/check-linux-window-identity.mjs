#!/usr/bin/env node

// Linux shell identity regression. Wayland compositors resolve a window icon by
// matching xdg_toplevel.app_id to a desktop-entry basename. Keep that identity
// stable without enabling GTK's own unique-instance routing, which would consume
// second launches before Tine can forward `--capture` through its plugin.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const config = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"));
const identityPath = path.join(root, "src-tauri/src/linux_window_identity.rs");
const libPath = path.join(root, "src-tauri/src/lib.rs");
const graphPath = path.join(root, "src-tauri/src/graph.rs");
const cargoPath = path.join(root, "src-tauri/Cargo.toml");

function requireMatch(text, pattern, message) {
  if (!pattern.test(text)) throw new Error(message);
}

if (config.identifier !== "page.tine.Tine") {
  throw new Error(`Linux desktop identity drifted: ${config.identifier}`);
}
if (config.app?.enableGTKAppId === true) {
  throw new Error(
    "enableGTKAppId makes GTK consume second launches before Tine's single-instance --capture forwarding",
  );
}
if (!fs.existsSync(identityPath)) {
  throw new Error("missing Linux per-window Wayland identity implementation");
}

const identity = fs.readFileSync(identityPath, "utf8");
const lib = fs.readFileSync(libPath, "utf8");
const graph = fs.readFileSync(graphPath, "utf8");
const cargo = fs.readFileSync(cargoPath, "utf8");

requireMatch(
  identity,
  /const APP_ID: &str = "page\.tine\.Tine";/,
  "Linux shell app ID drifted from page.tine.Tine",
);
requireMatch(
  identity,
  /gdk_wayland_window_set_application_id/,
  "Wayland windows do not advertise page.tine.Tine",
);
requireMatch(
  identity,
  /SignalId::lookup\("xdg-toplevel-realized"[\s\S]*connect_local_id[\s\S]*set_wayland_app_id/,
  "newer GTK Wayland windows do not assign the app ID before the first surface commit",
);
requireMatch(
  identity,
  /connect_map\(set_mapped_wayland_app_id\)/,
  "older GTK Wayland windows do not update the app ID after xdg_toplevel mapping",
);
requireMatch(
  identity,
  /page\.tine\.Tine\.desktop[\s\S]*Icon=page\.tine\.Tine/,
  "raw Linux runs do not provide the desktop entry used for Wayland icon lookup",
);
requireMatch(
  lib,
  /install_desktop_identity\(\)/,
  "desktop identity is not installed before the Tauri event loop starts",
);
requireMatch(
  lib,
  /get_webview_window\("main"\)[\s\S]*apply_to_window\(&window\)/,
  "configured main window does not receive the Linux shell identity",
);
requireMatch(
  lib,
  /get_webview_window\("capture"\)[\s\S]*apply_to_window\(&window\)/,
  "configured capture window does not receive the Linux shell identity",
);
requireMatch(
  graph,
  /apply_to_window\(&window\)/,
  "dynamically-created graph windows do not receive the Linux shell identity",
);
requireMatch(
  cargo,
  /^x11 = "2\.21\.0"$/m,
  "Linux builds do not directly link the Xlib thread initializer",
);
requireMatch(
  lib,
  /fn init_xlib_threads\(\)[\s\S]*x11::xlib::XInitThreads\(\)/,
  "Linux startup does not initialize Xlib for the secondary-instance handoff",
);
requireMatch(
  lib,
  /pub fn run\(\) \{\s*#\[cfg\(target_os = "linux"\)\]\s*init_xlib_threads\(\);/,
  "Xlib thread initialization must precede every GTK, Xlib, and Tauri startup call",
);

console.log("linux native startup OK: page.tine.Tine identity + Xlib-safe single-instance handoff");

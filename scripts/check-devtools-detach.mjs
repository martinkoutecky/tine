import fs from "node:fs";

const source = fs.readFileSync(new URL("../src-tauri/src/commands.rs", import.meta.url), "utf8");
const start = source.indexOf("pub(crate) fn tine_open_devtools");
const end = source.indexOf("\n#[tauri::command]", start + 1);
if (start < 0 || end < 0) throw new Error("could not locate tine_open_devtools");
const body = source.slice(start, end);

const attach = body.indexOf("connect_attach");
const open = body.lastIndexOf("window.open_devtools()");
if (attach < 0) throw new Error("Linux devtools must subscribe to WebKit's attach signal");
if (open < 0 || attach > open) throw new Error("the attach hook must be installed before devtools open");
if (!body.includes("idle_add_local_once")) {
  throw new Error("detach must run after WebKit finishes the attach signal, without a timer");
}
if (!body.includes("disconnect(handler_id)")) {
  throw new Error("the automatic detach hook must disconnect after its first attach event");
}
if (/sleep\s*\(|timeout_add|setTimeout/.test(body)) {
  throw new Error("devtools detach must not depend on a timeout");
}

console.log("Developer-tools detach lifecycle OK: hook-before-open, one-shot, timer-free.");

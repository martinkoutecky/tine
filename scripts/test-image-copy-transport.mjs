// GH #700: run after Cargo has fetched the pinned Tauri source. Exercise the
// actual Android / desktop fallback serializers, rather than mocking invoke.
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import vm from "node:vm";
import assert from "node:assert/strict";

const lock = fs.readFileSync("Cargo.lock", "utf8");
const version = lock.match(/name = "tauri"\nversion = "([^"]+)"/)[1];
const registry = path.join(process.env.CARGO_HOME || path.join(os.homedir(), ".cargo"), "registry/src");
const source = fs.readdirSync(registry).map(dir => path.join(registry, dir, `tauri-${version}`))
  .find(dir => fs.existsSync(path.join(dir, "scripts/ipc-protocol.js")));
assert.ok(source, "fetch the locked Tauri crate before running the transport test");
const processMessage = fs.readFileSync(path.join(source, "scripts/process-ipc-message-fn.js"), "utf8");
const protocol = fs.readFileSync(path.join(source, "scripts/ipc-protocol.js"), "utf8");
assert.match(fs.readFileSync(path.join(source, "src/ipc/mod.rs"), "utf8"), /On Android, \[InvokeBody::Raw\] is not supported/);

async function wirePayload(osName, payload, failCustomProtocol = false) {
  let message;
  let raw;
  const window = {
    __TAURI_INTERNALS__: { convertFileSrc: cmd => `ipc://${cmd}`, runCallback() {} },
    ipc: { postMessage: data => { message = JSON.parse(data); } },
  };
  const context = vm.createContext({ window, Uint8Array, ArrayBuffer, Headers, console: { warn() {} },
    fetch: async (_url, init) => {
      if (failCustomProtocol) throw new Error("custom protocol blocked");
      raw = init.body;
      return { headers: new Headers({ "Tauri-Response": "ok", "content-type": "application/json" }), json: async () => null };
    },
  });
  vm.runInContext(protocol.replace("__TEMPLATE_invoke_key__", '"test-key"')
    .replace("__RAW_process_ipc_message_fn__", processMessage)
    .replace("__TEMPLATE_os_name__", JSON.stringify(osName))
    .replace("__TEMPLATE_fetch_channel_data_command__", '"plugin:__TAURI_CHANNEL__|fetch"'), context);
  window.__TAURI_INTERNALS__.postMessage({ cmd: "copy_image_to_clipboard", callback: 1, error: 2, payload });
  // Drain fetch/response/fallback promise callbacks.
  await new Promise(resolve => setImmediate(resolve));
  return message?.payload ?? raw;
}

// Same payloads pinned by backend.imageClipboard.test.ts and consumed in
// platform.rs::clipboard_accepts_android_json_and_desktop_transport_fallback.
assert.deepEqual(await wirePayload("android", { bytesB64: "iVBORw==" }), { bytesB64: "iVBORw==" });
const bytes = new Uint8Array([137, 80, 78, 71]);
assert.equal(await wirePayload("linux", bytes), bytes);
assert.deepEqual(await wirePayload("linux", bytes, true), [137, 80, 78, 71]);
console.log(`PASS Tauri ${version}: Android base64 JSON, desktop raw, desktop postMessage byte-array fallback`);

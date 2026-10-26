#!/usr/bin/env node
// Real native dialog → loopback GitHub → encrypted gist → production browser
// viewer. Only disposable graph/app data under artifacts are read or written.
import fs from "node:fs";
import path from "node:path";
import http from "node:http";
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { remote } from "webdriverio";
import { checkViewer } from "./check-quick-share-viewer.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { answerNativeDialog } from "./lib/e2e-native-dialog.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/quick-share-viewer/native")); fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "journey-"));
const graph = path.join(out, "graph");
for (const dir of ["graph/pages", "graph/journals", "graph/logseq", "data", "config", "cache"]) fs.mkdirSync(path.join(out, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
// An author selects both pages through a query. The page-to-page link must
// work in the viewer with the original secret prefix intact.
fs.writeFileSync(path.join(graph, "pages/Project.md"), "- Native share sentinel\n- [[Notes ü]]\n- {{tine-query @page and (name = 'Project' or name = 'Notes ü')}}\n");
fs.writeFileSync(path.join(graph, "pages/Notes ü.md"), "- Native linked page sentinel\n");
fs.writeFileSync(path.join(graph, "pages/Exports.md"), "tine/exports:: true\npublic:: false\n\n- {{query (or (property export) (property tine.share))}}\n");
const gist = "1234567890abcdef1234567890abcdef";
let envelope, creates = 0, deletes = 0, updates = 0;
const mock = http.createServer(async (req, res) => {
  const chunks = []; for await (const chunk of req) chunks.push(chunk);
  const body = Buffer.concat(chunks).toString("utf8");
  res.setHeader("content-type", "application/json");
  if (req.url === "/login/device/code") return res.end(JSON.stringify({ device_code: "mock-private-code", user_code: "MOCK-CODE", verification_uri: "https://github.com/login/device", expires_in: 300, interval: 1 }));
  if (req.url === "/login/oauth/access_token") return res.end(JSON.stringify({ access_token: "gho_disposableMockToken", token_type: "bearer", scope: "gist" }));
  if (req.url === "/user") { res.setHeader("x-oauth-scopes", "gist"); return res.end(JSON.stringify({ login: "octo-cat", id: 4242 })); }
  if (req.method === "POST" && req.url === "/gists") {
    creates++; envelope = JSON.parse(body).files["tine-share.bin"].content;
    res.statusCode = 201; return res.end(JSON.stringify({ id: gist, owner: { login: "octo-cat", id: 4242 } }));
  }
  if ((req.method === "GET" || req.method === "PATCH") && req.url === `/gists/${gist}`) {
    if (req.method === "PATCH") {
      if (req.headers["if-match"]) throw new Error("undocumented conditional PATCH");
      updates++; envelope = JSON.parse(body).files["tine-share.bin"].content;
    }
    const revision = (updates ? "b" : "a").repeat(40);
    return res.end(JSON.stringify({ id: gist, description: "Tine share (encrypted)", owner: { id: 4242 },
      history: [{ version: revision }], files: { "README.md": {}, "tine-share.bin": {
        raw_url: `http://127.0.0.1:${mock.address().port}/octo-cat/${gist}/raw/${revision}/tine-share.bin`,
      } } }));
  }
  if (req.method === "GET" && req.url.includes("/raw/")) { res.setHeader("content-type", "text/plain"); return res.end(envelope); }
  if (req.method === "DELETE" && req.url === `/gists/${gist}`) {
    deletes++; res.statusCode = 204; return res.end();
  }
  res.statusCode = 404; res.end("{}");
});
await new Promise((resolve) => mock.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${mock.address().port}`;
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"), steps: [] };
const port = Number(process.env.E2E_DRIVER_PORT || 4866);
const env = { ...process.env, TINE_GRAPH: graph, TINE_QUICK_SHARE_TEST_ORIGIN: origin,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser, journey = "startup";
function mark(value) { journey = value; proof.steps.push(value); console.log(value); }
try {
  const deadline = Date.now() + 15_000;
  while (true) { try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {} if (Date.now() > deadline) throw new Error("driver startup failed"); await new Promise((resolve) => setTimeout(resolve, 100)); }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 30_000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser); await browser.setWindowSize(1400, 1000);
  await browser.$(".page-title").waitForExist({ timeout: 30_000 }); await openPageByName(browser, "Project");
  mark("share dialog selects pages with a query");
  await browser.$(".query-block .qc-share").waitForExist({ timeout: 20_000 });
  await browser.execute(() => document.querySelector(".query-block .qc-share").click());
  await browser.$(".share-dialog").waitForExist({ timeout: 15_000 });
  await browser.waitUntil(() => browser.execute(() => document.querySelector('.share-dialog .query-export-pages')?.textContent?.includes("Notes ü")), { timeout: 20_000 });
  await browser.execute(() => { const radio = document.querySelector('.share-dialog input[name="share-where"][value="link"]'); if (radio.disabled) throw new Error("test binary lacks the local viewer compatibility attestation"); radio.click(); });
  await browser.execute(() => document.querySelector(".share-dialog .share-export").click());
  await browser.$(".quick-share-dialog").waitForExist({ timeout: 15_000 });
  mark("connects through mocked GitHub device flow");
  await browser.$(".quick-share-connect-start").click();
  await browser.$(".quick-share-create").waitForExist({ timeout: 20_000 });
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".quick-share-create").disabled), { timeout: 20_000 });
  mark("creates exactly one encrypted gist and saves its graph marker");
  await browser.$(".quick-share-create").click();
  await browser.$(".quick-share-copy").waitForExist({ timeout: 20_000 });
  if (creates !== 1 || !envelope) throw new Error("expected exactly one encrypted gist");
  await waitForFileText(path.join(graph, "pages/Project.md"), (text) => text.includes(`tine.share:: octo-cat/${gist}`), { timeoutMs: 15_000 });
  const identity = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/app-identity.json"), "utf8"));
  const records = JSON.parse(fs.readFileSync(path.join(out, "data", identity.identities[identity.ship].identifier, "quick-shares.json"), "utf8"));
  const row = records.shares[0];
  // Read the OS clipboard outside the app webview; the app never returns a key
  // through one of its quick-share command results.
  const link = execFileSync("python3", ["-c", `import ctypes
gtk=ctypes.CDLL('libgtk-3.so.0'); gdk=ctypes.CDLL('libgdk-3.so.0')
gtk.gtk_init(None,None)
gdk.gdk_atom_intern_static_string.argtypes=[ctypes.c_char_p]; gdk.gdk_atom_intern_static_string.restype=ctypes.c_void_p
gtk.gtk_clipboard_get.argtypes=[ctypes.c_void_p]; gtk.gtk_clipboard_get.restype=ctypes.c_void_p
gtk.gtk_clipboard_wait_for_text.argtypes=[ctypes.c_void_p]; gtk.gtk_clipboard_wait_for_text.restype=ctypes.c_void_p
p=gtk.gtk_clipboard_wait_for_text(gtk.gtk_clipboard_get(gdk.gdk_atom_intern_static_string(b'CLIPBOARD')))
if not p: raise RuntimeError('share clipboard is empty')
print(ctypes.string_at(p).decode())`], { encoding: "utf8" }).trim();
  if (link !== `https://tine.page/s/1/#${row.login}/${row.gist_id}/${row.key}`) throw new Error("clipboard does not hold the created share link");
  mark("the built viewer renders the uploaded copy and follows its page link");
  await checkViewer(process.env.QUICK_SHARE_VIEWER_DIR || path.join(root, "artifacts/quick-share-viewer/build"), { envelope, link, expectedText: "Native share sentinel" });
  await browser.execute(() => document.querySelector('.quick-share-dialog .settings-pane-head button').click());
  mark("edits the shared page then reviews Re-share from the Exports overview");
  const firstEnvelope = envelope;
  fs.appendFileSync(path.join(graph, "pages/Project.md"), "- Native re-share edited sentinel\n");
  await browser.waitUntil(() => browser.execute(() => document.body.textContent.includes("Native re-share edited sentinel")), { timeout: 20_000 });
  await openPageByName(browser, "Exports");
  await browser.$(".query-results-region .quick-share-export-reshare").waitForExist({ timeout: 20_000 });
  await browser.execute(() => document.querySelector('.query-results-region .quick-share-export-reshare').click());
  await browser.$(".share-dialog").waitForExist({ timeout: 15_000 });
  if (!await browser.execute(() => document.querySelector('.share-dialog input[name="share-where"][value="link"]').checked)) throw new Error("Re-share did not select Link");
  await browser.execute(() => document.querySelector('.share-dialog .share-export').click());
  await browser.$(".quick-share-membership").waitForExist({ timeout: 20_000 });
  if (!await browser.execute(() => document.querySelector('.quick-share-membership').textContent.includes("+0 new / −0 gone"))) throw new Error("membership counted the synthetic home or lost a real page");
  if (updates !== 0) throw new Error("Re-share uploaded before confirmation");
  await browser.waitUntil(() => browser.execute(() => !document.querySelector('.quick-share-create').disabled), { timeout: 20_000 });
  await browser.$(".quick-share-create").click();
  await browser.$(".quick-share-created").waitForExist({ timeout: 20_000 });
  if (updates !== 1 || creates !== 1 || envelope === firstEnvelope) throw new Error("Re-share did not update exactly the original gist");
  const after = JSON.parse(fs.readFileSync(path.join(out, "data", identity.identities[identity.ship].identifier, "quick-shares.json"), "utf8"));
  if (after.shares.length !== 1 || after.shares[0].gist_id !== row.gist_id || after.shares[0].key !== row.key || after.shares[0].share_id !== row.share_id) throw new Error("Re-share changed the original link or added a record");
  const project = fs.readFileSync(path.join(graph, "pages/Project.md"), "utf8");
  if (project.split("tine.share::").length !== 2 || fs.readFileSync(path.join(graph, "pages/Exports.md"), "utf8").includes("tine.share::")) throw new Error("Re-share duplicated a graph definition");
  mark("the same link in the built viewer shows the edit after one PATCH");
  await checkViewer(process.env.QUICK_SHARE_VIEWER_DIR || path.join(root, "artifacts/quick-share-viewer/build"), { envelope, link, expectedText: "Native re-share edited sentinel" });
  await browser.execute(() => document.querySelector('.quick-share-dialog .settings-pane-head button').click());
  mark("Exports overview offers Delete; declining keeps the hosted share");
  await browser.$(".query-results-region .quick-share-export-delete").waitForExist({ timeout: 20_000 });
  await browser.execute(() => document.querySelector('.query-results-region .quick-share-export-delete').click());
  await answerNativeDialog("no", { env });
  if (deletes !== 0) throw new Error("declining Delete still changed the hosted share");
  mark("confirmed Delete removes the hosted gist and its graph marker through the guarded save");
  await browser.waitUntil(() => browser.execute(() => !document.querySelector('.quick-share-export-delete')?.disabled), { timeout: 10_000 });
  await browser.execute(() => document.querySelector('.query-results-region .quick-share-export-delete').click());
  await answerNativeDialog("yes", { env });
  await waitForFileText(path.join(graph, "pages/Project.md"), (text) => !text.includes("tine.share::"), { timeoutMs: 15_000 });
  if (deletes !== 1) throw new Error("expected exactly one gist deletion");
  proof.creates = creates; proof.updates = updates; proof.deletes = deletes; proof.sameLink = true;
  proof.result = "passed";
} catch (error) {
  proof.failure = { journey, expected: "native share then Re-share shows the edit at the same link, followed by confirmed Delete", observed: String(error).split("\n")[0], classification: browser ? "ambiguous; inspect DOM and log" : "infrastructure" };
  if (browser) {
    await browser.saveScreenshot(path.join(out, "failure.png")).catch(() => {});
    fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  await browser?.deleteSession().catch(() => {}); try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log); await new Promise((resolve) => mock.close(resolve));
  console.log(`Artifacts: ${out}`);
}

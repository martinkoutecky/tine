// Real-app regression for GH #61: current Logseq highlight sidecars use #uuid
// reader tags, list-shaped :rects, and x1/y1/x2/y2 coordinates. Opening the PDF
// used to make the native process allocate without bound before the pane mounted.
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  startWebdriverApplication,
  stopWebdriverApplication,
  tauriCapabilities,
  webdriverServerArgs,
} from "./e2e-capabilities.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine");
const TD = process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4520);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4521);
const TMP = path.join(os.tmpdir(), `tine-pdf-logseq-e2e-${process.pid}`);
const GRAPH = path.join(TMP, "graph");
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;
const APP_DATA = path.join(TMP, "xdg", "data", "page.tine.Tine");
const SETTINGS = path.join(APP_DATA, "tine-settings.json");
const SAMPLE_ID = "6a5604f8-a337-4336-a711-2ba6bc14fbfd";
const PDF = "JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUl0gL0NvdW50IDEgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA2MTIgNzkyXSAvUmVzb3VyY2VzIDw8IC9Gb250IDw8IC9GMSA1IDAgUiA+PiA+PiAvQ29udGVudHMgNCAwIFIgPj4KZW5kb2JqCjQgMCBvYmoKPDwgL0xlbmd0aCAyMDUgPj4Kc3RyZWFtCkJUIC9GMSAyMCBUZiA3MiA3MjAgVGQgKFRpbmUgUERGIHZpZXdlcikgVGogRVQKQlQgL0YxIDEzIFRmIDcyIDY5MCBUZCAoU2VsZWN0IHRoaXMgdGV4dCB0byBjcmVhdGUgYSBoaWdobGlnaHQuKSBUaiBFVApCVCAvRjEgMTMgVGYgNzIgNjY4IFRkIChIaWdobGlnaHRzIHBlcnNpc3QgdG8gYXNzZXRzLzxrZXk+LmVkbiArIGFuIGhsc19fIHBhZ2UuKSBUaiBFVAoKZW5kc3RyZWFtCmVuZG9iago1IDAgb2JqCjw8IC9UeXBlIC9Gb250IC9TdWJ0eXBlIC9UeXBlMSAvQmFzZUZvbnQgL0hlbHZldGljYSA+PgplbmRvYmoKeHJlZgowIDYKMDAwMDAwMDAwMCA2NTUzNSBmIAowMDAwMDAwMDA5IDAwMDAwIG4gCjAwMDAwMDAwNTggMDAwMDAgbiAKMDAwMDAwMDExNSAwMDAwMCBuIAowMDAwMDAwMjQxIDAwMDAwIG4gCjAwMDAwMDA0OTcgMDAwMDAgbiAKdHJhaWxlcgo8PCAvU2l6ZSA2IC9Sb290IDEgMCBSID4+CnN0YXJ0eHJlZgo1NjcKJSVFT0Y=";
const MARKDOWN_UPLOAD_SOURCE = path.join(TMP, "literal-markdown.pdf");
const ORG_UPLOAD_SOURCE = path.join(TMP, "literal-org.pdf");
const OUTLINE_PAGE = path.join(GRAPH, "pages", "PDF Outline.md");
const MARKDOWN_PAGE = path.join(GRAPH, "pages", "PDF Upload.md");
const ORG_PAGE = path.join(GRAPH, "pages", "PDF Org.org");
const PASTE_PAGE = path.join(GRAPH, "pages", "PDF Paste.md");
const MARKDOWN_STORED = path.join(GRAPH, "assets", "e2e-literal-markdown.pdf");
const ORG_STORED = path.join(GRAPH, "assets", "e2e-literal-org.pdf");
const OUTLINE_STORED = path.join(GRAPH, "assets", "e2e-outline.pdf");
const ORG_SIDECAR = path.join(GRAPH, "assets", "e2e-literal-org.edn");
const ORG_HLS_PAGE = path.join(GRAPH, "pages", "hls__e2e-literal-org.org");
const MARKDOWN_UPLOAD_MARKUP = "![literal-markdown.pdf](../assets/e2e-literal-markdown.pdf)";
const ORG_UPLOAD_MARKUP = "[[../assets/e2e-literal-org.pdf][literal-org.pdf]]";
const HIGHLIGHT_TEXT = "Select this deterministic text for a highlight.";
const SECOND_FIRST_ID = "7b6704f8-a337-4336-a711-2ba6bc14fbf1";
const SECOND_SECOND_ID = "7b6704f8-a337-4336-a711-2ba6bc14fbf2";
const EDN = `{:highlights [{:id #uuid "6a5604f8-a337-4336-a711-2ba6bc14fbfd"
  :page 1
  :position {:bounding {:x1 96.7058823529 :y1 937.452777778 :x2 333.098039216 :y2 972.372222222
                        :width 822 :height 1063.7}
             :rects ({:x1 96.7058823529 :y1 937.452777778 :x2 333.098039216 :y2 972.372222222
                      :width 822 :height 1063.7})
             :page 1}
  :content {:text "Tine PDF viewer"}
  :properties {:color "yellow"}}]
  :extra {:page 1 :scale 1.75 :plugin "keep"}
  :future-root 42}
`;
const EDN_SECOND = `{:highlights [{:id #uuid "${SECOND_FIRST_ID}"
  :page 1
  :position {:bounding {:x1 96.7058823529 :y1 937.452777778 :x2 333.098039216 :y2 972.372222222
                        :width 822 :height 1063.7}
             :rects ({:x1 96.7058823529 :y1 937.452777778 :x2 333.098039216 :y2 972.372222222
                      :width 822 :height 1063.7})
             :page 1}
  :content {:text "Second PDF first"}
  :properties {:color "yellow"}}
 {:id #uuid "${SECOND_SECOND_ID}"
  :page 1
  :position {:bounding {:x1 96.7058823529 :y1 850 :x2 333.098039216 :y2 884
                        :width 822 :height 1063.7}
             :rects ({:x1 96.7058823529 :y1 850 :x2 333.098039216 :y2 884
                      :width 822 :height 1063.7})
             :page 1}
  :content {:text "Second PDF second"}
  :properties {:color "green"}}]
  :extra {:page 1 :scale 1.25 :plugin "second-keep"}
  :future-root 84}
`;

// A self-contained, two-page PDF with nested bookmarks. The parent has an
// explicit page-reference destination; its child resolves a named destination.
// The filled red/green/blue rectangles make outline navigation easy to inspect.
function makeNamedOutlinePdf() {
  const firstPageContent = [
    "q",
    "1 0 0 rg", "360 560 160 48 re f",
    "0 0.62 0 rg", "360 490 160 48 re f",
    "0 0.25 1 rg", "360 420 160 48 re f",
    "Q",
    "BT /F1 22 Tf 72 720 Td (Outline fixture page one) Tj ET",
    "BT /F1 13 Tf 72 690 Td (Select this deterministic text for a highlight.) Tj ET",
    "BT /F1 13 Tf 72 668 Td (Explicit and named outline destinations are real.) Tj ET",
  ].join("\n");
  const secondPageContent = [
    "q",
    "0.85 0 0.85 rg", "360 560 160 48 re f",
    "0 0.6 0.85 rg", "360 490 160 48 re f",
    "Q",
    "BT /F1 22 Tf 72 720 Td (Outline target page two) Tj ET",
    "BT /F1 13 Tf 72 690 Td (Named bookmark navigation succeeded.) Tj ET",
  ].join("\n");
  const stream = (content) => `<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`;
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R /Outlines 8 0 R /PageMode /UseOutlines /Names << /Dests 11 0 R >> >>",
    "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 7 0 R >> >> /Contents 4 0 R >>",
    stream(firstPageContent),
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>",
    stream(secondPageContent),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    "<< /Type /Outlines /First 9 0 R /Last 9 0 R /Count 2 >>",
    "<< /Title (Explicit Page One) /Parent 8 0 R /First 10 0 R /Last 10 0 R /Count 1 /Dest [3 0 R /XYZ null null null] >>",
    "<< /Title (Named Page Two) /Parent 9 0 R /Dest /named-page-two >>",
    "<< /Names [(named-page-two) [5 0 R /XYZ null null null]] >>",
  ];
  let pdf = "%PDF-1.7\n";
  const offsets = [0];
  for (const [index, object] of objects.entries()) {
    offsets.push(Buffer.byteLength(pdf));
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
  }
  const xref = Buffer.byteLength(pdf);
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (let index = 1; index < offsets.length; index += 1) {
    pdf += `${String(offsets[index]).padStart(10, "0")} 00000 n \n`;
  }
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(pdf, "utf8");
}

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.mkdirSync(APP_DATA, { recursive: true });
fs.writeFileSync(SETTINGS, '{"asset_name_format":"e2e-%assetname.%ext"}\n');
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{:preferred-format \"Org\"}\n");
const outlineBytes = makeNamedOutlinePdf();
fs.writeFileSync(MARKDOWN_UPLOAD_SOURCE, outlineBytes);
fs.writeFileSync(ORG_UPLOAD_SOURCE, outlineBytes);
fs.writeFileSync(OUTLINE_STORED, outlineBytes);
fs.writeFileSync(OUTLINE_PAGE, "- ![Outline fixture](../assets/e2e-outline.pdf)\n");
fs.writeFileSync(MARKDOWN_PAGE, "- Markdown upload target\n");
fs.writeFileSync(ORG_PAGE, "* Org upload target\n");
fs.writeFileSync(PASTE_PAGE, "- Paste target\n");
fs.writeFileSync(path.join(GRAPH, "assets", "logseq-sample.pdf"), Buffer.from(PDF, "base64"));
const secondPdf = Buffer.from(PDF, "base64");
const pdfLabelOffset = secondPdf.indexOf(Buffer.from("Tine PDF viewer"));
if (pdfLabelOffset < 0) throw new Error("PDF fixture label was not found");
Buffer.from("Second PDF test").copy(secondPdf, pdfLabelOffset); // same byte length: xref stays valid
fs.writeFileSync(path.join(GRAPH, "assets", "logseq-second.pdf"), secondPdf);
const sidecar = path.join(GRAPH, "assets", "logseq-sample.edn");
fs.writeFileSync(sidecar, EDN);
const originalSidecar = fs.readFileSync(sidecar, "utf8");
const secondSidecar = path.join(GRAPH, "assets", "logseq-second.edn");
fs.writeFileSync(secondSidecar, EDN_SECOND);
const originalSecondSidecar = fs.readFileSync(secondSidecar, "utf8");
const hlsPage = path.join(GRAPH, "pages", "hls__logseq-sample.org");
// Existing sidecar plus an existing annotation page whose block is missing:
// Copy ref / Linked references must repair the pair through the guarded writer
// before exposing SAMPLE_ID (GH #168).
fs.writeFileSync(hlsPage, [
  "#+FILE: [[../assets/logseq-sample.pdf][Logseq sample]]",
  "#+FILE-PATH: ../assets/logseq-sample.pdf",
  "",
].join("\n"));
fs.writeFileSync(path.join(GRAPH, "pages", "hls__logseq-second.org"), [
  "#+FILE: [[../assets/logseq-second.pdf][Logseq second]]",
  "#+FILE-PATH: ../assets/logseq-second.pdf",
  "* Second PDF first",
  ":PROPERTIES:",
  ":hl-page: 1",
  ":hl-color: yellow",
  ":ls-type: annotation",
  `:id: ${SECOND_FIRST_ID}`,
  ":END:",
  "* Second PDF second",
  ":PROPERTIES:",
  ":hl-page: 1",
  ":hl-color: green",
  ":ls-type: annotation",
  `:id: ${SECOND_SECOND_ID}`,
  ":END:",
  "",
].join("\n"));
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${journal}.md`), [
  "- ![Logseq sample](../assets/logseq-sample.pdf)",
  "- ![Logseq second](../assets/logseq-second.pdf)",
  `- First exact annotation ((${SECOND_FIRST_ID}))`,
  `- Second exact annotation ((${SECOND_SECOND_ID}))`,
  `- First sample annotation ((${SAMPLE_ID}))`,
  "- [[hls__logseq-second]]",
  "- [[PDF Upload]]",
  "- [[PDF Org]]",
  "",
].join("\n"));

// Windows Tauri/WebView2 can outlive tauri-driver briefly after a preceding
// scenario. On an isolated CI runner, remove only this test binary before
// starting so the single-instance plugin cannot forward us into the old graph.
function stopCiWindowsApp() {
  if (process.platform === "win32" && process.env.CI === "true") {
    spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
  }
}
stopCiWindowsApp();

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "xdg", "data"),
  XDG_CONFIG_HOME: path.join(TMP, "xdg", "config"),
  XDG_CACHE_HOME: path.join(TMP, "xdg", "cache"),
  APPDATA: path.join(TMP, "appdata"),
  LOCALAPPDATA: path.join(TMP, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
const XDOTOOL = process.env.E2E_XDOTOOL || "xdotool";
const xdo = (...args) => execFileSync(XDOTOOL, args, {
  encoding: "utf8",
  env: process.env.E2E_XDOTOOL_LIB
    ? { ...env, LD_LIBRARY_PATH: process.env.E2E_XDOTOOL_LIB }
    : env,
  timeout: 15_000,
}).trim();
const wmLog = process.platform === "linux" && process.env.E2E_WINDOW_MANAGER
  ? fs.openSync(path.join(ARTIFACTS, "window-manager.log"), "w")
  : undefined;
const wm = wmLog !== undefined
  ? spawn(process.env.E2E_WINDOW_MANAGER, ["--sm-disable"], {
      env,
      stdio: ["ignore", wmLog, wmLog],
      detached: true,
    })
  : undefined;
if (wm) await sleep(600);
if (wm?.exitCode != null) {
  throw new Error(`window manager exited before the app launched: ${fs.readFileSync(path.join(ARTIFACTS, "window-manager.log"), "utf8")}`);
}

const log = fs.openSync(path.join(ARTIFACTS, "tauri-driver.log"), "w");
const driverArgs = webdriverServerArgs(
  DRIVER_PORT,
  NATIVE_PORT,
  process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver",
);
let webviewTarget;
let td;
const killDriverTree = () => {
  try {
    if (process.platform === "win32" && td?.pid) {
      spawnSync("taskkill", ["/PID", String(td.pid), "/T", "/F"], { stdio: "ignore" });
    }
    else if (td?.pid) process.kill(-td.pid, "SIGKILL");
  } catch {}
  stopWebdriverApplication(webviewTarget);
};
const startDriverTree = async (session) => {
  webviewTarget = await startWebdriverApplication(APP, env, NATIVE_PORT, session);
  td = spawn(TD, driverArgs, {
    env: webviewTarget.env,
    stdio: ["ignore", log, log],
    detached: process.platform !== "win32",
  });
  await sleep(2500);
};
await startDriverTree("initial");

function processTreeRssKiB() {
  if (process.platform !== "linux") return null;
  const rows = [];
  for (const entry of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(entry)) continue;
    try {
      const status = fs.readFileSync(`/proc/${entry}/status`, "utf8");
      const ppid = Number(status.match(/^PPid:\s+(\d+)$/m)?.[1] || 0);
      const rss = Number(status.match(/^VmRSS:\s+(\d+)\s+kB$/m)?.[1] || 0);
      let exe = "";
      try { exe = fs.readlinkSync(`/proc/${entry}/exe`); } catch {}
      rows.push({ pid: Number(entry), ppid, rss, exe });
    } catch {}
  }
  const owned = new Set(rows.filter((row) => row.exe === APP).map((row) => row.pid));
  let changed = true;
  while (changed) {
    changed = false;
    for (const row of rows) {
      if (owned.has(row.ppid) && !owned.has(row.pid)) {
        owned.add(row.pid);
        changed = true;
      }
    }
  }
  return rows.filter((row) => owned.has(row.pid)).reduce((sum, row) => sum + row.rss, 0);
}

let browser;
async function nativeShiftDrag(start, end) {
  const mid = {
    x: Math.round((start.x + end.x) / 2),
    y: Math.round((start.y + end.y) / 2),
  };
  try {
    await browser.performActions([
      {
        type: "key",
        id: "pdf-area-shift",
        actions: [
          { type: "keyDown", value: "\uE008" },
          { type: "pause", duration: 0 },
          { type: "pause", duration: 120 },
          { type: "pause", duration: 120 },
          { type: "pause", duration: 0 },
          { type: "keyUp", value: "\uE008" },
        ],
      },
      {
        type: "pointer",
        id: "pdf-area-pointer",
        parameters: { pointerType: "mouse" },
        actions: [
          { type: "pointerMove", duration: 0, origin: "viewport", x: Math.round(start.x), y: Math.round(start.y) },
          { type: "pointerDown", button: 0 },
          { type: "pointerMove", duration: 120, origin: "viewport", x: mid.x, y: mid.y },
          { type: "pointerMove", duration: 120, origin: "viewport", x: Math.round(end.x), y: Math.round(end.y) },
          { type: "pointerUp", button: 0 },
          { type: "pause", duration: 0 },
        ],
      },
    ]);
  } finally {
    await browser.releaseActions();
  }
}

async function nativeClickAt(point, id) {
  try {
    await browser.performActions([{
      type: "pointer",
      id,
      parameters: { pointerType: "mouse" },
      actions: [
        { type: "pointerMove", duration: 0, origin: "viewport", x: Math.round(point.x), y: Math.round(point.y) },
        { type: "pointerDown", button: 0 },
        { type: "pointerUp", button: 0 },
      ],
    }]);
  } finally {
    await browser.releaseActions();
  }
}

async function nativePointerDrag(start, end, id, duration = 450) {
  try {
    await browser.performActions([{
      type: "pointer",
      id,
      parameters: { pointerType: "mouse" },
      actions: [
        { type: "pointerMove", duration: 0, origin: "viewport", x: Math.round(start.x), y: Math.round(start.y) },
        { type: "pointerDown", button: 0 },
        { type: "pointerMove", duration, origin: "viewport", x: Math.round(end.x), y: Math.round(end.y) },
        { type: "pointerUp", button: 0 },
      ],
    }]);
  } finally {
    await browser.releaseActions();
  }
}

async function nextPaint() {
  await browser.execute(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

async function toolbarAreaInteriorDrag() {
  return browser.execute(() => {
    const scroll = document.querySelector(".pdf-scroll");
    const page = document.querySelector(".pdf-page");
    if (!(scroll instanceof HTMLElement) || !(page instanceof HTMLElement)) return null;
    // Use the broad interior of the fixture's coloured rectangle, away from text
    // glyphs and every page edge. This proves an area-selection journey, not an
    // incidental drag-length implementation threshold.
    scroll.scrollTop = Math.max(0, page.offsetTop);
    const rect = page.getBoundingClientRect();
    const scale = rect.width / 612;
    const start = {
      x: Math.round(rect.left + 380 * scale),
      y: Math.round(rect.top + 194 * scale),
    };
    const end = {
      x: Math.round(rect.left + 500 * scale),
      y: Math.round(rect.top + 222 * scale),
    };
    const viewport = { width: window.innerWidth, height: window.innerHeight };
    const inside = (point) =>
      point.x > rect.left + 2 && point.x < rect.right - 2 && point.y > rect.top + 2 && point.y < rect.bottom - 2 &&
      point.x > 2 && point.x < viewport.width - 2 && point.y > 2 && point.y < viewport.height - 2;
    const hitText = [...document.elementsFromPoint(start.x, start.y)]
      .some((element) => element.matches(".textLayer span"));
    if (!Number.isFinite(scale) || scale <= 0 || !inside(start) || !inside(end) || hitText) return null;
    return { start, end };
  });
}

async function nativeKeyChord(modifier, key, id) {
  try {
    await browser.performActions([{
      type: "key",
      id,
      actions: [
        { type: "keyDown", value: modifier },
        { type: "keyDown", value: key },
        { type: "keyUp", value: key },
        { type: "keyUp", value: modifier },
      ],
    }]);
  } finally {
    await browser.releaseActions();
  }
}

async function nativeClickSelector(selector, id, text) {
  const target = await browser.execute((wanted, wantedText) => {
    const candidates = [...document.querySelectorAll(wanted)]
      .filter((candidate) => wantedText == null || candidate.textContent?.trim() === wantedText)
      .map((element) => {
        const rect = element.getBoundingClientRect();
        const style = getComputedStyle(element);
        const x = rect.left + rect.width / 2;
        const y = rect.top + rect.height / 2;
        const inViewport = x >= 0 && x < innerWidth && y >= 0 && y < innerHeight;
        const receivesPointer = inViewport && document.elementsFromPoint(x, y)
          .some((hit) => hit === element || element.contains(hit));
        const actionable = rect.width > 0 && rect.height > 0
          && style.display !== "none" && style.visibility !== "hidden"
          && receivesPointer;
        return {
          x, y, actionable,
          diagnostic: {
            title: element.getAttribute("title"),
            ariaLabel: element.getAttribute("aria-label"),
            rect: { left: rect.left, top: rect.top, width: rect.width, height: rect.height },
            display: style.display,
            visibility: style.visibility,
            inViewport,
            receivesPointer,
          },
        };
      });
    const element = candidates.find((candidate) => candidate.actionable);
    return { point: element && { x: element.x, y: element.y }, candidates: candidates.map((candidate) => candidate.diagnostic) };
  }, selector, text ?? null);
  if (!target.point) {
    const state = target.candidates.length === 0 ? "absent" : "not visibly actionable";
    throw new Error(`native pointer target was ${state}: ${selector} ${text ?? ""}; ${JSON.stringify(target.candidates)}`.trim());
  }
  await nativeClickAt(target.point, id);
}

async function nativeClickIndexed(selector, index, id) {
  const point = await browser.execute((wanted, wantedIndex) => {
    const element = document.querySelectorAll(wanted)[wantedIndex];
    const rect = element?.getBoundingClientRect();
    return rect && rect.width > 0 && rect.height > 0
      ? { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }
      : null;
  }, selector, index);
  if (!point) throw new Error(`native pointer target was absent: ${selector}[${index}]`);
  await nativeClickAt(point, id);
}

async function typeKeys(text) {
  for (const key of text) await browser.keys([key]);
}

async function routeToPage(name) {
  const current = await browser.$("h1.page-title").getText().catch(() => "");
  if (current.trim() === name) return;
  await nativeClickSelector('button[title^="Search (Ctrl+K)"]', `route-${name.replaceAll(/\W+/g, "-")}`);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 5000 });
  await typeKeys(name);
  await browser.waitUntil(() => browser.execute((wanted) => {
    const active = document.querySelector(".switcher-row.active .switcher-name");
    if (active?.textContent?.trim() === wanted) return true;
    return [...document.querySelectorAll(".switcher-row:not(.block-result) .switcher-name")]
      .some((node) => node.textContent?.trim() === wanted);
  }, name), {
    timeout: 10_000,
    timeoutMsg: `switcher did not expose routed page ${name}`,
  });
  const exactActive = await browser.execute((wanted) =>
    document.querySelector(".switcher-row.active .switcher-name")?.textContent?.trim() === wanted, name);
  if (!exactActive) {
    const point = await browser.execute((wanted) => {
      const nameElement = [...document.querySelectorAll(".switcher-row:not(.block-result) .switcher-name")]
        .find((node) => node.textContent?.trim() === wanted);
      const rect = nameElement?.closest(".switcher-row")?.getBoundingClientRect();
      return rect ? { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 } : null;
    }, name);
    if (!point) throw new Error(`exact switcher result disappeared for ${name}`);
    await nativeClickAt(point, `route-result-${name.replaceAll(/\W+/g, "-")}`);
  } else {
    await browser.keys(["Enter"]);
  }
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === name, {
    timeout: 10_000,
    timeoutMsg: `named page ${name} did not become the active route`,
  });
}

function sha256(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function graphSnapshot() {
  const snapshot = {};
  const walk = (directory) => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const absolute = path.join(directory, entry.name);
      if (entry.isDirectory()) walk(absolute);
      else if (entry.isFile()) snapshot[path.relative(GRAPH, absolute)] = sha256(absolute);
    }
  };
  walk(GRAPH);
  return snapshot;
}

function fileTreeSnapshot(directory) {
  const files = [];
  const walk = (current) => {
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const absolute = path.join(current, entry.name);
      if (entry.isDirectory()) walk(absolute);
      else if (entry.isFile()) files.push({
        path: path.relative(directory, absolute),
        size: fs.statSync(absolute).size,
        sha256: sha256(absolute),
      });
    }
  };
  walk(directory);
  return files.sort((a, b) => a.path.localeCompare(b.path));
}

// A pending area chooser must be completely side-effect free. Hashes are useful
// for the final receipt, but retain the actual bytes here so a cancellation
// proves literal equality for the real sidecar, hls page, and every asset.
function treeByteSnapshot(directory) {
  const snapshot = new Map();
  const walk = (current) => {
    if (!fs.existsSync(current)) return;
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const absolute = path.join(current, entry.name);
      if (entry.isDirectory()) walk(absolute);
      else if (entry.isFile()) snapshot.set(path.relative(directory, absolute), fs.readFileSync(absolute));
    }
  };
  walk(directory);
  return snapshot;
}

function captureAreaWriteState(sidecarFile, hlsFile) {
  return {
    sidecar: fs.readFileSync(sidecarFile),
    hls: fs.readFileSync(hlsFile),
    assets: treeByteSnapshot(path.join(GRAPH, "assets")),
  };
}

function assertAreaWriteStateUnchanged(before, sidecarFile, hlsFile, label) {
  const nowSidecar = fs.readFileSync(sidecarFile);
  const nowHls = fs.readFileSync(hlsFile);
  if (!before.sidecar.equals(nowSidecar) || !before.hls.equals(nowHls)) {
    throw new Error(`${label} changed the real sidecar or hls annotation page`);
  }
  const afterAssets = treeByteSnapshot(path.join(GRAPH, "assets"));
  const names = [...new Set([...before.assets.keys(), ...afterAssets.keys()])].sort();
  for (const name of names) {
    const expected = before.assets.get(name);
    const actual = afterAssets.get(name);
    if (!expected || !actual || !expected.equals(actual)) {
      throw new Error(`${label} changed graph asset bytes at ${name}`);
    }
  }
}

function ednIds(content) {
  return [...content.matchAll(/#uuid "([^"]+)"/g)].map((match) => match[1]);
}

function ednHighlightEntry(content, id, color) {
  const escapedId = id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const escapedColor = color.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const entry = content.match(new RegExp(
    `\\{:id #uuid "${escapedId}"[\\s\\S]*?:properties \\{:color "${escapedColor}"\\}\\}`,
  ))?.[0];
  if (!entry) throw new Error(`sidecar has no ${color} highlight entry for ${id}`);
  return entry;
}

function ednBounding(entry) {
  const match = entry.match(/:bounding \{:x1 ([^ ]+) :y1 ([^ ]+) :x2 ([^ ]+) :y2 ([^ ]+) :width ([^ ]+) :height ([^ }\n]+)/);
  if (!match) throw new Error(`sidecar highlight has no Logseq bounding geometry: ${entry}`);
  const [x1, y1, x2, y2, width, height] = match.slice(1).map(Number);
  if (![x1, y1, x2, y2, width, height].every(Number.isFinite)) {
    throw new Error(`sidecar highlight geometry is not numeric: ${entry}`);
  }
  return { x1, y1, x2, y2, width, height };
}

function assertNear(actual, expected, label, tolerance = 1.25) {
  if (Math.abs(actual - expected) > tolerance) {
    throw new Error(`${label} mismatch: expected ${expected}, received ${actual} (±${tolerance})`);
  }
}

async function chooseGtkFile(source) {
  if (process.platform !== "linux") throw new Error("literal GTK file chooser automation is Linux-only");
  let raw;
  try {
    raw = xdo("search", "--sync", "--onlyvisible", "--name", "^Choose a file$");
  } catch (error) {
    throw new Error(`GTK chooser did not appear for ${source}: ${error?.stderr?.toString().trim() || error?.message || error}`);
  }
  const ids = raw.split(/\s+/).filter(Boolean);
  const id = ids.at(-1);
  if (!id) throw new Error(`xdotool returned no GTK chooser window for ${source}`);
  const title = xdo("getwindowname", id);
  xdo("windowactivate", "--sync", id);
  // GTK4's file chooser owns a focused child surface; XTEST reaches it only
  // through the active X11 window, not xdotool's --window XSendEvent path.
  xdo("key", "--clearmodifiers", "ctrl+l");
  xdo("type", "--clearmodifiers", "--delay", "1", source);
  try {
    execFileSync("import", ["-window", id, path.join(ARTIFACTS, `gtk-chooser-${path.basename(source)}.png`)], {
      env,
      stdio: "ignore",
      timeout: 5_000,
    });
  } catch {}
  // GTK's location entry first resolves the absolute path; on the current
  // rfd/GTK bridge it keeps the chooser open until the default Open accelerator
  // is invoked afterwards. This is still entirely native X11 keyboard input.
  xdo("key", "--clearmodifiers", "Return");
  await sleep(120);
  try {
    const visible = xdo("search", "--onlyvisible", "--name", "^Choose a file$").split(/\s+/);
    if (visible.includes(id)) xdo("key", "--clearmodifiers", "alt+o");
  } catch {
    // The first Return already accepted the path and closed the chooser.
  }
  return { id, title };
}

async function closePdfWithNativePointer(id) {
  await nativeClickSelector('button[title="Close PDF"]', id);
  await browser.$(".pdf-viewer").waitForExist({ reverse: true, timeout: 10_000 });
}

async function nativeClickPdfLink(filename, label, id) {
  const point = await browser.execute((wanted, wantedLabel) => {
    const link = [...document.querySelectorAll(".page-blocks .pdf-link")]
      .find((candidate) => candidate.outerHTML.includes(wanted) || candidate.textContent?.includes(wantedLabel));
    const rect = link?.getBoundingClientRect();
    return rect && rect.width > 0 && rect.height > 0
      ? { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }
      : null;
  }, filename, label);
  if (!point) {
    const links = await browser.execute(() => [...document.querySelectorAll(".page-blocks .pdf-link")]
      .map((link) => link.outerHTML));
    throw new Error(`rendered PDF link for ${filename} was absent: ${JSON.stringify(links)}`);
  }
  await nativeClickAt(point, id);
}

async function reopenCurrentPagePdf(filename, label, id) {
  await browser.waitUntil(() => browser.execute((wanted, wantedLabel) =>
    [...document.querySelectorAll(".page-blocks .pdf-link")]
      .some((link) => link.outerHTML.includes(wanted) || link.textContent?.includes(wantedLabel)), filename, label), {
    timeout: 10_000,
    timeoutMsg: `page did not render the PDF link for ${filename}/${label}`,
  });
  await nativeClickPdfLink(filename, label, id);
  await browser.waitUntil(() => browser.execute((wanted) =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === wanted
      && document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready") === "true", filename), {
    timeout: 20_000,
    timeoutMsg: `real PDF reopen did not mount ${filename}`,
  });
}

async function uploadOnCurrentPage(source, stored, expectedMarkup, receiptKey) {
  await nativeClickSelector(".page-blocks .ls-block .block-content-wrapper", `${receiptKey}-editor`);
  const editor = await browser.$(".page-blocks textarea.block-editor");
  await editor.waitForExist({ timeout: 5000 });
  await browser.keys(["End"]);
  await typeKeys(" /upload");
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".autocomplete .ac-item.active .ac-label")?.textContent?.trim() === "Upload an asset"), {
    timeout: 10_000,
    timeoutMsg: "literal /upload typing did not activate Upload an asset",
  });
  await browser.keys(["Enter"]);
  const chooser = await chooseGtkFile(source);
  literalReceipt.rows.upload[receiptKey] = { chooser, source, stored, fixtureHash: sha256(source) };
  await browser.waitUntil(() => fs.existsSync(stored), {
    timeout: 15_000,
    timeoutMsg: `GTK-selected asset was not stored at ${stored}`,
  });
  const pageFile = receiptKey === "markdown" ? MARKDOWN_PAGE : ORG_PAGE;
  await browser.waitUntil(() => fs.readFileSync(pageFile, "utf8").includes(expectedMarkup), {
    timeout: 10_000,
    timeoutMsg: `uploaded ${receiptKey} source did not persist ${expectedMarkup}`,
  });
  const actualHash = sha256(stored);
  const fixtureHash = sha256(source);
  if (actualHash !== fixtureHash) {
    throw new Error(`${receiptKey} upload changed fixture bytes: ${actualHash} != ${fixtureHash}`);
  }
  literalReceipt.rows.upload[receiptKey].sourceMarkup = expectedMarkup;
  await browser.keys(["Escape"]);
  try {
    await browser.waitUntil(() => browser.execute((filename, label) =>
      [...document.querySelectorAll(".page-blocks .pdf-link")]
        .some((link) => link.outerHTML.includes(filename) || link.textContent?.includes(label)), path.basename(stored), path.basename(source)), {
      timeout: 10_000,
      timeoutMsg: `${receiptKey} page did not render its exact uploaded PDF link`,
    });
  } catch (error) {
    const diagnostic = await browser.execute(() => ({
      title: document.querySelector("h1.page-title")?.textContent,
      blocks: [...document.querySelectorAll(".page-blocks")].map((page) => page.innerHTML.slice(0, 4000)),
      links: [...document.querySelectorAll(".page-blocks .pdf-link")].map((link) => link.outerHTML),
      editor: document.querySelector("textarea.block-editor")?.value,
    }));
    fs.writeFileSync(path.join(ARTIFACTS, `${receiptKey}-upload-render-diagnostic.json`), JSON.stringify(diagnostic, null, 2));
    throw new Error(`${error.message}: ${JSON.stringify(diagnostic)}`, { cause: error });
  }
  await nativeClickPdfLink(path.basename(stored), path.basename(source), `${receiptKey}-uploaded-pdf-link`);
  try {
    await browser.waitUntil(() => browser.execute((filename) =>
      document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === filename
        && document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready") === "true", path.basename(stored)), {
      timeout: 20_000,
      timeoutMsg: `${receiptKey} uploaded PDF did not reopen in the real viewer`,
    });
  } catch (error) {
    const diagnostic = await browser.execute(() => ({
      viewer: document.querySelector(".pdf-viewer")?.outerHTML.slice(0, 1500),
      links: [...document.querySelectorAll(".pdf-link")].map((link) => ({
        text: link.textContent,
        href: link.getAttribute("href"),
        outer: link.outerHTML.slice(0, 800),
        rect: (() => {
          const r = link.getBoundingClientRect();
          return { left: r.left, top: r.top, width: r.width, height: r.height };
        })(),
      })),
      route: location.href,
    }));
    fs.writeFileSync(path.join(ARTIFACTS, `${receiptKey}-upload-reopen-diagnostic.json`), JSON.stringify(diagnostic, null, 2));
    try { await browser.saveScreenshot(path.join(ARTIFACTS, `${receiptKey}-upload-reopen-diagnostic.png`)); } catch {}
    throw new Error(`${error.message}: ${JSON.stringify(diagnostic)}`, { cause: error });
  }
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".textLayer")?.textContent?.includes("Outline fixture page one")), {
    timeout: 10_000,
    timeoutMsg: `${receiptKey} uploaded PDF did not render its exact fixture text`,
  });
}

async function proveNativeUploadsThemesAndHighlights() {
  // Exercise the deterministic outline before GTK creates any transient dialog
  // surfaces. It is a normal graph asset, not a backend shortcut.
  await closePdfWithNativePointer("close-existing-pdf-before-native-upload");
  await routeToPage("PDF Outline");
  await reopenCurrentPagePdf(path.basename(OUTLINE_STORED), "Outline fixture", "pdf-outline-open-fixture");
  await browser.$(".pdf-page .textLayer").waitForExist({ timeout: 10_000 });
  // Every theme choice is a literal pointer click. Verify its accessible selected
  // state; rendering internals and palettes are intentionally not the contract.
  await nativeClickSelector('button[title="More settings"]', "pdf-theme-settings-open");
  await browser.$(".pdf-settings-menu").waitForExist({ timeout: 5_000 });
  const themeState = {};
  for (const theme of ["light", "warm", "dark"]) {
    await nativeClickSelector(`button[aria-label="${theme[0].toUpperCase()}${theme.slice(1)} PDF theme"]`, `pdf-theme-${theme}`);
    const state = await browser.execute((expectedTheme) => {
      const choice = document.querySelector(`button[aria-label="${expectedTheme[0].toUpperCase()}${expectedTheme.slice(1)} PDF theme"]`);
      return {
        pressed: choice?.getAttribute("aria-pressed"),
        selected: [...document.querySelectorAll('button[aria-label$="PDF theme"]')]
          .filter((button) => button.getAttribute("aria-pressed") === "true")
          .map((button) => button.getAttribute("aria-label")),
      };
    }, theme);
    const expectedLabel = `${theme[0].toUpperCase()}${theme.slice(1)} PDF theme`;
    if (state.pressed !== "true" || JSON.stringify(state.selected) !== JSON.stringify([expectedLabel])) {
      throw new Error(`native ${theme} theme click did not expose one accessible selection: ${JSON.stringify(state)}`);
    }
    themeState[theme] = state;
  }
  try { await browser.saveScreenshot(path.join(ARTIFACTS, "pdf-theme-dark-colour-proof.png")); } catch {}
  await browser.keys(["Escape"]);
  await browser.$(".pdf-settings-menu").waitForExist({ reverse: true, timeout: 5_000 });

  // This is deliberately stronger than the later PdfViewer remount check: end
  // the WebDriver session and the actual app/driver process tree, then start a
  // new native WebKit process with the exact same isolated XDG directories.
  const processRelaunchTheme = {
    isolatedXdg: {
      data: env.XDG_DATA_HOME,
      config: env.XDG_CONFIG_HOME,
      cache: env.XDG_CACHE_HOME,
    },
    before: themeState.dark.selected,
  };
  if (JSON.stringify(processRelaunchTheme.before) !== JSON.stringify(["Dark PDF theme"])) {
    throw new Error(`dark choice was not present before the clean process relaunch: ${JSON.stringify(processRelaunchTheme.before)}`);
  }
  await browser.deleteSession();
  browser = undefined;
  killDriverTree();
  await sleep(800);
  const terminatedXdg = path.join(ARTIFACTS, "localstorage-relaunch-xdg-after-clean-termination");
  fs.cpSync(path.join(TMP, "xdg"), terminatedXdg, { recursive: true, force: true });
  processRelaunchTheme.storageFilesAfterTermination = fileTreeSnapshot(terminatedXdg);
  await startDriverTree("pdf-theme-relaunch");
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, "pdf-theme-relaunch", process.platform, webviewTarget.debuggerAddress),
  });
  await browser.$(".ls-block").waitForExist({ timeout: 30_000 });
  await routeToPage("PDF Outline");
  await reopenCurrentPagePdf(path.basename(OUTLINE_STORED), "Outline fixture", "outline-reopen-after-process-relaunch");
  await nativeClickSelector('button[title="More settings"]', "pdf-theme-settings-reopen-after-process-relaunch");
  await browser.$(".pdf-settings-menu").waitForExist({ timeout: 5_000 });
  processRelaunchTheme.after = await browser.execute(() => [...document.querySelectorAll('button[aria-label$="PDF theme"]')]
    .filter((button) => button.getAttribute("aria-pressed") === "true")
    .map((button) => button.getAttribute("aria-label")));
  if (JSON.stringify(processRelaunchTheme.after) !== JSON.stringify(["Dark PDF theme"])) {
    throw new Error(`dark PDF theme did not survive a clean real application relaunch: ${JSON.stringify(processRelaunchTheme)}`);
  }
  await browser.keys(["Escape"]);
  await browser.$(".pdf-settings-menu").waitForExist({ reverse: true, timeout: 5_000 });

  // The real fixture contains an explicit page-reference parent and a nested
  // named destination. Expansion and both navigation kinds are pointer actions;
  // the DOM only observes their visible result.
  await nativeClickSelector('button[title="Outline"]', "pdf-outline-open");
  try {
    await browser.waitUntil(() => browser.execute(() =>
      document.querySelector(".pdf-outline-panel")?.textContent?.includes("Explicit Page One")), {
      timeout: 10_000,
      timeoutMsg: "embedded nested PDF outline did not parse into the native reader",
    });
  } catch (error) {
    const diagnostic = await browser.execute(() => ({
      filename: document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename"),
      ready: document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready"),
      outlineButton: document.querySelector('button[title="Outline"]')?.outerHTML,
      outline: document.querySelector(".pdf-outline-panel")?.outerHTML,
      pageText: document.querySelector(".textLayer")?.textContent,
    })).catch((driverError) => ({ driverError: String(driverError) }));
    fs.writeFileSync(path.join(ARTIFACTS, "pdf-outline-diagnostic.json"), JSON.stringify(diagnostic, null, 2));
    try { await browser.saveScreenshot(path.join(ARTIFACTS, "pdf-outline-diagnostic.png")); } catch {}
    throw new Error(`${error.message}: ${JSON.stringify(diagnostic)}`, { cause: error });
  }
  await nativeClickSelector(".pdf-outline-disclosure", "pdf-outline-expand");
  await browser.$(".pdf-outline-children .pdf-outline-label").waitForExist({ timeout: 5_000 });
  const expanded = await browser.$(".pdf-outline-disclosure").getAttribute("aria-expanded");
  if (expanded !== "true") throw new Error(`real outline expansion did not set aria-expanded=true: ${expanded}`);
  await nativeClickSelector(".pdf-outline-children .pdf-outline-label", "pdf-outline-named-destination", "Named Page Two");
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".pdf-page-input")?.value === "2"), {
    timeout: 10_000,
    timeoutMsg: "nested named outline pointer navigation did not move to page two",
  });
  await nativeClickSelector(".pdf-outline-list .pdf-outline-label", "pdf-outline-explicit-destination", "Explicit Page One");
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".pdf-page-input")?.value === "1"), {
    timeout: 10_000,
    timeoutMsg: "explicit outline pointer navigation did not return to page one",
  });
  await browser.keys(["Escape"]);
  await browser.$(".pdf-outline-panel").waitForExist({ reverse: true, timeout: 5_000 });
  await nativeClickSelector('button[title="Outline"]', "pdf-outline-reopen");
  await browser.$(".pdf-outline-panel").waitForExist({ timeout: 5_000 });
  await nativeClickSelector(".pdf-scroll", "pdf-outline-outside-dismiss");
  await browser.$(".pdf-outline-panel").waitForExist({ reverse: true, timeout: 5_000 });

  // Closing and reopening remounts PdfViewer. The selected theme must remain
  // discoverable through the reader's accessible theme controls.
  await closePdfWithNativePointer("outline-close-for-theme-reopen");
  await reopenCurrentPagePdf(path.basename(OUTLINE_STORED), "Outline fixture", "outline-reopen-after-theme");
  await nativeClickSelector('button[title="More settings"]', "pdf-theme-settings-reopen-after-remount");
  await browser.$(".pdf-settings-menu").waitForExist({ timeout: 5_000 });
  const remountedTheme = await browser.execute(() => [...document.querySelectorAll('button[aria-label$="PDF theme"]')]
    .filter((button) => button.getAttribute("aria-pressed") === "true")
    .map((button) => button.getAttribute("aria-label")));
  if (JSON.stringify(remountedTheme) !== JSON.stringify(["Dark PDF theme"])) {
    throw new Error(`PDF theme was not locally persistent after a real close/reopen: ${JSON.stringify(remountedTheme)}`);
  }
  await browser.keys(["Escape"]);
  await browser.$(".pdf-settings-menu").waitForExist({ reverse: true, timeout: 5_000 });
  literalReceipt.rows.themesOutline = {
    choices: themeState,
    processRelaunchTheme,
    remountedTheme,
    explicitDestinationPage: 1,
    namedDestinationPage: 2,
  };

  // The two upload pages deliberately have explicit extensions: the same literal
  // picker action must write Markdown versus Org syntax, while both stored assets
  // use the device-local e2e-%assetname.%ext naming template seeded above.
  await closePdfWithNativePointer("outline-close-before-native-uploads");
  await routeToPage("PDF Upload");
  await uploadOnCurrentPage(
    MARKDOWN_UPLOAD_SOURCE,
    MARKDOWN_STORED,
    MARKDOWN_UPLOAD_MARKUP,
    "markdown",
  );
  await closePdfWithNativePointer("markdown-close-uploaded-pdf");

  await routeToPage("PDF Org");
  await uploadOnCurrentPage(
    ORG_UPLOAD_SOURCE,
    ORG_STORED,
    ORG_UPLOAD_MARKUP,
    "org",
  );

  // Select the deterministic line with an actual pointer drag across the live
  // PDF.js text layer. No Range/selection event is synthesized in this proof.
  await browser.waitUntil(() => browser.execute((expectedText) =>
    [...document.querySelectorAll(".textLayer span")].some((span) => span.textContent?.trim() === expectedText), HIGHLIGHT_TEXT), {
    timeout: 10_000,
    timeoutMsg: "deterministic PDF text was not exposed for literal pointer selection",
  });
  const selectionTarget = await browser.execute((expectedText) => {
    const span = [...document.querySelectorAll(".textLayer span")]
      .find((candidate) => candidate.textContent?.trim() === expectedText);
    const rect = span?.getBoundingClientRect();
    return rect && rect.width > 20 && rect.height > 5
      ? { start: { x: rect.left + 1, y: rect.top + rect.height / 2 }, end: { x: rect.right - 1, y: rect.top + rect.height / 2 } }
      : null;
  }, HIGHLIGHT_TEXT);
  if (!selectionTarget) throw new Error("deterministic PDF text has no usable native-drag geometry");
  await nativePointerDrag(selectionTarget.start, selectionTarget.end, "pdf-text-highlight-selection");
  await browser.$(".pdf-color-menu").waitForExist({ timeout: 5_000 });
  const selected = await browser.execute(() => {
    const selection = window.getSelection();
    const range = selection?.rangeCount ? selection.getRangeAt(0) : null;
    const page = range?.commonAncestorContainer.parentElement?.closest(".pdf-page")
      ?? document.querySelector(".pdf-page");
    const pageRect = page?.getBoundingClientRect();
    const rects = range ? [...range.getClientRects()].filter((rect) => rect.width > 0 && rect.height > 0) : [];
    const zoom = Number(document.querySelector(".pdf-zoom-level")?.textContent?.replace("%", "")) / 100;
    return {
      text: selection?.toString().trim() ?? "",
      page: Number(page?.getAttribute("data-page")),
      zoom,
      pageWidth: pageRect?.width,
      pageHeight: pageRect?.height,
      pageLeft: pageRect?.left,
      pageTop: pageRect?.top,
      rects: rects.map((rect) => ({ left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom })),
    };
  });
  if (selected.text !== HIGHLIGHT_TEXT || selected.page !== 1 || !Number.isFinite(selected.zoom) || selected.rects.length !== 1) {
    throw new Error(`native pointer selection was not the exact deterministic line: ${JSON.stringify(selected)}`);
  }
  const beforeTextIds = fs.existsSync(ORG_SIDECAR) ? ednIds(fs.readFileSync(ORG_SIDECAR, "utf8")) : [];
  await nativeClickIndexed(".pdf-color-swatch", 4, "pdf-text-highlight-purple");
  await browser.$(".pdf-color-menu").waitForExist({ reverse: true, timeout: 5_000 });
  let textHighlightId;
  await browser.waitUntil(() => {
    if (!fs.existsSync(ORG_SIDECAR) || !fs.existsSync(ORG_HLS_PAGE)) return false;
    const written = fs.readFileSync(ORG_SIDECAR, "utf8");
    textHighlightId = ednIds(written).find((id) => !beforeTextIds.includes(id));
    return !!textHighlightId && written.includes(`:content {:text "${HIGHLIGHT_TEXT}"}`)
      && written.includes(':properties {:color "purple"}')
      && fs.readFileSync(ORG_HLS_PAGE, "utf8").includes(`:id: ${textHighlightId}`);
  }, { timeout: 15_000, timeoutMsg: "native purple text selection did not persist a matching sidecar and hls annotation" });
  const textSidecar = fs.readFileSync(ORG_SIDECAR, "utf8");
  const textHls = fs.readFileSync(ORG_HLS_PAGE, "utf8");
  const textEntry = ednHighlightEntry(textSidecar, textHighlightId, "purple");
  if (!textEntry.includes(`:content {:text "${HIGHLIGHT_TEXT}"}`)
    || !textHls.includes(`* ${HIGHLIGHT_TEXT}`)
    || !textHls.includes(":hl-page: 1")
    || !textHls.includes(":hl-color: purple")
    || !textHls.includes(`:id: ${textHighlightId}`)) {
    throw new Error(`native text highlight sidecar/hls identity is incomplete: ${JSON.stringify({ textEntry, textHls })}`);
  }
  const bounding = ednBounding(textEntry);
  const rect = selected.rects[0];
  const expectedGeometry = {
    x1: (rect.left - selected.pageLeft) / selected.zoom,
    y1: (rect.top - selected.pageTop) / selected.zoom,
    x2: (rect.right - selected.pageLeft) / selected.zoom,
    y2: (rect.bottom - selected.pageTop) / selected.zoom,
    width: selected.pageWidth / selected.zoom,
    height: selected.pageHeight / selected.zoom,
  };
  for (const key of Object.keys(expectedGeometry)) assertNear(bounding[key], expectedGeometry[key], `text highlight ${key}`);
  await browser.waitUntil(() => browser.execute((id) => !!document.querySelector(`.pdf-hl[data-highlight-id="${id}"]`), textHighlightId), {
    timeout: 5_000,
    timeoutMsg: "persisted text highlight did not paint a live overlay",
  });
  // Tauri copied the generated block ref to the native clipboard; this actual
  // Ctrl+V in a separately routed Markdown editor proves it is pasteable text.
  const textRef = `((${textHighlightId}))`;
  await closePdfWithNativePointer("org-close-for-clipboard-paste");
  await routeToPage("PDF Paste");
  await nativeClickSelector(".page-blocks .ls-block .block-content-wrapper", "pdf-ref-paste-editor");
  const pasteEditor = await browser.$(".page-blocks textarea.block-editor");
  await pasteEditor.waitForExist({ timeout: 5_000 });
  await browser.keys(["End"]);
  await nativeKeyChord("\uE009", "v", "pdf-generated-ref-native-paste");
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => fs.readFileSync(PASTE_PAGE, "utf8").includes(textRef), {
    timeout: 10_000,
    timeoutMsg: "the generated PDF ref was not pasted from the native clipboard into Markdown",
  });

  await routeToPage("PDF Org");
  await reopenCurrentPagePdf(path.basename(ORG_STORED), path.basename(ORG_UPLOAD_SOURCE), "org-reopen-text-highlight");
  await browser.$(`.pdf-hl[data-highlight-id="${textHighlightId}"]`).waitForExist({ timeout: 10_000 });

  // Drive the real toolbar journey from a broad, unambiguous page interior.
  // The native test proves capability and its durable result, not a drag-length
  // implementation threshold.
  let areaHighlightId;
  if (process.platform !== "darwin") {
    await nativeClickSelector('button[title^="Area highlight"]', "pdf-toolbar-area-enable");
    const toolbarEnabled = await browser.execute(() =>
      document.querySelector('button[title^="Area highlight"]')?.classList.contains("active") ?? false);
    if (!toolbarEnabled) throw new Error("real toolbar Area button did not become active");
    const areaDrag = await toolbarAreaInteriorDrag();
    if (!areaDrag) throw new Error("the PDF fixture did not expose an unambiguous interior toolbar-area drag");
    const areaPendingBefore = captureAreaWriteState(ORG_SIDECAR, ORG_HLS_PAGE);
    const beforeAreaIds = ednIds(fs.readFileSync(ORG_SIDECAR, "utf8"));
    await nativePointerDrag(areaDrag.start, areaDrag.end, "pdf-toolbar-area-interior", 180);
    await browser.$(".pdf-color-menu").waitForExist({ timeout: 5_000 });
    const pendingAreaState = await browser.execute(() => ({
      chooser: !!document.querySelector(".pdf-color-menu"),
      active: document.querySelector('button[title^="Area highlight"]')?.classList.contains("active") ?? false,
    }));
    if (!pendingAreaState.chooser) {
      throw new Error(`toolbar Area drag did not open a pending chooser: ${JSON.stringify(pendingAreaState)}`);
    }
    assertAreaWriteStateUnchanged(areaPendingBefore, ORG_SIDECAR, ORG_HLS_PAGE, "opening the toolbar Area chooser before color choice");
    await nativeClickIndexed(".pdf-color-swatch", 3, "pdf-area-red");
    await browser.$(".pdf-color-menu").waitForExist({ reverse: true, timeout: 5_000 });
    await browser.waitUntil(() => {
      const written = fs.readFileSync(ORG_SIDECAR, "utf8");
      areaHighlightId = ednIds(written).find((id) => !beforeAreaIds.includes(id));
      if (!areaHighlightId || !written.includes(':properties {:color "red"}')) return false;
      const imageDir = path.join(GRAPH, "assets", "e2e-literal-org");
      if (!fs.existsSync(imageDir)) return false;
      const image = fs.readdirSync(imageDir)
        .find((name) => name.startsWith(`1_${areaHighlightId}_`) && name.endsWith(".png"));
      return !!image && fs.statSync(path.join(imageDir, image)).size > 0
        && fs.readFileSync(ORG_HLS_PAGE, "utf8").includes(`:id: ${areaHighlightId}`);
    }, { timeout: 15_000, timeoutMsg: "native toolbar Area drag did not persist its red crop and hls annotation" });
    const areaEntry = ednHighlightEntry(fs.readFileSync(ORG_SIDECAR, "utf8"), areaHighlightId, "red");
    const areaHls = fs.readFileSync(ORG_HLS_PAGE, "utf8");
    if (!areaEntry.includes(":image ") || !areaHls.includes(":hl-color: red") || !areaHls.includes(":hl-type: area")) {
      throw new Error(`native area highlight persistence is incomplete: ${JSON.stringify({ areaEntry, areaHls })}`);
    }
    const resetAfterValidArea = await browser.execute(() =>
      document.querySelector('button[title^="Area highlight"]')?.classList.contains("active") ?? false);
    if (resetAfterValidArea) throw new Error("toolbar Area mode remained active after a valid drag and colour selection");

    // This is deliberately the same real canvas region as the toolbar drag but
    // without Shift or the toolbar mode. WebKit may turn an ordinary drag into
    // an ordinary text selection when its transparent text layer owns that
    // point; that is acceptable only when the non-empty browser selection proves
    // it did not enter the pending-area path.
    const ordinaryDragBefore = captureAreaWriteState(ORG_SIDECAR, ORG_HLS_PAGE);
    await browser.execute(() => window.getSelection()?.removeAllRanges());
    await nativePointerDrag(areaDrag.start, areaDrag.end, "pdf-toolbar-area-ordinary-after-reset", 180);
    await nextPaint();
    const ordinaryDragState = await browser.execute(() => ({
      chooser: !!document.querySelector(".pdf-color-menu"),
      active: document.querySelector('button[title^="Area highlight"]')?.classList.contains("active") ?? false,
      band: document.querySelectorAll(".pdf-area-band").length,
      selection: window.getSelection()?.toString() ?? "",
    }));
    if (ordinaryDragState.active || ordinaryDragState.band
      || (ordinaryDragState.chooser && !ordinaryDragState.selection)) {
      throw new Error(`ordinary unmodified drag was hijacked after toolbar Area reset: ${JSON.stringify(ordinaryDragState)}`);
    }
    if (ordinaryDragState.chooser) {
      await browser.keys(["Escape"]);
      await browser.$(".pdf-color-menu").waitForExist({ reverse: true, timeout: 5_000 });
    }
    assertAreaWriteStateUnchanged(ordinaryDragBefore, ORG_SIDECAR, ORG_HLS_PAGE, "ordinary drag after toolbar Area reset");
    await closePdfWithNativePointer("org-close-area-reopen");
    await reopenCurrentPagePdf(path.basename(ORG_STORED), path.basename(ORG_UPLOAD_SOURCE), "org-reopen-area-highlight");
    await browser.$(`.pdf-hl-area[data-highlight-id="${areaHighlightId}"]`).waitForExist({ timeout: 10_000 });
    literalReceipt.rows.area = {
      id: areaHighlightId,
      color: "red",
      reopened: true,
      toolbar: { areaDrag, pendingAreaState, resetAfterValidArea, ordinaryDragState },
    };
  }
  literalReceipt.rows.textHighlight = {
    id: textHighlightId,
    text: HIGHLIGHT_TEXT,
    color: "purple",
    sidecar: ORG_SIDECAR,
    hls: ORG_HLS_PAGE,
    bounding,
    clipboardPaste: { page: PASTE_PAGE, reference: textRef },
    reopened: true,
  };
  literalReceipt.graph = graphSnapshot();
  try { await browser.saveScreenshot(path.join(ARTIFACTS, "pdf-native-proof.png")); } catch {}
}

const literalReceipt = {
  schemaVersion: 1,
  app: path.resolve(APP),
  appSha256: sha256(APP),
  settings: { asset_name_format: "e2e-%assetname.%ext" },
  rows: { upload: {}, textHighlight: {}, area: {}, themesOutline: {} },
};

try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, "default", process.platform, webviewTarget.debuggerAddress),
  });
  await browser.$(".ls-block").waitForExist({ timeout: 30_000 });
  const pdfLink = browser.$(".pdf-link");
  try {
    await pdfLink.waitForExist({ timeout: 20_000 });
  } catch (error) {
    const diagnostic = await browser.execute(() => ({
      href: location.href,
      title: document.title,
      bodyText: document.body.innerText.slice(0, 4000),
      blocks: Array.from(document.querySelectorAll(".ls-block")).slice(0, 20).map((block) => ({
        text: block.textContent?.slice(0, 500),
        html: block.innerHTML.slice(0, 1500),
      })),
    }));
    const evidenceDir = process.env.E2E_ARTIFACT_DIR || TMP;
    fs.writeFileSync(path.join(evidenceDir, "startup-diagnostic.json"), JSON.stringify(diagnostic, null, 2));
    try { await browser.saveScreenshot(path.join(evidenceDir, "startup-diagnostic.png")); } catch {}
    throw new Error(`PDF link did not render: ${JSON.stringify(diagnostic)}`, { cause: error });
  }
  const before = processTreeRssKiB();
  await pdfLink.click();
  let peak = before;
  for (let i = 0; i < 15; i++) {
    await sleep(200);
    const rss = processTreeRssKiB();
    if (rss != null && before != null) {
      peak = Math.max(peak, rss);
      if (peak - before > 768 * 1024) throw new Error("opening the Logseq PDF grew resident memory by more than 768 MiB");
    }
  }
  await browser.$(".pdf-page canvas").waitForExist({ timeout: 20_000 });
  await browser.$(".pdf-hl").waitForExist({ timeout: 10_000 });
  const restoredZoom = await browser.$(".pdf-zoom-level").getText();
  if (restoredZoom !== "175%") throw new Error(`PDF scale was not restored from :extra: ${restoredZoom}`);
  if (fs.readFileSync(sidecar, "utf8") !== originalSidecar) {
    throw new Error("opening the PDF rewrote an existing Logseq sidecar before a user change");
  }
  const hls = fs.readFileSync(hlsPage, "utf8");
  for (const expected of [
    "#+FILE: [[../assets/logseq-sample.pdf][Logseq sample]]",
    "#+FILE-PATH: ../assets/logseq-sample.pdf",
  ]) {
    if (!hls.includes(expected)) throw new Error(`OG-compatible Org hls page is missing ${expected}: ${hls}`);
  }
  if (hls.includes(`:id: ${SAMPLE_ID}`)) {
    throw new Error("opening an existing hls page unexpectedly repaired its missing annotation before a reference action");
  }
  // `logseq-sample.pdf` deliberately has no outline dictionary. Open the real
  // reader's outline popover by pointer and observe its visible empty state;
  // this is not a component mock or a synthetic outline result.
  await nativeClickSelector('button[title="Outline"]', "pdf-no-outline-open");
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".pdf-outline-empty")?.textContent?.trim() === "No outlines"), {
    timeout: 10_000,
    timeoutMsg: "real no-outline PDF did not visibly report No outlines",
  });
  await browser.keys(["Escape"]);
  await browser.$(".pdf-outline-panel").waitForExist({ reverse: true, timeout: 5_000 });
  const geometry = await browser.execute(() => {
    const highlight = document.querySelector(".pdf-hl");
    const page = document.querySelector(".pdf-page");
    return highlight ? {
      left: highlight.style.left,
      top: highlight.style.top,
      width: highlight.style.width,
      height: highlight.style.height,
      leftRatio: parseFloat(highlight.style.left) / parseFloat(page.style.width),
      widthRatio: parseFloat(highlight.style.width) / parseFloat(page.style.width),
    } : null;
  });
  if (!geometry || Object.values(geometry).some((value) => !value || String(value).includes("NaN"))) {
    throw new Error(`Logseq highlight geometry was not converted safely: ${JSON.stringify(geometry)}`);
  }
  if (Math.abs(geometry.leftRatio - 72 / 612) > 0.005 || Math.abs(geometry.widthRatio - 176 / 612) > 0.005) {
    throw new Error(`Logseq zoom-space highlight was misplaced: ${JSON.stringify(geometry)}`);
  }

  // OG 1.0.0 area-highlight parity on non-macOS: a literal native Shift drag
  // opens the shared color chooser without writing. Only a subsequent native
  // swatch click may crop, persist the chosen color, create the annotation, and
  // copy its block reference. Keep this branch free of DOM-dispatched gestures.
  if (process.platform !== "darwin") {
    const areaImageDir = path.join(GRAPH, "assets", "logseq-sample");
    await browser.$(".pdf-page").scrollIntoView({ block: "center", inline: "center" });
    const areaDrag = await browser.execute(() => {
      const rect = document.querySelector(".pdf-page")?.getBoundingClientRect();
      if (!rect) return null;
      const visibleLeft = Math.max(rect.left + 30, 20);
      const visibleRight = Math.min(rect.right - 30, window.innerWidth - 20);
      const visibleTop = Math.max(rect.top + 60, 80);
      const visibleBottom = Math.min(rect.bottom - 40, window.innerHeight - 80);
      const start = { x: visibleLeft + 20, y: visibleTop + 10 };
      const end = {
        x: Math.min(start.x + 120, visibleRight),
        y: Math.min(start.y + 80, visibleBottom),
      };
      return end.x - start.x > 10 && end.y - start.y > 10 ? { start, end } : null;
    });
    if (!areaDrag) throw new Error("the visible PDF page did not expose a >10px native area-drag region");

    // The chooser is a pending mutation, not an already-created annotation.
    // Snapshot the actual bytes, dismiss a real chooser with WebDriver Escape,
    // and prove the sidecar, notes page, and complete asset tree are identical.
    const cancelledAreaBefore = captureAreaWriteState(sidecar, hlsPage);
    await nativeShiftDrag(areaDrag.start, areaDrag.end);
    await browser.$(".pdf-color-menu").waitForExist({ timeout: 5000 });
    await browser.keys(["Escape"]);
    await browser.$(".pdf-color-menu").waitForExist({ reverse: true, timeout: 5000 });
    await nextPaint();
    assertAreaWriteStateUnchanged(cancelledAreaBefore, sidecar, hlsPage, "cancelling the native area chooser");
    literalReceipt.rows.areaCancellation = {
      dismiss: "Escape",
      sidecar: sidecar,
      hls: hlsPage,
      assetFiles: [...cancelledAreaBefore.assets.keys()].sort(),
    };

    // Repeat the same real Shift path and choose a swatch to retain the existing
    // OG-compatible direct-area persistence proof.
    const areaWriteBefore = captureAreaWriteState(sidecar, hlsPage);
    await nativeShiftDrag(areaDrag.start, areaDrag.end);
    await browser.$(".pdf-color-menu").waitForExist({ timeout: 5000 });
    assertAreaWriteStateUnchanged(areaWriteBefore, sidecar, hlsPage, "opening the native area chooser before color choice");

    const bluePoint = await browser.execute(() => {
      const swatch = document.querySelectorAll(".pdf-color-swatch")[2];
      const rect = swatch?.getBoundingClientRect();
      return rect ? { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 } : null;
    });
    if (!bluePoint) throw new Error("the area chooser did not expose its blue swatch");
    await nativeClickAt(bluePoint, "pdf-area-blue-swatch");
    await browser.$(".pdf-color-menu").waitForExist({ reverse: true, timeout: 5000 });
    await browser.waitUntil(() => {
      const written = fs.readFileSync(sidecar, "utf8");
      const ids = [...written.matchAll(/#uuid "([^"]+)"/g)].map((match) => match[1]);
      const areaId = ids.find((id) => id !== SAMPLE_ID);
      const images = fs.existsSync(areaImageDir) ? fs.readdirSync(areaImageDir) : [];
      const image = areaId && images.find((name) => name.startsWith(`1_${areaId}_`) && name.endsWith(".png"));
      if (!areaId || !image || !written.includes(':properties {:color "blue"}')) return false;
      const annotation = fs.readFileSync(hlsPage, "utf8");
      return annotation.includes(`:id: ${areaId}`)
        && annotation.includes(":hl-color: blue")
        && fs.statSync(path.join(areaImageDir, image)).size > 0;
    }, {
      timeout: 10_000,
      timeoutMsg: "native Shift area selection did not persist the chosen blue crop and annotation reference",
    });
    await browser.waitUntil(() => browser.execute(() =>
      [...document.querySelectorAll(".toast")].some((toast) => toast.textContent?.includes("Copied highlight ref"))), {
      timeout: 5000,
      timeoutMsg: "native area selection did not copy its persisted block reference",
    });
  }

  // GH #168: both click and right-click share the existing-highlight menu. The
  // reference actions must safely ensure a missing annotation block before
  // copying or routing, and Linked references must reveal ordinary referrers.
  // Post-#161 closure: Find and the highlight menu are peer transient owners.
  // Open Find first, then the real highlight menu; one Escape must peel only
  // the newer menu without changing PDF state or sidecar bytes, and the next
  // Escape closes Find while leaving the viewer mounted.
  await browser.$('button[title="Find in document (Ctrl+F)"]').click();
  await browser.$(".pdf-find-bar").waitForExist({ timeout: 5000 });
  const transientSidecarBefore = fs.readFileSync(sidecar, "utf8");
  const transientViewBefore = await browser.execute(() => ({
    filename: document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename"),
    zoom: document.querySelector(".pdf-zoom-level")?.textContent?.trim(),
    target: document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-highlight-target"),
  }));
  await browser.execute(() => {
    const highlight = document.querySelector(".pdf-hl");
    const rect = highlight?.getBoundingClientRect();
    highlight?.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: rect?.left ?? 120,
      clientY: rect?.top ?? 120,
      view: window,
    }));
  });
  await browser.$(".pdf-color-menu").waitForExist({ timeout: 5000 });
  await browser.keys(["Escape"]);
  await browser.$(".pdf-color-menu").waitForExist({ reverse: true, timeout: 5000 });
  if (!(await browser.$(".pdf-find-bar").isExisting())) {
    throw new Error("PDF highlight-menu Escape also closed the older Find owner");
  }
  const transientViewAfterMenu = await browser.execute(() => ({
    filename: document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename"),
    zoom: document.querySelector(".pdf-zoom-level")?.textContent?.trim(),
    target: document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-highlight-target"),
  }));
  if (JSON.stringify(transientViewAfterMenu) !== JSON.stringify(transientViewBefore)
    || fs.readFileSync(sidecar, "utf8") !== transientSidecarBefore) {
    throw new Error(`PDF menu dismissal mutated state: ${JSON.stringify({ transientViewBefore, transientViewAfterMenu })}`);
  }
  await browser.keys(["Escape"]);
  await browser.$(".pdf-find-bar").waitForExist({ reverse: true, timeout: 5000 });
  if (!(await browser.$(".pdf-viewer").isExisting())) throw new Error("PDF Find Escape closed the viewer");

  // Reopen the real menu for the existing reference-action checks below.
  await browser.execute(() => {
    const highlight = document.querySelector(".pdf-hl");
    const rect = highlight?.getBoundingClientRect();
    highlight?.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: rect?.left ?? 120,
      clientY: rect?.top ?? 120,
      view: window,
    }));
  });
  await browser.$(".pdf-color-menu").waitForExist({ timeout: 5000 });
  // WebKitDriver can invalidate the session when several element-text commands
  // are issued together. Snapshot both labels in one document command instead.
  const existingActions = await browser.execute(() =>
    [...document.querySelectorAll(".pdf-hl-action")].map((element) => element.textContent?.trim()));
  if (!existingActions.includes("Copy ref") || !existingActions.includes("Linked references")) {
    throw new Error(`existing PDF highlight menu omitted reference actions: ${JSON.stringify(existingActions)}`);
  }
  await browser.execute(() => {
    const action = [...document.querySelectorAll(".pdf-hl-action")]
      .find((element) => element.textContent?.trim() === "Copy ref");
    action?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, view: window }));
  });
  await browser.waitUntil(() => fs.readFileSync(hlsPage, "utf8").includes(`:id: ${SAMPLE_ID}`), {
    timeout: 10_000,
    timeoutMsg: "Copy ref did not safely upsert the missing annotation block",
  });
  await browser.waitUntil(() => browser.execute(() =>
    [...document.querySelectorAll(".toast")].some((toast) => toast.textContent?.includes("Copied highlight ref"))), {
    timeout: 5000,
    timeoutMsg: "Copy ref did not reach the native clipboard success boundary",
  });
  const repairedHls = fs.readFileSync(hlsPage, "utf8");
  for (const expected of ["* Tine PDF viewer", ":hl-page: 1", ":ls-type: annotation", `:id: ${SAMPLE_ID}`]) {
    if (!repairedHls.includes(expected)) throw new Error(`repaired annotation page is missing ${expected}: ${repairedHls}`);
  }

  await browser.execute(() => {
    document.querySelector(".pdf-hl")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
  });
  await browser.execute(() => {
    const action = [...document.querySelectorAll(".pdf-hl-action")]
      .find((element) => element.textContent?.trim() === "Linked references");
    action?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, view: window }));
  });
  await browser.waitUntil(() => browser.execute((highlightId) => {
    const block = document.querySelector(`.ls-block[data-block-ref="${highlightId}"]`);
    return block?.querySelector(".block-references")?.textContent?.includes("First sample annotation") ?? false;
  }, SAMPLE_ID), {
    timeout: 10_000,
    timeoutMsg: "Linked references did not open the annotation block with its ordinary referrers visible",
  });
  await browser.$('button[title="Go back"]').click();
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelectorAll(".pdf-link").length >= 2 &&
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "logseq-sample.pdf"), {
    timeout: 10_000,
    timeoutMsg: "returning from PDF highlight references did not restore the journal surface",
  });

  // OG carries an annotation entity through block-ref navigation, then scrolls
  // to that exact highlight. A filename is the resource identity; a second
  // target in the same file must not remount it. A different filename still is
  // a complete boundary: cleanup flushes A to A, then B opens only B's state.
  await browser.$('button[title="Zoom in"]').click();

  // First prove ordinary direct-link A -> B -> A with real pointer clicks and
  // visibly distinct document bytes, before exercising highlight entry paths.
  let pdfLinks = await browser.$$(".pdf-link");
  await pdfLinks[1].click();
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "logseq-second.pdf" &&
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready") === "true"), {
    timeout: 10_000,
    timeoutMsg: "a direct PDF link did not switch from A to B",
  });
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".textLayer")?.textContent?.includes("Second PDF test")), {
    timeout: 10_000,
    timeoutMsg: "PDF B's distinct text never appeared",
  });
  await browser.waitUntil(() => {
    const written = fs.readFileSync(sidecar, "utf8");
    return written.includes(":scale 1.93") && written.includes(':plugin "keep"') && written.includes(":future-root 42");
  }, { timeout: 10_000, timeoutMsg: "PDF A cleanup did not persist A's pending view state" });
  if (fs.readFileSync(secondSidecar, "utf8") !== originalSecondSidecar) {
    throw new Error("switching from PDF A wrote A's pending state or baseline into PDF B");
  }
  pdfLinks = await browser.$$(".pdf-link");
  await pdfLinks[0].click();
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "logseq-sample.pdf" &&
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready") === "true"), {
    timeout: 10_000,
    timeoutMsg: "a direct PDF link did not switch from B back to A",
  });
  const directReturnedZoom = await browser.$(".pdf-zoom-level").getText();
  if (directReturnedZoom !== "193%") throw new Error(`PDF A did not restore its own state after direct B: ${directReturnedZoom}`);

  const firstSecondRef = browser.$(`[data-block-ref="${SECOND_FIRST_ID}"]`);
  await firstSecondRef.waitForExist({ timeout: 10_000 });
  await browser.waitUntil(() => browser.execute((highlightId) =>
    !document.querySelector(`[data-block-ref="${highlightId}"]`)?.classList.contains("block-ref-missing"), SECOND_FIRST_ID), {
    timeout: 10_000,
    timeoutMsg: "the first PDF annotation block reference did not resolve",
  });
  await firstSecondRef.click();
  try {
    await browser.waitUntil(() => browser.execute((highlightId) =>
      document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "logseq-second.pdf" &&
      document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-highlight-target") === highlightId &&
      document.querySelector(".pdf-hl-target")?.getAttribute("data-highlight-id") === highlightId &&
      document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready") === "true", SECOND_FIRST_ID), {
      timeout: 10_000,
      timeoutMsg: "a block reference did not open PDF B at its exact first highlight",
    });
  } catch (error) {
    const diagnostic = await browser.execute(() => ({
      filename: document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename"),
      target: document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-highlight-target"),
      ready: document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready"),
      highlights: [...document.querySelectorAll(".pdf-hl")].map((element) => ({
        id: element.getAttribute("data-highlight-id"),
        target: element.classList.contains("pdf-hl-target"),
      })),
      toast: document.querySelector(".toast")?.textContent,
      references: [...document.querySelectorAll("[data-block-ref]")].map((element) => ({
        id: element.getAttribute("data-block-ref"),
        className: element.className,
        title: element.getAttribute("title"),
      })),
      location: location.href,
      headings: [...document.querySelectorAll("h1,h2,.page-title")].map((element) => element.textContent),
    }));
    throw new Error(`${error.message}: ${JSON.stringify(diagnostic)}`, { cause: error });
  }
  const secondViewerElementId = (await browser.$(".pdf-viewer")).elementId;
  const secondZoom = await browser.$(".pdf-zoom-level").getText();
  if (secondZoom !== "125%") throw new Error(`second PDF inherited the first PDF scale: ${secondZoom}`);
  if (fs.readFileSync(secondSidecar, "utf8") !== originalSecondSidecar) {
    throw new Error("switching from PDF A wrote A's pending state or baseline into PDF B");
  }

  await browser.$(`[data-block-ref="${SECOND_SECOND_ID}"]`).click();
  await browser.waitUntil(() => browser.execute((highlightId) =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-highlight-target") === highlightId &&
    document.querySelector(".pdf-hl-target")?.getAttribute("data-highlight-id") === highlightId, SECOND_SECOND_ID), {
    timeout: 10_000,
    timeoutMsg: "a same-PDF block reference did not scroll to its exact second highlight",
  });
  if ((await browser.$(".pdf-viewer")).elementId !== secondViewerElementId) {
    throw new Error("same-PDF highlight navigation remounted the PDF resource");
  }
  if (fs.readFileSync(secondSidecar, "utf8") !== originalSecondSidecar) {
    throw new Error("same-PDF highlight navigation rewrote the sidecar");
  }

  // OG direct-link parity: reopening the already-current resource without a
  // page/highlight intent is a no-op, not an implicit jump to page 1. The exact
  // current highlight and the mounted reader identity must survive.
  pdfLinks = await browser.$$(".pdf-link");
  await pdfLinks[1].click();
  await sleep(250);
  const sameResourceDirectState = await browser.execute(() => ({
    target: document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-highlight-target"),
    overlay: document.querySelector(".pdf-hl-target")?.getAttribute("data-highlight-id"),
  }));
  if (sameResourceDirectState.target !== SECOND_SECOND_ID || sameResourceDirectState.overlay !== SECOND_SECOND_ID) {
    throw new Error(`same-resource direct link discarded the current location: ${JSON.stringify(sameResourceDirectState)}`);
  }
  if ((await browser.$(".pdf-viewer")).elementId !== secondViewerElementId) {
    throw new Error("same-resource direct link remounted the PDF resource");
  }

  // Return to A while the journal's real links are still present. Recoloring is
  // a real mutation and must preserve foreign root metadata while writing the
  // UUID/list/corner shape Logseq consumes.
  pdfLinks = await browser.$$(".pdf-link");
  await pdfLinks[0].click();
  await browser.waitUntil(() => browser.execute(() =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "logseq-sample.pdf" &&
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready") === "true"), {
    timeout: 10_000,
    timeoutMsg: "switching back to PDF A did not remount and finish loading its identity",
  });
  const returnedZoom = await browser.$(".pdf-zoom-level").getText();
  if (returnedZoom !== "193%") throw new Error(`PDF A did not restore its own state after B: ${returnedZoom}`);
  await browser.$(".pdf-hl").waitForExist({
    timeout: 10_000,
    timeoutMsg: "PDF A restored its view state before its highlight overlay was ready",
  });
  await browser.execute(() => document.querySelector(".pdf-hl")?.scrollIntoView({ block: "center", inline: "center" }));
  await browser.execute(() => {
    document.querySelector(".pdf-hl")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
  });
  await browser.$(".pdf-color-menu").waitForExist({ timeout: 5000 });
  await browser.execute(() => {
    document.querySelector(".pdf-color-swatch")
      ?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, view: window }));
  });
  await browser.waitUntil(() => {
    const written = fs.readFileSync(sidecar, "utf8");
    return written.includes('#uuid "6a5604f8-a337-4336-a711-2ba6bc14fbfd"') &&
      written.includes(":rects (") && written.includes(":x1 ") &&
      written.includes(":extra {:page 1") && written.includes(':plugin "keep"') && written.includes(":future-root 42");
  }, { timeout: 10_000, timeoutMsg: "Tine did not persist a Logseq-compatible sidecar" });

  // Exercise the sibling entry surface too: navigate to the hls page and click
  // a different rendered annotation badge, not merely an inline ((block ref)).
  await browser.$(`[data-block-ref="${SECOND_FIRST_ID}"]`).click();
  await browser.waitUntil(() => browser.execute((highlightId) =>
    document.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename") === "logseq-second.pdf" &&
    document.querySelector(".pdf-hl-target")?.getAttribute("data-highlight-id") === highlightId, SECOND_FIRST_ID), {
    timeout: 10_000,
    timeoutMsg: "the second B open did not restore its exact first highlight",
  });
  const annotationViewerElementId = (await browser.$(".pdf-viewer")).elementId;
  const hlsLink = browser.$('//a[contains(@class,"ref") and contains(normalize-space(.),"logseq-second")]');
  await hlsLink.waitForExist({ timeout: 10_000 });
  await hlsLink.click();
  await browser.$(`.pdf-annotation-line .hl-prefix[data-highlight-id="${SECOND_SECOND_ID}"]`)
    .waitForExist({ timeout: 10_000 });
  await browser.$(`.pdf-annotation-line .hl-prefix[data-highlight-id="${SECOND_SECOND_ID}"]`).click();
  await browser.waitUntil(() => browser.execute((highlightId) =>
    document.querySelector(".pdf-hl-target")?.getAttribute("data-highlight-id") === highlightId, SECOND_SECOND_ID), {
    timeout: 10_000,
    timeoutMsg: "a rendered AnnotationBody did not navigate to its exact highlight",
  });
  if ((await browser.$(".pdf-viewer")).elementId !== annotationViewerElementId) {
    throw new Error("AnnotationBody navigation remounted the same PDF resource");
  }
  // The literal upload proof drives the native GTK chooser. Keep the existing
  // cross-platform PDF compatibility assertions above in Windows smoke, while
  // running this additional native desktop workflow only on Linux.
  if (process.platform === "linux") await proveNativeUploadsThemesAndHighlights();
  console.log(`PASS: Logseq PDFs keep asset identity separate from exact highlight navigation, with bounded resources, restored view state, OG-compatible notes, correct geometry, and compatible write-back on ${process.platform}`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  killDriverTree();
  stopCiWindowsApp();
  if (wm?.pid) {
    try { process.kill(-wm.pid, "SIGKILL"); } catch {}
  }
  if (wmLog !== undefined) fs.closeSync(wmLog);
  literalReceipt.finishedAt = new Date().toISOString();
  literalReceipt.graph = literalReceipt.graph || graphSnapshot();
  try { fs.writeFileSync(path.join(ARTIFACTS, "pdf-native-receipt.json"), `${JSON.stringify(literalReceipt, null, 2)}\n`); } catch {}
  fs.closeSync(log);
}

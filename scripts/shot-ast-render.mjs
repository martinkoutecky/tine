// Real-app render verification for the lsdoc AST cutover: seeds a kitchen-sink
// page, opens it in the REAL Tauri app (real backend → real lsdoc ASTs) under
// Xvfb via tauri-driver, and screenshots the rendered page so the new AST render
// path can be eyeballed against OG/the old renderer. Usage: node scripts/shot-ast-render.mjs
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";

const G = "/tmp/tgraph-ast";
const APP = "/home/koutecky/research/tine";
const TD = "/aux/koutecky/logseq/.toolchain/cargo/bin/tauri-driver";
const OUT = process.argv[2] || "/tmp/claude-3042/-aux-koutecky-logseq/2e921412-0c07-49c5-87de-46be358044a0/scratchpad/ast-render.png";

const KITCHEN = [
  "- # Heading one",
  "- ## Heading two with a [[Page Ref]]",
  "- TODO [#A] a task with **bold** *italic* ~~strike~~ ==highlight== `code`",
  "- A [[Page Ref]], a #tag, a #[[multi word]] tag, and inline math $x^2 + y^2$",
  "- An external [link](https://example.com) and a bare https://bare.example.com",
  "- ```rust",
  "  fn main() { println!(\"hi\"); }",
  "  ```",
  "- | left | right |",
  "  | --- | --- |",
  "  | a | b |",
  "- > [!NOTE] Heads up",
  "  > a callout body line",
  "- An in-block list:",
  "  * first item",
  "  * second item",
  "    * nested item",
  "- A checklist:",
  "  * [ ] todo",
  "  * [x] done",
  "- > a plain blockquote",
  "- $$ E = mc^2 $$",
  "- ---",
  "- A block with a property",
  "  author:: Martin",
  "",
].join("\n");

function seed() {
  fs.rmSync(G, { recursive: true, force: true });
  fs.mkdirSync(`${G}/pages`, { recursive: true });
  fs.mkdirSync(`${G}/journals`, { recursive: true });
  fs.mkdirSync(`${G}/logseq`, { recursive: true });
  fs.writeFileSync(`${G}/pages/Kitchen.md`, KITCHEN);
  fs.writeFileSync(`${G}/journals/2026_06_28.md`, "- open [[Kitchen]]\n");
}

seed();
fs.rmSync("/tmp/txdg-ast", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/txdg-ast/${d}`, { recursive: true });

// Start a virtual display.
const xvfb = spawn("Xvfb", [":99", "-screen", "0", "1400x1600x24"], { stdio: "ignore" });
await sleep(1500);

const env = {
  ...process.env,
  DISPLAY: ":99",
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/txdg-ast/data",
  XDG_CONFIG_HOME: "/tmp/txdg-ast/config",
  XDG_CACHE_HOME: "/tmp/txdg-ast/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/td-ast.log", "w");
const td = spawn(TD, ["--port", "4446", "--native-port", "4447", "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", tdLog, tdLog] });
await sleep(3000);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: 4446,
    path: "/",
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });

  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20000 });
  await sleep(1200);
  // Open Kitchen from the journal feed.
  for (const sel of ["a.page-ref=Kitchen", "span.page-ref=Kitchen", ".page-ref=Kitchen", "*=Kitchen"]) {
    const el = await browser.$(sel);
    if (await el.isExisting()) { await el.click(); break; }
  }
  // Let blocks parse (parse_blocks IPC) + KaTeX/hljs load.
  await sleep(4000);
  const title = await browser.$(".page-title");
  console.log("PAGE TITLE:", (await title.isExisting()) ? await title.getText() : "(none)");
  await browser.saveScreenshot(OUT);
  console.log("screenshot →", OUT);
} catch (e) {
  console.error("ERROR:", e.message);
} finally {
  try { await browser?.deleteSession(); } catch {}
  td.kill("SIGKILL");
  xvfb.kill("SIGKILL");
}

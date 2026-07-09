// Sheets Phase 0 (spec §8 / §11 phase 0) — THROWAWAY render-perf spike.
// Drives the REAL Tine binary (WebKitGTK) under Xvfb via tauri-driver, injects a
// disposable 50×10 editable grid overlay (three auto-fit strategies + a no-grid
// control), and measures keystroke→paint and resize-reflow latency in-page via
// double-rAF. No app code is touched; results feed the go/no-go gate in
// docs/plans/sheets-progress.md.
//
// Usage: DISPLAY=:99 node scripts/spike-sheets-perf.mjs
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const G = "/tmp/spike-graph";
const APP = process.env.TINE_APP || `${repo}/target/release/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");

// -- seed a minimal graph ----------------------------------------------------
fs.rmSync(G, { recursive: true, force: true });
fs.mkdirSync(`${G}/pages`, { recursive: true });
fs.mkdirSync(`${G}/journals`, { recursive: true });
fs.writeFileSync(`${G}/pages/Spike.md`, "- spike page\n- second block\n");

fs.rmSync("/tmp/spike-xdg", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/spike-xdg/${d}`, { recursive: true });
const env = {
  ...process.env,
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/spike-xdg/data",
  XDG_CONFIG_HOME: "/tmp/spike-xdg/config",
  XDG_CACHE_HOME: "/tmp/spike-xdg/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/spike-td.log", "w");
const td = spawn(
  TD,
  ["--port", "4444", "--native-port", "4445", "--native-driver", "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", tdLog, tdLog] }
);
await sleep(3000);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: 4444,
    path: "/",
    capabilities: {
      browserName: "wry",
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP },
    },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20000 });
  await browser.setTimeout({ script: 300000 });

  // One in-page async run per variant. cfg: {kind, rows, cols, nested}
  const runVariant = (cfg) =>
    browser.executeAsync(function (cfg, done) {
      (async () => {
        const raf2 = () =>
          new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
        const words = ["alpha", "beta rho", "some text", "TODO item", "42", "lorem ipsum dolor"];
        const stats = (a) => {
          const s = [...a].sort((x, y) => x - y);
          const q = (p) => s[Math.min(s.length - 1, Math.floor(p * s.length))];
          return {
            n: s.length,
            median: +q(0.5).toFixed(2),
            p95: +q(0.95).toFixed(2),
            max: +s[s.length - 1].toFixed(2),
          };
        };

        // ---- build the overlay ----
        const old = document.getElementById("spike-overlay");
        if (old) old.remove();
        const ov = document.createElement("div");
        ov.id = "spike-overlay";
        ov.style.cssText =
          "position:fixed;inset:0;z-index:99999;background:#fff;color:#111;overflow:auto;font:13px sans-serif;padding:8px";
        document.body.appendChild(ov);

        let focusCell; // the cell we type into
        const mkNested = (mkCellText) => {
          // 5×5 sub-grid, same strategy as host, inside one cell
          const n = document.createElement(cfg.kind === "table" ? "table" : "div");
          if (cfg.kind === "table") {
            n.style.cssText = "table-layout:auto;border-collapse:collapse";
            for (let r = 0; r < 5; r++) {
              const tr = n.insertRow();
              for (let c = 0; c < 5; c++) {
                const td = tr.insertCell();
                td.style.cssText = "border:1px solid #bbb;padding:2px 6px";
                td.textContent = mkCellText(r, c);
              }
            }
          } else {
            n.style.cssText =
              "display:grid;grid-template-columns:repeat(5," +
              (cfg.kind === "grid-fixed" ? "60px" : "max-content") +
              ");gap:1px;background:#bbb";
            for (let i = 0; i < 25; i++) {
              const d = document.createElement("div");
              d.style.cssText = "background:#fff;padding:2px 6px";
              d.textContent = mkCellText((i / 5) | 0, i % 5);
              n.appendChild(d);
            }
          }
          return n;
        };
        const cellText = (r, c) => words[(r * 7 + c * 3) % words.length];

        if (cfg.kind === "control") {
          const d = document.createElement("div");
          d.contentEditable = "true";
          d.style.cssText = "border:1px solid #bbb;padding:4px;width:400px";
          d.textContent = "control cell";
          ov.appendChild(d);
          focusCell = d;
        } else if (cfg.kind === "table") {
          const t = document.createElement("table");
          t.style.cssText = "table-layout:auto;border-collapse:collapse";
          for (let r = 0; r < cfg.rows; r++) {
            const tr = t.insertRow();
            for (let c = 0; c < cfg.cols; c++) {
              const td = tr.insertCell();
              td.style.cssText = "border:1px solid #bbb;padding:2px 6px;vertical-align:top";
              td.contentEditable = "true";
              td.textContent = cellText(r, c);
              if (r === 25 && c === 5) focusCell = td;
              if (cfg.nested && r === 20 && c === 3) {
                td.contentEditable = "false";
                td.textContent = "";
                td.appendChild(mkNested(cellText));
              }
            }
          }
          ov.appendChild(t);
        } else {
          // CSS grid: grid-maxcontent | grid-fixed
          const g = document.createElement("div");
          g.style.cssText =
            "display:grid;grid-template-columns:repeat(" +
            cfg.cols +
            "," +
            (cfg.kind === "grid-fixed" ? "120px" : "max-content") +
            ");gap:1px;background:#bbb;width:max-content";
          for (let r = 0; r < cfg.rows; r++)
            for (let c = 0; c < cfg.cols; c++) {
              const d = document.createElement("div");
              d.style.cssText = "background:#fff;padding:2px 6px;min-width:20px";
              d.contentEditable = "true";
              d.textContent = cellText(r, c);
              if (r === 25 && c === 5) focusCell = d;
              if (cfg.nested && r === 20 && c === 3) {
                d.contentEditable = "false";
                d.textContent = "";
                d.appendChild(mkNested(cellText));
              }
              g.appendChild(d);
            }
          ov.appendChild(g);
        }

        await raf2(); // initial layout settles
        const res = { kind: cfg.kind, rows: cfg.rows, cols: cfg.cols };
        const root = ov.children[0];
        // Sync layout cost = mutate → forced flush (offsetHeight read). This is
        // the number that matters: double-rAF saturates at the ~2-frame floor
        // (~32 ms here) and can't distinguish variants. Frame floor recorded once.
        const flush = () => void root.offsetHeight;
        {
          const t0 = performance.now();
          await raf2();
          res.frameFloorMs = +(performance.now() - t0).toFixed(2);
        }

        // ---- 0. initial build+layout of the whole grid ----
        res.initialLayoutMs = (() => {
          root.style.display = "none";
          void ov.offsetHeight;
          const t0 = performance.now();
          root.style.display = cfg.kind === "table" ? "table" : cfg.kind === "control" ? "block" : "grid";
          flush();
          return +(performance.now() - t0).toFixed(2);
        })();

        // ---- 1. keystroke: insertText → forced layout ----
        focusCell.focus();
        const sel = window.getSelection();
        sel.selectAllChildren(focusCell);
        sel.collapseToEnd();
        const type = [];
        for (let i = 0; i < 40; i++) {
          const t0 = performance.now();
          document.execCommand("insertText", false, "x");
          flush();
          type.push(performance.now() - t0);
          if (i % 8 === 7) await raf2(); // let paints happen occasionally
        }
        res.type = stats(type);

        // ---- 2. resize-reflow: alternate short/long content (forces track resize) ----
        const grow = [];
        for (let i = 0; i < 30; i++) {
          const t0 = performance.now();
          focusCell.textContent =
            i % 2 ? "short" : "a-much-longer-piece-of-cell-content-forcing-column-growth";
          flush();
          grow.push(performance.now() - t0);
          if (i % 6 === 5) await raf2();
        }
        res.grow = stats(grow);

        // ---- 3. row insert/remove at the middle (seam responsiveness) ----
        const rowop = [];
        const midRow = Math.min(25, cfg.rows - 1);
        if (cfg.kind === "table") {
          const t = root;
          for (let i = 0; i < 12; i++) {
            const t0 = performance.now();
            const tr = t.insertRow(midRow);
            for (let c = 0; c < cfg.cols; c++) {
              const td = tr.insertCell();
              td.style.cssText = "border:1px solid #bbb;padding:2px 6px";
              td.textContent = "new";
            }
            flush();
            rowop.push(performance.now() - t0);
            tr.remove();
            flush();
            if (i % 4 === 3) await raf2();
          }
        } else if (cfg.kind !== "control") {
          const g = root;
          for (let i = 0; i < 12; i++) {
            const t0 = performance.now();
            const frag = document.createDocumentFragment();
            for (let c = 0; c < cfg.cols; c++) {
              const d = document.createElement("div");
              d.style.cssText = "background:#fff;padding:2px 6px";
              d.textContent = "new";
              frag.appendChild(d);
            }
            g.insertBefore(frag, g.children[midRow * cfg.cols]);
            flush();
            rowop.push(performance.now() - t0);
            for (let c = 0; c < cfg.cols; c++) g.children[midRow * cfg.cols].remove();
            flush();
            if (i % 4 === 3) await raf2();
          }
        }
        if (rowop.length) res.rowInsert = stats(rowop);

        ov.remove();
        done(res);
      })().catch((e) => done({ kind: cfg.kind, error: String(e && e.stack || e) }));
    }, cfg);

  const results = [];
  for (const cfg of [
    { kind: "control", rows: 1, cols: 1, nested: false },
    { kind: "table", rows: 50, cols: 10, nested: true },
    { kind: "grid-maxcontent", rows: 50, cols: 10, nested: true },
    { kind: "grid-fixed", rows: 50, cols: 10, nested: true },
    // stress: the cap/log decision point (spec §8 "log/cap on very large grids")
    { kind: "table", rows: 200, cols: 10, nested: true },
    { kind: "grid-maxcontent", rows: 200, cols: 10, nested: true },
    { kind: "grid-fixed", rows: 200, cols: 10, nested: true },
  ]) {
    console.log(`--- variant: ${cfg.kind} ${cfg.rows}x${cfg.cols}`);
    const r = await runVariant(cfg);
    console.log(JSON.stringify(r));
    results.push(r);
  }

  fs.writeFileSync(
    "/tmp/spike-results.json",
    JSON.stringify({ when: new Date().toISOString(), display: process.env.DISPLAY, results }, null, 2)
  );
  console.log("\nWrote /tmp/spike-results.json");
} finally {
  try { await browser?.deleteSession(); } catch {}
  td.kill();
}

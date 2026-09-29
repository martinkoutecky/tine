// Deterministic Chromium acceptance for the system-inset gutters (port of
// master e3b1c3868; GH #205). jsdom applies no layout, so this is the layer
// that can see whether a fixed overlay lands under the system bars.
//
// `.app-container` pads the app SHELL by env(safe-area-inset-*), but every
// `position: fixed` overlay is laid out against the viewport and escapes that
// padding — which is how the Settings modal ended up under the transparent
// status bar on Android (Martin, 2026-08-18). This harness mounts the real App
// with the real stylesheets at a phone viewport and MEASURES where each
// overlay's content actually lands.
//
// Desktop Chromium always reports env(safe-area-inset-*) as 0, so the insets
// are injected through the same four --overlay-inset-* custom properties the
// production CSS reads. That is the only substitution: the padding arithmetic,
// the overlays and the components under test are production code.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { createServer } from "vite";
import solid from "vite-plugin-solid";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ENTRY_REQUEST = "/__tine_mobile_safe_area.tsx";
const ENTRY_ID = "virtual:tine-mobile-safe-area.tsx";

// A Pixel-class portrait viewport with a transparent status bar and a gesture
// bar. The exact numbers do not matter — only that no surface may assume them.
const VIEWPORT = { width: 412, height: 915 };
const INSETS = { top: 44, right: 0, bottom: 24, left: 0 };

function assert(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details === undefined ? "" : `: ${JSON.stringify(details)}`}`);
  }
}

const entrySource = String.raw`
  import { render } from "solid-js/web";
  import { App } from "/src/App.tsx";
  // main.tsx, not App.tsx, is what pulls the stylesheets into the real app.
  // Without these two imports the overlays have no CSS at all and every
  // measurement below is meaningless.
  import "/src/styles/theme.css";
  import "/src/styles/app.css";
  import {
    closeExportModal,
    closePdfExport,
    closeSettings,
    closeSwitcher,
    closeWelcome,
    openExportModal,
    openPdfExport,
    openSettings,
    openSwitcher,
    openWelcome,
  } from "/src/ui.ts";
  import { pushToast } from "/src/toasts.ts";

  // app.css defines --overlay-inset-* as env(safe-area-inset-*), which desktop
  // Chromium always reports as 0. An inline style on the root element outranks
  // any stylesheet, so this is the one place the harness substitutes a value.
  for (const [name, value] of Object.entries(globalThis.__tineHarnessInsets)) {
    document.documentElement.style.setProperty("--overlay-inset-" + name, value + "px");
  }

  render(() => <App />, document.getElementById("root"));

  const surfaces = {
    settings: {
      open: () => openSettings(),
      close: closeSettings,
      // The overlay is the padded box; the modal is what the user can reach.
      content: ".settings-modal",
    },
    "copy-export": {
      open: () => openExportModal(["nonexistent-block"]),
      close: closeExportModal,
      content: ".modal-overlay > *",
    },
    "print-to-pdf": {
      open: () => openPdfExport("Some page"),
      close: closePdfExport,
      content: ".modal-overlay > *",
    },
    "quick-switcher": {
      open: () => openSwitcher(),
      close: closeSwitcher,
      content: ".switcher",
    },
    welcome: {
      open: () => openWelcome(),
      close: closeWelcome,
      content: ".welcome-card",
    },
    toasts: {
      open: () => pushToast("Safe-area probe", "info", { sticky: true }),
      close: () => {
        for (const button of document.querySelectorAll(".toast .toast-close")) button.click();
      },
      content: ".toast-stack",
    },
    "help-fab": {
      open: () => {},
      close: () => {},
      content: ".help-corner",
    },
  };

  globalThis.__tineSafeArea = {
    names: Object.keys(surfaces),
    open(name) { surfaces[name].open(); },
    close(name) { surfaces[name].close(); },
    measure(name) {
      const element = document.querySelector(surfaces[name].content);
      if (!element) return null;
      const rect = element.getBoundingClientRect();
      return { top: rect.top, right: rect.right, bottom: rect.bottom, left: rect.left };
    },
    ready: true,
  };
`;

function harnessHtml(insets) {
  return `<!doctype html>
    <html><head>
      <meta charset="utf-8">
      <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
      <style>
        html, body, #root { width: 100%; height: 100%; margin: 0; }
        body { overflow: hidden; }
      </style>
      <script>globalThis.__tineHarnessInsets = ${JSON.stringify(insets)};</script>
    </head><body><div id="root"></div><script type="module" src="${ENTRY_REQUEST}"></script></body></html>`;
}

async function startHarnessServer() {
  const server = await createServer({
    root: ROOT,
    configFile: false,
    appType: "custom",
    logLevel: "error",
    // vite.config.ts injects these at build time; the app reads them at import
    // time, so the harness has to supply the same three constants.
    define: {
      __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00.000Z"),
      __GIT_COMMIT__: JSON.stringify(""),
      __TINE_COMMUNITY_REGISTRY__: JSON.stringify(false),
    },
    cacheDir: path.join(ROOT, "test-results", "mobile-safe-area", ".vite-cache"),
    plugins: [
      {
        name: "tine-mobile-safe-area-harness",
        enforce: "pre",
        configureServer(devServer) {
          devServer.middlewares.use((request, response, next) => {
            const pathname = new URL(request.url || "/", "http://safe-area.invalid").pathname;
            if (pathname !== "/") return next();
            response.statusCode = 200;
            response.setHeader("content-type", "text/html; charset=utf-8");
            response.end(harnessHtml(INSETS));
          });
        },
        resolveId(source) {
          return source === ENTRY_REQUEST ? ENTRY_ID : null;
        },
        load(id) {
          return id === ENTRY_ID ? entrySource : null;
        },
      },
      solid(),
    ],
    server: { host: "127.0.0.1", port: 0, strictPort: false },
  });
  await server.listen();
  const address = server.httpServer?.address();
  assert(address && typeof address !== "string", "Vite safe-area harness did not expose a TCP address");
  return { server, url: `http://127.0.0.1:${address.port}/` };
}

const { server, url } = await startHarnessServer();
const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
const failures = [];
try {
  const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 2, hasTouch: true });
  const page = await context.newPage();
  page.on("pageerror", (error) => failures.push(`page error: ${error.message}`));
  if (process.env.TINE_SAFE_AREA_DEBUG) {
    page.on("console", (message) => console.log(`[page:${message.type()}]`, message.text()));
    page.on("pageerror", (error) => console.log("[pageerror]", error.stack || error.message || String(error)));
    page.on("response", (response) => { if (response.status() >= 400) console.log("[http]", response.status(), response.url()); });
  }
  await page.goto(url, { waitUntil: "load" });
  await page.waitForFunction(() => globalThis.__tineSafeArea?.ready === true, null, { timeout: 60_000 });
  await page.waitForSelector("header.topbar", { timeout: 60_000 });

  const safe = {
    top: INSETS.top,
    right: VIEWPORT.width - INSETS.right,
    bottom: VIEWPORT.height - INSETS.bottom,
    left: INSETS.left,
  };
  const names = await page.evaluate(() => globalThis.__tineSafeArea.names);
  const measurements = {};

  for (const name of names) {
    await page.evaluate((surface) => globalThis.__tineSafeArea.open(surface), name);
    const rect = await page.evaluate(async (surface) => {
      const api = globalThis.__tineSafeArea;
      for (let attempt = 0; attempt < 60; attempt += 1) {
        const measured = api.measure(surface);
        if (measured) return measured;
        await new Promise((resolve) => requestAnimationFrame(() => resolve(undefined)));
      }
      return null;
    }, name);

    if (!rect) {
      failures.push(`${name}: the harness could not find its content, so nothing was measured`);
    } else {
      measurements[name] = rect;
      // Half a pixel of tolerance for device-pixel rounding, and no more: the
      // whole point is that a surface must not sit under the system bars.
      if (rect.top < safe.top - 0.5) failures.push(`${name}: top ${rect.top} is above the status-bar inset ${safe.top}`);
      if (rect.bottom > safe.bottom + 0.5) failures.push(`${name}: bottom ${rect.bottom} is below the gesture-bar inset boundary ${safe.bottom}`);
      if (rect.left < safe.left - 0.5) failures.push(`${name}: left ${rect.left} is outside the left inset ${safe.left}`);
      if (rect.right > safe.right + 0.5) failures.push(`${name}: right ${rect.right} is outside the right inset ${safe.right}`);
    }
    await page.evaluate((surface) => globalThis.__tineSafeArea.close(surface), name);
    await page.waitForTimeout(50);
  }

  const artifacts = path.join(ROOT, "test-results", "mobile-safe-area");
  fs.mkdirSync(artifacts, { recursive: true });
  fs.writeFileSync(
    path.join(artifacts, "measurements.json"),
    `${JSON.stringify({ viewport: VIEWPORT, insets: INSETS, safe, measurements, failures }, null, 2)}\n`,
  );
} finally {
  await browser.close();
  await server.close();
}

if (failures.length) {
  console.error(`Mobile safe-area check failed (${failures.length} problem(s)):`);
  for (const failure of failures) console.error(`  ${failure}`);
  process.exit(1);
}
console.log(`Mobile safe-area OK: every measured fixed overlay stays inside ${JSON.stringify(INSETS)} on ${VIEWPORT.width}x${VIEWPORT.height}.`);

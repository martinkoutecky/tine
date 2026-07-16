// Deterministic Chromium acceptance for GH #161's width-driven drawer shell.
//
// Unlike the original static HTML fixture, this harness runtime-imports the
// production classifier, UI state, MobileDrawerShell components, and CSS through
// Vite. It intentionally does not mount the whole backend-owning App: mounted
// App/Sidebar/RightSidebar semantics remain covered by the render/native suites.
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { createServer } from "vite";
import solid from "vite-plugin-solid";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ARTIFACTS = process.env.E1_ARTIFACT_DIR
  ? path.resolve(process.env.E1_ARTIFACT_DIR)
  : path.join(ROOT, "test-results", "issue-161-browser-drawers");
const ENTRY_REQUEST = "/__tine_e1_mobile_drawers.tsx";
const ENTRY_ID = "virtual:tine-e1-mobile-drawers.tsx";
const HEIGHT = 720;
const EPSILON = 1;

const productionInputs = [
  "src/components/MobileDrawerShell.tsx",
  "src/mobileDrawers.ts",
  "src/ui.ts",
  "src/styles/theme.css",
  "src/styles/app.css",
];

function assert(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details === undefined ? "" : `: ${JSON.stringify(details)}`}`);
  }
}

function near(left, right, tolerance = EPSILON) {
  return Math.abs(left - right) <= tolerance;
}

function sameRect(left, right) {
  return left && right
    && near(left.x, right.x)
    && near(left.y, right.y)
    && near(left.width, right.width)
    && near(left.height, right.height);
}

function fingerprints() {
  return Object.fromEntries(productionInputs.map((relative) => {
    const bytes = fs.readFileSync(path.join(ROOT, relative));
    return [relative, createHash("sha256").update(bytes).digest("hex")];
  }));
}

const entrySource = String.raw`
  import { Show } from "solid-js";
  import { render } from "solid-js/web";
  import {
    DrawerBackground,
    MobileDrawerController,
    MobileDrawerPanel,
    dismissDrawerAndRestore,
  } from "/src/components/MobileDrawerShell.tsx";
  import { mobileDrawerMode } from "/src/mobileDrawers.ts";
  import {
    activeDrawer,
    rightSidebarOpen,
    setLeftSidebarOpen,
    setRightSidebarOpen,
    setRightSidebarWidth,
    setSidebarWidth,
    rightSidebarWidth,
    sidebarOpen,
    sidebarWidth,
  } from "/src/ui.ts";
  import "/src/styles/theme.css";
  import "/src/styles/app.css";

  const observations = {
    underActivations: { left: 0, right: 0 },
    lastScrimPointerDown: null,
    lastScrimClick: null,
  };

  setLeftSidebarOpen(false);
  setRightSidebarOpen(false);
  setSidebarWidth(280);
  setRightSidebarWidth(320);

  function leftStyle() {
    const width = sidebarWidth() + "px";
    return { flex: "0 0 " + width, width, "--mobile-drawer-width": width };
  }

  function rightStyle() {
    const width = rightSidebarWidth() + "px";
    return { flex: "0 0 " + width, width, "--mobile-drawer-width": width };
  }

  function Shell() {
    return (
      <div
        class="app-container e1-shell"
        data-mobile-drawer-mode={mobileDrawerMode() ? "true" : "false"}
        data-active-drawer={activeDrawer() ?? ""}
      >
        <Show when={sidebarOpen()}>
          <MobileDrawerPanel side="left" label="Navigation sidebar" class="left-sidebar" style={leftStyle()}>
            <div class="left-sidebar-scroll">
              <Show when={mobileDrawerMode()}>
                <button class="mobile-drawer-close e1-left-close" onClick={() => dismissDrawerAndRestore("explicit")}>Close navigation sidebar</button>
              </Show>
              <button class="e1-left-action">Left drawer action</button>
            </div>
            <div class="sidebar-resizer" data-e1-resizer="left" />
          </MobileDrawerPanel>
        </Show>

        <DrawerBackground class="main-container" blockedBy="left">
          <DrawerBackground class="e1-top-region" blockedBy="right">
            <button class="e1-open-left" onClick={(event) => setLeftSidebarOpen(true, event.currentTarget)}>Open left</button>
            <button class="e1-open-right" onClick={(event) => setRightSidebarOpen(true, event.currentTarget)}>Open right</button>
            <div class="resize-grip grip-n" data-e1-resizer="window" />
          </DrawerBackground>
          <div class="content-row">
            <DrawerBackground class="drawer-workspace" blockedBy="right">
              <main class="main-content pane-focused" tabindex={-1}>
                <div class="e1-workspace-label">Underlying workspace</div>
                <button
                  class="e1-under e1-under-left"
                  onClick={() => { observations.underActivations.left += 1; }}
                >Under left edge</button>
                <button
                  class="e1-under e1-under-right"
                  onClick={() => { observations.underActivations.right += 1; }}
                >Under right edge</button>
              </main>
            </DrawerBackground>
            <Show when={rightSidebarOpen()}>
              <MobileDrawerPanel side="right" label="Reference sidebar" class="right-sidebar" style={rightStyle()}>
                <div class="rs-resizer" data-e1-resizer="right" />
                <div class="right-sidebar-header">
                  <span>Reference sidebar</span>
                  <button class="e1-right-close" onClick={() => dismissDrawerAndRestore("explicit")}>Close reference sidebar</button>
                </div>
                <button class="e1-right-action">Right drawer action</button>
              </MobileDrawerPanel>
            </Show>
          </div>
        </DrawerBackground>

        <MobileDrawerController />
        <DrawerBackground class="e1-floating-region" blockedBy="any">
          <button>Ordinary floating control</button>
        </DrawerBackground>
      </div>
    );
  }

  render(() => <Shell />, document.getElementById("root"));

  // Solid installs delegated handlers while rendering. Register this observer
  // afterwards so it records the final prevention/propagation state produced by
  // the production scrim handlers.
  for (const type of ["pointerdown", "click"]) {
    document.addEventListener(type, (event) => {
      if (!(event.target instanceof Element) || !event.target.closest("[data-mobile-drawer-scrim]")) return;
      queueMicrotask(() => {
        const value = {
          defaultPrevented: event.defaultPrevented,
          cancelBubble: event.cancelBubble,
          target: event.target instanceof Element ? event.target.className : "",
        };
        if (type === "pointerdown") observations.lastScrimPointerDown = value;
        else observations.lastScrimClick = value;
      });
    });
  }

  globalThis.__tineE1 = {
    openLeft() {
      setLeftSidebarOpen(true, document.querySelector(".e1-open-left"));
    },
    openRight() {
      setRightSidebarOpen(true, document.querySelector(".e1-open-right"));
    },
    closeAll() {
      setLeftSidebarOpen(false);
      setRightSidebarOpen(false);
    },
    setWidths(left, right) {
      setSidebarWidth(left);
      setRightSidebarWidth(right);
    },
    resetObservations() {
      observations.underActivations.left = 0;
      observations.underActivations.right = 0;
      observations.lastScrimPointerDown = null;
      observations.lastScrimClick = null;
    },
    observations,
    ready: true,
  };
`;

const harnessCss = `
  html, body, #root { width: 100%; height: 100%; margin: 0; }
  body { overflow: hidden; }
  .e1-top-region { height: 52px; flex: 0 0 52px; display: flex; align-items: center; gap: 8px; padding: 0 10px; }
  .e1-shell .main-content { flex: 1; min-width: 0; overflow: hidden; }
  .e1-workspace-label { padding: 20px; }
  .e1-under { position: fixed; top: 156px; z-index: 1; width: 38px; height: 56px; padding: 0; font-size: 8px; }
  .e1-under-left { left: 3px; }
  .e1-under-right { right: 3px; }
  .e1-floating-region { position: fixed; left: 50%; bottom: 8px; z-index: 5; }
  .left-sidebar, .right-sidebar { min-height: 0; }
`;

function harnessHtml() {
  return `<!doctype html>
    <html><head>
      <meta charset="utf-8">
      <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
      <style>${harnessCss}</style>
    </head><body><div id="root"></div><script type="module" src="${ENTRY_REQUEST}"></script></body></html>`;
}

async function startHarnessServer() {
  const server = await createServer({
    root: ROOT,
    configFile: false,
    appType: "custom",
    logLevel: "error",
    cacheDir: path.join(ARTIFACTS, ".vite-cache"),
    plugins: [
      {
        name: "tine-e1-browser-harness",
        enforce: "pre",
        configureServer(devServer) {
          devServer.middlewares.use((request, response, next) => {
            const pathname = new URL(request.url || "/", "http://e1.invalid").pathname;
            if (pathname !== "/") return next();
            response.statusCode = 200;
            response.setHeader("content-type", "text/html; charset=utf-8");
            response.end(harnessHtml());
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
  assert(address && typeof address !== "string", "Vite E1 harness did not expose a TCP address");
  return { server, url: `http://127.0.0.1:${address.port}/` };
}

async function snapshot(page) {
  return page.evaluate(() => {
    const box = (selector) => {
      const element = document.querySelector(selector);
      if (!element) return null;
      const rect = element.getBoundingClientRect();
      return {
        x: rect.x, y: rect.y, width: rect.width, height: rect.height,
        left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom,
      };
    };
    const state = (selector) => {
      const element = document.querySelector(selector);
      if (!element) return null;
      const style = getComputedStyle(element);
      return {
        box: box(selector),
        display: style.display,
        position: style.position,
        inert: element.inert || element.hasAttribute("inert"),
        role: element.getAttribute("role"),
        ariaModal: element.getAttribute("aria-modal"),
        ariaLabel: element.getAttribute("aria-label"),
      };
    };
    const app = document.querySelector(".app-container");
    const appStyle = getComputedStyle(app);
    return {
      viewport: { width: innerWidth, height: innerHeight },
      classifier: app.dataset.mobileDrawerMode,
      activeDrawer: app.dataset.activeDrawer,
      media: {
        max639: matchMedia("(max-width: 639px)").matches,
        fine: matchMedia("(pointer: fine)").matches,
        coarse: matchMedia("(pointer: coarse)").matches,
        hover: matchMedia("(hover: hover)").matches,
        touchPoints: navigator.maxTouchPoints,
        userAgent: navigator.userAgent,
      },
      safeArea: {
        top: Number.parseFloat(appStyle.paddingTop),
        right: Number.parseFloat(appStyle.paddingRight),
        bottom: Number.parseFloat(appStyle.paddingBottom),
        left: Number.parseFloat(appStyle.paddingLeft),
      },
      workspace: box(".drawer-workspace"),
      main: box(".main-content"),
      left: state(".left-sidebar"),
      right: state(".right-sidebar"),
      scrim: state("[data-mobile-drawer-scrim]"),
      scrimCount: document.querySelectorAll("[data-mobile-drawer-scrim]").length,
      panelCount: document.querySelectorAll("[data-mobile-drawer-panel]").length,
      backgrounds: [...document.querySelectorAll("[data-drawer-background]")].map((element) => ({
        className: element.className,
        blockedBy: element.dataset.drawerBackground,
        inert: element.inert || element.hasAttribute("inert"),
      })),
      resizers: {
        left: document.querySelector("[data-e1-resizer='left']")
          ? getComputedStyle(document.querySelector("[data-e1-resizer='left']")).display : "missing",
        right: document.querySelector("[data-e1-resizer='right']")
          ? getComputedStyle(document.querySelector("[data-e1-resizer='right']")).display : "missing",
        window: getComputedStyle(document.querySelector("[data-e1-resizer='window']")).display,
      },
      focusClass: document.activeElement?.className || document.activeElement?.tagName || "",
      observations: JSON.parse(JSON.stringify(globalThis.__tineE1.observations)),
    };
  });
}

async function waitForMode(page, expected) {
  await page.waitForFunction((value) =>
    document.querySelector(".app-container")?.getAttribute("data-mobile-drawer-mode") === value,
  expected ? "true" : "false");
}

async function waitForDrawer(page, side) {
  await page.waitForFunction((value) =>
    document.querySelector(".app-container")?.getAttribute("data-active-drawer") === value,
  side);
}

async function openByPointer(page, side) {
  await page.locator(side === "left" ? ".e1-open-left" : ".e1-open-right").click();
  await page.waitForFunction(({ side: expected, compact }) => {
    const root = document.querySelector(".app-container");
    if (!root) return false;
    if (compact) return root.getAttribute("data-active-drawer") === expected;
    return document.querySelector(`.${expected}-sidebar`) != null;
  }, { side, compact: await page.evaluate(() => globalThis.__tineE1 != null && document.querySelector(".app-container")?.dataset.mobileDrawerMode === "true") });
}

async function clickProvenUnderTarget(page, side) {
  const edge = side === "left" ? "right" : "left";
  const selector = edge === "left" ? ".e1-under-left" : ".e1-under-right";
  await page.evaluate(() => globalThis.__tineE1.resetObservations());
  await page.locator(selector).click();
  const liveSpy = await page.evaluate((key) => globalThis.__tineE1.observations.underActivations[key], edge);
  assert(liveSpy === 1, `underlying ${side} activation spy was not live before opening the drawer`, liveSpy);
  await page.evaluate(() => globalThis.__tineE1.resetObservations());
  return { selector, edge };
}

async function consumeScrimAtTarget(page, side, target) {
  const point = await page.locator(target.selector).evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  });
  const hit = await page.evaluate(({ x, y }) => {
    const element = document.elementFromPoint(x, y);
    return { className: element?.className || "", isScrim: Boolean(element?.closest("[data-mobile-drawer-scrim]")) };
  }, point);
  assert(hit.isScrim, `${side} scrim did not own the underlying target coordinates`, { point, hit });
  await page.mouse.click(point.x, point.y);
  await page.waitForFunction(() => document.querySelectorAll("[data-mobile-drawer-scrim]").length === 0);
  await page.evaluate(() => new Promise((resolve) => queueMicrotask(resolve)));
  const result = await snapshot(page);
  assert(result.observations.underActivations[target.edge] === 0,
    `${side} scrim click activated the underlying control`, result.observations);
  assert(result.observations.lastScrimPointerDown?.defaultPrevented === true,
    `${side} scrim pointerdown was not default-prevented`, result.observations);
  assert(result.observations.lastScrimClick?.defaultPrevented === true,
    `${side} scrim click was not default-prevented`, result.observations);
  assert(result.observations.lastScrimPointerDown?.cancelBubble === true
    && result.observations.lastScrimClick?.cancelBubble === true,
  `${side} scrim did not stop pointer/click propagation`, result.observations);
  return { point, hit, after: result };
}

function assertCompactState(state, side, closed) {
  const panel = state[side];
  assert(state.classifier === "true" && state.media.max639,
    `${side} compact state did not come from the production max-639 classifier`, state);
  assert(state.activeDrawer === side && state.panelCount === 1,
    `${side} compact state did not own exactly one panel`, state);
  assert(state.scrimCount === 1 && state.scrim?.display !== "none",
    `${side} compact state did not expose one consuming scrim`, state);
  assert(panel?.role === "dialog" && panel.ariaModal === "true" && panel.ariaLabel,
    `${side} compact panel lacks production dialog semantics`, panel);
  assert(panel.position === "fixed", `${side} compact panel is not an overlay`, panel);
  assert(sameRect(state.workspace, closed.workspace),
    `${side} compact panel changed the underlying workspace rectangle`, { closed: closed.workspace, open: state.workspace });
  assert(state.resizers.window === "none", `${side} compact mode left the window resize grip visible`, state.resizers);
  assert(state.resizers[side] === "none", `${side} compact mode left its sidebar resizer visible`, state.resizers);
  const cap = state.viewport.width - 44 - state.safeArea.left - state.safeArea.right;
  assert(panel.box.width <= cap + EPSILON, `${side} drawer exceeded its safe-area-aware width cap`, { panel: panel.box, cap, state });
  if (side === "left") {
    assert(near(panel.box.left, state.safeArea.left), "left drawer is not anchored after the left safe area", state);
    assert(state.backgrounds.some((entry) => entry.className.includes("main-container") && entry.inert),
      "left drawer did not inert the production main background", state.backgrounds);
  } else {
    assert(near(panel.box.right, state.viewport.width - state.safeArea.right),
      "right drawer is not anchored before the right safe area", state);
    assert(state.backgrounds.some((entry) => entry.className.includes("e1-top-region") && entry.inert)
      && state.backgrounds.some((entry) => entry.className.includes("drawer-workspace") && entry.inert),
    "right drawer did not inert top/workspace backgrounds", state.backgrounds);
  }
  assert(state.backgrounds.some((entry) => entry.className.includes("e1-floating-region") && entry.inert),
    `${side} drawer left an ordinary floating region interactive`, state.backgrounds);
}

function assertCleanClosed(state) {
  assert(state.scrimCount === 0 && state.panelCount === 0 && state.activeDrawer === "",
    "drawer close left stale panel/scrim state", state);
  assert(state.backgrounds.every((entry) => !entry.inert),
    "drawer close left stale inert state", state.backgrounds);
}

function assertPersistentState(state, width) {
  assert(state.viewport.width === width && state.classifier === "false" && !state.media.max639,
    `persistent state at ${width}px did not come from the production classifier`, state);
  assert(state.activeDrawer === "" && state.scrimCount === 0 && state.panelCount === 2,
    `persistent state at ${width}px did not retain both sidebars without a scrim`, state);
  assert(state.backgrounds.every((entry) => !entry.inert),
    `persistent state at ${width}px retained drawer inert state`, state.backgrounds);
  assert(state.left?.role === null && state.left?.ariaModal === null
    && state.right?.role === null && state.right?.ariaModal === null,
  `persistent state at ${width}px retained modal semantics`, state);
  assert(state.left?.position === "relative" && state.right?.position === "relative",
    `persistent state at ${width}px did not use ordinary flex sidebars`, state);
  assert(state.resizers.left !== "none" && state.resizers.right !== "none" && state.resizers.window !== "none",
    `persistent state at ${width}px hid a resize seam`, state.resizers);
}

const profiles = {
  fineDesktop: {
    name: "fine-desktop",
    context: { isMobile: false, hasTouch: false },
    expect: { fine: true, coarse: false, touch: 0 },
  },
  coarseMobile: {
    name: "coarse-mobile-touch",
    context: {
      isMobile: true,
      hasTouch: true,
      userAgent: "Mozilla/5.0 (Linux; Android 15; E1Tablet) AppleWebKit/537.36 Chrome/131 Safari/537.36",
    },
    expect: { fine: false, coarse: true, touchAtLeast: 1 },
  },
};

function assertProfile(state, profile) {
  assert(state.media.fine === profile.expect.fine && state.media.coarse === profile.expect.coarse,
    `${profile.name} did not expose its expected pointer configuration`, state.media);
  if (profile.expect.touch !== undefined) {
    assert(state.media.touchPoints === profile.expect.touch,
      `${profile.name} touch configuration drifted`, state.media);
  } else {
    assert(state.media.touchPoints >= profile.expect.touchAtLeast,
      `${profile.name} did not expose touch input`, state.media);
  }
}

async function newHarnessPage(browser, url, profile, width, safeArea) {
  const context = await browser.newContext({
    viewport: { width, height: HEIGHT },
    screen: { width, height: HEIGHT },
    deviceScaleFactor: 1,
    ...profile.context,
  });
  const page = await context.newPage();
  if (safeArea) {
    const cdp = await context.newCDPSession(page);
    await cdp.send("Emulation.setSafeAreaInsetsOverride", { insets: safeArea });
  }
  await page.goto(url, { waitUntil: "networkidle" });
  await page.waitForFunction(() => globalThis.__tineE1?.ready === true);
  await waitForMode(page, width < 640);
  return { context, page };
}

async function runCompact(browser, url, profile, { safeArea, exerciseResize = false, screenshots = {} } = {}) {
  const { context, page } = await newHarnessPage(browser, url, profile, 639, safeArea);
  try {
    await page.evaluate(() => globalThis.__tineE1.setWidths(900, 900));
    const closed = await snapshot(page);
    assertProfile(closed, profile);

    const underLeft = await clickProvenUnderTarget(page, "left");
    await openByPointer(page, "left");
    const left = await snapshot(page);
    assertCompactState(left, "left", closed);
    if (screenshots.left) await page.screenshot({ path: path.join(ARTIFACTS, screenshots.left), fullPage: true });
    const leftDismissal = await consumeScrimAtTarget(page, "left", underLeft);
    assertCleanClosed(leftDismissal.after);

    const underRight = await clickProvenUnderTarget(page, "right");
    await openByPointer(page, "right");
    const right = await snapshot(page);
    assertCompactState(right, "right", closed);
    if (screenshots.right) await page.screenshot({ path: path.join(ARTIFACTS, screenshots.right), fullPage: true });
    const rightDismissal = await consumeScrimAtTarget(page, "right", underRight);
    assertCleanClosed(rightDismissal.after);

    await openByPointer(page, "left");
    await page.evaluate(() => globalThis.__tineE1.openRight());
    await waitForDrawer(page, "right");
    const switchedRight = await snapshot(page);
    assertCompactState(switchedRight, "right", closed);
    assert(switchedRight.left === null && switchedRight.scrimCount === 1,
      "left-to-right switch left duplicate/stale drawer state", switchedRight);

    await page.evaluate(() => globalThis.__tineE1.openLeft());
    await waitForDrawer(page, "left");
    const switchedLeft = await snapshot(page);
    assertCompactState(switchedLeft, "left", closed);
    assert(switchedLeft.right === null && switchedLeft.scrimCount === 1,
      "right-to-left switch left duplicate/stale drawer state", switchedLeft);

    let resizeTransition = null;
    if (exerciseResize) {
      // The cap itself was proven above with deliberately oversized persisted
      // widths. Use ordinary desktop widths for the reactive boundary crossing
      // so the 640px persistent-neighbor geometry stays meaningful.
      await page.evaluate(() => globalThis.__tineE1.setWidths(220, 260));
      await page.setViewportSize({ width: 640, height: HEIGHT });
      await waitForMode(page, false);
      await page.evaluate(() => globalThis.__tineE1.openRight());
      const at640 = await snapshot(page);
      assertPersistentState(at640, 640);
      await page.setViewportSize({ width: 639, height: HEIGHT });
      await waitForMode(page, true);
      await waitForDrawer(page, "right");
      const backTo639 = await snapshot(page);
      assertCompactState(backTo639, "right", closed);
      assert(backTo639.left === null && backTo639.scrimCount === 1,
        "640-to-639 transition did not normalize to right-wins", backTo639);
      resizeTransition = { at640, backTo639 };
    }

    await page.evaluate(() => globalThis.__tineE1.closeAll());
    await page.waitForFunction(() => document.querySelectorAll("[data-mobile-drawer-panel]").length === 0);
    const cleaned = await snapshot(page);
    assertCleanClosed(cleaned);

    return {
      name: `${profile.name}-639-compact`,
      profile: closed.media,
      safeArea: closed.safeArea,
      closed,
      left,
      right,
      leftDismissal,
      rightDismissal,
      switchedRight,
      switchedLeft,
      resizeTransition,
      cleaned,
    };
  } finally {
    await context.close();
  }
}

async function runPersistent(browser, url, profile, width, screenshot) {
  const { context, page } = await newHarnessPage(browser, url, profile, width);
  try {
    await page.evaluate(() => globalThis.__tineE1.setWidths(220, 260));
    const closed = await snapshot(page);
    assertProfile(closed, profile);
    assert(closed.classifier === "false" && closed.scrimCount === 0,
      `${profile.name} ${width}px neighbor unexpectedly entered drawer mode`, closed);

    await openByPointer(page, "left");
    const leftOnly = await snapshot(page);
    assert(leftOnly.left?.position === "relative" && leftOnly.scrimCount === 0,
      `${profile.name} ${width}px left sidebar was not persistent`, leftOnly);
    assert(leftOnly.workspace.x > closed.workspace.x && leftOnly.workspace.width < closed.workspace.width,
      `${profile.name} ${width}px left sidebar did not consume its persisted flex width`, { closed, leftOnly });

    await openByPointer(page, "right");
    const both = await snapshot(page);
    assertPersistentState(both, width);
    assert(both.workspace.width < leftOnly.workspace.width,
      `${profile.name} ${width}px right sidebar did not consume its persisted flex width`, { leftOnly, both });
    if (screenshot) await page.screenshot({ path: path.join(ARTIFACTS, screenshot), fullPage: true });

    return { name: `${profile.name}-${width}-persistent`, profile: closed.media, closed, leftOnly, both };
  } finally {
    await context.close();
  }
}

fs.rmSync(ARTIFACTS, { recursive: true, force: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });

const { server, url } = await startHarnessServer();
const browser = await chromium.launch({ headless: true });
const proof = {
  scenario: "issue-161-browser-drawer-proof",
  observationBoundary: "Chromium + production Solid drawer primitives/classifier/UI state/CSS; not full App or native WebKit",
  inputs: fingerprints(),
  artifacts: [],
  matrix: [],
};

try {
  proof.matrix.push(await runCompact(browser, url, profiles.fineDesktop, {
    exerciseResize: true,
    screenshots: { left: "639-fine-left.png" },
  }));
  proof.artifacts.push("639-fine-left.png");

  proof.matrix.push(await runCompact(browser, url, profiles.coarseMobile, {
    safeArea: { top: 7, right: 17, bottom: 9, left: 11 },
    screenshots: { right: "639-coarse-touch-right-safe-area.png" },
  }));
  proof.artifacts.push("639-coarse-touch-right-safe-area.png");

  proof.matrix.push(await runPersistent(browser, url, profiles.fineDesktop, 640));
  proof.matrix.push(await runPersistent(browser, url, profiles.coarseMobile, 640, "640-coarse-touch-persistent.png"));
  proof.artifacts.push("640-coarse-touch-persistent.png");

  proof.matrix.push(await runPersistent(browser, url, profiles.fineDesktop, 1024, "1024-fine-persistent.png"));
  proof.artifacts.push("1024-fine-persistent.png");
  proof.matrix.push(await runPersistent(browser, url, profiles.coarseMobile, 900));

  fs.writeFileSync(path.join(ARTIFACTS, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
  console.log(`PASS: GH #161 browser drawer matrix (${proof.matrix.length} profiles); artifacts ${ARTIFACTS}`);
} finally {
  await browser.close();
  await server.close();
  fs.rmSync(path.join(ARTIFACTS, ".vite-cache"), { recursive: true, force: true });
}

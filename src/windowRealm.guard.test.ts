import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

// OG-MULTIWINDOW P2. Tine renders secondary workspace windows from the main
// window's JavaScript, so the global `window`/`document` always mean MAIN. A
// listener, selection, focus read, measurement, frame, observer or DOM-class
// `instanceof` taken from the global realm is silently wrong (or dead) in a
// popup. Resolve the realm through the blessed helpers in src/windowRealm.ts
// (windowOf / documentOf / activeDocument / onEachWindow / requestFrame /
// newResizeObserver / isElementNode ...). Exemplar conversions:
// src/keybindings.ts installKeybindings (onEachWindow) and src/lazyObserve.ts
// (one observer per window). Process-wide uses are listed with a reason in
// REALM_ALLOW below.

const BLESSED = "src/windowRealm.ts";
const RULE = "I-12/OG-MULTIWINDOW P2: resolve the window realm through src/windowRealm.ts " +
  "(windowOf(el), activeDocument(), onEachWindow(...), requestFrame(...), newResizeObserver(...), isElementNode(...)); " +
  "exemplars: installKeybindings in src/keybindings.ts, src/lazyObserve.ts.";

/** Events whose meaning is the whole JS process (or main's own page lifecycle),
 * which the global window is the right owner for. */
const PROCESS_EVENTS = new Set(["error", "unhandledrejection", "popstate", "hashchange", "beforeunload", "storage",
  "online", "offline", "message", "languagechange"]);
const WINDOW_MEMBERS = new Set(["addEventListener", "removeEventListener", "getSelection", "innerWidth", "innerHeight",
  "outerWidth", "outerHeight", "devicePixelRatio", "visualViewport", "matchMedia", "requestAnimationFrame",
  "cancelAnimationFrame", "scrollBy", "scrollTo", "scrollX", "scrollY", "pageXOffset", "pageYOffset", "ResizeObserver",
  "IntersectionObserver", "document", "screenX", "screenY"]);
const DOCUMENT_MEMBERS = new Set(["addEventListener", "removeEventListener", "activeElement", "getSelection",
  "elementFromPoint", "elementsFromPoint", "caretRangeFromPoint", "caretPositionFromPoint", "createRange", "hasFocus",
  "hidden", "visibilityState", "body", "querySelector", "querySelectorAll", "getElementById", "getElementsByClassName",
  "getElementsByTagName", "fullscreenElement", "scrollingElement"]);
const BARE_CALLS = new Set(["requestAnimationFrame", "cancelAnimationFrame", "getSelection", "addEventListener",
  "removeEventListener", "matchMedia"]);
const BARE_READS = new Set(["innerWidth", "innerHeight", "devicePixelRatio", "visualViewport"]);
const OBSERVERS = new Set(["ResizeObserver", "IntersectionObserver"]);
const DOM_CLASS = /^(HTML\w*Element|SVG\w*Element|Element|Node|Text|CharacterData|Document|DocumentFragment|ShadowRoot|Range|Selection|Window|(Keyboard|Mouse|Pointer|Focus|Input|Clipboard|Drag|Touch|Wheel|UI|Composition)Event|Event)$/;
/** Process-wide or main-only uses: `<file> <key>` (or `<file> *` for a file that
 * never runs in a workspace window) → why the global realm is right. */
const REALM_ALLOW: Record<string, string> = {
  "src/capture.tsx *":
    "The quick-capture window is its own webview with its own JS realm; it never hosts workspace windows.",
  "src/main.tsx *":
    "Boots the main window's own #root (and the published-snapshot error text) before any workspace window exists.",
  "src/publishedBackend.ts *":
    "The published static export runs in a browser tab with no Tauri and therefore no workspace windows.",
  "src/components/MobileKeyboardToolbar.tsx *":
    "Renders only when isMobilePlatform; workspace windows are desktop-only (absent on Android/iOS).",
  "src/edgeSwipe.ts *":
    "Installed only on touch platforms (App.tsx returns on platform === 'desktop'); workspace windows are desktop-only.",
  "src/sessionActivity.ts *":
    "installSessionActivity returns immediately unless deps.isMobile; workspace windows are desktop-only.",
  "src/nativeChrome.ts window.innerWidth":
    "isTabletViewport classifies the mobile shell of the main window; workspace windows are desktop-only.",
  "src/nativeChrome.ts window.innerHeight":
    "isTabletViewport classifies the mobile shell of the main window; workspace windows are desktop-only.",
  "src/themePreference.ts window.matchMedia":
    "prefers-color-scheme is an OS-wide preference; the resulting theme attributes are mirrored into workspace windows centrally.",
  "src/lsShim.ts document.getElementById":
    "Writes the main window's <head> stylesheet, the single source that src/workspaceWindows.ts mirrors into every workspace window.",
  "src/themeGallery.ts document.getElementById":
    "Writes the main window's <head> stylesheet, the single source that src/workspaceWindows.ts mirrors into every workspace window.",
  "src/graph.ts document.getElementById":
    "Writes the main window's <head> stylesheet, the single source that src/workspaceWindows.ts mirrors into every workspace window.",
  "src/App.tsx mainWindow.document.addEventListener":
    "installMobileExternalLinkHandler returns inert on desktop; on iOS/Android main is the only window.",
  "src/App.tsx mainWindow.document.removeEventListener":
    "installMobileExternalLinkHandler returns inert on desktop; on iOS/Android main is the only window.",
  "src/App.tsx mainWindow.document.querySelector":
    "The edge-swipe surface is installed only on touch platforms, where main is the only window.",
  "src/App.tsx mainWindow.addEventListener":
    "The left sidebar and its resizer exist only in the main window (workspace windows render no sidebars).",
  "src/App.tsx mainWindow.removeEventListener":
    "The left sidebar and its resizer exist only in the main window (workspace windows render no sidebars).",
  "src/components/Macro.tsx mainWindow.document.getElementById":
    "The YouTube IFrame API is one process-wide script loaded once into the main realm's <head>.",
  "src/components/MobileDrawerShell.tsx mainWindow.document.querySelector":
    "Mobile drawers classify and contain focus in the main window's sidebars; workspace windows are desktop-only and have no sidebars.",
  "src/components/MobileDrawerShell.tsx mainWindow.document.addEventListener":
    "Mobile drawers classify and contain focus in the main window's sidebars; workspace windows are desktop-only and have no sidebars.",
  "src/components/MobileDrawerShell.tsx mainWindow.document.removeEventListener":
    "Mobile drawers classify and contain focus in the main window's sidebars; workspace windows are desktop-only and have no sidebars.",
  "src/components/RightSidebar.tsx mainWindow.document.activeElement":
    "The right sidebar (its focus handling and resizer) exists only in the main window; workspace windows render no sidebars.",
  "src/components/RightSidebar.tsx mainWindow.document.querySelectorAll":
    "The right sidebar (its focus handling and resizer) exists only in the main window; workspace windows render no sidebars.",
  "src/components/RightSidebar.tsx mainWindow.innerWidth":
    "The right sidebar (its focus handling and resizer) exists only in the main window; workspace windows render no sidebars.",
  "src/components/RightSidebar.tsx mainWindow.addEventListener":
    "The right sidebar (its focus handling and resizer) exists only in the main window; workspace windows render no sidebars.",
  "src/components/RightSidebar.tsx mainWindow.removeEventListener":
    "The right sidebar (its focus handling and resizer) exists only in the main window; workspace windows render no sidebars.",
  "src/mobileDrawers.ts mainWindow.matchMedia":
    "Mobile drawers classify and contain focus in the main window's sidebars; workspace windows are desktop-only and have no sidebars.",
  "src/mobileDrawers.ts mainWindow.document.querySelector":
    "Mobile drawers classify and contain focus in the main window's sidebars; workspace windows are desktop-only and have no sidebars.",
  "src/mobileDrawers.ts mainWindow.document.activeElement":
    "Mobile drawers classify and contain focus in the main window's sidebars; workspace windows are desktop-only and have no sidebars.",
  "src/outlineViewport.ts mainWindow.addEventListener":
    "Printing runs from a hidden frame in the main window, which owns the OS print dialog and the bundle's stylesheets.",
  "src/print.ts mainWindow.document.querySelectorAll":
    "Printing runs from a hidden frame in the main window, which owns the OS print dialog and the bundle's stylesheets.",
  "src/print.ts mainWindow.addEventListener":
    "Printing runs from a hidden frame in the main window, which owns the OS print dialog and the bundle's stylesheets.",
  "src/print.ts mainWindow.removeEventListener":
    "Printing runs from a hidden frame in the main window, which owns the OS print dialog and the bundle's stylesheets.",
  "src/print.ts mainWindow.document.body":
    "Printing runs from a hidden frame in the main window, which owns the OS print dialog and the bundle's stylesheets.",
  "src/router.ts mainWindow.addEventListener":
    "Mobile history back (popstate) is installed only on iOS/Android, where main is the only window.",
  "src/router.ts mainWindow.document.querySelector":
    "Fallback only before a pane registered its scroller (App.tsx setScrollerElement does so in every window); main's feed is the pre-multiwindow default.",
  "src/smoothScroll.ts mainWindow.document.querySelector":
    "The experimental opt-in Lenis smoother is one instance on main's feed scroller; workspace windows scroll natively (declared gap in RECEIPT-OG-MW).",
  "src/workspaceWindows.ts mainWindow.document":
    "Main's document is the single stylesheet/attribute source mirrored into every workspace window.",
  "src/workspaceWindows.ts mainWindow.addEventListener":
    "Main's own pagehide takes the shared JS realm, so every workspace window, with it; the listener belongs to main by definition.",
  "src/workspaceWindows.ts mainWindow.removeEventListener":
    "Main's own pagehide takes the shared JS realm, so every workspace window, with it; the listener belongs to main by definition.",
};

// `mainWindow` (exported by the blessed module) is the global realm under
// another name: a listener or focus read on it is the same P2 bug as on
// `window`, so it is judged as a global owner too (review F5).
const GLOBAL_WINDOWS = new Set(["window", "globalThis", "self", "mainWindow"]);
const ownerKey = (owner: string) => (owner === "mainWindow" ? "mainWindow" : "window");

/** Whether identifier `node` reads a global binding rather than naming a
 * property, declaration, parameter or type member. */
function isGlobalRead(node: ts.Identifier): boolean {
  const parent = node.parent;
  if (ts.isPropertyAccessExpression(parent) && parent.name === node) return false;
  if ((ts.isPropertyAssignment(parent) || ts.isVariableDeclaration(parent) || ts.isParameter(parent)
    || ts.isBindingElement(parent) || ts.isPropertySignature(parent) || ts.isPropertyDeclaration(parent)
    || ts.isMethodDeclaration(parent)) && parent.name === node) return false;
  if (ts.isBindingElement(parent) && parent.propertyName === node) return false;
  if (ts.isTypeOfExpression(parent)) return false;
  if (ts.isQualifiedName(parent) || ts.isTypeReferenceNode(parent)) return false;
  return true;
}

export function realmViolations(file: string, source: string): { key: string; line: number }[] {
  const sf = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true,
    file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const out: { key: string; line: number }[] = [];
  const report = (node: ts.Node, key: string) => {
    out.push({ key, line: sf.getLineAndCharacterOfPosition(node.getStart(sf)).line + 1 });
  };
  const firstArgEvent = (node: ts.Node): string | null => {
    const call = node.parent;
    if (!call || !ts.isCallExpression(call) || call.expression !== node) return null;
    const arg = call.arguments[0];
    return arg && ts.isStringLiteralLike(arg) ? arg.text : null;
  };
  const visit = (node: ts.Node) => {
    if (ts.isPropertyAccessExpression(node) && ts.isIdentifier(node.expression)) {
      const owner = node.expression.text;
      const member = node.name.text;
      if (GLOBAL_WINDOWS.has(owner) && WINDOW_MEMBERS.has(member)) {
        const event = member.endsWith("EventListener") ? firstArgEvent(node) : null;
        // `window.document.X` is judged by its document member below.
        const isDocumentChain = member === "document" && ts.isPropertyAccessExpression(node.parent);
        if (!(event && PROCESS_EVENTS.has(event)) && !isDocumentChain) report(node, `${ownerKey(owner)}.${member}`);
      }
      if (owner === "document" && DOCUMENT_MEMBERS.has(member)) report(node, `document.${member}`);
    }
    // window.document.X
    if (ts.isPropertyAccessExpression(node) && ts.isPropertyAccessExpression(node.expression)
      && ts.isIdentifier(node.expression.expression) && GLOBAL_WINDOWS.has(node.expression.expression.text)
      && node.expression.name.text === "document" && DOCUMENT_MEMBERS.has(node.name.text)) {
      const owner = node.expression.expression.text;
      report(node, owner === "mainWindow" ? `mainWindow.document.${node.name.text}` : `document.${node.name.text}`);
    }
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && BARE_CALLS.has(node.expression.text)) {
      const event = node.arguments[0] && ts.isStringLiteralLike(node.arguments[0]) ? node.arguments[0].text : null;
      if (!(event && PROCESS_EVENTS.has(event))) report(node, `${node.expression.text}()`);
    }
    if (ts.isIdentifier(node) && BARE_READS.has(node.text) && isGlobalRead(node)) report(node, node.text);
    if (ts.isNewExpression(node)) {
      const callee = node.expression;
      const name = ts.isIdentifier(callee) ? callee.text
        : ts.isPropertyAccessExpression(callee) && ts.isIdentifier(callee.expression) && GLOBAL_WINDOWS.has(callee.expression.text)
          ? callee.name.text : null;
      if (name && OBSERVERS.has(name)) report(node, `new ${name}`);
    }
    if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.InstanceOfKeyword
      && ts.isIdentifier(node.right) && DOM_CLASS.test(node.right.text)) {
      report(node, `instanceof ${node.right.text}`);
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return out;
}

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === "fixtures" || entry.name === "tests" ? [] : sourceFiles(file);
    return /\.tsx?$/.test(entry.name) && !/\.(test|spec)\.tsx?$/.test(entry.name) && !entry.name.endsWith(".d.ts")
      ? [file] : [];
  });
}

describe("window realm guard (OG-MULTIWINDOW P2)", () => {
  const files = sourceFiles("src").filter((file) => file !== BLESSED);

  it("bans global-realm listeners, focus, selection, measurement, frames, observers and DOM instanceof", () => {
    const failures: string[] = [];
    const used = new Set<string>();
    for (const file of files) {
      for (const violation of realmViolations(file, readFileSync(file, "utf8"))) {
        const allowKey = [`${file} ${violation.key}`, `${file} *`].find((key) => REALM_ALLOW[key]);
        if (allowKey) { used.add(allowKey); continue; }
        failures.push(`${file}:${violation.line}: ${violation.key}`);
      }
    }
    expect(failures, `${RULE}\n${failures.join("\n")}`).toEqual([]);
    // The allow-list only shrinks: an entry that no longer matches is removed.
    const stale = Object.keys(REALM_ALLOW).filter((key) => !used.has(key));
    expect(stale, "stale REALM_ALLOW entries; delete them").toEqual([]);
  });

  it("every allow-list entry states why the global realm is right", () => {
    for (const [key, reason] of Object.entries(REALM_ALLOW)) {
      expect(reason.length, `${key} needs a reason`).toBeGreaterThan(20);
    }
  });

  it("is not vacuous: the scan reaches the app and the blessed module", () => {
    expect(files.length).toBeGreaterThan(300);
    expect(readFileSync(BLESSED, "utf8")).toContain("export function onEachWindow");
  });

  it("catches each planted violation (mutation probe)", () => {
    const planted: Array<[string, string]> = [
      ["window.addEventListener(\"keydown\", f);", "window.addEventListener"],
      // `mainWindow` is the global realm under another name (review F5).
      ["mainWindow.addEventListener(\"keydown\", f);", "mainWindow.addEventListener"],
      ["const a = mainWindow.document.activeElement;", "mainWindow.document.activeElement"],
      ["const s = mainWindow.getSelection();", "mainWindow.getSelection"],
      ["document.addEventListener(\"pointerdown\", f);", "document.addEventListener"],
      ["const a = document.activeElement;", "document.activeElement"],
      ["const s = window.getSelection();", "window.getSelection"],
      ["const s = getSelection();", "getSelection()"],
      ["document.elementFromPoint(1, 2);", "document.elementFromPoint"],
      ["const r = document.createRange();", "document.createRange"],
      ["const w = window.innerWidth;", "window.innerWidth"],
      ["const h = innerHeight;", "innerHeight"],
      ["const d = window.devicePixelRatio;", "window.devicePixelRatio"],
      ["const v = window.visualViewport;", "window.visualViewport"],
      ["window.matchMedia(\"(x)\");", "window.matchMedia"],
      ["requestAnimationFrame(f);", "requestAnimationFrame()"],
      ["window.requestAnimationFrame(f);", "window.requestAnimationFrame"],
      ["new ResizeObserver(f);", "new ResizeObserver"],
      ["new IntersectionObserver(f);", "new IntersectionObserver"],
      ["document.body.append(x);", "document.body"],
      ["document.querySelector(\".x\");", "document.querySelector"],
      ["if (document.hasFocus()) f();", "document.hasFocus"],
      ["if (document.hidden) f();", "document.hidden"],
      ["window.document.activeElement;", "document.activeElement"],
      ["if (x instanceof HTMLElement) f();", "instanceof HTMLElement"],
      ["if (e instanceof KeyboardEvent) f();", "instanceof KeyboardEvent"],
    ];
    for (const [source, key] of planted) {
      expect(realmViolations("src/planted.ts", source).map((v) => v.key), source).toContain(key);
    }
    // Process-wide events and realm-safe uses stay legal.
    for (const source of [
      "window.addEventListener(\"error\", f);",
      "window.addEventListener(\"unhandledrejection\", f);",
      "window.location.href;",
      "document.createElement(\"div\");",
      "if (typeof ResizeObserver === \"undefined\") f();",
      "const { innerWidth } = viewport;",
      "win.addEventListener(\"keydown\", f);",
      "if (x instanceof Error) f();",
    ]) {
      expect(realmViolations("src/planted.ts", source), source).toEqual([]);
    }
  });
});

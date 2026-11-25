// GH #1039 (UI-GH1039-FILE-DROP-TARGET): which block a dropped file goes after.
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block, SurfaceContext } from "./components/Block";
import { doc } from "./document/model";
import { backend } from "./backend";
import { initParser } from "./render/parse";
import { loadFeed, loadRoutedPage, pageByName, resetStore } from "./document";
import { dropPointCss, installFileDrop } from "./filedrop";
import { setToasts, toasts } from "./toasts";

const drag = vi.hoisted(() => ({ listener: null as null | ((event: any) => Promise<void>) }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async (listener: (event: any) => Promise<void>) => {
      drag.listener = listener;
      return () => { drag.listener = null; };
    },
  }),
}));

const LINUX_UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";
const WINDOWS_UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0";

beforeAll(() => initParser());
afterEach(() => { vi.restoreAllMocks(); resetStore(); setToasts([]); document.body.innerHTML = ""; });

const day = (name: string, id: string, raw: string) =>
  ({ name, title: name, kind: "journal" as const, pre_block: null, blocks: [{ id, raw, collapsed: false, children: [] }] });

it("a drop beside the blocks of a single routed page writes into no file", async () => {
  // The journals feed was loaded earlier; the main view now shows today's page
  // on its own (a page route), which is the only page on screen.
  expect(loadFeed([day("Oct 31st, 2026", "old-day", "older note")])).toBe("published");
  expect(loadRoutedPage(day("Nov 23rd, 2026", "today", "today note"))).toBeNull();
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <SurfaceContext.Provider value="main"><Block id="today" /></SurfaceContext.Provider>, root);
  // The pointer is over the page, beside its blocks.
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => root });
  const imported = vi.spyOn(backend(), "importAsset").mockResolvedValue("example.pdf");
  const uninstall = await installFileDrop();
  try {
    await drag.listener!({ payload: { type: "drop", paths: ["/tmp/example.pdf"], position: { x: 10, y: 10 } } });
    expect(imported).not.toHaveBeenCalled();
    expect(pageByName("Oct 31st, 2026")!.roots.map(id => doc.byId[id].raw)).toEqual(["older note"]);
    expect(pageByName("Nov 23rd, 2026")!.roots.map(id => doc.byId[id].raw)).toEqual(["today note"]);
    expect(toasts().map(t => [t.kind, t.message])).toEqual([["error", "Drop a file onto a block to insert it."]]);
  } finally { uninstall(); dispose(); }
});

// The native position's unit, per wry's drop handlers: GTK `drag-drop` and
// AppKit `draggingLocation` give view (logical) points whatever the display
// scale; WebView2 gives client pixels after `ScreenToClient`.
it.each([
  ["Linux, device scale 2, no zoom", false, 2, 1, { x: 300, y: 200 }, { x: 300, y: 200 }],
  ["Linux, device scale 2, 150% interface zoom", false, 2, 1.5, { x: 300, y: 210 }, { x: 200, y: 140 }],
  ["macOS Retina, 125% interface zoom", false, 2, 1.25, { x: 250, y: 100 }, { x: 200, y: 80 }],
  ["Windows, 150% display scale", true, 1.5, 1, { x: 450, y: 300 }, { x: 300, y: 200 }],
] as const)("a drop position maps to CSS pixels (%s)", (_label, windows, dpr, zoom, position, css) => {
  expect(dropPointCss(position, { windows, devicePixelRatio: dpr, zoom })).toEqual(css);
});

it.each([
  // A 150% KDE Wayland session gives WebKitGTK a device scale of 2.
  ["Linux, device scale 2", LINUX_UA, 2, { x: 300, y: 200 }, { x: 300, y: 200 }],
  ["Windows, 150% display scale", WINDOWS_UA, 1.5, { x: 450, y: 300 }, { x: 300, y: 200 }],
] as const)("a drop hit-tests the point under the pointer (%s)", async (_label, ua, dpr, position, css) => {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue(ua);
  vi.spyOn(window, "devicePixelRatio", "get").mockReturnValue(dpr);
  const hits: [number, number][] = [];
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: (x: number, y: number) => { hits.push([x, y]); return null; } });
  const uninstall = await installFileDrop();
  try {
    await drag.listener!({ payload: { type: "drop", paths: ["/tmp/example.pdf"], position } });
    expect(hits).toEqual([[css.x, css.y]]);
  } finally { uninstall(); }
});

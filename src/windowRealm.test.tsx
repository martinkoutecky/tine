// OG-MULTIWINDOW P1/P5: the realm helpers against a SECOND jsdom document.
// An iframe gives jsdom a genuinely separate window + document (its own
// constructors, focus, visibility and frame clock), which is the shape a
// workspace popup has in the real app.
import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import {
  MAIN_WINDOW_ID,
  WindowContext,
  activeWindowId,
  anyWindowFocused,
  anyWindowVisible,
  documentOf,
  isElementNode,
  isHTMLElementNode,
  isPointerEventValue,
  listen,
  mainWindow,
  onAppReturn,
  onEachWindow,
  queryAllWindows,
  registerWindow,
  requestFrame,
  useOwnerWindow,
  windowIds,
  windowOf,
} from "./windowRealm";
import { observeNear, resetNearObserverForTests, unobserveNear } from "./lazyObserve";
import { installBackgroundFlush } from "./backgroundFlush";

const cleanups: (() => void)[] = [];
afterEach(() => {
  while (cleanups.length) cleanups.pop()!();
  document.body.innerHTML = "";
  resetNearObserverForTests();
});

function openPopup(id = "ws-1-1"): { win: Window; doc: Document; close: () => void } {
  const frame = document.createElement("iframe");
  document.body.append(frame);
  const win = frame.contentWindow!;
  const unregister = registerWindow(id, win);
  const close = () => { unregister(); frame.remove(); };
  cleanups.push(close);
  return { win, doc: win.document, close };
}

function setVisibility(doc: Document, state: DocumentVisibilityState) {
  Object.defineProperty(doc, "visibilityState", { value: state, configurable: true });
}

describe("window realm helpers with a second document", () => {
  it("resolves an element, a text node, an event and a document to their own window", () => {
    const { win, doc } = openPopup();
    const el = doc.createElement("div");
    doc.body.append(el);
    const text = doc.createTextNode("x");
    el.append(text);
    expect(windowOf(el)).toBe(win);
    expect(windowOf(text)).toBe(win);
    expect(windowOf(doc)).toBe(win);
    expect(documentOf(el)).toBe(doc);
    let seen: Window | null = null;
    el.addEventListener("click", (event) => { seen = windowOf(event); });
    el.dispatchEvent(new win.MouseEvent("click", { bubbles: true }));
    expect(seen).toBe(win);
    expect(windowOf(document.body)).toBe(mainWindow);
  });

  it("recognizes nodes and events across realms where instanceof does not", () => {
    const { win, doc } = openPopup();
    const el = doc.createElement("textarea");
    expect(el instanceof HTMLElement).toBe(false); // the cross-realm trap the helpers exist for
    expect(isElementNode(el)).toBe(true);
    expect(isHTMLElementNode(el)).toBe(true);
    expect(isHTMLElementNode(doc.createElementNS("http://www.w3.org/2000/svg", "svg"))).toBe(false);
    const PointerCtor = (win as Window & { PointerEvent?: typeof PointerEvent }).PointerEvent;
    if (PointerCtor) expect(isPointerEventValue(new PointerCtor("pointerdown", { pointerId: 3 }))).toBe(true);
    expect(isPointerEventValue(new win.MouseEvent("mousedown"))).toBe(false);
  });

  it("runs onEachWindow hooks for existing and later windows and cleans each up on close", () => {
    const seen: string[] = [];
    const cleaned: string[] = [];
    const stop = onEachWindow((_win, id) => { seen.push(id); return () => cleaned.push(id); });
    cleanups.push(stop);
    expect(seen).toEqual([MAIN_WINDOW_ID]);
    const popup = openPopup("ws-2-1");
    expect(seen).toEqual([MAIN_WINDOW_ID, "ws-2-1"]);
    expect(windowIds()).toContain("ws-2-1");
    popup.close();
    popup.close(); // idempotent
    expect(cleaned).toEqual(["ws-2-1"]);
    expect(windowIds()).not.toContain("ws-2-1");
  });

  it("listen() attaches to the target's own window and document", () => {
    const { win, doc } = openPopup();
    const el = doc.createElement("div");
    doc.body.append(el);
    const onWin = vi.fn();
    const onDoc = vi.fn();
    const offWin = listen(el, "resize", onWin);
    const offDoc = listen(el, "keydown", onDoc, { on: "document" });
    win.dispatchEvent(new win.Event("resize"));
    mainWindow.dispatchEvent(new Event("resize"));
    doc.dispatchEvent(new win.KeyboardEvent("keydown"));
    expect(onWin).toHaveBeenCalledOnce();
    expect(onDoc).toHaveBeenCalledOnce();
    offWin();
    offDoc();
    win.dispatchEvent(new win.Event("resize"));
    expect(onWin).toHaveBeenCalledOnce();
  });

  it("tracks the focused Tine window and falls back to main when it closes", () => {
    const popup = openPopup("ws-3-1");
    popup.win.dispatchEvent(new popup.win.FocusEvent("focus"));
    expect(activeWindowId()).toBe("ws-3-1");
    popup.close();
    expect(activeWindowId()).toBe(MAIN_WINDOW_ID);
  });

  it("P5: a minimized main with a visible popup still counts as visible and focused", () => {
    const popup = openPopup();
    setVisibility(document, "hidden");
    cleanups.push(() => setVisibility(document, "visible"));
    setVisibility(popup.doc, "visible");
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    vi.spyOn(popup.doc, "hasFocus").mockReturnValue(true);
    expect(anyWindowVisible()).toBe(true);
    expect(anyWindowFocused()).toBe(true);
    setVisibility(popup.doc, "hidden");
    expect(anyWindowVisible()).toBe(false);
    vi.restoreAllMocks();
  });

  it("P5: background flush fires only when every Tine window is hidden", async () => {
    const popup = openPopup();
    const flushAll = vi.fn(async () => true);
    const stop = installBackgroundFlush({ endEdit: () => {}, flushAll, closeInFlight: () => false });
    cleanups.push(stop);
    cleanups.push(() => setVisibility(document, "visible"));
    setVisibility(popup.doc, "visible");
    setVisibility(document, "hidden");
    document.dispatchEvent(new Event("visibilitychange"));
    await Promise.resolve();
    expect(flushAll).not.toHaveBeenCalled(); // main minimized, popup still in use
    setVisibility(popup.doc, "hidden");
    popup.doc.dispatchEvent(new popup.win.Event("visibilitychange"));
    await vi.waitFor(() => expect(flushAll).toHaveBeenCalledOnce());
  });

  it("onAppReturn fires when any Tine window regains focus", () => {
    const popup = openPopup();
    const back = vi.fn();
    cleanups.push(onAppReturn(back));
    popup.win.dispatchEvent(new popup.win.FocusEvent("focus"));
    mainWindow.dispatchEvent(new FocusEvent("focus"));
    expect(back).toHaveBeenCalledTimes(2);
  });

  it("queryAllWindows finds an overlay in a popup document", () => {
    const { doc } = openPopup();
    const overlay = doc.createElement("div");
    overlay.className = "modal-overlay";
    doc.body.append(overlay);
    expect(queryAllWindows(".modal-overlay")).toBe(overlay);
  });

  it("a frame pending in a popup still runs on main when the popup closes, and stays cancellable", async () => {
    const popup = openPopup();
    const el = popup.doc.createElement("div");
    popup.doc.body.append(el);
    let nextId = 40; // a popup that is closing never delivers its frames
    vi.spyOn(popup.win, "requestAnimationFrame").mockImplementation(() => ++nextId);
    const ran = vi.fn();
    requestFrame(el, ran);
    const cancelled = vi.fn();
    const cancel = requestFrame(el, cancelled);
    popup.close();
    cancel();
    await new Promise((resolve) => setTimeout(resolve, 5));
    expect(ran).toHaveBeenCalledOnce();
    expect(cancelled).not.toHaveBeenCalled();
  });

  it("lazyObserve uses one IntersectionObserver per window, from that window's realm", () => {
    const made: { realm: string; observed: Element[] }[] = [];
    const ioFor = (realm: string) => class {
      observed: Element[] = [];
      constructor() { made.push({ realm, observed: this.observed }); }
      observe(el: Element) { this.observed.push(el); }
      unobserve(el: Element) { this.observed.splice(this.observed.indexOf(el), 1); }
      disconnect() {}
    };
    const popup = openPopup();
    vi.stubGlobal("IntersectionObserver", ioFor("main"));
    Object.defineProperty(popup.win, "IntersectionObserver", { value: ioFor("popup"), configurable: true });
    cleanups.push(() => vi.unstubAllGlobals());
    const a = document.createElement("div");
    const b = document.createElement("div");
    const c = popup.doc.createElement("div");
    const d = popup.doc.createElement("div");
    document.body.append(a, b);
    popup.doc.body.append(c, d);
    for (const el of [a, b, c, d]) observeNear(el, () => {});
    expect(made.map((m) => m.realm)).toEqual(["main", "popup"]);
    expect(made[0].observed).toEqual([a, b]);
    expect(made[1].observed).toEqual([c, d]);
    unobserveNear(c);
    expect(made[1].observed).toEqual([d]);
    popup.close();
    // A window opened later gets its own fresh observer.
    const next = openPopup("ws-9-1");
    Object.defineProperty(next.win, "IntersectionObserver", { value: ioFor("popup2"), configurable: true });
    const e = next.doc.createElement("div");
    next.doc.body.append(e);
    observeNear(e, () => {});
    expect(made.map((m) => m.realm)).toEqual(["main", "popup", "popup2"]);
  });

  it("useOwnerWindow resolves the WindowContext's registered window", () => {
    const { win, doc } = openPopup("ws-4-1");
    let owner: Window | null = null;
    const Probe = () => { owner = useOwnerWindow(); return null; };
    const dispose = render(() => (
      <WindowContext.Provider value="ws-4-1"><Probe /></WindowContext.Provider>
    ), doc.body);
    cleanups.push(dispose);
    expect(owner).toBe(win);
    let mainOwner: Window | null = null;
    const MainProbe = () => { mainOwner = useOwnerWindow(); return null; };
    cleanups.push(render(() => <MainProbe />, document.body));
    expect(mainOwner).toBe(mainWindow);
  });
});

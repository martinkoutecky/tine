// Previous / Next change in the seen notice (ADR 0079, D-20261029-SS): the buttons
// step through changed blocks and removed-block markers in page order, wrap with a
// visible "3 of 3", open a collapsed parent without an edit, and never enter edit mode.
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import type { SeenPageChanges } from "../backendTypes";
import { initParser } from "../render/parse";
import { node, resetStore } from "../document";
import { editingId } from "../editorController";
import { setGraphMeta } from "../graphSession";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { paletteCommands } from "../keybindings";
import { setToasts } from "../toasts";
import { noteSeenAnswer, resetSeenForTests } from "../seen/seenState";
import { resetNearObserverForTests } from "../lazyObserve";
import { maybeEditorIn } from "../tests/editorDriver";
import { PageView } from "./Page";

// The windowing tests' observer: nothing is "near", so windows beyond the first stay unmounted shells.
class WindowObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

beforeAll(async () => { await initParser(); });

const blocks = () => [
  { id: "seen-0", has_id: false, raw: "Alpha", children: [{ id: "seen-0a", has_id: false, raw: "Alpha child", children: [], collapsed: false }], collapsed: true },
  { id: "seen-1", has_id: false, raw: "Beta", children: [], collapsed: false },
  { id: "seen-2", has_id: false, raw: "Gamma", children: [], collapsed: false },
];
const reply = (over: Partial<SeenPageChanges> = {}): SeenPageChanges => ({
  available: true, reason: null, head_commit: "h", baseline_commit: "b", current_text_sha256: "x",
  identity_known: true, unattributed: false, blocks: [], removed: [], ...over,
});
// seen-0a (inside a collapsed parent), a removed block after seen-0 (not at a changed block's place), seen-2: three stops in page order.
const threeChanges = () => reply({
  blocks: [{ block_path: [0, 0], persisted_id: null }, { block_path: [2], persisted_id: null }],
  removed: [{ text: "Deleted by Jo", baseline_block_path: [1], parent_path: null, previous_sibling_path: [0] }],
});

function openPage() {
  vi.spyOn(backend(), "getPage").mockResolvedValue({
    id: "pages/Seen.md", name: "Seen", title: "Seen", kind: "page", pre_block: null, blocks: blocks(),
  });
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Seen", pageKind: "page" });
  const host = document.createElement("div");
  host.className = "main-content";
  document.body.append(host);
  return { host, dispose: render(() => <PageView />, host) };
}
const button = (host: Element, which: "previous" | "next") => host.querySelector<HTMLButtonElement>(`[data-seen-step="${which}"]`)!;
const position = (host: Element) => host.querySelector("[data-seen-position]")?.textContent ?? null;
const flashed = (host: Element) => host.querySelector(".block-flash");
const settle = () => vi.waitFor(() => expect(host0?.querySelector("[data-seen-header]")).not.toBeNull());
let host0: Element | undefined;

beforeEach(() => { resetSeenForTests(); noteSeenAnswer(true); setGraphMeta({ root: "/graphs/seen" } as never); });
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  resetNearObserverForTests();
  resetStore();
  resetTabsToJournals();
  setGraphMeta(null);
  setToasts([]);
  document.body.replaceChildren();
  host0 = undefined;
});

it("shows Previous and Next change only while there are changes", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply());
  const { host, dispose } = openPage();
  try {
    await vi.waitFor(() => expect(host.querySelectorAll(".ls-block").length).toBeGreaterThanOrEqual(3));
    await Promise.resolve();
    expect(host.querySelector("[data-seen-step]")).toBeNull();
  } finally { dispose(); }
});

it("offers both steps for a single change and wraps back to it", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({
    blocks: [{ block_path: [2], persisted_id: null }],
  }));
  const { host, dispose } = openPage();
  host0 = host;
  try {
    await settle();
    expect(button(host, "previous").textContent).toBe("Previous change");
    expect(button(host, "next").textContent).toBe("Next change");
    button(host, "next").click();
    await vi.waitFor(() => expect(position(host)).toBe("1 of 1"));
    await vi.waitFor(() => expect(flashed(host)?.getAttribute("data-block-id")).toBe("seen-2"));
    button(host, "previous").click();
    await vi.waitFor(() => expect(position(host)).toBe("1 of 1, wrapped around"));
    await vi.waitFor(() => expect(flashed(host)?.getAttribute("data-block-id")).toBe("seen-2"));
    expect(editingId()).toBeNull();
  } finally { dispose(); }
});

it("steps through changed blocks and removed markers in page order, wrapping with a visible indication", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(threeChanges());
  const { host, dispose } = openPage();
  host0 = host;
  try {
    await settle();
    expect(button(host, "previous").textContent).toBe("Previous change");
    expect(button(host, "next").textContent).toBe("Next change");
    expect(position(host)).toBeNull();

    button(host, "next").click();
    await vi.waitFor(() => expect(flashed(host)?.getAttribute("data-block-id")).toBe("seen-0a"));
    expect(position(host)).toBe("1 of 3");

    button(host, "next").click();
    await vi.waitFor(() => expect(host.querySelector("[data-seen-removed].block-flash")).not.toBeNull());
    expect(position(host)).toBe("2 of 3");

    button(host, "next").click();
    await vi.waitFor(() => expect(flashed(host)?.getAttribute("data-block-id")).toBe("seen-2"));
    expect(position(host)).toBe("3 of 3");

    button(host, "next").click();
    await vi.waitFor(() => expect(position(host)).toBe("1 of 3, wrapped around"));
    await vi.waitFor(() => expect(flashed(host)?.getAttribute("data-block-id")).toBe("seen-0a"));

    button(host, "previous").click();
    await vi.waitFor(() => expect(position(host)).toBe("3 of 3, wrapped around"));
    button(host, "previous").click();
    await vi.waitFor(() => expect(position(host)).toBe("2 of 3"));
  } finally { dispose(); }
});

it("Previous from nowhere lands on the last change", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(threeChanges());
  const { host, dispose } = openPage();
  host0 = host;
  try {
    await settle();
    button(host, "previous").click();
    await vi.waitFor(() => expect(position(host)).toBe("3 of 3"));
    await vi.waitFor(() => expect(flashed(host)?.getAttribute("data-block-id")).toBe("seen-2"));
  } finally { dispose(); }
});

it("opens a collapsed parent in memory only and never enters edit mode", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(threeChanges());
  const { host, dispose } = openPage();
  host0 = host;
  try {
    await settle();
    expect(host.querySelector('[data-block-id="seen-0a"]')).toBeNull();
    button(host, "next").click();
    await vi.waitFor(() => expect(host.querySelector('[data-block-id="seen-0a"]')).not.toBeNull());
    expect(node("seen-0")?.collapsed).toBe(false);
    expect(node("seen-0")?.raw).toBe("Alpha"); // no collapsed:: edit, so nothing to save
    expect(editingId()).toBeNull();
    expect(maybeEditorIn(host)).toBeNull();
  } finally { dispose(); }
});

it("the palette's Next / Previous change work while the notice is shown, and are offered only then", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(threeChanges());
  const ids = () => paletteCommands().map((c) => c.id).filter((id) => id.endsWith("-change"));
  expect(ids()).toEqual([]);
  const { host, dispose } = openPage();
  host0 = host;
  try {
    await settle();
    expect(ids()).toEqual(["seen/next-change", "seen/previous-change"]);
    const command = (id: string) => paletteCommands().find((c) => c.id === id)!;
    expect(command("seen/next-change").label).toBe("Next change since you last looked");
    command("seen/next-change").run();
    await vi.waitFor(() => expect(position(host)).toBe("1 of 3"));
    command("seen/previous-change").run();
    await vi.waitFor(() => expect(position(host)).toBe("3 of 3, wrapped around"));
  } finally { dispose(); }
  expect(ids()).toEqual([]);
});

it("reaches a change in a window that is not mounted yet on a 2,000-block page", async () => {
  vi.stubGlobal("IntersectionObserver", WindowObserver);
  const many = Array.from({ length: 2000 }, (_, i) => ({ id: `w-${i}`, has_id: false, raw: `Row ${i} unique content`, children: [], collapsed: false }));
  vi.spyOn(backend(), "getPage").mockResolvedValue({ id: "pages/Seen.md", name: "Seen", title: "Seen", kind: "page", pre_block: null, blocks: many });
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({
    blocks: [{ block_path: [1500], persisted_id: null }, { block_path: [1999], persisted_id: null }],
  }));
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Seen", pageKind: "page" });
  const host = document.createElement("div");
  host.className = "main-content";
  document.body.append(host);
  const dispose = render(() => <PageView />, host);
  host0 = host;
  try {
    await settle();
    expect(host.querySelectorAll(".ls-block").length).toBeLessThan(100);
    expect(host.querySelector('[data-block-id="w-1500"]')).toBeNull();
    button(host, "next").click();
    await vi.waitFor(() => expect(flashed(host)?.getAttribute("data-block-id")).toBe("w-1500"));
    button(host, "next").click();
    await vi.waitFor(() => expect(flashed(host)?.getAttribute("data-block-id")).toBe("w-1999"));
    expect(position(host)).toBe("2 of 2");
    expect(editingId()).toBeNull();
  } finally { dispose(); }
});

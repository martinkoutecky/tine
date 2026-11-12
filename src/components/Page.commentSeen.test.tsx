// Unread margin comments = "Changed since you last looked" (DESIGN §12, ADR 0079): the page's one
// seen read marks a changed comment block; the comment card and the folded block's "N below" chip
// draw and count it. Git graphs only; no second unread store and no read per card.
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import type { SeenPageChanges } from "../backendTypes";
import type { BlockDto } from "../types";
import { blockRegions, initParser } from "../render/parse";
import { resetStore } from "../document";
import { setGraphMeta } from "../graphSession";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { resetSeenForTests } from "../seen/seenState";
import { persistCommentMarginHidden } from "../commentMarginPref";
import { PageView } from "./Page";

beforeAll(async () => { await initParser(); });

const D = "\ntine.date:: 2026-11-07T13:37Z";
const leaf = (id: string, raw: string, children: BlockDto[] = [], collapsed = false): BlockDto => ({
  id, has_id: false, raw, children, collapsed, properties: blockRegions(raw, "md").properties.filter(p => p.primary).map(p => [p.key, p.value]),
});
const blocks = () => [
  leaf("seen-0", "Alpha passage", [
    leaf("c1", "First thread\ntine.quote:: Alpha" + D, [leaf("r1", "Fresh reply" + D)]),
    leaf("c2", "Second thread\ntine.quote:: passage" + D),
  ]),
  leaf("seen-1", "Outer", [
    leaf("seen-1a", "Inner passage", [
      leaf("c3", "Deep thread\ntine.quote:: Inner" + D),
      leaf("c4", "Deeper thread\ntine.quote:: passage" + D),
    ]),
  ], true),
];
const reply = (over: Partial<SeenPageChanges> = {}): SeenPageChanges => ({
  available: true, reason: null, head_commit: "h", baseline_commit: "b", current_text_sha256: "x",
  identity_known: true, unattributed: false, blocks: [], removed: [], ...over,
});
// seen-0 = [0]; c1 = [0,0]; r1 = [0,0,0]; c2 = [0,1]; seen-1a = [1,0]; c3 = [1,0,0]; c4 = [1,0,1]
const REPLY_CHANGED = { block_path: [0, 0, 0], persisted_id: null };
const DEEP_CHANGED = { block_path: [1, 0, 1], persisted_id: null };

function openPage() {
  vi.spyOn(backend(), "getPage").mockResolvedValue({ id: "pages/Seen.md", name: "Seen", title: "Seen", kind: "page", pre_block: null, blocks: blocks() });
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Seen", pageKind: "page" });
  // A wide pane: cards draw in the margin column (the narrower tiers open them from a highlight).
  const host = document.createElement("main"); host.className = "main-content";
  Object.defineProperty(host, "clientWidth", { get: () => 1400 });
  document.body.append(host);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  return { host, dispose: render(() => <PageView />, host) };
}
const ready = (host: Element) => vi.waitFor(() => expect(host.querySelector('[data-block-id="seen-0"]')).not.toBeNull());
const card = (host: Element, id: string) => host.querySelector<HTMLElement>(`.comment-card[data-block-id="${id}"]`);
const chip = (host: Element) => host.querySelector<HTMLElement>(".comment-below-chip");

beforeEach(() => { resetSeenForTests(); setGraphMeta({ root: "/graphs/seen" } as never); persistCommentMarginHidden(false); });
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); resetStore(); resetTabsToJournals(); setGraphMeta(null); document.body.replaceChildren(); });

it("draws the change mark on the card whose reply changed and on no other card", async () => {
  const read = vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({ blocks: [REPLY_CHANGED] }));
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await vi.waitFor(() => expect(host.querySelector('[data-block-id="r1"] .seen-changed')).not.toBeNull());
    expect(card(host, "c1")?.classList.contains("comment-seen-changed")).toBe(true);
    expect(card(host, "c2")?.classList.contains("comment-seen-changed")).toBe(false);
    // One read for the page, whatever the number of cards: the marks cost no IPC per card.
    expect(read).toHaveBeenCalledTimes(1);
  } finally { dispose(); }
});

it("an unchanged page marks no card and no row", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply());
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await Promise.resolve();
    expect(card(host, "c1")).not.toBeNull();
    expect(host.querySelector(".comment-seen-changed, .seen-changed")).toBeNull();
    expect(chip(host)?.textContent).toBe("2 below");
  } finally { dispose(); }
});

it("a graph that cannot answer (not a git repository) shows no mark and no changed count", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({ available: false, reason: "not a git repository", blocks: [REPLY_CHANGED, DEEP_CHANGED] }));
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await Promise.resolve();
    expect(host.querySelector(".comment-seen-changed, .seen-changed")).toBeNull();
    expect(chip(host)?.textContent).toBe("2 below");
  } finally { dispose(); }
});

it("counts threads with a changed message in the folded block's chip, and says so to assistive tech", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({ blocks: [DEEP_CHANGED] }));
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await vi.waitFor(() => expect(chip(host)?.textContent).toBe("2 below, 1 changed"));
    expect(chip(host)?.classList.contains("comment-below-changed")).toBe(true);
    expect(chip(host)?.getAttribute("aria-label")).toContain("1 changed since you last looked");
  } finally { dispose(); }
});

it("a folded thread keeps its changed mark on the card", async () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(reply({ blocks: [REPLY_CHANGED] }));
  const { host, dispose } = openPage();
  try {
    await ready(host);
    await vi.waitFor(() => expect(card(host, "c1")?.classList.contains("comment-seen-changed")).toBe(true));
    card(host, "c1")!.querySelector<HTMLButtonElement>(".comment-collapse")!.click();
    // Folded to "N replies": the message rows are not drawn, the card still says it changed.
    await vi.waitFor(() => expect(card(host, "c1")?.querySelector(".comment-expand")).not.toBeNull());
    expect(host.querySelector('[data-block-id="r1"]')).toBeNull();
    expect(card(host, "c1")?.classList.contains("comment-seen-changed")).toBe(true);
  } finally { dispose(); }
});

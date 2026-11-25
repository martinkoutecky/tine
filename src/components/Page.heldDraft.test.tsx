// tine#1041: a route pinned to a path whose store read is `held-absent` (the
// host holds a draft of that path and there is no published file) asks the host
// once and shows the held draft with its whole-page choices. Every other
// not-found read keeps the refusal without asking the host. The host's answers
// replayed here are the ones `binding_adopted_draft_tests.rs` records: Open of
// the path answers the held text, conflicted, on no file.
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { conflicts, isConflicted, node, pageByName, resetStore, resolveConflict } from "../document";
import { initParser } from "../render/parse";
import { mainPaneRouter, openFileAgain, resetTabsToJournals } from "../router";
import { setGraphMeta } from "../graphSession";
import { setToasts } from "../toasts";
import { bindTestHost, mailPage, type TestHost } from "../document/host/wiring.test.support";
import type { PageDto, PageReadSource } from "../types";
import { ConflictBar } from "./ConflictBar";
import { PageView } from "./Page";

const KEY = "pages/Repro 964.md";
const NAME = "Repro 964";
const REFUSED = "no longer available at that path";

beforeAll(initParser);
let root: HTMLDivElement;
let dispose: (() => void) | undefined;
beforeEach(() => { root = document.createElement("div"); document.body.appendChild(root); });
afterEach(() => {
  dispose?.(); dispose = undefined;
  vi.useRealTimers();
  vi.restoreAllMocks();
  resetStore();
  setGraphMeta(null);
  setToasts([]);
  document.body.innerHTML = "";
  resetTabsToJournals();
});

const held = (): PageDto => ({ name: NAME, kind: "page", title: NAME, pre_block: null, rev: null,
  blocks: [{ id: "held-1", raw: "mine", collapsed: false, children: [] }] });

/** The store's read of `KEY`: not found, with `source`. */
function storeReads(source: PageReadSource | null) {
  return vi.spyOn(backend(), "getPageByPath").mockImplementation(async (_path, onSource) => {
    if (source) onSource?.(source);
    return null;
  });
}

/** The host's Open of `KEY`: the held draft (conflicted, no file) while `holds()`, else no file. */
function hostOpens(test: TestHost, holds: () => boolean) {
  return vi.spyOn(backend(), "pageOpen").mockImplementation(async (session, id, page) => {
    const draft = holds();
    setTimeout(() => test.deliver({ session, key: KEY, answer: { id, version: 2, took: false, outcome: { kind: "applied" } },
      notice: { conflictReported: draft },
      page: draft ? mailPage(2, held(), { conflict: true, risk: true, disk: { kind: "no-file" } }) : mailPage(2, null) }), 0);
    return { key: KEY, baselineEntry: page.path === KEY };
  });
}

function show() {
  mainPaneRouter.openFile(KEY, NAME, "page", { inPlace: true });
  dispose = render(() => <><ConflictBar /><PageView /></>, root);
}

const buttons = () => [...root.querySelectorAll(".conflict-banner button")].map((b) => b.textContent);

it.each(["disk", "mine"] as const)("held-absent, host holds a conflicted draft: one Open of the path, the draft and its choices (%s)", async (choice) => {
  const test = await bindTestHost();
  storeReads("held-absent");
  const open = hostOpens(test, () => true);
  show();
  await vi.waitFor(() => expect(isConflicted(NAME)).toBe(true));
  await vi.waitFor(() => expect(root.textContent).toContain("mine"));
  expect(root.textContent).not.toContain(REFUSED);
  expect(open).toHaveBeenCalledOnce();
  expect(open.mock.calls[0]?.[2]).toEqual({ name: NAME, kind: "page", path: KEY });
  expect(node(pageByName(NAME)!.roots[0])?.raw).toBe("mine");
  expect(conflicts()).toEqual([NAME]);
  expect(buttons()).toHaveLength(2);
  const discard = vi.spyOn(backend(), "pageDiscard").mockResolvedValue(null);
  const submit = vi.spyOn(backend(), "pageSubmit").mockResolvedValue(null);
  void resolveConflict(NAME, choice);
  await vi.waitFor(() => expect(discard.mock.calls.length + submit.mock.calls.length).toBe(1));
  if (choice === "disk") expect(discard.mock.calls[0]?.[2]).toBe(KEY);
  else {
    expect(submit.mock.calls[0]?.[2]).toBe(KEY);
    expect((submit.mock.calls[0]?.[3] as PageDto).blocks.map((b) => b.raw)).toEqual(["mine"]);
    expect(submit.mock.calls[0]?.[5]).toEqual({ kind: "no-file" });
  }
});

it("the route already refused when the draft arrives (adoption): the retry probe opens it", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  const test = await bindTestHost();
  storeReads("held-absent");
  let adopted = false;
  const open = hostOpens(test, () => adopted);
  show();
  await vi.waitFor(() => expect(root.textContent).toContain(REFUSED));
  expect(pageByName(NAME)).toBeUndefined(); // nothing left loaded by the refused Open
  adopted = true;
  await vi.advanceTimersByTimeAsync(3100);
  await vi.waitFor(() => expect(isConflicted(NAME)).toBe(true));
  await vi.waitFor(() => expect(root.textContent).not.toContain(REFUSED));
  expect(root.textContent).toContain("mine");
  expect(buttons()).toHaveLength(2);
  expect(open).toHaveBeenCalledTimes(2);
});

it("the route already refused when the draft arrives: Open of that path (a refused switch's action) is not a no-op", async () => {
  const test = await bindTestHost();
  storeReads("held-absent");
  let adopted = false;
  hostOpens(test, () => adopted);
  show();
  await vi.waitFor(() => expect(root.textContent).toContain(REFUSED));
  adopted = true;
  openFileAgain(KEY, NAME);
  await vi.waitFor(() => expect(isConflicted(NAME)).toBe(true));
  await vi.waitFor(() => expect(root.textContent).not.toContain(REFUSED));
  expect(buttons()).toHaveLength(2);
});

it("held-absent, host holds no draft (a clean held page deleted on disk): the refusal, nothing left loaded", async () => {
  const test = await bindTestHost();
  storeReads("held-absent");
  const open = hostOpens(test, () => false);
  show();
  await vi.waitFor(() => expect(root.textContent).toContain(REFUSED));
  expect(open).toHaveBeenCalledOnce();
  expect(pageByName(NAME)).toBeUndefined();
  expect(conflicts()).toEqual([]);
});

it.each([
  ["a plain not-found read", null],
  ["a held-unindexed read (identity against the held keys unproved)", "held-unindexed"],
] as const)("%s: the refusal, and the host is not asked", async (_label, source) => {
  const test = await bindTestHost();
  storeReads(source);
  const open = hostOpens(test, () => true);
  const discard = vi.spyOn(backend(), "pageDiscard");
  show();
  await vi.waitFor(() => expect(root.textContent).toContain(REFUSED));
  expect(open).not.toHaveBeenCalled();
  expect(discard).not.toHaveBeenCalled();
  expect(conflicts()).toEqual([]);
  expect(root.querySelector(".conflict-banner")).toBeNull();
});

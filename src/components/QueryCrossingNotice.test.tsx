// Ported from master src/components/QueryCrossingNotice.test.tsx.
// **The user half of the persisted-form crossing (SPEC §4.3 "Notice", §7.5; T3).**
//
// The mechanical half already shipped: an edit OG cannot express rewrites the
// block from `{{query …}}` to `{{tine-query …}}` on save. Until this packet it
// did so in complete silence — no notice, no toast, no undo affordance — so the
// user learned that Logseq now shows their query as plain text by opening
// Logseq. What this file is the evidence for:
//
//  C1  The crossing SAYS SO, in the spec's exact words, as a `role="status"`
//      region that takes focus.
//  C2  [Undo that change] is the ORDINARY undo, and is offered only while the
//      ordinary undo would still take back THIS change — in both history modes.
//      After the undo the block's bytes are what they were before the save.
//  C3  [Keep it] closes it; "Don't show this again" is device-local
//      keyed by graph (D-11) and silences the NEXT crossing.
//  C4  A save that does not cross says nothing.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { tryFreezeGraphRewrite } from "../document/graphRewriteState";
import { toasts, setToasts } from "../toasts";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { flushPage, resetStore, setRaw, toggleUndoRedoMode, undo } from "../document";
import { historyPageOnlyMode } from "../document/history";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { resetCrossingNoticeForTests as resetDismissedNoticesForTests } from "./Macro";
import { bumpGraphEpoch } from "../graphSession";
import type { ParsedQuery } from "../editor/queryIr";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import type { RefGroup } from "../types";
import { resetQueryTextOpenForTests } from "../navSettings";

beforeAll(async () => {
  await initParser();
});

// GH #619 item 4: the query text is behind an "Edit as text" toggle, closed by default. These
// tests are about the text pane, so they open the toggle the way a user who wants the text has.
beforeEach(() => {
  resetQueryTextOpenForTests(true);
});

afterEach(() => {
  setToasts([]);
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  resetDismissedNoticesForTests();
  if (historyPageOnlyMode()) toggleUndoRedoMode();
  localStorage.clear();
  document.body.innerHTML = "";
});

const NOTICE_KEY = "queryCrossingNoticeDismissed";
const NOTICE_TEXT =
  "This query now uses Tine features Logseq can't read. Logseq will show the block as plain text.";

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) await wait(0);
}

function page(roots: string[]): FeedPage {
  return {
    name: "Sheet", kind: "page", title: "Sheet", preBlock: null,
    roots, format: "md", readOnly: false, guide: false,
  };
}

function node(id: string, raw: string): StoreNode {
  return { id, raw, collapsed: false, parent: null, page: "Sheet", children: [] };
}

function groups(): RefGroup[] {
  return [{
    page: "Sheet",
    kind: "page",
    blocks: [{ id: "todo", raw: "TODO A tracked row", collapsed: false, children: [] }],
  }];
}

function load(raw: string): void {
  setDoc({
    byId: { query: node("query", raw), todo: node("todo", "TODO A tracked row") },
    pages: [page(["query", "todo"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

function parsedAs(text: string): ParsedQuery {
  return {
    query: {
      anchor: "block",
      filter: { kind: "raw", text, diagnostic_kind: "not_applicable" },
      diagnostics: [],
      source: { kind: "tql", original: text, og_options: "" },
    },
    view: {},
  };
}

/** The text pane lives in the SHEET's footer now, and the sheet is portalled to
 *  <body> (`.query-block`'s own compositing layer would otherwise trap it), so
 *  the pane is reached by opening the sheet and querying the document. */
async function openSheet(root: HTMLElement): Promise<HTMLElement> {
  const gear = await vi.waitFor(() => {
    const found = root.querySelector<HTMLButtonElement>(".qs-gear");
    if (!found) throw new Error("the query sentence never appeared");
    return found;
  });
  if (!document.querySelector(".qs-sheet")) gear.click();
  return await vi.waitFor(() => {
    const sheet = document.querySelector<HTMLElement>(".qs-sheet");
    if (!sheet) throw new Error("the sheet never opened");
    return sheet;
  });
}

/** P4 retired the `<details>` disclosure: inside an open sheet the pane is
 *  visible and editable (§7.5). Opening the sheet is opening the pane. */
async function openPane(root: HTMLElement): Promise<HTMLTextAreaElement> {
  const sheet = await openSheet(root);
  return await vi.waitFor(() => {
    const input = sheet.querySelector<HTMLTextAreaElement>(".query-text-pane-input");
    if (!input) throw new Error("the pane has no input");
    return input;
  });
}

async function saveThroughPane(root: HTMLElement, text: string): Promise<void> {
  const input = await openPane(root);
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (source: string) => parsedAs(source));
  input.value = text;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  const save = await vi.waitFor(() => {
    const button = document.querySelector<HTMLButtonElement>(".query-text-pane-save");
    if (!button || button.disabled) throw new Error("save is not enabled yet");
    return button;
  });
  save.click();
  await settle();
}

/** Arm the run/print/expressible answers for a save that DOES cross. */
function arrangeCrossing(dismissed: string[] = []): void {
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
  vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(false);
  vi.spyOn(backend(), "printQuery").mockResolvedValue("-- task DONE");
  vi.spyOn(backend(), "getAppBool").mockImplementation(async (key, fallback) =>
    key === NOTICE_KEY ? dismissed.includes("query-crossing") : fallback);
  resetDismissedNoticesForTests();
  bumpGraphEpoch();
}

/** **Document scope, not the block's element (P4, N3).** The one notice moves:
 *  it is inline under the block while the sheet is shut, and inside the query
 *  text pane — which is PORTALLED to <body> with the sheet — while it is open.
 *  Every behavioural assertion below is unchanged; only where the selector
 *  looks is. `root` is kept in the signature because "the notice belongs to
 *  this block" is still what is being asserted, and these tests mount one. */
function notice(root: HTMLElement): HTMLElement | null {
  void root;
  return document.querySelector<HTMLElement>(".query-crossing-notice");
}

async function waitForNotice(root: HTMLElement): Promise<HTMLElement> {
  return await vi.waitFor(() => {
    const found = notice(root);
    if (!found) throw new Error("the crossing notice never appeared");
    return found;
  });
}

const undoButton = (root: HTMLElement) => {
  void root;
  return document.querySelector<HTMLButtonElement>(".query-crossing-notice-undo")!;
};
const keepButton = () =>
  document.querySelector<HTMLButtonElement>(".query-crossing-notice-keep")!;

describe("C1: a crossing says so", () => {
  it("shows the §7.5 notice, in its exact words, as a status region that takes focus", async () => {
    load('{{query (task TODO)}}');
    arrangeCrossing();

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- task DONE");
      const shown = await waitForNotice(root);

      // The words are the contract. P2 renders NO `(<feature>)` parenthetical:
      // nothing in this packet can name the feature without becoming a second
      // producer of OG-expressibility (D-14), so it says the true general thing
      // rather than a guessed specific one.
      expect(shown.textContent?.replace(/\s+/g, " ").trim()).toContain(NOTICE_TEXT);
      expect(shown.getAttribute("role")).toBe("status");
      expect(document.activeElement).toBe(shown);
      expect(shown.textContent).toContain("Undo that change");
      expect(shown.textContent).toContain("Keep it");
      expect(shown.querySelector<HTMLInputElement>("input[type=checkbox]")).not.toBeNull();
    } finally {
      dispose();
    }
  });
});

describe("C4: a save that does not cross says nothing", () => {
  it("stays silent when the edit is still OG-expressible", async () => {
    load('{{query (task TODO)}}');
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("(task DONE)");
    const load_ = vi.spyOn(backend(), "getAppBool");

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- task DONE");
      await settle();
      expect(doc.byId.query.raw).toBe("{{query (task DONE)}}");
      expect(notice(root)).toBeNull();
      // Nothing was crossed, so the device-local store is never even consulted.
      expect(load_.mock.calls.filter(([key]) => key === NOTICE_KEY)).toHaveLength(0);
    } finally {
      dispose();
    }
  });

  it("says nothing when a {{tine-query}} block is edited again (it never crosses twice)", async () => {
    load('{{tine-query -- task TODO}}');
    arrangeCrossing();

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- task DONE");
      await settle();
      expect(doc.byId.query.raw).toBe("{{tine-query -- task DONE}}");
      expect(notice(root)).toBeNull();
    } finally {
      dispose();
    }
  });
});

describe("C2: [Undo that change] is the ordinary undo, and knows when it is not", () => {
  it("keeps the notice and block when a frozen rewrite refuses Undo", async () => {
    load('{{query (task TODO)}}');
    arrangeCrossing();
    const { root, dispose } = mount(() => <Block id="query" />);
    let release: (() => void) | null = null;
    try {
      await saveThroughPane(root, "-- task DONE");
      const shown = await waitForNotice(root);
      const crossed = doc.byId.query.raw;
      release = tryFreezeGraphRewrite();
      expect(release).not.toBeNull();
      shown.querySelector<HTMLButtonElement>(".query-crossing-notice-undo")!.click();
      expect(doc.byId.query.raw).toBe(crossed);
      expect(notice(root)).not.toBeNull();
      expect(toasts().some((toast) => toast.message.includes("Undo is unavailable"))).toBe(true);
    } finally {
      release?.();
      dispose();
    }
  });
  for (const pageOnly of [false, true]) {
    it(`restores the block byte-for-byte and closes (${pageOnly ? "page-only" : "global"} history)`, async () => {
      load('{{query (task TODO)}}');
      const before = doc.byId.query.raw;
      arrangeCrossing();
      if (pageOnly) toggleUndoRedoMode();

      const { root, dispose } = mount(() => <Block id="query" />);
      try {
        await saveThroughPane(root, "-- task DONE");
        const shown = await waitForNotice(root);
        expect(doc.byId.query.raw).not.toBe(before);

        expect(undoButton(root).disabled).toBe(false);
        expect(undoButton(root).textContent).toContain("Undo that change");
        undoButton(root).click();

        // I-4: what lands back on disk is what was there. Not "equivalent" —
        // identical.
        await vi.waitFor(() => expect(doc.byId.query.raw).toBe(before));
        await vi.waitFor(() => expect(notice(root)).toBeNull());
        expect(shown.isConnected).toBe(false);
      } finally {
        dispose();
      }
    });

    it(`disables the button once a later edit owns the undo (${pageOnly ? "page-only" : "global"} history)`, async () => {
      load('{{query (task TODO)}}');
      arrangeCrossing();
      if (pageOnly) toggleUndoRedoMode();

      const { root, dispose } = mount(() => <Block id="query" />);
      try {
        await saveThroughPane(root, "-- task DONE");
        await waitForNotice(root);
        const crossed = doc.byId.query.raw;

        // Something else happened since. Undo would now take THAT back, so the
        // button must not claim it undoes the crossing.
        setRaw("todo", "TODO A tracked row, edited");
        await settle();

        await vi.waitFor(() => expect(undoButton(root).disabled).toBe(true));
        expect(undoButton(root).textContent).toContain("Undo (use Ctrl+Z)");
        expect(doc.byId.query.raw).toBe(crossed);
      } finally {
        dispose();
      }
    });
  }
});

describe("C3: [Keep it] and the device-local dismissal", () => {
  it("closes on [Keep it] and writes nothing to the notices store", async () => {
    load('{{query (task TODO)}}');
    arrangeCrossing();
    const save = vi.spyOn(backend(), "setAppBool").mockResolvedValue(undefined);

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- task DONE");
      await waitForNotice(root);
      keepButton().click();
      await vi.waitFor(() => expect(notice(root)).toBeNull());
      expect(doc.byId.query.raw).toContain("{{tine-query");
      expect(save.mock.calls.filter(([key]) => key === NOTICE_KEY)).toHaveLength(0);
    } finally {
      dispose();
    }
  });

  it("writes the device preference when the box is ticked", async () => {
    load('{{query (task TODO)}}');
    arrangeCrossing();
    const save = vi.spyOn(backend(), "setAppBool").mockResolvedValue(undefined);

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- task DONE");
      const shown = await waitForNotice(root);
      const box = shown.querySelector<HTMLInputElement>("input[type=checkbox]")!;
      box.checked = true;
      box.dispatchEvent(new Event("change", { bubbles: true }));
      keepButton().click();
      await settle();

      // D-11: the dismissal is graph-keyed device app-data, not graph content.
      expect(save).toHaveBeenCalledWith(NOTICE_KEY, true);
      expect(doc.byId.query.raw).not.toContain("query-crossing");
    } finally {
      dispose();
    }
  });

  it("shows no notice on the next crossing once this graph has dismissed it", async () => {
    load('{{query (task TODO)}}');
    arrangeCrossing(["query-crossing"]);

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- task DONE");
      await settle();
      await settle();
      // The crossing itself still happens — the notice is what was silenced.
      expect(doc.byId.query.raw).toBe("{{tine-query -- task DONE}}");
      expect(notice(root)).toBeNull();
    } finally {
      dispose();
    }
  });

  it("reloads the dismissal after switching graphs without resetting the window cache", async () => {
    load('{{query (task TODO)}}');
    arrangeCrossing(["query-crossing"]);
    const get = vi.mocked(backend().getAppBool);
    let view = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(view.root, "-- task DONE");
      expect(notice(view.root)).toBeNull();
      view.dispose();
      resetStore();
      bumpGraphEpoch();
      load('{{query (task TODO)}}');
      get.mockResolvedValue(false);
      view = mount(() => <Block id="query" />);
      await saveThroughPane(view.root, "-- task DONE");
      await waitForNotice(view.root);
      expect(get.mock.calls.filter(([key]) => key === NOTICE_KEY)).toHaveLength(2);
    } finally { view.dispose(); }
  });

  it("ignores a graph A dismissal read that finishes after graph B crosses", async () => {
    load('{{query (task TODO)}}');
    arrangeCrossing();
    let finish!: (value: boolean) => void;
    const get = vi.mocked(backend().getAppBool);
    get.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    let view = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(view.root, "-- task DONE");
      expect(notice(view.root)).toBeNull();
      view.dispose(); resetStore(); bumpGraphEpoch();
      load('{{query (task TODO)}}');
      get.mockResolvedValue(false);
      view = mount(() => <Block id="query" />);
      await saveThroughPane(view.root, "-- task DONE");
      await waitForNotice(view.root);
      finish(true);
      await settle();
      expect(notice(view.root)).not.toBeNull();
    } finally { view.dispose(); }
  });

  it("still crosses, and still offers the notice, when the notices store cannot be read", async () => {
    load('{{query (task TODO)}}');
    arrangeCrossing();
    vi.spyOn(backend(), "getAppBool").mockRejectedValue(new Error("no app-data dir"));
    resetDismissedNoticesForTests();
    bumpGraphEpoch();

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- task DONE");
      // D-3/G2: recovery over refusal. A damaged dismissal record costs one
      // extra notice, never the edit and never the graph.
      await waitForNotice(root);
      expect(doc.byId.query.raw).toBe("{{tine-query -- task DONE}}");
    } finally {
      dispose();
    }
  });
});

// C5 (P4). **One notice, which MOVES.** The sheet is portalled to <body>, so a
// notice drawn under the block while the sheet is open sits somewhere the
// reader is not looking — at the far end of an overlay, behind the sheet they
// are editing in. So the same component, with the same state, is hosted by the
// pane while the sheet is open and inline under the block while it is shut.
// What must never happen is a SECOND copy: two notices means two undo buttons,
// one of which is about a change the other already took back.
describe("C5: the notice is one instance that changes host with the sheet", () => {
  it("sits inside the text pane while the sheet is open, and under the block once it is shut", async () => {
    load('{{query (task TODO)}}');
    arrangeCrossing();

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- task DONE");
      const shown = await waitForNotice(root);
      expect(document.querySelectorAll(".query-crossing-notice")).toHaveLength(1);
      expect(shown.closest(".query-text-pane")).not.toBeNull();

      // Shut the sheet: the notice is still there, still one, and now inline
      // under the block where the reader is.
      root.querySelector<HTMLButtonElement>(".qs-gear")!.click();
      await settle();
      expect(document.querySelector(".qs-sheet")).toBeNull();
      const inline = await waitForNotice(root);
      expect(document.querySelectorAll(".query-crossing-notice")).toHaveLength(1);
      expect(inline.closest(".query-text-pane")).toBeNull();
      expect(root.contains(inline)).toBe(true);
      expect(inline.textContent?.replace(/\s+/g, " ")).toContain(NOTICE_TEXT);
      expect(inline.querySelector(".query-crossing-notice-undo")).not.toBeNull();

      // ...and back again, without ever being two.
      root.querySelector<HTMLButtonElement>(".qs-gear")!.click();
      await settle();
      const reopened = await waitForNotice(root);
      expect(document.querySelectorAll(".query-crossing-notice")).toHaveLength(1);
      expect(reopened.closest(".query-text-pane")).not.toBeNull();
    } finally {
      dispose();
    }
  });

  it("shows a bounded excerpt of what the block now reads, without printing again", async () => {
    load('{{query (task TODO)}}');
    arrangeCrossing();
    const print = vi.mocked(backend().printQuery);

    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await saveThroughPane(root, "-- task DONE");
      const shown = await waitForNotice(root);
      const changed = shown.querySelector<HTMLElement>(".query-crossing-notice-changed");
      expect(changed).not.toBeNull();
      expect(changed!.textContent).toContain("The block now reads:");
      // The excerpt is the query ARGUMENT the save wrote, bounded — not the
      // whole macro, and not a second rendering of the query.
      expect(changed!.querySelector("code")?.textContent).toBe("-- task DONE");
      // The excerpt is a slice of the bytes the save ALREADY produced. Asking
      // the printer a second time to describe the crossing would be a second
      // print dialect on the save path, which the print-dialect pin forbids.
      const dialects = print.mock.calls.map(([, , dialect]) => dialect);
      expect(dialects).not.toContain("og");
    } finally {
      dispose();
    }
  });
});

// Kill-and-reopen for the crossing save (batch 14q4a). The crossing is two
// store edits — `setRaw` (the macro rename) and `setBlockProperty` (the `tine.*`
// view key the OG text can no longer carry) — in ONE undo unit. On disk it must
// also be ONE write: a process killed after the save either finds both or
// neither on reopen, never a `{{tine-query}}` without its sort. The page write
// itself is temp+fsync+rename (the audited path); this pins that the two edits
// reach it in the same page payload, and that undo takes both back together.
describe("the crossing save is one page write", () => {
  it("persists the renamed macro and its tine.* view key in a single savePages payload", async () => {
    // The OG text carries the sort; TQL cannot, so the crossing moves it to
    // `tine.sort` on the same block.
    load("{{query (task TODO) (sort-by priority desc)}}");
    arrangeCrossing();
    const save = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["rev-2"] });
    vi.spyOn(backend(), "parseQuery").mockImplementation(async (source: string) => ({
      ...parsedAs(source), view: { sort: [["priority", "desc"]] },
    }));
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      const input = await openPane(root);
      input.value = "-- task DONE";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      const button = await vi.waitFor(() => {
        const found = document.querySelector<HTMLButtonElement>(".query-text-pane-save");
        if (!found || found.disabled) throw new Error("save is not enabled yet");
        return found;
      });
      button.click();
      await vi.waitFor(() => expect(doc.byId.query.raw).toContain("tine.sort:: priority desc"));
      expect(doc.byId.query.raw.startsWith("{{tine-query -- task DONE}}")).toBe(true);

      expect(await flushPage("Sheet")).toBe(true);
      expect(save).toHaveBeenCalledTimes(1);
      const written = JSON.stringify(save.mock.calls[0][0]);
      expect(written).toContain("{{tine-query -- task DONE}}");
      expect(written).toContain("tine.sort:: priority desc");

      undo();
      expect(doc.byId.query.raw).toBe("{{query (task TODO) (sort-by priority desc)}}");
    } finally {
      dispose();
    }
  });
});

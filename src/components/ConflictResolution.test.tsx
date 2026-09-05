import { afterEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { render } from "solid-js/web";
import { PageConflictResolution } from "./ConflictResolution";
import { __setBackendForTest, SaveConflictError, type Backend } from "../backend";
import {
  doc,
  loadSingle,
  pageByName,
  resetStore,
  setRaw,
  setDoc,
  takeEditorLease,
} from "../store";
import {
  conflictQueue,
  registerLiveSaveConflict,
  restoreLiveSaveConflicts,
  setConflictQueue,
  setGraphMeta,
  setToasts,
  toasts,
} from "../ui";
import type {
  ConflictObject,
  MarkerConflictDiff,
  MergeDecision,
  PageDto,
  SyncConflictDiff,
} from "../types";

// Concord P4 (L4 + L5). Fail-before: nothing in Tine could resolve a
// VCS-marker conflict at all — a marker-bearing page showed a banner telling the
// user to go and fix it in another tool, and a conflict copy could only be
// merged from a Settings modal. These assert the in-page surface: the sides the
// artifact itself named, a suggested resolution pre-selected from the markers'
// own common ancestor, keep-both as the no-loss fallback, and an apply that goes
// through the guarded backend path with the file's own base_rev.

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  document.body.innerHTML = "";
  __setBackendForTest(null);
  setConflictQueue([]);
  setGraphMeta(null);
  setToasts([]);
  localStorage.clear();
  resetStore();
});

const view = (text: string) => ({ uuid: "", text, child_count: 0 });

const markerObject: ConflictObject = {
  id: "markers:pages/Merged.md",
  source: "vcs-markers",
  page_name: "Merged",
  page_path: "pages/Merged.md",
  kind: "page",
  sides: [
    { role: "mine", label: "HEAD" },
    { role: "theirs", label: "feature" },
    { role: "base", label: "Common ancestor" },
  ],
  block_conflicts: 2,
  markers: ["<<<<<<<", "|||||||", "=======", ">>>>>>>"],
};

/** A 3-way marker diff: one row only we changed, one only they changed, and one
 *  both changed (so it has no suggestion and falls back to keep-both). */
const markerDiff: MarkerConflictDiff = {
  mine_label: "HEAD",
  theirs_label: "feature",
  regions: 1,
  diff: {
    base_rev: "marker-file-rev",
    conflict_rev: "marker-file-rev",
    rows: [
      { id: "0", kind: "unchanged", mine: view("shared top"), theirs: view("shared top"), children: [] },
      {
        id: "1",
        kind: "modified",
        mine: view("alpha edited here"),
        theirs: view("alpha"),
        children: [],
        verdict: "mine-only",
        suggestion: "mine",
      },
      {
        id: "2",
        kind: "modified",
        mine: view("beta"),
        theirs: view("beta edited there"),
        children: [],
        verdict: "theirs-only",
        suggestion: "theirs",
      },
      {
        id: "3",
        kind: "modified",
        mine: view("gamma my way"),
        theirs: view("gamma their way"),
        children: [],
        verdict: "both-changed",
      },
    ],
    mine_pre: null,
    theirs_pre: null,
    pre_differs: false,
    blocks_identical: false,
    three_way: true,
  },
};

function stubBackend(overrides: Partial<Backend>): void {
  __setBackendForTest({
    vcsMarkerConflictDiff: async () => markerDiff,
    syncConflictDiff: async () => null,
    resolveVcsMarkerConflict: async () => {},
    resolveSyncConflict: async () => {},
    listSyncConflicts: async () => [],
    listVcsMarkerConflicts: async () => [],
    conflictQueue: async () => [],
    liveSaveConflictDiff: async () => markerDiff.diff,
    captureLiveSaveConflict: async () => ({
      diff: markerDiff.diff,
      base_text: "- base\n",
      disk_rev: "disk-rev",
    }),
    durableLiveSaveConflictDiff: async () => markerDiff.diff,
    resolveDurableLiveSaveConflict: async (page: PageDto) => ({ ...page, rev: "resolved-rev" }),
    resolveLiveSaveConflict: async (page: PageDto) => ({ ...page, rev: "resolved-rev" }),
    ...overrides,
  } as unknown as Backend);
}

function mount(conflict: ConflictObject): { host: HTMLElement; dispose: () => void } {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <PageConflictResolution conflict={conflict} />, host);
  return { host, dispose };
}

describe("in-page conflict resolution", () => {
  it("has no message-sniffing conflict branch in the production component", () => {
    const source = readFileSync("src/components/ConflictResolution.tsx", "utf8");
    expect(source).not.toContain('.includes("conflict")');
    expect(source).not.toContain("String(e)");
  });

  it("routes ordinary prose containing conflict through the generic failure path", async () => {
    stubBackend({
      resolveVcsMarkerConflict: async () => {
        throw new Error("ordinary prose containing conflict");
      },
    });
    const { host, dispose } = mount(markerObject);
    try {
      await flush();
      await flush();
      [...host.querySelectorAll("button")]
        .find((button) => button.textContent?.includes("Apply resolution"))!
        .click();
      await flush();
      await flush();
      expect(toasts().at(-1)?.message).toBe(
        "Couldn’t resolve it: ordinary prose containing conflict",
      );
      expect(toasts().some((toast) => toast.message.includes("file changed on disk"))).toBe(false);
    } finally {
      dispose();
    }
  });

  it("takes the disk-changed recovery branch only for the typed conflict the call funnel mints", async () => {
    // Fail-before (wave-2 review H2-1): the seven resolve commands rejected with
    // the raw `conflict:<epoch>` string, only save_page was classified, and this
    // branch was unreachable — the user got a generic failure instead of a
    // re-read. The funnel now types every native rejection; the component
    // consumes the type and nothing else.
    stubBackend({
      resolveVcsMarkerConflict: async () => {
        throw new SaveConflictError(7);
      },
    });
    const { host, dispose } = mount(markerObject);
    try {
      await flush();
      await flush();
      [...host.querySelectorAll("button")]
        .find((button) => button.textContent?.includes("Apply resolution"))!
        .click();
      await flush();
      await flush();
      expect(toasts().at(-1)?.message).toContain("file changed on disk");
    } finally {
      dispose();
    }
  });

  it("names the sides the marker file itself named", async () => {
    stubBackend({});
    const { host, dispose } = mount(markerObject);
    try {
      await flush();
      await flush();
      const legend = host.querySelector(".page-conflict-legend")!;
      expect(legend.textContent).toContain("HEAD");
      expect(legend.textContent).toContain("feature");
      // The third side is a first-class part of the object, not an assumption
      // that a conflict has exactly two sides.
      expect(legend.textContent).toContain("Common ancestor");
    } finally {
      dispose();
    }
  });

  it("pre-selects the suggested side, and keeps BOTH where no side is suggested", async () => {
    stubBackend({});
    const { host, dispose } = mount(markerObject);
    try {
      await flush();
      await flush();
      const active = (id: string) =>
        host
          .querySelector(`[data-row-id="${id}"]`)!
          .querySelector(".sync-merge-seg.active")!
          .getAttribute("data-decision");
      expect(active("1")).toBe("mine"); // suggestion: mine
      expect(active("2")).toBe("theirs"); // suggestion: theirs
      // Both sides moved away from the ancestor: no suggestion is possible, so
      // the no-loss default takes over instead of silently dropping a side.
      expect(active("3")).toBe("both");
    } finally {
      dispose();
    }
  });

  it("counts the regions needing a decision and offers navigation", async () => {
    stubBackend({});
    const { host, dispose } = mount(markerObject);
    try {
      await flush();
      await flush();
      expect(host.querySelector(".page-conflict-count")!.textContent).toBe("3 conflicts");
      expect(host.querySelectorAll(".page-conflict-nav button").length).toBe(2);
    } finally {
      dispose();
    }
  });

  it("applies through the guarded marker path with the file's own base_rev", async () => {
    const resolve = vi.fn(async () => {});
    stubBackend({ resolveVcsMarkerConflict: resolve as unknown as Backend["resolveVcsMarkerConflict"] });
    const { host, dispose } = mount(markerObject);
    try {
      await flush();
      await flush();
      expect(resolve).not.toHaveBeenCalled(); // nothing auto-applies
      const apply = [...host.querySelectorAll("button")].find((b) =>
        b.textContent?.includes("Apply resolution")
      )!;
      apply.click();
      await flush();
      await flush();
      expect(resolve).toHaveBeenCalledTimes(1);
      const [path, decisions, baseRev] = resolve.mock.calls[0] as unknown as [
        string,
        Record<string, string>,
        string,
      ];
      expect(path).toBe("pages/Merged.md");
      expect(baseRev).toBe("marker-file-rev");
      expect(decisions).toEqual({ "1": "mine", "2": "theirs", "3": "both" });
    } finally {
      dispose();
    }
  });

  it("routes a conflict copy through the existing resolve path, not the marker one", async () => {
    const copyDiff: SyncConflictDiff = {
      base_rev: "winner-rev",
      conflict_rev: "copy-rev",
      rows: [{ id: "0", kind: "modified", mine: view("mine"), theirs: view("theirs"), children: [] }],
      mine_pre: null,
      theirs_pre: null,
      pre_differs: false,
      blocks_identical: false,
    };
    const resolveCopy = vi.fn(async (): Promise<PageDto> => ({
      name: "Note",
      kind: "page",
      title: "Note",
      pre_block: null,
      path: "pages/Note.md",
      rev: "merged-rev",
      blocks: [{ id: "note", raw: "mine", collapsed: false, children: [] }],
    }));
    const resolveMarkers = vi.fn(async () => {});
    stubBackend({
      syncConflictDiff: (async () => copyDiff) as unknown as Backend["syncConflictDiff"],
      resolveSyncConflict: resolveCopy as unknown as Backend["resolveSyncConflict"],
      resolveVcsMarkerConflict: resolveMarkers as unknown as Backend["resolveVcsMarkerConflict"],
    });
    const copyObject: ConflictObject = {
      id: "copy:pages/Note.sync-conflict-20260817-101010-ABCDEFG.md",
      source: "sync-copy",
      page_name: "Note",
      page_path: "pages/Note.md",
      kind: "page",
      sides: [
        { role: "mine", label: "This device", path: "pages/Note.md" },
        {
          role: "theirs",
          label: "sync-conflict-20260817-101010-ABCDEFG",
          path: "pages/Note.sync-conflict-20260817-101010-ABCDEFG.md",
        },
      ],
      block_conflicts: 1,
    };
    setDoc({
      byId: {
        note: { id: "note", raw: "mine", collapsed: false, parent: null, page: "Note", children: [] },
      },
      pages: [{
        name: "Note",
        kind: "page",
        title: "Note",
        preBlock: null,
        roots: ["note"],
        format: "md",
        readOnly: false,
        guide: false,
        path: "pages/Note.md",
      }],
      feed: ["Note"],
      loaded: true,
    });
    const { host, dispose } = mount(copyObject);
    try {
      await flush();
      await flush();
      [...host.querySelectorAll("button")]
        .find((b) => b.textContent?.includes("Apply resolution"))!
        .click();
      await flush();
      await flush();
      expect(resolveMarkers).not.toHaveBeenCalled();
      expect(resolveCopy).toHaveBeenCalledTimes(1);
      const [winner, copy, , baseRev, conflictRev] = resolveCopy.mock
        .calls[0] as unknown as [string, string, unknown, string, string];
      expect(winner).toBe("pages/Note.md");
      expect(copy).toBe("pages/Note.sync-conflict-20260817-101010-ABCDEFG.md");
      expect(baseRev).toBe("winner-rev");
      expect(conflictRev).toBe("copy-rev");
    } finally {
      dispose();
    }
  });

  it("replaces the open editor with the exact page committed by a conflict-copy resolution", async () => {
    const copyDiff: SyncConflictDiff = {
      base_rev: "winner-rev",
      conflict_rev: "copy-rev",
      rows: [{ id: "0", kind: "modified", mine: view("desktop"), theirs: view("phone"), children: [] }],
      mine_pre: null,
      theirs_pre: null,
      pre_differs: false,
      blocks_identical: false,
    };
    const resolved: PageDto = {
      name: "Note",
      kind: "page",
      title: "Note",
      pre_block: null,
      path: "pages/Note.md",
      rev: "merged-rev",
      blocks: [{ id: "merged", raw: "desktop and phone", collapsed: false, children: [] }],
    };
    setDoc({
      byId: {
        old: { id: "old", raw: "desktop", collapsed: false, parent: null, page: "Note", children: [] },
      },
      pages: [{
        name: "Note",
        kind: "page",
        title: "Note",
        preBlock: null,
        roots: ["old"],
        format: "md",
        readOnly: false,
        guide: false,
        path: "pages/Note.md",
      }],
      feed: ["Note"],
      loaded: true,
    });
    const resolveCopy = vi.fn(async () => resolved);
    stubBackend({
      syncConflictDiff: async () => copyDiff,
      resolveSyncConflict: resolveCopy as unknown as Backend["resolveSyncConflict"],
      activateEditor: async (path) => ({ activation: 17, target: path, prospective: false }),
    });
    const copyObject: ConflictObject = {
      id: "copy:pages/Note.sync-conflict-20260817-101010-ABCDEFG.md",
      source: "sync-copy",
      page_name: "Note",
      page_path: "pages/Note.md",
      kind: "page",
      sides: [
        { role: "mine", label: "This device", path: "pages/Note.md" },
        { role: "theirs", label: "Phone", path: "pages/Note.sync-conflict-20260817-101010-ABCDEFG.md" },
      ],
      block_conflicts: 1,
    };
    const { host, dispose } = mount(copyObject);
    try {
      await flush();
      await flush();
      [...host.querySelectorAll("button")]
        .find((button) => button.textContent?.includes("Apply resolution"))!
        .click();
      await flush();
      await flush();
      await flush();
      const installed = pageByName("Note")!;
      expect(installed.roots).toHaveLength(1);
      expect(doc.byId[installed.roots[0]].raw).toBe("desktop and phone");
    } finally {
      dispose();
    }
  });

  it("does not commit a conflict-copy resolution over component-local uncommitted input", async () => {
    const copyDiff: SyncConflictDiff = {
      base_rev: "winner-rev",
      conflict_rev: "copy-rev",
      rows: [{ id: "0", kind: "modified", mine: view("desktop"), theirs: view("phone"), children: [] }],
      mine_pre: null,
      theirs_pre: null,
      pre_differs: false,
      blocks_identical: false,
    };
    setDoc({
      byId: {
        note: { id: "note", raw: "desktop", collapsed: false, parent: null, page: "Note", children: [] },
      },
      pages: [{
        name: "Note",
        kind: "page",
        title: "Note",
        preBlock: null,
        roots: ["note"],
        format: "md",
        readOnly: false,
        guide: false,
        path: "pages/Note.md",
      }],
      feed: ["Note"],
      loaded: true,
    });
    const resolveCopy = vi.fn(async () => {
      throw new Error("must not be called");
    });
    stubBackend({
      syncConflictDiff: async () => copyDiff,
      resolveSyncConflict: resolveCopy as unknown as Backend["resolveSyncConflict"],
    });
    const releaseLease = takeEditorLease("Note");
    const copyObject: ConflictObject = {
      id: "copy:pages/Note.sync-conflict-20260822-120000-PHONE.md",
      source: "sync-copy",
      page_name: "Note",
      page_path: "pages/Note.md",
      kind: "page",
      sides: [
        { role: "mine", label: "This device", path: "pages/Note.md" },
        { role: "theirs", label: "Phone", path: "pages/Note.sync-conflict-20260822-120000-PHONE.md" },
      ],
      block_conflicts: 1,
    };
    const { host, dispose } = mount(copyObject);
    try {
      await flush();
      await flush();
      [...host.querySelectorAll("button")]
        .find((button) => button.textContent?.includes("Apply resolution"))!
        .click();
      await flush();
      expect(resolveCopy).not.toHaveBeenCalled();
      expect(doc.byId.note.raw).toBe("desktop");
    } finally {
      releaseLease();
      dispose();
    }
  });

  it("routes an in-memory save conflict through Concord's guarded live resolution", async () => {
    const draft: PageDto = {
      name: "Note",
      kind: "page",
      title: "Note",
      pre_block: null,
      path: "pages/Note.md",
      rev: "editor-base-rev",
      activation: 7,
      blocks: [{ id: "1", raw: "my edit", collapsed: false, children: [] }],
    };
    const liveDiff: SyncConflictDiff = {
      ...markerDiff.diff,
      base_rev: "editor-base-rev",
      conflict_rev: "42",
    };
    const resolve = vi.fn(async (
      _page: PageDto,
      _baseRev: string | null,
      _epoch: number,
      _decisions: Record<string, MergeDecision>,
      _preChoice?: "mine" | "theirs" | "union",
    ) => ({ ...draft, rev: "resolved-rev" }));
    stubBackend({
      liveSaveConflictDiff: (async () => liveDiff) as Backend["liveSaveConflictDiff"],
      resolveLiveSaveConflict: resolve as Backend["resolveLiveSaveConflict"],
      getPageByPath: async () => null,
    });
    const liveObject: ConflictObject = {
      id: "live:pages/Note.md",
      source: "live-save",
      page_name: "Note",
      page_path: "pages/Note.md",
      kind: "page",
      sides: [
        { role: "mine", label: "Your retained draft" },
        { role: "theirs", label: "Current file on disk" },
        { role: "base", label: "Last version this editor loaded" },
      ],
      live: { page: draft, base_rev: "editor-base-rev", conflict_epoch: 42, draft_version: 1 },
    };
    loadSingle(draft);
    const { host, dispose } = mount(liveObject);
    try {
      await flush();
      await flush();
      expect(host.querySelector(".page-conflict-title")!.textContent).toContain(
        "Your draft and the current file both changed",
      );
      [...host.querySelectorAll("button")]
        .find((button) => button.textContent?.includes("Apply resolution"))!
        .click();
      await flush();
      await flush();
      expect(resolve).toHaveBeenCalledTimes(1);
      const [page, baseRev, epoch, decisions] = resolve.mock.calls[0];
      expect(page).toMatchObject({
        name: draft.name,
        path: draft.path,
        blocks: draft.blocks,
      });
      expect(baseRev).toBe("editor-base-rev");
      expect(epoch).toBe(42);
      expect(decisions).toEqual({ "1": "mine", "2": "theirs", "3": "both" });
    } finally {
      dispose();
    }
  });

  it("requires a fresh review instead of discarding a draft changed after the live diff appeared", async () => {
    const draft: PageDto = {
      name: "Note",
      kind: "page",
      title: "Note",
      pre_block: null,
      path: "pages/Note.md",
      blocks: [{ id: "1", raw: "reviewed draft", collapsed: false, children: [] }],
    };
    loadSingle(draft);
    const resolve = vi.fn(async (page: PageDto) => ({ ...page, rev: "resolved-rev" }));
    stubBackend({ resolveLiveSaveConflict: resolve as Backend["resolveLiveSaveConflict"] });
    const liveObject: ConflictObject = {
      id: "live:pages/Note.md",
      source: "live-save",
      page_name: "Note",
      page_path: "pages/Note.md",
      kind: "page",
      sides: [
        { role: "mine", label: "Your retained draft" },
        { role: "theirs", label: "Current file on disk" },
        { role: "base", label: "Last version this editor loaded" },
      ],
      live: { page: draft, base_rev: "base", conflict_epoch: 9, draft_version: 1 },
    };
    const { host, dispose } = mount(liveObject);
    try {
      await flush();
      await flush();
      setRaw("1", "reviewed draft plus a newer edit");
      [...host.querySelectorAll("button")]
        .find((button) => button.textContent?.includes("Apply resolution"))!
        .click();
      await flush();
      await flush();
      expect(resolve).not.toHaveBeenCalled();
      expect(doc.byId["1"].raw).toBe("reviewed draft plus a newer edit");
    } finally {
      dispose();
    }
  });

  it("rehydrates a durable live conflict after restart and uses its revision guard", async () => {
    const draft: PageDto = {
      name: "Durable",
      kind: "page",
      title: "Durable",
      pre_block: null,
      path: "pages/Durable.md",
      rev: "base-rev",
      blocks: [{ id: "1", raw: "draft", collapsed: false, children: [] }],
    };
    setGraphMeta({ root: "/graph", preferred_format: "md" } as never);
    registerLiveSaveConflict(draft, "base-rev", 5, {
      base_text: "- base\n",
      disk_rev: "disk-rev",
    });
    setConflictQueue([]); // process memory is gone; app-private capsule remains
    restoreLiveSaveConflicts("/graph");
    const restored = conflictQueue()[0];
    expect(restored.live?.page.blocks[0].raw).toBe("draft");
    loadSingle({
      ...draft,
      rev: "disk-rev",
      blocks: [{ id: "disk", raw: "disk from phone", collapsed: false, children: [] }],
    });

    const resolve = vi.fn(async (
      _page: PageDto,
      _diskRev: string,
      _decisions: Record<string, MergeDecision>,
      _preChoice?: "mine" | "theirs" | "union",
    ) => ({ ...draft, rev: "resolved-rev" }));
    stubBackend({
      durableLiveSaveConflictDiff: async () => ({
        ...markerDiff.diff,
        conflict_rev: "disk-rev",
      }),
      resolveDurableLiveSaveConflict: resolve,
      getPageByPath: async () => null,
    });
    const { host, dispose } = mount(restored);
    try {
      await flush();
      await flush();
      [...host.querySelectorAll("button")]
        .find((button) => button.textContent?.includes("Apply resolution"))!
        .click();
      await flush();
      await flush();
      expect(resolve).toHaveBeenCalledTimes(1);
      expect(resolve.mock.calls[0][0].blocks[0].raw).toBe("draft");
      expect(resolve.mock.calls[0][1]).toBe("disk-rev");
    } finally {
      dispose();
    }
  });

  // Concord P5. The Settings modal was the only surface that let the user choose
  // what happens to the page's OWN properties when the two sides' pre-blocks
  // differ; the in-page resolver hardcoded "union". Retiring the modal without
  // this would have silently dropped a capability.
  it("offers the page-property choice the retired Settings modal used to own", async () => {
    const resolve = vi.fn(async () => {});
    const preDiff: MarkerConflictDiff = {
      ...markerDiff,
      diff: {
        ...markerDiff.diff,
        mine_pre: "alias:: here",
        theirs_pre: "alias:: there",
        pre_differs: true,
      },
    };
    stubBackend({
      vcsMarkerConflictDiff: (async () => preDiff) as unknown as Backend["vcsMarkerConflictDiff"],
      resolveVcsMarkerConflict: resolve as unknown as Backend["resolveVcsMarkerConflict"],
    });
    const { host, dispose } = mount(markerObject);
    try {
      await flush();
      await flush();
      const choice = host.querySelector<HTMLSelectElement>(".page-conflict-preblock-choice")!;
      // No-loss by default, consistent with this surface's row policy.
      expect(choice.value).toBe("union");
      expect([...choice.options].map((o) => o.value)).toEqual(["union", "mine", "theirs"]);

      choice.value = "mine";
      choice.dispatchEvent(new Event("change"));
      [...host.querySelectorAll("button")]
        .find((b) => b.textContent?.includes("Apply resolution"))!
        .click();
      await flush();
      await flush();
      const [, , , preChoice] = resolve.mock.calls[0] as unknown as [
        string,
        Record<string, string>,
        string,
        string,
      ];
      expect(preChoice).toBe("mine");
    } finally {
      dispose();
    }
  });

  it("hides the page-property choice when the two sides agree on them", async () => {
    stubBackend({});
    const { host, dispose } = mount(markerObject);
    try {
      await flush();
      await flush();
      expect(host.querySelector(".page-conflict-preblock-choice")).toBeNull();
    } finally {
      dispose();
    }
  });

  // Concord's fourth outcome. The surface treats a suggested MERGED body like
  // any other suggestion — it is counted, "Apply all suggested" restores it, and
  // the decision reaches the guarded resolver as the plain string the backend
  // re-derives from. Nothing about the merged text itself is ever sent.
  it("counts a merged suggestion and sends it through the guarded copy resolver", async () => {
    const copyDiff: SyncConflictDiff = {
      base_rev: "winner-rev",
      conflict_rev: "copy-rev",
      rows: [{
        id: "0",
        kind: "modified",
        mine: view("Desktop"),
        theirs: view("Desktop 5 kk"),
        children: [],
        verdict: "both-changed",
        suggestion: "merged",
        merged: { text: "Desktop kk", source: "computed" },
      }],
      mine_pre: null,
      theirs_pre: null,
      pre_differs: false,
      blocks_identical: false,
      three_way: true,
    };
    const resolveCopy = vi.fn(async (): Promise<PageDto> => ({
      name: "Note",
      kind: "page",
      title: "Note",
      pre_block: null,
      path: "pages/Note.md",
      rev: "merged-rev",
      blocks: [{ id: "note", raw: "Desktop kk", collapsed: false, children: [] }],
    }));
    stubBackend({
      syncConflictDiff: (async () => copyDiff) as unknown as Backend["syncConflictDiff"],
      resolveSyncConflict: resolveCopy as unknown as Backend["resolveSyncConflict"],
    });
    const copyObject: ConflictObject = {
      id: "copy:pages/Note.sync-conflict-20260824-090000-MERGED1.md",
      source: "sync-copy",
      page_name: "Note",
      page_path: "pages/Note.md",
      kind: "page",
      sides: [
        { role: "mine", label: "This device", path: "pages/Note.md" },
        { role: "theirs", label: "Phone", path: "pages/Note.sync-conflict-20260824-090000-MERGED1.md" },
      ],
      block_conflicts: 1,
    };
    setDoc({
      byId: {
        note: { id: "note", raw: "Desktop", collapsed: false, parent: null, page: "Note", children: [] },
      },
      pages: [{
        name: "Note",
        kind: "page",
        title: "Note",
        preBlock: null,
        roots: ["note"],
        format: "md",
        readOnly: false,
        guide: false,
        path: "pages/Note.md",
      }],
      feed: ["Note"],
      loaded: true,
    });
    const { host, dispose } = mount(copyObject);
    try {
      await flush();
      await flush();
      expect(host.querySelector(".sync-merge-toolbar")!.textContent).toContain("1 of 1 pre-selected");
      expect(
        host.querySelector(".sync-merge-seg.active")!.getAttribute("data-decision"),
      ).toBe("merged");
      // Overriding and then restoring goes through the same suggestion path.
      (host.querySelector('.sync-merge-seg[data-decision="both"]') as HTMLElement).click();
      await flush();
      [...host.querySelectorAll("button")]
        .find((b) => b.textContent?.includes("Apply all suggested"))!
        .click();
      await flush();
      expect(
        host.querySelector(".sync-merge-seg.active")!.getAttribute("data-decision"),
      ).toBe("merged");
      [...host.querySelectorAll("button")]
        .find((b) => b.textContent?.includes("Apply resolution"))!
        .click();
      await flush();
      await flush();
      const [, , decisions] = resolveCopy.mock.calls[0] as unknown as [
        string,
        string,
        Record<string, MergeDecision>,
      ];
      expect(decisions).toEqual({ "0": "merged" });
    } finally {
      dispose();
    }
  });

  it("warns quietly — never blocks — when the page is left unresolved", async () => {
    stubBackend({});
    setConflictQueue([markerObject]);
    const { dispose } = mount(markerObject);
    await flush();
    await flush();
    // Unmounting IS leaving the page. The object is still queued, so a note is
    // pushed; no dialog and no navigation veto exist anywhere in this path.
    dispose();
    await flush();
    expect(document.querySelector(".sync-merge-overlay")).toBeNull();
    expect(conflictQueue()).toHaveLength(1);
  });
});

// The conflict dock (spec: tine-agents/specs/concord-conflict-dock.md).
// Fail-before: the panel rendered only at the top of the page and scrolled
// away with it — on a phone a conflict was invisible until the user happened
// to scroll up. These assert the dock's state machine with a hand-fired
// IntersectionObserver (jsdom has none): bar when the panel is entirely above
// the viewport, unroll-in-place of the SAME panel node, Escape/scroll-back
// collapse, and decision state surviving the moves.
class ManualIO {
  static instances: ManualIO[] = [];
  callback: IntersectionObserverCallback;
  constructor(cb: IntersectionObserverCallback) {
    this.callback = cb;
    ManualIO.instances.push(this);
  }
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
  takeRecords(): IntersectionObserverEntry[] {
    return [];
  }
  fire(isIntersecting: boolean, top: number): void {
    this.callback(
      [{ isIntersecting, boundingClientRect: { top } } as unknown as IntersectionObserverEntry],
      this as unknown as IntersectionObserver,
    );
  }
}

describe("the conflict dock", () => {
  const realIO = globalThis.IntersectionObserver;
  const withIO = async (run: (io: ManualIO, host: HTMLElement) => Promise<void>) => {
    (globalThis as { IntersectionObserver?: unknown }).IntersectionObserver =
      ManualIO as unknown as typeof IntersectionObserver;
    stubBackend({});
    const { host, dispose } = mount(markerObject);
    try {
      await flush();
      await flush();
      const io = ManualIO.instances.at(-1)!;
      expect(io).toBeDefined();
      await run(io, host);
    } finally {
      dispose();
      ManualIO.instances = [];
      (globalThis as { IntersectionObserver?: unknown }).IntersectionObserver = realIO;
    }
  };

  it("shows no bar while the panel is in view", async () => {
    await withIO(async (io, host) => {
      io.fire(true, 120);
      await flush();
      expect(host.querySelector(".page-conflict-dock")).toBeNull();
      expect(host.querySelector(".page-conflict-slot .page-conflict")).not.toBeNull();
    });
  });

  it("docks a slim bar carrying the title and count when the panel scrolls above the viewport", async () => {
    await withIO(async (io, host) => {
      io.fire(false, -40);
      await flush();
      const bar = host.querySelector(".page-conflict-dockbar")!;
      expect(bar).not.toBeNull();
      expect(bar.getAttribute("aria-expanded")).toBe("false");
      expect(bar.textContent).toContain("Unresolved merge from your version-control tool");
      expect(bar.textContent).toContain("3 to review");
      // Collapsed bar renders no second panel; the inline one keeps its slot.
      expect(host.querySelector(".page-conflict-sheet")).toBeNull();
      expect(host.querySelector(".page-conflict-slot .page-conflict")).not.toBeNull();
    });
  });

  it("does NOT dock for a sentinel below the fold (short window at page top)", async () => {
    await withIO(async (io, host) => {
      io.fire(false, 900);
      await flush();
      expect(host.querySelector(".page-conflict-dock")).toBeNull();
    });
  });

  it("unrolls the SAME panel node into the sheet and returns it on Escape", async () => {
    await withIO(async (io, host) => {
      const panel = host.querySelector(".page-conflict")!;
      io.fire(false, -40);
      await flush();
      (host.querySelector(".page-conflict-dockbar") as HTMLButtonElement).click();
      await flush();
      const sheet = host.querySelector(".page-conflict-sheet")!;
      expect(sheet.querySelector(".page-conflict")).toBe(panel);
      expect(host.querySelector(".page-conflict-slot .page-conflict")).toBeNull();
      expect(
        host.querySelector(".page-conflict-dockbar")!.getAttribute("aria-expanded"),
      ).toBe("true");
      sheet.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
      await flush();
      expect(host.querySelector(".page-conflict-sheet")).toBeNull();
      expect(host.querySelector(".page-conflict-slot .page-conflict")).toBe(panel);
    });
  });

  it("keeps decisions made inside the sheet after collapsing and undocking", async () => {
    await withIO(async (io, host) => {
      io.fire(false, -40);
      await flush();
      (host.querySelector(".page-conflict-dockbar") as HTMLButtonElement).click();
      await flush();
      const seg = host.querySelector(
        '[data-row-id="1"] .sync-merge-seg[data-decision="theirs"]',
      ) as HTMLButtonElement;
      seg.click();
      await flush();
      io.fire(true, 60); // scrolled back to top: undock + collapse
      await flush();
      expect(host.querySelector(".page-conflict-dock")).toBeNull();
      const active = host
        .querySelector('[data-row-id="1"]')!
        .querySelector(".sync-merge-seg.active")!;
      expect(active.getAttribute("data-decision")).toBe("theirs");
    });
  });
});

// GH #490. A rejected diff must become a message, never a permanently stuck
// panel. Reading an errored Solid resource THROWS (solid.js read()), and there
// is no ErrorBoundary anywhere in src/, so the throw reaches runUpdates' catch,
// which nulls the pending Effects queue before rehandling. The effect that would
// have swapped the fallback text off "Reading both versions…" is in that queue,
// and so is every other DOM effect batched with it -- which is why the reporter
// saw a frozen panel AND a blank page body at the same time.
describe("a conflict diff that fails (GH #490)", () => {
  const failingCopy: ConflictObject = {
    id: "copy:pages/Stuck.sync-conflict-20260905-101010-ABCDEFG.md",
    source: "sync-copy",
    page_name: "Stuck",
    page_path: "pages/Stuck.md",
    kind: "page",
    sides: [
      { role: "mine", label: "This device", path: "pages/Stuck.md" },
      {
        role: "theirs",
        label: "sync-conflict-20260905-101010-ABCDEFG",
        path: "pages/Stuck.sync-conflict-20260905-101010-ABCDEFG.md",
      },
    ],
    block_conflicts: 1,
  };

  it("says it could not be read instead of staying on “Reading both versions…”", async () => {
    stubBackend({
      syncConflictDiff: (async () => {
        throw new Error("the conflict copy could not be read");
      }) as unknown as Backend["syncConflictDiff"],
    });
    const { host, dispose } = mount(failingCopy);
    await flush();
    await flush();

    const empty = host.querySelector(".page-conflict-empty")?.textContent ?? "";
    expect(empty).not.toContain("Reading both versions");
    expect(empty).toContain("Couldn’t read this conflict");
    dispose();
  });

  // "Couldn't read this conflict." on its own tells the user nothing they can
  // act on or report. The reason the backend gave belongs on screen.
  it("names the reason the backend gave", async () => {
    stubBackend({
      syncConflictDiff: (async () => {
        throw new Error("the conflict copy could not be read");
      }) as unknown as Backend["syncConflictDiff"],
    });
    const { host, dispose } = mount(failingCopy);
    await flush();
    await flush();

    expect(host.querySelector(".page-conflict-empty")?.textContent)
      .toContain("the conflict copy could not be read");
    dispose();
  });
});

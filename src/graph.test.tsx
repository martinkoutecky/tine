import { afterEach, describe, expect, it, vi } from "vitest";
import type { GraphMeta, PageRead } from "./types";

const META: GraphMeta = {
  root: "/tmp/template-graph",
  journals_dir: "journals",
  pages_dir: "pages",
  preferred_workflow: "now",
  shortcuts: {},
  start_of_week: 6,
  block_hidden_properties: [],
  default_journal_template: "Daily",
  favorites: [],
  journal_page_title_format: "MMM do, yyyy",
  journal_file_name_format: "yyyy_MM_dd",
  preferred_format: "md",
  macros: {},
  enable_timetracking: true,
  show_brackets: true,
  logbook_with_second_support: true,
  logbook_enabled_in_timestamped_blocks: false,
  logbook_enabled_in_all_blocks: false,
  guide_announced: true,
};

async function loadHarness(
  existing: PageRead | null,
  access = { graph_root: META.root, external_assets_path: null as string | null, approved: true },
  confirm = true,
  warm = false
) {
  vi.resetModules();
  const events: string[] = [];
  let meta: GraphMeta | null = null;
  const api = {
    inspectGraphAccess: vi.fn(async () => access),
    approveExternalAssets: vi.fn(async () => {}),
    confirm: vi.fn(async () => confirm),
    loadGraph: vi.fn(async () => ({ kind: "loaded" as const, meta: META, binding_generation: 1 })),
    pickFolder: vi.fn(async () => "/tmp"),
    createGraph: vi.fn(async () => META.root),
    getPage: vi.fn(async () => existing),
    resolvePage: vi.fn(async () => ({ kind: "absent" as const, id: "journals/2026_07_10.md" })),
    listTemplates: vi.fn(async () => [
      {
        name: "Daily",
        page: "Templates",
        kind: "page" as const,
        blocks: [{ id: "template", raw: "Template body", collapsed: false, children: [] }],
      },
    ]),
    savePage: vi.fn(async () => {
      events.push("save-template");
      return "new-rev";
    }),
    readCustomCss: vi.fn(async () => ""),
  };
  // Records how many events preceded each reset (order without adding events).
  const resetPageIndex = vi.fn(() => { resetAt.push(events.length); });
  const resetAt: number[] = [];
  const waitForWarmCache = vi.fn(async () => warm);
  const applyTemplateVars = vi.fn((raw: string, _currentPage?: string) => raw);
  const prepareTemplateVars = vi.fn(async () => {});
  const openPage = vi.fn();
  const drainPdfWork = vi.fn(async () => {
    events.push("drain-pdf");
    return true;
  });
  const retirePdfOwnership = vi.fn(() => { events.push("retire-pdf"); });
  const activatePdfOwnership = vi.fn((root: string) => { events.push(`activate-pdf:${root}`); });
  const closePdf = vi.fn(() => { events.push("close-pdf"); });

  vi.doMock("./backend", () => ({ backend: () => api }));
  vi.doMock("./ui", () => ({
    setGraphMeta: (next: GraphMeta | null) => { meta = next; },
    graphMeta: () => meta,
    graphEpoch: () => 0,
    bumpGraphEpoch: () => { events.push("bump-epoch"); },
    setWorkflow: vi.fn(),
    setRightSidebar: vi.fn(),
    seedFavorites: vi.fn(),
    renamePageInNavigation: vi.fn(),
    pruneSidebarBlocks: vi.fn(),
    pushToast: vi.fn(),
    refreshJournalConflicts: vi.fn(async () => {}),
    refreshSyncConflicts: vi.fn(async () => {}),
    clearRecent: vi.fn(),
    resetLeftSidebarSections: vi.fn(),
    graphTransitioning: () => false,
    setGraphTransitioning: vi.fn(),
    closePdf,
  }));
  vi.doMock("./pdfOwnership", () => ({
    drainPdfWork,
    retirePdfOwnership,
    activatePdfOwnership,
  }));
  vi.doMock("./store", () => ({ resetStore: vi.fn(), flushAll: vi.fn(async () => true) }));
  vi.doMock("./assetCache", () => ({ clearAssetBlobCache: vi.fn() }));
  vi.doMock("./router", () => ({
    resetTabsToJournals: vi.fn(),
    openPage,
    restoreSession: vi.fn(async () => {}),
    flushSession: vi.fn(async () => {}),
  }));
  vi.doMock("./panes", () => ({ resetPaneLayoutToSingle: vi.fn() }));
  vi.doMock("./journal", () => ({
    journalTitle: () => "Jul 10th, 2026",
    setJournalTitleFormat: vi.fn(),
  }));
  vi.doMock("./editor/templateVars", () => ({ applyTemplateVars, prepareTemplateVars }));
  vi.doMock("./warmCache", () => ({ waitForWarmCache }));
  vi.doMock("./pageIndex", () => ({ resetPageIndex }));
  vi.doMock("./lsShim", () => ({ CUSTOM_CSS_STYLE_ID: "test-css", ensureLsShimStyle: vi.fn() }));
  vi.doMock("./themeGallery", () => ({ ensureThemeStyle: vi.fn() }));
  vi.doMock("./platform", () => ({ isMobile: () => false, platformKind: vi.fn(async () => "desktop") }));
  vi.doMock("./guide", () => ({ maybeShowGuideAnnouncement: vi.fn() }));
  vi.doMock("./editorController", () => ({ endEdit: vi.fn() }));

  const { loadGraphPath, createNewGraph, refreshAfterRename } = await import("./graph");
  return {
    loadGraphPath, createNewGraph, refreshAfterRename, api, events, resetPageIndex, resetAt, waitForWarmCache,
    drainPdfWork, retirePdfOwnership, activatePdfOwnership, closePdf,
    applyTemplateVars, prepareTemplateVars, openPage,
  };
}

afterEach(() => {
  document.body.innerHTML = "";
  document.head.querySelector("#test-css")?.remove();
  localStorage.clear();
  vi.restoreAllMocks();
  vi.resetModules();
});

describe("default journal template graph bind", () => {
  it("drops template insertion when its page read lands after a graph switch (I-20)", async () => {
    const { loadGraphPath, api } = await loadHarness(null);
    let finish!: (page: PageRead | null) => void;
    api.getPage.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const opening = loadGraphPath(META.root);
    await vi.waitFor(() => expect(api.getPage).toHaveBeenCalled());
    const { invalidateBinding } = await import("./binding");
    invalidateBinding();
    finish(null);
    await opening;
    expect(api.savePage).not.toHaveBeenCalled();
  });

  it("drops demo seed and Welcome navigation when its page read lands after a graph switch (I-20)", async () => {
    const { createNewGraph, api, openPage } = await loadHarness(null);
    let finish!: (page: PageRead | null) => void;
    api.getPage.mockResolvedValueOnce(null).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const creating = createNewGraph();
    await vi.waitFor(() => expect(api.getPage).toHaveBeenCalledTimes(2));
    const before = api.savePage.mock.calls.length;
    const { invalidateBinding } = await import("./binding");
    invalidateBinding();
    finish(null);
    await creating;
    expect(api.savePage).toHaveBeenCalledTimes(before);
    expect(openPage).not.toHaveBeenCalled();
  });

  // Ported from "loads real page identities once and lets them win colliding
  // aliases" (with "refreshes real-page precedence after a same-session page
  // creation", "folds NFD alias keys…" and "discards an older same-epoch
  // page-inventory response"): the name answering and its refresh moved to
  // pageIndex.ts and are pinned in pageIndex.test.ts. graph.ts keeps only the
  // reset on bind, before the epoch bump that refetches, and waits for no warm
  // cache (page_inventory waits for the load itself).
  it("resets the page index on bind before the epoch bump, with no warm-cache gate", async () => {
    const { loadGraphPath, events, resetPageIndex, resetAt, waitForWarmCache } = await loadHarness(null, undefined, true, true);

    await loadGraphPath(META.root);
    expect(resetPageIndex).toHaveBeenCalledTimes(1);
    expect(resetAt[0]).toBe(events.indexOf("bump-epoch"));
    expect(waitForWarmCache).not.toHaveBeenCalled();
  });

  it("resets the page index after a rename, before the epoch bump", async () => {
    const { refreshAfterRename, events, resetPageIndex, resetAt } = await loadHarness(null);

    refreshAfterRename("Old", "New");
    expect(resetPageIndex).toHaveBeenCalledTimes(1);
    expect(resetAt).toEqual([0]);
    expect(events).toEqual(["bump-epoch"]);
  });

  it("invalidates stale loads before awaiting template work, then refreshes after save", async () => {
    const { loadGraphPath, events } = await loadHarness(null);

    await loadGraphPath(META.root);

    expect(events).toEqual([
      `activate-pdf:${META.root}`,
      "bump-epoch",
      "save-template",
      "bump-epoch",
    ]);
  });

  it("routes default-journal template blocks through the shared variable expander", async () => {
    const { loadGraphPath, api, applyTemplateVars, prepareTemplateVars } = await loadHarness(null);

    await loadGraphPath(META.root);

    expect(prepareTemplateVars).toHaveBeenCalledOnce();
    expect(applyTemplateVars).toHaveBeenCalledWith("Template body", "Jul 10th, 2026");
    // No journal file yet: the save goes to the backend's Absent id (B15b).
    expect(api.resolvePage).toHaveBeenCalledWith("Jul 10th, 2026", "journal");
    expect(api.savePage).toHaveBeenCalledWith(
      "journals/2026_07_10.md",
      expect.objectContaining({
        blocks: [expect.objectContaining({ raw: "Template body" })],
      }),
      null,
      false,
      0
    );
  });

  it("uses an empty journal's revision as the conflict baseline", async () => {
    const existing: PageRead = {
      name: "Jul 10th, 2026",
      kind: "journal",
      title: "Jul 10th, 2026",
      pre_block: null,
      blocks: [{ id: "empty", raw: "", collapsed: false, children: [] }],
      rev: "empty-journal-rev",
      id: "journals/Jul 10th, 2026.org",
    };
    const { loadGraphPath, api } = await loadHarness(existing);

    await loadGraphPath(META.root);

    // The empty journal's own file (its id), with its rev as the baseline; no
    // name lookup.
    expect(api.savePage).toHaveBeenCalledWith("journals/Jul 10th, 2026.org", expect.any(Object), "empty-journal-rev", false, 0);
    expect(api.resolvePage).not.toHaveBeenCalled();
  });

  it("refuses to write a template journal onto an alias name (B15b)", async () => {
    const { loadGraphPath, api } = await loadHarness(null);
    api.resolvePage.mockResolvedValue({ kind: "alias", owners: ["pages/Owner.md"] } as never);

    await loadGraphPath(META.root);

    expect(api.resolvePage).toHaveBeenCalledWith("Jul 10th, 2026", "journal");
    expect(api.savePage).not.toHaveBeenCalled();
  });
});

describe("external assets trust", () => {
  const external = {
    graph_root: META.root,
    external_assets_path: "/mnt/media/tine-assets",
    approved: false,
  };

  it("approves the exact resolved target before loading the graph", async () => {
    const { loadGraphPath, api } = await loadHarness(null, external, true);

    await loadGraphPath(META.root);

    expect(api.confirm).toHaveBeenCalledWith(
      expect.stringContaining("/mnt/media/tine-assets"),
      "Allow external assets directory?"
    );
    expect(api.approveExternalAssets).toHaveBeenCalledWith(
      META.root,
      "/mnt/media/tine-assets"
    );
    expect(api.approveExternalAssets.mock.invocationCallOrder[0]).toBeLessThan(
      api.loadGraph.mock.invocationCallOrder[0]
    );
  });

  it("does not bind a graph when external assets access is declined", async () => {
    const { loadGraphPath, api } = await loadHarness(null, external, false);

    await expect(loadGraphPath(META.root)).resolves.toEqual({ kind: "aborted" });

    expect(api.approveExternalAssets).not.toHaveBeenCalled();
    expect(api.loadGraph).not.toHaveBeenCalled();
  });
});

describe("PDF graph ownership", () => {
  it("drains and retires the old PDF owner before binding another graph", async () => {
    const harness = await loadHarness(null);
    await harness.loadGraphPath(META.root);
    harness.events.length = 0;
    const nextMeta = { ...META, root: "/tmp/other-graph" };
    harness.api.loadGraph.mockImplementationOnce(async () => {
      harness.events.push("load-next");
      return { kind: "loaded" as const, meta: nextMeta, binding_generation: 2 };
    });

    await harness.loadGraphPath(nextMeta.root);

    expect(harness.events).toEqual(expect.arrayContaining([
      "drain-pdf", "retire-pdf", "close-pdf", "load-next",
    ]));
    expect(harness.events.indexOf("drain-pdf")).toBeLessThan(harness.events.indexOf("retire-pdf"));
    expect(harness.events.indexOf("retire-pdf")).toBeLessThan(harness.events.indexOf("close-pdf"));
    expect(harness.events.indexOf("close-pdf")).toBeLessThan(harness.events.indexOf("load-next"));
    expect(harness.activatePdfOwnership).toHaveBeenLastCalledWith(nextMeta.root);
  });

  it("keeps the old graph bound and viewer live when PDF drain fails", async () => {
    const harness = await loadHarness(null);
    await harness.loadGraphPath(META.root);
    harness.events.length = 0;
    harness.drainPdfWork.mockResolvedValueOnce(false);

    await expect(harness.loadGraphPath("/tmp/other-graph")).resolves.toEqual({ kind: "aborted" });

    expect(harness.drainPdfWork).toHaveBeenCalledOnce();
    expect(harness.events).toEqual([]);
    expect(harness.retirePdfOwnership).not.toHaveBeenCalled();
    expect(harness.closePdf).not.toHaveBeenCalled();
    expect(harness.api.loadGraph).toHaveBeenCalledOnce();
  });

  it("publishes a fresh PDF generation for a same-root force refresh", async () => {
    const harness = await loadHarness(null);
    await harness.loadGraphPath(META.root);
    harness.events.length = 0;
    (harness.api.loadGraph as any).mockImplementationOnce(async () => {
      harness.events.push("load-refresh");
      return { kind: "already_current" as const, meta: META, binding_generation: 1 };
    });

    await harness.loadGraphPath(META.root, { forceRefresh: true });

    expect(harness.events.slice(0, 5)).toEqual([
      "drain-pdf",
      "retire-pdf",
      "close-pdf",
      "load-refresh",
      `activate-pdf:${META.root}`,
    ]);
    expect(harness.activatePdfOwnership).toHaveBeenCalledTimes(2);
  });
});

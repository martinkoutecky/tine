import { afterEach, describe, expect, it, vi } from "vitest";
import type { GraphMeta, PageDto } from "./types";

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
  existing: PageDto | null,
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
    getPage: vi.fn(async () => existing),
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
    pageAliases: vi.fn(async () => [["page1", "other"], ["shortcut", "other"]] as [string, string][]),
    listPages: vi.fn(async () => [
      { name: "page1", kind: "page" as const, date_key: null, path: "pages/page1.md" },
      { name: "Jul 10th, 2026", kind: "journal" as const, date_key: 20260710, path: "journals/2026_07_10.md" },
    ]),
  };
  const setAliasMap = vi.fn();
  const applyTemplateVars = vi.fn((raw: string, _currentPage?: string) => raw);
  const prepareTemplateVars = vi.fn(async () => {});
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
    setAliasMap,
    pageIdentityKey: (name: string) => {
      const lowered = name.trim().toLowerCase();
      const withoutLeading = lowered.startsWith("/") ? lowered.slice(1) : lowered;
      const withoutBoundaries = withoutLeading.endsWith("/")
        ? withoutLeading.slice(0, -1)
        : withoutLeading;
      return withoutBoundaries.normalize("NFC");
    },
    seedFavorites: vi.fn(),
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
    openPage: vi.fn(),
    restoreSession: vi.fn(async () => {}),
    flushSession: vi.fn(async () => {}),
  }));
  vi.doMock("./panes", () => ({ resetPaneLayoutToSingle: vi.fn() }));
  vi.doMock("./journal", () => ({
    journalTitle: () => "Jul 10th, 2026",
    setJournalTitleFormat: vi.fn(),
  }));
  vi.doMock("./editor/templateVars", () => ({ applyTemplateVars, prepareTemplateVars }));
  vi.doMock("./warmCache", () => ({ waitForWarmCache: vi.fn(async () => warm) }));
  vi.doMock("./lsShim", () => ({ CUSTOM_CSS_STYLE_ID: "test-css", ensureLsShimStyle: vi.fn() }));
  vi.doMock("./themeGallery", () => ({ ensureThemeStyle: vi.fn() }));
  vi.doMock("./platform", () => ({ isMobile: () => false, platformKind: vi.fn(async () => "desktop") }));
  vi.doMock("./guide", () => ({ maybeShowGuideAnnouncement: vi.fn() }));
  vi.doMock("./editorController", () => ({ endEdit: vi.fn() }));

  const { loadGraphPath, refreshAliases, refreshPageIdentities } = await import("./graph");
  return {
    loadGraphPath, refreshAliases, refreshPageIdentities, api, events, setAliasMap,
    drainPdfWork, retirePdfOwnership, activatePdfOwnership, closePdf,
    applyTemplateVars, prepareTemplateVars,
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
  it("loads real page identities once and lets them win colliding aliases", async () => {
    const { loadGraphPath, refreshAliases, refreshPageIdentities, api, setAliasMap } = await loadHarness(null, undefined, true, true);

    await loadGraphPath(META.root);
    await vi.waitFor(() => expect(setAliasMap).toHaveBeenLastCalledWith({
      page1: "page1",
      shortcut: "other",
    }));
    expect(api.listPages).toHaveBeenCalledTimes(1);

    await refreshAliases();
    expect(api.pageAliases).toHaveBeenCalledTimes(2);
    expect(api.listPages).toHaveBeenCalledTimes(1);

    await refreshPageIdentities();
    expect(api.listPages).toHaveBeenCalledTimes(2);
  });

  it("refreshes real-page precedence after a same-session page creation", async () => {
    const { loadGraphPath, refreshAliases, refreshPageIdentities, api, setAliasMap } = await loadHarness(null, undefined, true, true);
    await loadGraphPath(META.root);
    await vi.waitFor(() => expect(api.listPages).toHaveBeenCalledTimes(1));

    api.pageAliases.mockResolvedValue([["new page", "Alias target"]]);
    api.listPages.mockResolvedValue([
      { name: "page1", kind: "page" as const, date_key: null, path: "pages/page1.md" },
      { name: "New Page", kind: "page" as const, date_key: null, path: "pages/New Page.md" },
    ]);
    await Promise.all([refreshAliases(), refreshPageIdentities()]);

    expect(setAliasMap).toHaveBeenLastCalledWith({
      "new page": "New Page",
      page1: "page1",
    });
  });

  it("folds NFD alias keys before real-page precedence is applied", async () => {
    const { loadGraphPath, api, setAliasMap } = await loadHarness(null, undefined, true, true);
    api.pageAliases.mockResolvedValue([["Cafe\u{301}", "Alias owner"]]);
    api.listPages.mockResolvedValue([
      { name: "Café", kind: "page" as const, date_key: null, path: "pages/Café.md" },
    ]);

    await loadGraphPath(META.root);
    await vi.waitFor(() => expect(setAliasMap).toHaveBeenLastCalledWith({ café: "Café" }));
  });

  it("discards an older same-epoch page-inventory response", async () => {
    const { loadGraphPath, refreshPageIdentities, api, setAliasMap } = await loadHarness(null, undefined, true, true);
    await loadGraphPath(META.root);
    await vi.waitFor(() => expect(api.listPages).toHaveBeenCalledTimes(1));

    let releaseStale!: (entries: Awaited<ReturnType<typeof api.listPages>>) => void;
    const stale = new Promise<Awaited<ReturnType<typeof api.listPages>>>((resolve) => {
      releaseStale = resolve;
    });
    api.listPages
      .mockImplementationOnce(() => stale)
      .mockResolvedValueOnce([
        { name: "Newest", kind: "page" as const, date_key: null, path: "pages/Newest.md" },
      ]);

    const older = refreshPageIdentities();
    const newer = refreshPageIdentities();
    await newer;
    releaseStale([
      { name: "Stale", kind: "page" as const, date_key: null, path: "pages/Stale.md" },
    ]);
    await older;

    expect(setAliasMap).toHaveBeenLastCalledWith(expect.objectContaining({ newest: "Newest" }));
    expect(setAliasMap).not.toHaveBeenLastCalledWith(expect.objectContaining({ stale: "Stale" }));
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
    expect(api.savePage).toHaveBeenCalledWith(
      expect.objectContaining({
        blocks: [expect.objectContaining({ raw: "Template body" })],
      }),
      null,
      false
    );
  });

  it("uses an empty journal's revision as the conflict baseline", async () => {
    const existing: PageDto = {
      name: "Jul 10th, 2026",
      kind: "journal",
      title: "Jul 10th, 2026",
      pre_block: null,
      blocks: [{ id: "empty", raw: "", collapsed: false, children: [] }],
      rev: "empty-journal-rev",
    };
    const { loadGraphPath, api } = await loadHarness(existing);

    await loadGraphPath(META.root);

    expect(api.savePage).toHaveBeenCalledWith(expect.any(Object), "empty-journal-rev", false);
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

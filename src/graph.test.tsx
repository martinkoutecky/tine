import { afterEach, describe, expect, it, vi } from "vitest";
import type { GraphFolderPickResult, PreparedGraphFolder } from "./backend";
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

const DIRECT_ADMISSION = { binding_generation: 1, authority: "direct" as const };

async function loadHarness(
  existing: PageDto | null,
  access = { graph_root: META.root, external_assets_path: null as string | null, approved: true },
  confirm = true,
  warm = false,
  platform: "android" | "ios" | "desktop" = "desktop",
  pickerResult: GraphFolderPickResult = { status: "cancelled" }
) {
  vi.resetModules();
  const events: string[] = [];
  let meta: GraphMeta | null = null;
  let epoch = 0;
  const api = {
    inspectGraphAccess: vi.fn(async () => access),
    approveExternalAssets: vi.fn(async () => {}),
    confirm: vi.fn(async () => confirm),
    loadGraph: vi.fn(async () => ({
      kind: "loaded" as const,
      meta: META,
      binding_generation: 1,
      application_page_admission: DIRECT_ADMISSION,
    })),
    getPage: vi.fn(async () => existing),
    renamePage: vi.fn(async () => {}),
    mergePages: vi.fn(async () => {}),
    getAppString: vi.fn(async (_key: string, fallback: string) => fallback),
    setAppString: vi.fn(async (_key: string, _value: string) => {}),
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
    pickGraphFolder: vi.fn(async () => pickerResult),
    prepareGraphFolder: vi.fn(async (): Promise<PreparedGraphFolder> => ({ status: "ready", location: "local" })),
    defaultGraphParent: vi.fn(async () => "/mock"),
    createGraph: vi.fn(async () => META.root),
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
  const suspendPdfForGraphTransition = vi.fn(() => {
    events.push("suspend-pdf");
    return { filename: "assets/paper.pdf", label: "Paper" };
  });
  const restorePdfSessionTarget = vi.fn(() => {
    events.push("restore-pdf");
    return true;
  });
  const restorePendingPdfSessionTarget = vi.fn(() => {
    events.push("restore-pending-pdf");
    return true;
  });
  const pushToast = vi.fn();

  vi.doMock("./backend", () => ({ backend: () => api }));
  vi.doMock("./ui", () => ({
    setGraphMeta: (next: GraphMeta | null) => { meta = next; },
    graphMeta: () => meta,
    graphEpoch: () => epoch,
    bumpGraphEpoch: () => { epoch += 1; events.push("bump-epoch"); },
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
    // Read by applyConfigDerivedState, which declines to re-seed favorites the
    // user is already being shown (docs/contracts/config-live-reload.md §4).
    favorites: () => [] as { name: string; kind: string }[],
    seedFavorites: vi.fn(),
    pruneSidebarBlocks: vi.fn(),
    pushToast,
    refreshJournalConflicts: vi.fn(async () => {}),
    refreshSyncConflicts: vi.fn(async () => {}),
    restoreLiveSaveConflicts: vi.fn(),
    clearRecent: vi.fn(),
    resetLeftSidebarSections: vi.fn(),
    graphTransitioning: () => false,
    setGraphTransitioning: vi.fn(),
    suspendPdfForGraphTransition,
    restorePdfSessionTarget,
    restorePendingPdfSessionTarget,
  }));
  vi.doMock("./pdfOwnership", () => ({
    drainPdfWork,
    retirePdfOwnership,
    activatePdfOwnership,
  }));
  vi.doMock("./managedStorageRuntime", () => ({
    managedStorageRuntime: {
      bind: vi.fn(),
      clear: vi.fn(),
      refresh: vi.fn(async () => null),
    },
  }));
  vi.doMock("./store", () => ({ resetStore: vi.fn(), flushAll: vi.fn(async () => true) }));
  vi.doMock("./assetCache", () => ({ clearAssetBlobCache: vi.fn() }));
  vi.doMock("./router", () => ({
    resetTabsToJournals: vi.fn(),
    openPage: vi.fn(),
    restoreSession: vi.fn(async () => {}),
    flushSession: vi.fn(async () => {}),
    route: vi.fn(() => ({ kind: "journals" })),
    sameRoute: vi.fn((left, right) => JSON.stringify(left) === JSON.stringify(right)),
  }));
  vi.doMock("./panes", () => ({
    resetPaneLayoutToSingle: vi.fn(),
    removePageTargetAcrossPanes: vi.fn(),
  }));
  vi.doMock("./journal", () => ({
    journalTitle: () => "Jul 10th, 2026",
    localDayKey: (date = new Date()) =>
      date.getFullYear() * 10_000 + (date.getMonth() + 1) * 100 + date.getDate(),
    localDayRolloverDelay: vi.fn(() => 1),
    setJournalTitleFormat: vi.fn(),
  }));
  vi.doMock("./editor/templateVars", () => ({ applyTemplateVars, prepareTemplateVars }));
  vi.doMock("./warmCache", () => ({ waitForWarmCache: vi.fn(async () => warm) }));
  vi.doMock("./lsShim", () => ({ CUSTOM_CSS_STYLE_ID: "test-css", ensureLsShimStyle: vi.fn() }));
  vi.doMock("./themeGallery", () => ({ ensureThemeStyle: vi.fn() }));
  vi.doMock("./platform", () => ({ isMobile: () => platform !== "desktop", platformKind: vi.fn(async () => platform) }));
  vi.doMock("./guide", () => ({ maybeShowGuideAnnouncement: vi.fn() }));
  vi.doMock("./editorController", () => ({ endEdit: vi.fn() }));

  const { createNewGraph, ensureJournalTemplateForDay, loadGraphPath, refreshAliases, refreshPageIdentities, renameOrMergePage, switchGraph } = await import("./graph");
  return {
    createNewGraph, ensureJournalTemplateForDay, loadGraphPath, refreshAliases, refreshPageIdentities, renameOrMergePage, switchGraph,
    api, events, setAliasMap, pushToast,
    drainPdfWork, retirePdfOwnership, activatePdfOwnership,
    suspendPdfForGraphTransition, restorePdfSessionTarget,
    restorePendingPdfSessionTarget,
    applyTemplateVars, prepareTemplateVars,
    setMeta: (next: GraphMeta | null) => { meta = next; },
    bumpEpoch: () => { epoch += 1; },
  };
}

describe("mobile graph folder picker", () => {
  it("opens a picked graph from Tine's iOS Documents container", async () => {
    const harness = await loadHarness(
      null,
      undefined,
      true,
      true,
      "ios",
      { status: "picked", path: META.root }
    );

    await expect(harness.switchGraph()).resolves.toEqual({ kind: "loaded", root: META.root });
    expect(harness.api.pickGraphFolder).toHaveBeenCalledOnce();
    expect(harness.api.prepareGraphFolder).toHaveBeenCalledWith(META.root);
    expect(harness.api.loadGraph).toHaveBeenCalledWith(META.root);
  });

  it("opens the rebased iOS container path after an app update", async () => {
    const stale = "/var/mobile/Containers/Data/Application/OLD/Documents/template-graph";
    const harness = await loadHarness(null, undefined, true, true, "ios");
    harness.api.prepareGraphFolder.mockResolvedValue({
      status: "ready",
      location: "local",
      path: META.root,
    } as PreparedGraphFolder);

    await expect(harness.loadGraphPath(stale)).resolves.toEqual({ kind: "loaded", root: META.root });
    expect(harness.api.prepareGraphFolder).toHaveBeenCalledWith(stale);
    expect(harness.api.inspectGraphAccess).toHaveBeenCalledWith(META.root);
    expect(harness.api.loadGraph).toHaveBeenCalledWith(META.root);
  });

  it("shows a clear refusal when iOS returns an outside-container folder", async () => {
    const harness = await loadHarness(
      null,
      undefined,
      true,
      false,
      "ios",
      { status: "refused" }
    );

    await expect(harness.switchGraph()).resolves.toEqual({ kind: "aborted" });
    expect(harness.api.loadGraph).not.toHaveBeenCalled();
    expect(harness.pushToast).toHaveBeenCalledWith(
      "Choose a folder inside On My iPhone or iCloud Drive → TineOutline. Other Files providers aren't supported yet.",
      "info"
    );
  });

  it("prepares the chosen iCloud location before creating an iOS graph", async () => {
    const harness = await loadHarness(
      null,
      undefined,
      true,
      true,
      "ios",
      { status: "picked", path: META.root }
    );

    await expect(harness.createNewGraph()).resolves.toEqual({ kind: "loaded", root: META.root });
    expect(harness.api.pickGraphFolder).toHaveBeenCalledOnce();
    expect(harness.api.defaultGraphParent).not.toHaveBeenCalled();
    expect(harness.api.prepareGraphFolder).toHaveBeenNthCalledWith(1, META.root);
    expect(harness.api.createGraph).toHaveBeenCalledWith(META.root);
  });

  it("refuses an iOS graph before native graph inspection when its container is outside scope", async () => {
    const harness = await loadHarness(
      null,
      undefined,
      true,
      false,
      "ios",
      { status: "picked", path: META.root }
    );
    harness.api.prepareGraphFolder.mockResolvedValue({ status: "refused" });

    await expect(harness.switchGraph()).resolves.toEqual({ kind: "aborted" });
    expect(harness.api.inspectGraphAccess).not.toHaveBeenCalled();
    expect(harness.api.loadGraph).not.toHaveBeenCalled();
  });

  it("keeps a partial-provider picked graph failure sticky and retries the same target", async () => {
    const harness = await loadHarness(
      null,
      undefined,
      true,
      false,
      "android",
      { status: "picked", path: META.root }
    );
    harness.api.loadGraph.mockRejectedValue(
      new Error(
        "Tine-managed storage sync data appears to still be arriving or is incomplete. Tine left this graph unchanged. Let your file-sync provider finish, then Retry."
      )
    );

    await expect(harness.switchGraph()).resolves.toEqual({ kind: "aborted" });
    expect(harness.pushToast).toHaveBeenCalledWith(
      "Tine-managed storage sync data appears to still be arriving or is incomplete. Tine left this graph unchanged. Let your file-sync provider finish, then Retry.",
      "error",
      expect.objectContaining({ sticky: true, action: expect.objectContaining({ label: "Retry" }) })
    );
    const options = harness.pushToast.mock.calls.at(-1)![2]!;
    options.action.run();
    await vi.waitFor(() => expect(harness.api.loadGraph).toHaveBeenCalledTimes(2));
    expect(harness.api.loadGraph).toHaveBeenLastCalledWith(META.root);
  });
});

afterEach(() => {
  document.body.innerHTML = "";
  document.head.querySelector("#test-css")?.remove();
  localStorage.clear();
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.resetModules();
});

describe("page rename collisions", () => {
  it("asks before merging into a real existing page and carries rename identities", async () => {
    const destination: PageDto = {
      name: "A",
      kind: "page",
      title: "A",
      pre_block: null,
      blocks: [],
      path: "pages/A.md",
    };
    const harness = await loadHarness(destination);
    const confirm = vi.fn(() => true);
    vi.spyOn(globalThis, "confirm").mockImplementation(confirm);

    await expect(harness.renameOrMergePage("B", "A", "pages/B.md")).resolves.toBe("merged");

    expect(confirm).toHaveBeenCalledWith("Page “A” already exists. Merge “B” into it?");
    expect(harness.api.mergePages).toHaveBeenCalledWith(
      "pages/B.md",
      "pages/A.md",
      { from: "B", to: "A" },
    );
    expect(harness.api.renamePage).not.toHaveBeenCalled();
  });

  it("cancels without mutating either page", async () => {
    const destination: PageDto = {
      name: "A",
      kind: "page",
      title: "A",
      pre_block: null,
      blocks: [],
      path: "pages/A.md",
    };
    const harness = await loadHarness(destination);
    vi.spyOn(globalThis, "confirm").mockReturnValue(false);

    await expect(harness.renameOrMergePage("B", "A", "pages/B.md")).resolves.toBe("cancelled");
    expect(harness.api.mergePages).not.toHaveBeenCalled();
    expect(harness.api.renamePage).not.toHaveBeenCalled();
  });

  it("keeps the ordinary rename path when the destination has no file", async () => {
    const harness = await loadHarness(null);

    await expect(harness.renameOrMergePage("B", "C", "pages/B.md")).resolves.toBe("renamed");
    expect(harness.api.renamePage).toHaveBeenCalledWith("B", "C", "pages/B.md");
    expect(harness.api.mergePages).not.toHaveBeenCalled();
  });
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

  it("materializes on the visible-journal request without reopening the graph", async () => {
    const { loadGraphPath, ensureJournalTemplateForDay, events } = await loadHarness(null);

    await loadGraphPath(META.root);
    await ensureJournalTemplateForDay(new Date());

    expect(events).toEqual([
      `activate-pdf:${META.root}`,
      "bump-epoch",
      "save-template",
    ]);
  });

  it("routes default-journal template blocks through the shared variable expander", async () => {
    const { loadGraphPath, ensureJournalTemplateForDay, api, applyTemplateVars, prepareTemplateVars } = await loadHarness(null);

    await loadGraphPath(META.root);
    await ensureJournalTemplateForDay(new Date());

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
    const { loadGraphPath, ensureJournalTemplateForDay, api } = await loadHarness(existing);

    await loadGraphPath(META.root);
    await ensureJournalTemplateForDay(new Date());

    expect(api.savePage).toHaveBeenCalledWith(expect.any(Object), "empty-journal-rev", false);
  });

  it("never overwrites a journal that already has content", async () => {
    const existing: PageDto = {
      name: "Jul 10th, 2026",
      kind: "journal",
      title: "Jul 10th, 2026",
      pre_block: null,
      blocks: [{ id: "existing", raw: "user content", collapsed: false, children: [] }],
      rev: "existing-rev",
    };
    const { loadGraphPath, ensureJournalTemplateForDay, api } = await loadHarness(existing);

    await loadGraphPath(META.root);
    await ensureJournalTemplateForDay(new Date());

    expect(api.listTemplates).not.toHaveBeenCalled();
    expect(api.savePage).not.toHaveBeenCalled();
  });

  it("drops template work that becomes stale across an in-place graph switch", async () => {
    const harness = await loadHarness(null);
    await harness.loadGraphPath(META.root);
    harness.api.getPage.mockClear();
    harness.api.listTemplates.mockClear();
    harness.api.savePage.mockClear();

    let releaseTemplates!: (templates: Awaited<ReturnType<typeof harness.api.listTemplates>>) => void;
    harness.api.listTemplates.mockImplementationOnce(() => new Promise((resolve) => {
      releaseTemplates = resolve;
    }));
    const pending = harness.ensureJournalTemplateForDay(new Date());
    await vi.waitFor(() => expect(harness.api.listTemplates).toHaveBeenCalledTimes(1));

    harness.setMeta({ ...META, root: "/tmp/rebound-graph" });
    harness.bumpEpoch();
    releaseTemplates([{
      name: "Daily",
      page: "Templates",
      kind: "page",
      blocks: [{ id: "template", raw: "must stay out", collapsed: false, children: [] }],
    }]);

    await expect(pending).resolves.toBe("stale");
    expect(harness.api.savePage).not.toHaveBeenCalled();
  });

  it("does no journal I/O when the loaded graph has no configured template", async () => {
    const harness = await loadHarness(null);
    await harness.loadGraphPath(META.root);
    harness.api.getPage.mockClear();
    harness.api.listTemplates.mockClear();
    harness.api.savePage.mockClear();
    harness.setMeta({ ...META, default_journal_template: null });

    await expect(harness.ensureJournalTemplateForDay(new Date())).resolves.toBe("ready");

    expect(harness.api.getPage).not.toHaveBeenCalled();
    expect(harness.api.listTemplates).not.toHaveBeenCalled();
    expect(harness.api.savePage).not.toHaveBeenCalled();
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
      return {
        kind: "loaded" as const,
        meta: nextMeta,
        binding_generation: 2,
        application_page_admission: { binding_generation: 2, authority: "direct" as const },
      };
    });

    await harness.loadGraphPath(nextMeta.root);

    expect(harness.events).toEqual(expect.arrayContaining([
      "drain-pdf", "retire-pdf", "load-next",
    ]));
    expect(harness.events.indexOf("drain-pdf")).toBeLessThan(harness.events.indexOf("retire-pdf"));
    expect(harness.events.indexOf("retire-pdf")).toBeLessThan(harness.events.indexOf("load-next"));
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
    expect(harness.suspendPdfForGraphTransition).not.toHaveBeenCalled();
    expect(harness.api.loadGraph).toHaveBeenCalledOnce();
  });

  it("publishes a fresh PDF generation for a same-root force refresh", async () => {
    const harness = await loadHarness(null);
    await harness.loadGraphPath(META.root);
    harness.events.length = 0;
    (harness.api.loadGraph as any).mockImplementationOnce(async () => {
      harness.events.push("load-refresh");
      return {
        kind: "already_current" as const,
        meta: META,
        binding_generation: 1,
        application_page_admission: DIRECT_ADMISSION,
      };
    });

    await harness.loadGraphPath(META.root, { forceRefresh: true });

    expect(harness.events.slice(0, 5)).toEqual([
      "drain-pdf",
      "retire-pdf",
      "load-refresh",
      `activate-pdf:${META.root}`,
      "bump-epoch",
    ]);
    expect(harness.activatePdfOwnership).toHaveBeenCalledTimes(2);
    expect(harness.restorePdfSessionTarget).not.toHaveBeenCalled();
  });

  it("restores fresh old-graph ownership while route state remains installed when rebind fails", async () => {
    const harness = await loadHarness(null);
    await harness.loadGraphPath(META.root);
    harness.events.length = 0;
    harness.api.loadGraph.mockRejectedValueOnce(new Error("rebind failed"));

    await expect(harness.loadGraphPath("/tmp/other-graph")).rejects.toThrow("rebind failed");

    expect(harness.events).toEqual([
      "drain-pdf",
      "retire-pdf",
      `activate-pdf:${META.root}`,
    ]);
    expect(harness.restorePdfSessionTarget).not.toHaveBeenCalled();
  });
});

describe("graph home page (GH #245)", () => {
  const DIRECTORY: PageDto = { name: "Directory", kind: "page", title: "Directory", pre_block: null, format: "md", blocks: [] };

  it("opens the configured home page in place on an ordinary first load", async () => {
    const harness = await loadHarness(DIRECTORY);
    harness.api.getAppString.mockResolvedValue("Directory");

    await harness.loadGraphPath(META.root);

    const router = await import("./router");
    expect(router.openPage).toHaveBeenCalledWith("Directory", "page", { inPlace: true });
  });

  it("keeps the ordinary landing when the configured page no longer resolves — nothing is created", async () => {
    const harness = await loadHarness(null);
    harness.api.getAppString.mockResolvedValue("Ghost");

    await harness.loadGraphPath(META.root);

    const router = await import("./router");
    expect(router.openPage).not.toHaveBeenCalled();
  });

  it("does not navigate when no home page is configured", async () => {
    const harness = await loadHarness(DIRECTORY);

    await harness.loadGraphPath(META.root);

    const router = await import("./router");
    expect(router.openPage).not.toHaveBeenCalled();
  });

  it("does not home-navigate on a same-graph force refresh", async () => {
    const harness = await loadHarness(DIRECTORY);
    harness.api.getAppString.mockResolvedValue("Directory");
    await harness.loadGraphPath(META.root);
    const router = await import("./router");
    vi.mocked(router.openPage).mockClear();

    await harness.loadGraphPath(META.root, { forceRefresh: true });

    expect(router.openPage).not.toHaveBeenCalled();
    expect(harness.api.getAppString).toHaveBeenCalledTimes(1); // only the first load read it
  });
});

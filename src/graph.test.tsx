import { afterEach, describe, expect, it, vi } from "vitest";
import type { GraphFolderPickResult, PreparedGraphFolder } from "./backend";
import type { GraphMeta, PageDto, PageRead } from "./types";

const META: GraphMeta = {
  root: "/tmp/template-graph",
  journals_dir: "journals",
  pages_dir: "pages",
  preferred_workflow: "now",
  shortcuts: {},
  start_of_week: 6,
  block_hidden_properties: [], linked_references_collapsed_threshold: 100,
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
  guide_announced: true, mobile_gestures_disabled_in_block_with_tags: [],
};

async function loadHarness(
  existing: PageRead | null,
  access = { graph_root: META.root, external_assets_path: null as string | null, approved: true },
  confirm = true,
  warm = false,
  onEpoch?: () => void,
  journal?: { journalTitle: () => string; setJournalTitleFormat: (format: string | null | undefined) => void },
  mobile: { platform?: "android" | "ios" | "desktop"; pickerResult?: GraphFolderPickResult } = {},
) {
  const platform = mobile.platform ?? "desktop";
  vi.resetModules();
  const events: string[] = [];
  let meta: GraphMeta | null = null;
  const api = {
    inspectGraphAccess: vi.fn(async () => access),
    approveExternalAssets: vi.fn(async () => {}),
    confirm: vi.fn(async () => confirm),
    loadGraph: vi.fn(async () => ({ kind: "loaded" as const, meta: META, binding_generation: 1 })),
    pickFolder: vi.fn(async () => "/tmp"),
    pickGraphFolder: vi.fn(async (): Promise<GraphFolderPickResult> => mobile.pickerResult ?? { status: "cancelled" }),
    prepareGraphFolder: vi.fn(async (_path: string): Promise<PreparedGraphFolder> => ({ status: "ready", location: "local" })),
    defaultGraphParent: vi.fn(async () => "/tmp"),
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
    savePages: vi.fn(async (_entries: import("./backend").SavePageEntry[], _bindingGeneration?: number) => {
      events.push("save-template");
      return { ok: ["new-rev"] };
    }),
    readCustomCss: vi.fn(async () => ""),
    storeDraft: vi.fn(async (_record: import("./types").DraftRecord, _graphRoot?: string) => {}),
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
  const resetTabsToJournals = vi.fn(() => { events.push("reset-tabs"); });
  const flushAll = vi.fn(async () => true);
  const unsaved: { name: string; state: string; path: string | null; page: PageDto | null }[] = [];
  const resetStore = vi.fn(() => { unsaved.length = 0; });

  vi.doMock("./backend", () => ({ backend: () => api }));
  vi.doMock("./ui", () => ({
    setGraphMeta: (next: GraphMeta | null) => { meta = next; },
    graphMeta: () => meta,
    graphEpoch: () => 0,
    bumpDataRev: vi.fn(),
    bumpGraphEpoch: () => { events.push("bump-epoch"); onEpoch?.(); },
    setWorkflow: vi.fn(),
    setRightSidebar: vi.fn(),
    seedFavorites: vi.fn(),
    favorites: () => [],
    renamePageInNavigation: vi.fn(),
    pruneSidebarBlocks: vi.fn(),
    pushToast: vi.fn(),
    refreshJournalConflicts: vi.fn(async () => {}),
    refreshSyncConflicts: vi.fn(async () => {}),
    clearRecent: vi.fn(),
    resetLeftSidebarSections: vi.fn(),
    graphTransitioning: () => false,
    setGraphTransitioning: vi.fn(),
    closePageProps: vi.fn(),
    setAudioPlayer: vi.fn(),
  }));
  vi.doMock("./graphSession", () => ({
    setGraphMeta: (next: GraphMeta | null) => { meta = next; },
    graphMeta: () => meta,
    graphEpoch: () => 0,
    bumpDataRev: vi.fn(),
    bumpGraphEpoch: () => { events.push("bump-epoch"); onEpoch?.(); },
  }));
  vi.doMock("./pdfOwnership", () => ({
    drainPdfWork,
    retirePdfOwnership,
    activatePdfOwnership,
  }));
  vi.doMock("./document", () => ({
    resetStore, flushAll, unsavedDrafts: () => unsaved, installDraftKeeper: vi.fn(),
    installRenameRefreshHandler: vi.fn(),
    favoritesArrangementPage: vi.fn(), favoritesArrangementBlocks: vi.fn(),
    reloadHlsIfLoaded: vi.fn(),
    createPage: (_name: string, dto: PageDto, options: { id: string; baseRev: string | null; bindingGeneration: number }) =>
      api.savePages([{ id: options.id, page: dto, baseRev: options.baseRev, force: false,
        kinds: [options.baseRev === null ? "create-page" : "replace-page"] }], options.bindingGeneration).then((result) => result.ok[0]),
    journalTemplatePage: (title: string, blocks: unknown[], page?: PageRead | null) => ({
      name: title, kind: "journal", title, pre_block: page?.pre_block ?? null, blocks, format: page?.format,
    }),
    demoJournalPage: (title: string) => ({ name: title, kind: "journal", title, pre_block: null, blocks: [{
      id: "", raw: "👋 This is **today's journal** — your daily notes land here. Try your quick-capture hotkey, or open [[Welcome to Tine]] for the tour.",
      collapsed: false, children: [],
    }] }),
  }));
  vi.doMock("./assetCache", () => ({ clearAssetBlobCache: vi.fn() }));
  vi.doMock("./router", () => ({
    resetTabsToJournals,
    openPage,
    openJournals: vi.fn(),
    route: () => ({ kind: "journals" }),
    sameRoute: (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b),
    restoreSession: vi.fn(async () => {}),
    flushSession: vi.fn(async () => {}),
  }));
  const focused = { activeId: () => "tab", routeIntentRevision: () => 0, route: () => ({ kind: "journals" }), openPage };
  vi.doMock("./panes", () => ({ resetPaneLayoutToSingle: vi.fn(), focusedRouter: () => focused }));
  vi.doMock("./journal", () => ({
    journalTitle: journal?.journalTitle ?? (() => "Jul 10th, 2026"),
    appNow: () => new Date(),
    localDayKey: (date = new Date()) => date.getFullYear() * 10_000 + (date.getMonth() + 1) * 100 + date.getDate(),
    setJournalTitleFormat: journal?.setJournalTitleFormat ?? vi.fn(),
    isJournalTitle: () => false,
  }));
  vi.doMock("./editor/templateVars", () => ({ applyTemplateVars, prepareTemplateVars }));
  vi.doMock("./warmCache", () => ({ waitForWarmCache }));
  vi.doMock("./pageIndex", () => ({ resetPageIndex }));
  vi.doMock("./lsShim", () => ({ CUSTOM_CSS_STYLE_ID: "test-css", ensureLsShimStyle: vi.fn() }));
  vi.doMock("./themeGallery", () => ({ ensureThemeStyle: vi.fn() }));
  vi.doMock("./platform", () => ({ isMobile: () => platform !== "desktop", platformKind: vi.fn(async () => platform) }));
  vi.doMock("./guide", () => ({ maybeShowGuideAnnouncement: vi.fn() }));
  vi.doMock("./workspaces", () => ({ clearWorkspaces: vi.fn() }));
  vi.doMock("./editorController", () => ({ endEdit: vi.fn() }));
  const closeAllWorkspaceWindows = vi.fn((_reason: string) => { events.push("close-windows"); });
  vi.doMock("./workspaceWindows", () => ({ closeAllWorkspaceWindows }));

  const { loadGraphPath, switchGraph, createNewGraph, refreshAfterRename, ensureJournalTemplateForDay, applyGraphConfigChange } = await import("./graph");
  return {
    loadGraphPath, switchGraph, createNewGraph, refreshAfterRename, ensureJournalTemplateForDay, applyGraphConfigChange, api, events, resetPageIndex, resetAt, waitForWarmCache,
    drainPdfWork, retirePdfOwnership, activatePdfOwnership, resetTabsToJournals, flushAll, resetStore, unsaved,
    applyTemplateVars, prepareTemplateVars, openPage, closeAllWorkspaceWindows,
  };
}

afterEach(() => {
  document.body.innerHTML = "";
  document.head.querySelector("#test-css")?.remove();
  localStorage.clear();
  vi.restoreAllMocks();
  vi.resetModules();
});

// Ported from master graph.test.tsx "mobile graph folder picker" (027b5ae15,
// fa78c7b74): iOS opens and creates graphs only inside the app's own
// On My iPhone / iCloud Drive containers, prepared natively first.
describe("mobile graph folder picker", () => {
  const OUTSIDE = "Choose a folder inside On My iPhone or iCloud Drive → TineOutline. Other Files providers aren't supported yet.";

  it("opens a picked graph from Tine's iOS Documents container", async () => {
    const harness = await loadHarness(null, undefined, true, true, undefined, undefined,
      { platform: "ios", pickerResult: { status: "picked", path: META.root } });

    await expect(harness.switchGraph()).resolves.toEqual({ kind: "loaded", root: META.root });
    expect(harness.api.pickGraphFolder).toHaveBeenCalledOnce();
    expect(harness.api.prepareGraphFolder).toHaveBeenCalledWith(META.root);
    expect(harness.api.loadGraph).toHaveBeenCalledWith(META.root);
  });

  it("opens and remembers the rebased iOS container path after an app update", async () => {
    const stale = "/var/mobile/Containers/Data/Application/OLD/Documents/template-graph";
    const harness = await loadHarness(null, undefined, true, true, undefined, undefined, { platform: "ios" });
    harness.api.prepareGraphFolder.mockResolvedValue({ status: "ready", location: "local", path: META.root });

    await expect(harness.loadGraphPath(stale)).resolves.toEqual({ kind: "loaded", root: META.root });
    expect(harness.api.prepareGraphFolder).toHaveBeenCalledWith(stale);
    expect(harness.api.inspectGraphAccess).toHaveBeenCalledWith(META.root);
    expect(harness.api.loadGraph).toHaveBeenCalledWith(META.root);
    expect(localStorage.getItem("tine.graphPath")).toBe(META.root);
  });

  it("shows a clear refusal when iOS returns an outside-container folder", async () => {
    const harness = await loadHarness(null, undefined, true, false, undefined, undefined,
      { platform: "ios", pickerResult: { status: "refused" } });

    await expect(harness.switchGraph()).resolves.toEqual({ kind: "aborted" });
    expect(harness.api.loadGraph).not.toHaveBeenCalled();
    const { toasts } = await import("./toasts");
    expect(toasts().at(-1)).toMatchObject({ kind: "info", message: OUTSIDE });
  });

  it("prepares the chosen iCloud location before creating an iOS graph", async () => {
    const harness = await loadHarness(null, undefined, true, true, undefined, undefined,
      { platform: "ios", pickerResult: { status: "picked", path: META.root } });

    await expect(harness.createNewGraph()).resolves.toEqual({ kind: "loaded", root: META.root });
    expect(harness.api.pickGraphFolder).toHaveBeenCalledOnce();
    expect(harness.api.defaultGraphParent).not.toHaveBeenCalled();
    expect(harness.api.prepareGraphFolder).toHaveBeenNthCalledWith(1, META.root);
    expect(harness.api.createGraph).toHaveBeenCalledWith(META.root);
  });

  it("refuses an iOS graph before native graph inspection when its container is outside scope", async () => {
    const harness = await loadHarness(null, undefined, true, false, undefined, undefined,
      { platform: "ios", pickerResult: { status: "picked", path: META.root } });
    harness.api.prepareGraphFolder.mockResolvedValue({ status: "refused" });

    await expect(harness.switchGraph()).resolves.toEqual({ kind: "aborted" });
    expect(harness.api.inspectGraphAccess).not.toHaveBeenCalled();
    expect(harness.api.loadGraph).not.toHaveBeenCalled();
  });

  it("never asks the iOS boundary to prepare a graph on Android", async () => {
    const harness = await loadHarness(null, undefined, true, true, undefined, undefined,
      { platform: "android", pickerResult: { status: "picked", path: META.root } });

    await expect(harness.switchGraph()).resolves.toEqual({ kind: "loaded", root: META.root });
    expect(harness.api.prepareGraphFolder).not.toHaveBeenCalled();
  });
});

describe("default journal template graph bind", () => {
  // Master 5bb8ce020 (GH #266): graph open does not await the optional
  // default-journal template (a getPage + listTemplates that can wait for the
  // whole-graph parse). The visible Journals surface owns materialization and
  // awaits it before its first feed read (Page.tsx), preserving #73.
  it("opens the graph without awaiting the default-journal template's page read", async () => {
    const { loadGraphPath, api } = await loadHarness(null);
    api.getPage.mockImplementation(() => new Promise(() => {}));
    const outcome = await Promise.race([
      loadGraphPath(META.root),
      new Promise<"blocked">((resolve) => setTimeout(() => resolve("blocked"), 200)),
    ]);
    expect(outcome).toMatchObject({ kind: "loaded" });
    expect(api.getPage).not.toHaveBeenCalled();
    expect(api.savePages).not.toHaveBeenCalled();
  });

  it("shares one template write across simultaneous feed refreshes", async () => {
    const { loadGraphPath, ensureJournalTemplateForDay, api } = await loadHarness(null);
    await loadGraphPath(META.root);
    api.savePages.mockClear();
    let finishRead!: (value: PageRead | null) => void;
    api.getPage.mockImplementation(() => new Promise((resolve) => { finishRead = resolve; }));
    const first = ensureJournalTemplateForDay(new Date());
    const second = ensureJournalTemplateForDay(new Date());
    finishRead(null);
    expect(await Promise.all([first, second])).toEqual(["ready", "ready"]);
    expect(api.getPage).toHaveBeenCalledTimes(1); // graph bind reads none; one shared refresh
    expect(api.savePages).toHaveBeenCalledTimes(1);
  });
  // OG-C5 L12-S1: opening Journals must never replace text the user wrote with the
  // template. `memo:: keep` is ordinary prose in an Org heading, and a root `id::`
  // is a block identity other pages may reference; neither is an empty journal.
  for (const [name, format, raw] of [
    ["Org heading prose that looks like a Markdown property", "org", "memo:: keep this sentence"],
    ["a Markdown root carrying only a block id", "md", "id:: 6679f1c2-0000-4000-8000-000000000001"],
    ["a property-shaped line nested under an empty root", "md", ""],
  ] as const) {
    it(`keeps today's journal when it holds ${name}`, async () => {
      const children = raw === "" ? [{ id: "c", raw: "note:: keep me", collapsed: false, children: [] }] : [];
      const existing = {
        id: `journals/2026_07_10.${format === "org" ? "org" : "md"}`, rev: "rev-1", name: "Jul 10th, 2026", kind: "journal",
        title: "Jul 10th, 2026", pre_block: null, format,
        blocks: [{ id: "b", raw, collapsed: false, children }],
      } as unknown as PageRead;
      const { loadGraphPath, ensureJournalTemplateForDay, api } = await loadHarness(existing);
      await loadGraphPath(META.root);
      api.savePages.mockClear();
      expect(await ensureJournalTemplateForDay(new Date())).toBe("ready");
      expect(api.savePages).not.toHaveBeenCalled();
    });
  }
  it("still fills an existing journal whose blocks hold only whitespace", async () => {
    const existing = {
      id: "journals/2026_07_10.md", rev: "rev-1", name: "Jul 10th, 2026", kind: "journal",
      title: "Jul 10th, 2026", pre_block: "title:: Jul 10th, 2026", format: "md",
      blocks: [{ id: "b", raw: "  \n", collapsed: false, children: [{ id: "c", raw: "", collapsed: false, children: [] }] }],
    } as unknown as PageRead;
    const { loadGraphPath, ensureJournalTemplateForDay, api } = await loadHarness(existing);
    await loadGraphPath(META.root);
    api.savePages.mockClear();
    expect(await ensureJournalTemplateForDay(new Date())).toBe("ready");
    expect(api.savePages).toHaveBeenCalledTimes(1);
    const [[entry]] = api.savePages.mock.calls[0];
    expect(entry).toMatchObject({ baseRev: "rev-1", page: { pre_block: "title:: Jul 10th, 2026" } });
  });
  it("returns a typed template read failure for the feed to surface and retry", async () => {
    const { loadGraphPath, ensureJournalTemplateForDay, api } = await loadHarness(null);
    await loadGraphPath(META.root);
    api.getPage.mockRejectedValueOnce(new Error("template read denied"));
    const result = await ensureJournalTemplateForDay(new Date());
    expect(result).toMatchObject({ kind: "error", error: expect.any(Error) });
    if (typeof result !== "string") expect(String(result.error)).toContain("template read denied");
  });
  it("keeps an edit typed while the next graph loads in the old graph's draft store (og T4)", async () => {
    const harness = await loadHarness(null);
    await harness.loadGraphPath(META.root);
    const page = { name: "Typed", kind: "page", title: "Typed", pre_block: null,
      blocks: [{ id: "b", raw: "typed during the load", collapsed: false, children: [] }] } as PageDto;
    harness.api.loadGraph.mockImplementationOnce(async () => {
      // The last flush already ran; this edit lands while load_graph is in flight.
      harness.unsaved.push({ name: "Typed", state: "Not saved", path: "pages/Typed.md", page });
      return { kind: "loaded" as const, meta: { ...META, root: "/tmp/next-graph" }, binding_generation: 2 };
    });
    await harness.loadGraphPath("/tmp/next-graph");
    expect(harness.api.storeDraft).toHaveBeenCalledTimes(1);
    const [record, root] = harness.api.storeDraft.mock.calls[0];
    expect(root).toBe(META.root);
    expect(record).toMatchObject({ kind: "unsaved", page_name: "Typed", path: "pages/Typed.md", page });
    expect(harness.resetStore).toHaveBeenCalled(); // the reset drops the working set (harness)
  });
  it("MX: the switch waits until the old graph's late edit is durably drafted", async () => {
    const harness = await loadHarness(null);
    await harness.loadGraphPath(META.root);
    const page = { name: "Typed", kind: "page", title: "Typed", pre_block: null,
      blocks: [{ id: "b", raw: "typed during the load", collapsed: false, children: [] }] } as PageDto;
    let finish!: () => void;
    harness.api.storeDraft.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
    harness.api.loadGraph.mockImplementationOnce(async () => {
      harness.unsaved.push({ name: "Typed", state: "Not saved", path: "pages/Typed.md", page });
      return { kind: "loaded" as const, meta: { ...META, root: "/tmp/next-graph" }, binding_generation: 2 };
    });
    let done = false;
    const switching = harness.loadGraphPath("/tmp/next-graph").then((outcome) => { done = true; return outcome; });
    for (let i = 0; i < 20; i++) await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(harness.api.storeDraft).toHaveBeenCalledTimes(1);
    expect(done, "the switch must not finish while the only copy of the edit is not durable").toBe(false);
    expect(harness.resetTabsToJournals).not.toHaveBeenCalled();
    finish();
    await expect(switching).resolves.toMatchObject({ kind: "loaded", root: "/tmp/next-graph" });
    expect(harness.resetTabsToJournals).toHaveBeenCalled();
  });
  it("MX: a late edit whose draft the store refuses stays in this window, and a sticky error says where", async () => {
    const harness = await loadHarness(null);
    await harness.loadGraphPath(META.root);
    const page = { name: "Typed", kind: "page", title: "Typed", pre_block: null,
      blocks: [{ id: "b", raw: "typed during the load", collapsed: false, children: [] }] } as PageDto;
    harness.api.storeDraft.mockRejectedValueOnce(new Error("the draft store keeps at most 64 pages"));
    harness.api.loadGraph.mockImplementationOnce(async () => {
      harness.unsaved.push({ name: "Typed", state: "Not saved", path: "pages/Typed.md", page });
      return { kind: "loaded" as const, meta: { ...META, root: "/tmp/next-graph" }, binding_generation: 2 };
    });
    await harness.loadGraphPath("/tmp/next-graph");
    const draftStore = await import("./draftStore");
    expect(draftStore.switchHeldDrafts?.().map((held) => [held.root, held.record.page_name, held.record.page.blocks[0].raw]))
      .toEqual([[META.root, "Typed", "typed during the load"]]);
    const { toasts } = await import("./toasts");
    const error = toasts().find((toast) => toast.kind === "error" && toast.message.includes("Typed"));
    expect(error).toMatchObject({ sticky: true, action: { label: "Review unsaved" } });
    // Dismissing is the user's explicit release.
    draftStore.dismissHeldDraft(draftStore.switchHeldDrafts()[0].record.id);
    expect(draftStore.switchHeldDrafts()).toEqual([]);
  });
  it("still switches graph when the current session cannot be saved", async () => {
    const { loadGraphPath, api } = await loadHarness(null);
    await loadGraphPath(META.root);
    const { flushSession } = await import("./router");
    vi.mocked(flushSession).mockRejectedValueOnce(new Error("session disk full"));
    expect(await loadGraphPath("/tmp/another-graph")).not.toEqual({ kind: "aborted" });
    expect(api.loadGraph).toHaveBeenCalledTimes(2);
  });
  it("releases a graph transition invalidated during blur", async () => {
    const { loadGraphPath, api } = await loadHarness(null);
    const { invalidateBinding } = await import("./binding");
    const { setGraphTransitioning } = await import("./ui");
    const pending = loadGraphPath(META.root);
    invalidateBinding();
    expect(await pending).toEqual({ kind: "aborted" });
    expect(setGraphTransitioning).toHaveBeenLastCalledWith(false);
    expect(api.loadGraph).not.toHaveBeenCalled();
  });
  it("does not open an old folder-picker choice after a newer graph binding", async () => {
    const { switchGraph, api } = await loadHarness(null);
    let finish!: (path: string) => void;
    api.pickFolder.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const pending = switchGraph();
    await vi.waitFor(() => expect(api.pickFolder).toHaveBeenCalled());
    const { invalidateBinding } = await import("./binding");
    invalidateBinding();
    finish("/tmp/old-choice");
    await pending;
    expect(api.loadGraph).not.toHaveBeenCalled();
  });

  it("clears the previous graph's expanded audio player on rebind", async () => {
    const { loadGraphPath } = await loadHarness(null);
    const { setAudioPlayer } = await import("./ui");
    await loadGraphPath(META.root);
    expect(setAudioPlayer).toHaveBeenCalledWith(null);
  });

  it("does not inject CSS from a graph whose read completes after rebinding", async () => {
    const { loadGraphPath, api } = await loadHarness(null);
    let finish!: (css: string) => void;
    api.readCustomCss.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    await loadGraphPath(META.root);
    await vi.waitFor(() => expect(api.readCustomCss).toHaveBeenCalledOnce());
    const { invalidateBinding } = await import("./binding");
    invalidateBinding();
    finish("body { color: red; }");
    await Promise.resolve();
    await Promise.resolve();
    expect(document.head.querySelector("#test-css")?.textContent ?? "").not.toContain("red");
  });

  it("drops template insertion when its page read lands after a graph switch (I-20)", async () => {
    const { loadGraphPath, ensureJournalTemplateForDay, api } = await loadHarness(null);
    let finish!: (page: PageRead | null) => void;
    api.getPage.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    await loadGraphPath(META.root);
    const materializing = ensureJournalTemplateForDay(new Date());
    await vi.waitFor(() => expect(api.getPage).toHaveBeenCalled());
    const { invalidateBinding } = await import("./binding");
    invalidateBinding();
    finish(null);
    await materializing;
    expect(api.savePages).not.toHaveBeenCalled();
  });

  it("drops demo seed and Welcome navigation when its page read lands after a graph switch (I-20)", async () => {
    const { createNewGraph, api, openPage } = await loadHarness(null);
    let finish!: (page: PageRead | null) => void;
    // Graph open no longer reads today's journal (master 5bb8ce020), so the
    // demo seed's page read is the first one.
    api.getPage.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const creating = createNewGraph();
    await vi.waitFor(() => expect(api.getPage).toHaveBeenCalledTimes(1));
    const before = api.savePages.mock.calls.length;
    const { invalidateBinding } = await import("./binding");
    invalidateBinding();
    finish(null);
    await creating;
    expect(api.savePages).toHaveBeenCalledTimes(before);
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

  it("invalidates stale loads on bind; the visible-journal request materializes the template", async () => {
    const { loadGraphPath, ensureJournalTemplateForDay, events } = await loadHarness(null);

    await loadGraphPath(META.root);
    expect(events).toEqual([`activate-pdf:${META.root}`, "bump-epoch"]);
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
    // No journal file yet: the save goes to the backend's Absent id (B15b).
    expect(api.resolvePage).toHaveBeenCalledWith("Jul 10th, 2026", "journal");
    expect(api.savePages).toHaveBeenCalledWith(
      [expect.objectContaining({
        id: "journals/2026_07_10.md",
        page: expect.objectContaining({ blocks: [expect.objectContaining({ raw: "Template body" })] }),
        baseRev: null,
        force: false,
      })],
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
    const { loadGraphPath, ensureJournalTemplateForDay, api } = await loadHarness(existing);

    await loadGraphPath(META.root);
    await ensureJournalTemplateForDay(new Date());

    // The empty journal's own file (its id), with its rev as the baseline; no
    // name lookup.
    expect(api.savePages).toHaveBeenCalledWith([{ id: "journals/Jul 10th, 2026.org", page: expect.any(Object), baseRev: "empty-journal-rev", force: false, kinds: ["replace-page"] }], 0);
    expect(api.resolvePage).not.toHaveBeenCalled();
  });

  it("keeps a journal whose only real content is in a descendant block", async () => {
    const existing: PageRead = {
      name: "Jul 10th, 2026", kind: "journal", title: "Jul 10th, 2026", pre_block: null,
      blocks: [{ id: "parent", raw: "", collapsed: false,
        children: [{ id: "child", raw: "A real note", collapsed: false, children: [] }] }],
      rev: "existing-rev", id: "journals/2026_07_10.md",
    };
    const { loadGraphPath, ensureJournalTemplateForDay, api } = await loadHarness(existing);
    await loadGraphPath(META.root);
    await ensureJournalTemplateForDay(new Date());
    expect(api.savePages).not.toHaveBeenCalled();
  });

  it("refuses to write a template journal onto an alias name (B15b)", async () => {
    const { loadGraphPath, ensureJournalTemplateForDay, api } = await loadHarness(null);
    api.resolvePage.mockResolvedValue({ kind: "alias", owners: ["pages/Owner.md"] } as never);

    await loadGraphPath(META.root);
    await ensureJournalTemplateForDay(new Date());

    expect(api.resolvePage).toHaveBeenCalledWith("Jul 10th, 2026", "journal");
    expect(api.savePages).not.toHaveBeenCalled();
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
      "drain-pdf", "retire-pdf", "load-next", "reset-tabs",
    ]));
    expect(harness.events.indexOf("drain-pdf")).toBeLessThan(harness.events.indexOf("retire-pdf"));
    expect(harness.events.indexOf("retire-pdf")).toBeLessThan(harness.events.indexOf("load-next"));
    expect(harness.events.indexOf("load-next")).toBeLessThan(harness.events.indexOf("reset-tabs"));
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
    expect(harness.resetTabsToJournals).not.toHaveBeenCalled();
    expect(harness.api.loadGraph).toHaveBeenCalledOnce();
  });

  it("keeps the workspace windows when the switch aborts at the PDF drain or the access prompt (review F11)", async () => {
    const harness = await loadHarness(null);
    await harness.loadGraphPath(META.root);
    harness.drainPdfWork.mockResolvedValueOnce(false);
    await expect(harness.loadGraphPath("/tmp/other-graph")).resolves.toEqual({ kind: "aborted" });
    harness.api.inspectGraphAccess.mockResolvedValueOnce({ graph_root: "/tmp/other-graph", external_assets_path: "/mnt/x", approved: false });
    harness.api.confirm.mockResolvedValueOnce(false);
    await expect(harness.loadGraphPath("/tmp/other-graph")).resolves.toEqual({ kind: "aborted" });
    expect(harness.closeAllWorkspaceWindows).not.toHaveBeenCalled();
    // A switch that goes through closes them, after the drain and before binding.
    harness.events.length = 0;
    harness.api.loadGraph.mockImplementationOnce(async () => {
      harness.events.push("load-next");
      return { kind: "loaded" as const, meta: { ...META, root: "/tmp/other-graph" }, binding_generation: 2 };
    });
    await harness.loadGraphPath("/tmp/other-graph");
    expect(harness.closeAllWorkspaceWindows).toHaveBeenCalledWith("graph-switch");
    expect(harness.events.indexOf("drain-pdf")).toBeLessThan(harness.events.indexOf("close-windows"));
    expect(harness.events.indexOf("close-windows")).toBeLessThan(harness.events.indexOf("load-next"));
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

    expect(harness.events.slice(0, 4)).toEqual([
      "drain-pdf",
      "retire-pdf",
      "load-refresh",
      `activate-pdf:${META.root}`,
    ]);
    expect(harness.resetTabsToJournals).not.toHaveBeenCalled();
    expect(harness.activatePdfOwnership).toHaveBeenCalledTimes(2);
  });
});

describe("an edit typed while a graph switch is in flight (og A, 20B open item 5)", () => {
  // The first flush runs before the session save, the access prompt and the
  // PDF drain; an edit that lands during those awaits used to reach resetStore
  // unsaved and was discarded with the old graph's working set.
  it("flushes again as the last step before binding another graph", async () => {
    const harness = await loadHarness(null);
    await harness.loadGraphPath(META.root);
    harness.flushAll.mockClear();
    harness.drainPdfWork.mockClear();
    harness.api.loadGraph.mockClear();
    const next = { ...META, root: "/tmp/other-graph" };
    harness.api.loadGraph.mockResolvedValueOnce({ kind: "loaded" as const, meta: next, binding_generation: 2 });
    await harness.loadGraphPath(next.root);
    const flushes = harness.flushAll.mock.invocationCallOrder;
    expect(flushes.length).toBe(2);
    expect(flushes[1]).toBeGreaterThan(harness.drainPdfWork.mock.invocationCallOrder[0]);
    expect(flushes[1]).toBeLessThan(harness.api.loadGraph.mock.invocationCallOrder[0]);
  });

  it("keeps the old graph (and its PDF owner) when that late edit cannot be saved", async () => {
    const harness = await loadHarness(null);
    await harness.loadGraphPath(META.root);
    harness.events.length = 0;
    harness.flushAll.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
    await expect(harness.loadGraphPath("/tmp/other-graph")).resolves.toEqual({ kind: "aborted" });
    expect(harness.api.loadGraph).toHaveBeenCalledOnce();
    expect(harness.resetTabsToJournals).not.toHaveBeenCalled();
    expect(harness.activatePdfOwnership).toHaveBeenLastCalledWith(META.root);
    const { toasts } = await import("./toasts");
    expect(toasts().at(-1)).toMatchObject({ kind: "error", message: expect.stringContaining("couldn't be saved") });
  });
});

describe("graph home page on open (config.edn :default-home)", () => {
  const DIRECTORY: PageRead = { id: "pages/Directory.md", name: "Directory", kind: "page", title: "Directory", pre_block: null, blocks: [], read_only: false, guide: false };
  const withHome = (home: string | null, root = META.root): GraphMeta => ({ ...META, root, default_home: home });
  const settle = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };

  async function harnessWithHome(home: string | null, page: PageRead | null) {
    const harness = await loadHarness(null);
    harness.api.loadGraph.mockImplementation(async (path?: string) =>
      ({ kind: "loaded" as const, meta: withHome(home, path ?? META.root), binding_generation: 1 }) as never);
    const getPage = harness.api.getPage as unknown as { mockImplementation(fn: (name: string) => Promise<PageRead | null>): void };
    getPage.mockImplementation(async (name) => (name === home ? page : null));
    return harness;
  }

  it("opens the configured home page in place on an ordinary first load", async () => {
    const harness = await harnessWithHome("Directory", DIRECTORY);
    await harness.loadGraphPath(META.root);
    await settle();
    expect(harness.api.getPage).toHaveBeenCalledWith("Directory", "page");
    expect(harness.openPage).toHaveBeenCalledWith("Directory", "page", { inPlace: true });
  });

  it("opens it on a graph switch too", async () => {
    const harness = await harnessWithHome("Directory", DIRECTORY);
    await harness.loadGraphPath(META.root);
    await settle();
    harness.openPage.mockClear();
    await harness.loadGraphPath("/tmp/other-graph");
    await settle();
    expect(harness.openPage).toHaveBeenCalledWith("Directory", "page", { inPlace: true });
  });

  it("keeps the ordinary landing when none is configured or the page no longer resolves", async () => {
    const none = await harnessWithHome(null, DIRECTORY);
    await none.loadGraphPath(META.root);
    await settle();
    expect(none.openPage).not.toHaveBeenCalled();

    const ghost = await harnessWithHome("Ghost", null);
    await ghost.loadGraphPath(META.root);
    await settle();
    expect(ghost.api.getPage).toHaveBeenCalledWith("Ghost", "page");
    expect(ghost.openPage).not.toHaveBeenCalled();
    expect(ghost.api.savePages.mock.calls.flatMap(([entries]) => entries.map((entry) => entry.page.name)))
      .not.toContain("Ghost"); // nothing is created for a missing home page
  });

  it("does not home-navigate on a same-graph force refresh", async () => {
    const harness = await harnessWithHome("Directory", DIRECTORY);
    await harness.loadGraphPath(META.root);
    await settle();
    harness.openPage.mockClear();
    await harness.loadGraphPath(META.root, { forceRefresh: true });
    await settle();
    expect(harness.openPage).not.toHaveBeenCalled();
  });
});


it("reports unreadable custom CSS once at graph open while applying no CSS", async () => {
  const { loadGraphPath, api } = await loadHarness(null);
  const failure = await import("./uiFailure");
  const report = vi.spyOn(failure, "reportUiFailure");
  api.readCustomCss.mockRejectedValue(new Error("CSS read denied"));
  expect(await loadGraphPath(META.root)).toMatchObject({ kind: "loaded" });
  await vi.waitFor(() => expect(report).toHaveBeenCalledWith("custom-css", expect.any(Error)));
  expect(report.mock.calls.filter(([family]) => family === "custom-css")).toHaveLength(1);
  expect(document.head.querySelector("#test-css")?.textContent).toBe("");
});

it("opens with the config-read problem available to Settings and clears it on a repaired reopen", async () => {
  const { loadGraphPath, api } = await loadHarness(null);
  const problem = { kind: "config-read" as const, message: "config read denied" };
  api.loadGraph.mockResolvedValueOnce({ kind: "loaded", meta: META, binding_generation: 1, config_problem: problem } as Awaited<ReturnType<typeof api.loadGraph>>);
  const failure = await import("./uiFailure");
  const report = vi.spyOn(failure, "reportUiFailure");
  expect(await loadGraphPath(META.root)).toMatchObject({ kind: "loaded" });
  const { graphConfigProblem } = await import("./graph");
  expect(graphConfigProblem()).toEqual(problem);
  expect(report).toHaveBeenCalledWith("config-read", problem);
  expect(api.savePages).not.toHaveBeenCalled();
  expect(await loadGraphPath(META.root, { forceRefresh: true })).toMatchObject({ kind: "loaded" });
  expect(graphConfigProblem()).toBeNull();
});


describe("OG-R3B custom journal format publication (I-4)", () => {
  for (const publication of ["open", "live"] as const) {
    it(`publishes the custom title before ${publication} wakes journal materialization`, async () => {
      let format = "MMM do, yyyy";
      let observe = false;
      let materialization: Promise<unknown> | undefined;
      let requested = "";
      const customTitle = "2026-07-10";
      const journal = {
        journalTitle: () => format === "yyyy-MM-dd" ? customTitle : "Jul 10th, 2026",
        setJournalTitleFormat: (next: string | null | undefined) => { format = next ?? "MMM do, yyyy"; },
      };
      const harness = await loadHarness(null, undefined, true, false,
        () => { if (observe) materialization = harness.ensureJournalTemplateForDay(new Date()); }, journal);
      const customMeta = { ...META, journal_page_title_format: "yyyy-MM-dd" };
      const delivered: PageRead = { id: "journals/2026_07_10.md", name: customTitle,
        kind: "journal", title: customTitle, pre_block: null, rev: "synced-rev",
        blocks: [{ id: "synced", raw: "Synced custom-format journal", collapsed: false, children: [] }] };
      harness.api.getPage.mockImplementation(async (...args: unknown[]) => {
        requested = args[0] as string;
        return requested === customTitle ? delivered : null;
      });
      if (publication === "live") await harness.loadGraphPath(META.root);
      observe = true;
      if (publication === "open") {
        harness.api.loadGraph.mockResolvedValueOnce({ kind: "loaded", meta: customMeta, binding_generation: 1 });
        await harness.loadGraphPath(META.root);
      } else {
        const { captureBinding } = await import("./binding");
        harness.applyGraphConfigChange({ meta: customMeta, binding_generation: captureBinding().backendGeneration! });
      }
      await materialization;
      expect(requested, "I-4: journal template must read the delivered custom-format journal; exemplar src/graph.ts").toBe(customTitle);
      expect(harness.api.savePages).not.toHaveBeenCalled();
    });
  }
});

describe("graph open failure recovery (master 9a9122b1544d)", () => {
  it("keeps a picked graph's open failure sticky and retries the same path", async () => {
    const harness = await loadHarness(null);
    harness.api.pickFolder.mockResolvedValue(META.root);
    harness.api.loadGraph.mockRejectedValue(new Error("disk went away"));
    const toastModule = await import("./toasts");
    toastModule.setToasts([]);

    await expect(harness.switchGraph()).resolves.toEqual({ kind: "aborted" });
    const failures = toastModule.toasts().filter((toast) => toast.kind === "error");
    expect(failures).toHaveLength(1);
    expect(failures[0]!.message).toContain("disk went away");
    expect(failures[0]!.sticky).toBe(true);
    expect(failures[0]!.action?.label).toBe("Retry");

    failures[0]!.action!.run();
    await vi.waitFor(() => expect(harness.api.loadGraph).toHaveBeenCalledTimes(2));
    expect(harness.api.loadGraph).toHaveBeenLastCalledWith(META.root);
    // Retry reopens the same target; it never re-asks the picker.
    expect(harness.api.pickFolder).toHaveBeenCalledTimes(1);
  });
});

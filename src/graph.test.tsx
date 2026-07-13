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
  logbook_with_second_support: true,
  logbook_enabled_in_timestamped_blocks: false,
  logbook_enabled_in_all_blocks: false,
  guide_announced: true,
};

async function loadHarness(existing: PageDto | null) {
  vi.resetModules();
  const events: string[] = [];
  let meta: GraphMeta | null = null;
  const api = {
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
    pageAliases: vi.fn(async () => []),
  };

  vi.doMock("./backend", () => ({ backend: () => api }));
  vi.doMock("./ui", () => ({
    setGraphMeta: (next: GraphMeta | null) => { meta = next; },
    graphMeta: () => meta,
    graphEpoch: () => 0,
    bumpGraphEpoch: () => { events.push("bump-epoch"); },
    setWorkflow: vi.fn(),
    setRightSidebar: vi.fn(),
    setAliasMap: vi.fn(),
    seedFavorites: vi.fn(),
    pruneSidebarBlocks: vi.fn(),
    pushToast: vi.fn(),
    refreshJournalConflicts: vi.fn(async () => {}),
    refreshSyncConflicts: vi.fn(async () => {}),
    clearRecent: vi.fn(),
    resetLeftSidebarSections: vi.fn(),
    graphTransitioning: () => false,
    setGraphTransitioning: vi.fn(),
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
  vi.doMock("./editor/templateVars", () => ({ applyTemplateVars: (raw: string) => raw }));
  vi.doMock("./warmCache", () => ({ waitForWarmCache: vi.fn(async () => false) }));
  vi.doMock("./lsShim", () => ({ CUSTOM_CSS_STYLE_ID: "test-css", ensureLsShimStyle: vi.fn() }));
  vi.doMock("./themeGallery", () => ({ ensureThemeStyle: vi.fn() }));
  vi.doMock("./platform", () => ({ isMobile: () => false, platformKind: vi.fn(async () => "desktop") }));
  vi.doMock("./guide", () => ({ maybeShowGuideAnnouncement: vi.fn() }));
  vi.doMock("./editorController", () => ({ endEdit: vi.fn() }));

  const { loadGraphPath } = await import("./graph");
  return { loadGraphPath, api, events };
}

afterEach(() => {
  document.body.innerHTML = "";
  document.head.querySelector("#test-css")?.remove();
  localStorage.clear();
  vi.restoreAllMocks();
  vi.resetModules();
});

describe("default journal template graph bind", () => {
  it("invalidates stale loads before awaiting template work, then refreshes after save", async () => {
    const { loadGraphPath, events } = await loadHarness(null);

    await loadGraphPath(META.root);

    expect(events).toEqual(["bump-epoch", "save-template", "bump-epoch"]);
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

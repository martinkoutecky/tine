// Conceptual model conformance, step 2 batch 5 (reference counts are Linked
// References; reference-only pages are pages): the counts and the page set
// Tine SHOWS, run against the conceptual model.
//
// The Rust harness (`crates/tine-store/src/cm_conformance_tests.rs`, through
// `shared::frontend_case`) writes one case per fixture graph to
// `crates/tine-store/tests/fixtures/cm-conformance/frontend/`: the store's
// `page_inventory` entries, the `page_backlink_counts` answer, per model page
// its Linked References size in the model and the store's `get_backlinks`
// answer, and the model key of every inventory name. This test mounts the
// real views on those answers (mocked IPC only):
// - All Pages (`components/AllPages.tsx`, journals shown): which pages have a
//   row, and the Backlinks cell of each row;
// - the Linked References panel (`components/LinkedReferences.tsx`) of every
//   model page: the count in its header;
// - the page list behind namespaces and the sidebar (`pageIndex.ts`
//   `allPageNames`): which reference-only pages Tine lists.
// D-20261102-CM4: a page's reference count is the size of its Linked
// References (CM-COUNTS), and reference-only pages, those named only by a task
// marker, a priority or a property key included, are pages (CM-REFONLY).
//
// Differences are compared with the `frontendCounts` part of
// `expected-differences.json`; `CM_REFRESH=1` rewrites that part.
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "./backend";
import { bumpGraphEpoch } from "./graphSession";
import { allPageNames, refreshPageIndex, resetPageIndex } from "./pageIndex";
import { resetStore } from "./document";
import { initParser } from "./render/parse";
import { hidden_from_all_pages } from "./render/wasm/lsdoc_wasm";
import { setJournalFormats } from "./journal";
import { resetReferenceSectionState } from "./referenceSectionState";
import { AllPages, hiddenFromAllPages } from "./components/AllPages";
import { allPages } from "./pages";
import { backlinkCountsByRow, countKey } from "./allPagesList";
import { LinkedReferences } from "./components/LinkedReferences";
import type { PaneRouter } from "./router";
import type { PageInventoryEntry, RefGroup } from "./types";

vi.mock("./components/LiveRefGroup", () => ({ LiveRefGroup: () => <div class="test-ref-group" /> }));

interface ModelPage {
  kind: "page" | "journal";
  key: string;
  /** The name Tine's name-keyed reads take for this page. */
  name: string;
  /** The page's owner file; null for a reference-only page. */
  owner: string | null;
  /** The size of the page's Linked References in the model. */
  linkedRefs: number;
}
interface Case {
  journalTitleFormat: string | null;
  journalFileFormat: string | null;
  conflicts: string[];
  inventory: PageInventoryEntry[];
  /** `page_backlink_counts`: [owner files by path, reference-only pages by name]. */
  counts: [[string, number][], [string, number][]];
  pages: ModelPage[];
  backlinks: Record<string, RefGroup[]>;
  keyOf: Record<string, string>;
}

const FIXTURES = path.resolve(__dirname, "../crates/tine-store/tests/fixtures/cm-conformance");
const EXPECTED = path.join(FIXTURES, "expected-differences.json");

function cases(): [string, Case][] {
  const dir = path.join(FIXTURES, "frontend");
  return readdirSync(dir)
    .filter((f) => f.endsWith(".json"))
    .sort()
    .map((f) => [f.slice(0, -".json".length), JSON.parse(readFileSync(path.join(dir, f), "utf8")) as Case]);
}

/** Sorted keys, so the file matches what serde_json (BTreeMap) writes. */
function sorted(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(sorted);
  if (v && typeof v === "object") {
    return Object.fromEntries(Object.keys(v).sort().map((k) => [k, sorted((v as Record<string, unknown>)[k])]));
  }
  return v;
}

const settle = async () => { for (let i = 0; i < 6; i++) await new Promise((resolve) => setTimeout(resolve, 0)); };
const show = (p: ModelPage) => `("${p.kind}", "${p.key}")`;

/** All Pages with journals shown: per row, its file path (a reference-only
 *  row, which has none: `ref:` and its name's key) and Backlinks cell. */
async function allPagesRows(c: Case): Promise<{ rows: Map<string, string>; hidden: string[] }> {
  vi.spyOn(backend(), "pageModified").mockResolvedValue({ rev: "1", modified: [] });
  vi.spyOn(backend(), "pageBacklinkCounts").mockResolvedValue([...c.counts, "1"]);
  vi.spyOn(backend(), "pageIcons").mockResolvedValue({});
  const router = { openPageTarget: vi.fn(), openPageTargetInNewTab: vi.fn() } as unknown as PaneRouter;
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <AllPages router={router} />, host);
  try {
    await vi.waitFor(() => expect(host.querySelector(".all-pages-journals input")).not.toBeNull());
    host.querySelector<HTMLInputElement>(".all-pages-journals input")!.click();
    const rows = () => [...host.querySelectorAll<HTMLElement>(".all-pages-row")];
    // A file named like an OG built-in page has no row (OG `get-all-pages`).
    const files = c.inventory.filter((e) => e.target.kind === "existing" && !hiddenFromAllPages({ name: e.name, kind: "page", date_key: null, path: "" })).length;
    if (files > 0) {
      await vi.waitFor(() => {
        expect(rows().length).toBeGreaterThan(0);
        // Counts arrive by their own read; a dash is "not loaded yet".
        if (c.counts[0].length || c.counts[1].length) expect(rows().some((r) => r.querySelector(".all-pages-backlinks")!.textContent !== "—")).toBe(true);
      });
    }
    await settle();
    const shown = new Map(rows().map((r) => {
      const open = r.querySelector<HTMLElement>(".all-pages-open")!;
      const name = open.textContent ?? "";
      return [
        open.title || `ref:${c.keyOf[name] ?? name}`,
        r.querySelector(".all-pages-backlinks")!.textContent ?? "",
      ];
    }));
    // A file All Pages leaves out as an OG built-in page (batch 5) keeps its
    // count checked: the cell its row would show, by the row's own key
    // (`countKey`) over the view's own count map (`backlinkCountsByRow`).
    const counts = backlinkCountsByRow([...c.counts, "1"]);
    const hidden: string[] = [];
    for (const page of allPages() ?? []) {
      if (!hiddenFromAllPages(page) || shown.has(page.path)) continue;
      hidden.push(page.path);
      shown.set(page.path, String(counts.get(countKey(page)) ?? "—"));
    }
    return { rows: shown, hidden };
  } finally {
    dispose();
    host.remove();
  }
}

/** The count in the Linked References header of `name`. */
async function headerCount(name: string, groups: RefGroup[]): Promise<string> {
  const read = vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
  vi.spyOn(backend(), "getBacklinkFilterContext").mockResolvedValue({ entries: [] });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <LinkedReferences name={name} />, host);
  try {
    await vi.waitFor(() => expect(read, `getBacklinks(${JSON.stringify(name)})`).toHaveBeenCalled());
    await settle();
    // An empty answer shows no panel at all (OG: no Linked References
    // section), which the user reads as 0.
    if (!groups.some((g) => g.blocks.length > 0)) {
      return host.querySelector(".references-count")?.textContent ?? "0";
    }
    await vi.waitFor(() => expect(host.querySelector(".references-count")).not.toBeNull());
    return host.querySelector(".references-count")!.textContent ?? "";
  } finally {
    dispose();
    host.remove();
    read.mockRestore();
  }
}

async function runCase(c: Case): Promise<string[]> {
  resetPageIndex();
  setJournalFormats(c.journalTitleFormat, c.journalFileFormat);
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: c.inventory });
  bumpGraphEpoch();
  await refreshPageIndex();
  const diffs: string[] = [];
  const conflicts = new Set(c.conflicts);

  // All Pages: rows and their Backlinks cells.
  const { rows, hidden } = await allPagesRows(c);
  for (const file of hidden) diffs.push(`frontend all pages: file ${file} has no row (hidden as an OG built-in page, A11)`);
  const owners = new Set<string>();
  const header = new Map<string, string>();
  for (const p of c.pages) {
    if (p.name === "") {
      // A model page no name opens (the empty namespace segment of n2).
      diffs.push(`frontend linked refs header of ${show(p)}: the page has no name to open`);
      continue;
    }
    const groups = c.backlinks[p.name] ?? [];
    if (groups.some((g) => "harness" in g)) {
      diffs.push(`harness: ${JSON.stringify(groups)}`);
      continue;
    }
    header.set(p.name, await headerCount(p.name, groups));
  }
  for (const p of c.pages) {
    const shown = header.get(p.name);
    if (shown !== undefined && shown !== String(p.linkedRefs)) {
      diffs.push(`frontend linked refs header of ${show(p)} [[${p.name}]]: model ${p.linkedRefs}, header ${shown}`);
    }
    const row = p.owner ?? `ref:${p.key}`;
    owners.add(row);
    const cell = rows.get(row);
    if (cell === undefined) {
      diffs.push(p.owner === null
        ? `frontend all pages: model page ${show(p)} is reference-only and has no row${hidden_from_all_pages(p.name) ? " (hidden as an OG built-in page, A11)" : ""}`
        : `frontend all pages: model page ${show(p)} (${p.owner}) has no row`);
      continue;
    }
    if (cell !== String(p.linkedRefs)) {
      diffs.push(`frontend all pages count of ${show(p)} (${p.owner}): model ${p.linkedRefs}, All Pages ${cell}`);
    }
    if (shown !== undefined && cell !== shown) {
      diffs.push(`frontend ${show(p)}: All Pages says ${cell}, its Linked References header says ${shown}`);
    }
  }
  for (const [file, cell] of rows) {
    if (!owners.has(file)) diffs.push(`frontend all pages: row ${file} (count ${cell}) owns no model page`);
  }

  // The page list (namespaces, sidebar): reference-only names it lists.
  const kinds = new Map<string, Set<string>>();
  for (const e of c.inventory) kinds.set(e.name, (kinds.get(e.name) ?? new Set()).add(e.target.kind));
  const listed = new Set<string>();
  for (const name of allPageNames()) {
    if (kinds.get(name)?.has("alias") && !kinds.get(name)?.has("existing") && !kinds.get(name)?.has("absent")) continue;
    const key = c.keyOf[name];
    if (key === undefined) diffs.push(`harness: allPageNames lists ${name}, which the inventory has no key for`);
    else listed.add(key);
  }
  const modelKeys = new Set(c.pages.map((p) => p.key));
  for (const key of modelKeys) {
    if (!listed.has(key) && !conflicts.has(key)) diffs.push(`frontend page list: model page "${key}" is not listed`);
  }
  for (const key of listed) {
    if (!modelKeys.has(key) && !conflicts.has(key)) diffs.push(`frontend page list: lists "${key}", which is no model page`);
  }
  return [...new Set(diffs)].sort();
}

beforeAll(() => initParser());
afterEach(() => {
  document.body.innerHTML = "";
  localStorage.clear();
  resetStore();
  resetPageIndex();
  resetReferenceSectionState();
  vi.restoreAllMocks();
});

describe("conceptual model conformance: the counts and the page set Tine shows", () => {
  it("differs from the model exactly as expected-differences.json records", async () => {
    const now: Record<string, string[]> = {};
    for (const [name, c] of cases()) {
      const diffs = await runCase(c).catch((e: unknown) => { throw new Error(`${name}: ${String(e)}`); });
      vi.restoreAllMocks();
      if (diffs.length) now[name] = diffs;
    }
    const file = JSON.parse(readFileSync(EXPECTED, "utf8")) as Record<string, unknown>;
    if (process.env.CM_REFRESH) {
      file.frontendCounts = { fixtures: now };
      writeFileSync(EXPECTED, JSON.stringify(sorted(file), null, 2) + "\n");
      return;
    }
    const was = ((file.frontendCounts as { fixtures?: Record<string, string[]> } | undefined)?.fixtures) ?? {};
    expect(
      now,
      "the shown counts or page set changed; if intended, refresh with CM_REFRESH=1 npx vitest run src/cmConformanceCounts.test.tsx",
    ).toEqual(was);
    expect(Object.values(now).flat().filter((d) => d.startsWith("harness:"))).toEqual([]);
  }, 180_000);
});

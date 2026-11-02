// Conceptual model conformance, step 2 (journal dates and what a link opens):
// the frontend's REAL link decision, run against the conceptual model.
//
// The Rust harness (`crates/tine-store/src/cm_conformance_tests.rs`) writes one
// case per fixture graph to `crates/tine-store/tests/fixtures/cm-conformance/
// frontend/` (and fails when a committed case is stale): the store's
// `page_inventory` entries in their wire shape, the title format GraphMeta hands
// the frontend, and per spelling the model's answer. This test installs that
// inventory in `pageIndex.ts` exactly as the app does (a mocked `page_inventory`
// IPC), sets the title format as `graph.ts` does, and runs the click path of
// `PageRef` (`render/inline.tsx`): `linkPageKind(name)`, then the router's
// `openPageTarget`, which routes a page-kind link to `navigationName(name)`.
// The page the route opens is the backend's `get_page(name, kind)`, read from
// the case's `open` table (Rust `WholeGraph::resolve`, first alias owner).
//
// Differences are compared with the `frontend` part of
// `expected-differences.json`; `CM_REFRESH=1` rewrites that part.
// Harness gap closed: H3 (the TS click path was emulated in Rust).
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PageInventoryEntry, PageKind } from "./types";

const backendMock = vi.hoisted(() => ({ pageInventory: vi.fn(), graphBindingGeneration: vi.fn(() => 1) }));
vi.mock("./backend", () => ({ backend: () => backendMock }));

type Page = [string, string];
interface Case {
  journalTitleFormat: string | null;
  conflicts: string[];
  inventory: PageInventoryEntry[];
  spellings: Record<string, { day: [number, number, number] | null; model: Page; backend: Page }>;
  open: Record<string, Record<PageKind, Page>>;
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

const show = (p: Page | undefined) => (p ? `("${p[0]}", "${p[1]}")` : "nothing");
const sameDay = (d: Date | null, want: [number, number, number] | null) =>
  d === null ? want === null : want !== null && d.getFullYear() === want[0] && d.getMonth() + 1 === want[1] && d.getDate() === want[2];
const showDay = (d: Date | null) =>
  d ? `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}` : "none";
const showWant = (w: [number, number, number] | null) =>
  w ? `${w[0]}-${String(w[1]).padStart(2, "0")}-${String(w[2]).padStart(2, "0")}` : "none";

/** Sorted keys, so the file matches what serde_json (BTreeMap) writes. */
function sorted(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(sorted);
  if (v && typeof v === "object") {
    return Object.fromEntries(Object.keys(v).sort().map((k) => [k, sorted((v as Record<string, unknown>)[k])]));
  }
  return v;
}

async function runCase(c: Case): Promise<string[]> {
  vi.resetModules();
  backendMock.pageInventory.mockReset().mockResolvedValue({ rev: "1", entries: c.inventory });
  await (await import("./render/parse")).initParser();
  const journal = await import("./journal");
  // graph.ts: setJournalTitleFormat(meta.journal_page_title_format).
  journal.setJournalTitleFormat(c.journalTitleFormat);
  const index = await import("./pageIndex");
  index.installPageIndex();
  await vi.waitFor(() => expect(index.allPages()).toBeDefined());
  const diffs: string[] = [];
  const conflicts = new Set(c.conflicts);
  for (const [s, want] of Object.entries(c.spellings)) {
    // The frontend's own "is this a journal date, which day" (journal.ts
    // journalParts) against the native parse list.
    const day = journal.parseJournalTitle(s);
    if (!sameDay(day, want.day)) {
      diffs.push(`frontend date [[${s}]]: native ${showWant(want.day)}, frontend ${showDay(day)}`);
    }
    if (conflicts.has(want.model[1])) continue;
    // PageRef: kind = linkPageKind(name); openPage(name, kind) -> the router's
    // openPageTarget: a page-kind route opens navigationName(name).
    const kind = index.linkPageKind(s);
    const routed = kind === "page" ? index.navigationName(s) : s;
    const opened = c.open[routed]?.[kind];
    if (!opened) {
      diffs.push(`harness: frontend routes [[${s}]] to (${kind}, ${routed}), which the case has no answer for`);
      continue;
    }
    if (opened[0] !== want.model[0] || opened[1] !== want.model[1]) {
      const agrees = opened[0] === want.backend[0] && opened[1] === want.backend[1];
      diffs.push(
        `frontend navigate [[${s}]]: model ${show(want.model)}, frontend ${show(opened)} via (${kind}, ${routed}); ` +
          `backend ${agrees ? "agrees with the frontend" : show(want.backend)}`,
      );
    } else if (opened[0] !== want.backend[0] || opened[1] !== want.backend[1]) {
      diffs.push(`frontend navigate [[${s}]]: frontend agrees with the model ${show(want.model)}, backend ${show(want.backend)}`);
    }
  }
  return diffs.sort();
}

beforeEach(() => {
  backendMock.graphBindingGeneration.mockReset().mockReturnValue(1);
});

describe("conceptual model conformance: the frontend's link decision", () => {
  it("differs from the model exactly as expected-differences.json records", async () => {
    const now: Record<string, string[]> = {};
    for (const [name, c] of cases()) {
      const diffs = await runCase(c);
      if (diffs.length) now[name] = diffs;
    }
    const file = JSON.parse(readFileSync(EXPECTED, "utf8")) as Record<string, unknown>;
    if (process.env.CM_REFRESH) {
      file.frontend = { fixtures: now };
      writeFileSync(EXPECTED, JSON.stringify(sorted(file), null, 2) + "\n");
      return;
    }
    const was = ((file.frontend as { fixtures?: Record<string, string[]> } | undefined)?.fixtures) ?? {};
    expect(
      now,
      "the frontend's differences changed; if intended, refresh with CM_REFRESH=1 npx vitest run src/cmConformanceNavigate.test.ts",
    ).toEqual(was);
    expect(Object.values(now).flat().filter((d) => d.startsWith("harness:"))).toEqual([]);
  }, 60_000);
});

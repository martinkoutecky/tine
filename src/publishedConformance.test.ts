// Conceptual model conformance, the published site (CM step 2, census Q13):
// every cm-conformance fixture graph, published by the static export
// (`tests/fixtures/published-answers/conformance/`, written by
// `crates/tine-graph-features/tests/cm_conformance_published.rs`), read through
// `publishedBackend` and the app's own click path: `linkTarget` (PageRef), the
// router's `navigationTarget` for an ordinary-page route, then `getPage`. Each
// answer is compared with the fixture's frontend case (the live store over the
// whole graph): what `[[s]]` opens (`spellings[s].backend`) and which file
// `getPage(name, kind)` reads (`owner`).
//
// Where the published answer differs by design (the build graph holds only the
// exported pages, so `public:: false` pages are not there), the difference is
// recorded in `tests/fixtures/published-answers/conformance-differences.json`
// with its reason in the design note; `CM_REFRESH=1` rewrites that file.
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it, vi } from "vitest";
import type { PublishedSnapshot } from "./publishedBackend";
import type { PageKind } from "./types";

const holder = vi.hoisted(() => ({ api: null as unknown }));
vi.mock("./backend", () => ({ backend: () => holder.api }));

type Page = [string, string];
interface Case {
  conflicts: string[];
  spellings: Record<string, { model: Page; backend: Page }>;
  owner: Record<string, Record<PageKind, string | null>>;
  /** The model key of every name the case knows (a date-named page's key is
   *  its day's). */
  keyOf: Record<string, string>;
}

const ROOT = path.resolve(__dirname, "..");
const PUBLISHED = path.join(ROOT, "tests/fixtures/published-answers/conformance");
const FRONTEND = path.join(ROOT, "crates/tine-store/tests/fixtures/cm-conformance/frontend");
const EXPECTED = path.join(ROOT, "tests/fixtures/published-answers/conformance-differences.json");

const show = (p: Page | null) => (p ? `("${p[0]}", "${p[1]}")` : "nothing");

async function runCase(snapshot: PublishedSnapshot, c: Case): Promise<string[]> {
  vi.resetModules();
  await (await import("./render/parse")).initParser();
  const { pageIdentityKey } = await import("./pageIdentity");
  const { publishedBackend } = await import("./publishedBackend");
  holder.api = publishedBackend(async () => snapshot);
  const api = holder.api as ReturnType<typeof publishedBackend>;
  const loaded = await api.loadGraph("");
  const journal = await import("./journal");
  // graph.ts: setJournalFormats(meta.journal_page_title_format, meta.journal_file_name_format).
  if (loaded.kind === "loaded") journal.setJournalFormats(loaded.meta.journal_page_title_format, loaded.meta.journal_file_name_format);
  const index = await import("./pageIndex");
  index.installPageIndex();
  await vi.waitFor(() => expect(index.allPages()).toBeDefined());
  const diffs: string[] = [];
  const conflicts = new Set(c.conflicts);
  for (const [s, want] of Object.entries(c.spellings)) {
    if (conflicts.has(want.model[1])) continue;
    const link = index.linkTarget(s);
    const route = link.kind === "page" ? index.navigationTarget(link.name) : link;
    const page = await api.getPage(route.name, route.kind);
    // The page shown: the file read, or the empty page of that name and kind,
    // keyed as the case keys pages (the model key, `keyOf`).
    const key = (name: string) => c.keyOf[name] ?? pageIdentityKey(name);
    const opened: Page = page ? [page.kind, key(page.name)] : [route.kind, key(route.name)];
    if (opened[0] !== want.backend[0] || opened[1] !== want.backend[1]) {
      diffs.push(`published navigate [[${s}]]: live ${show(want.backend)}, published ${show(opened)}`);
    }
  }
  for (const [name, at] of Object.entries(c.owner)) {
    for (const kind of ["page", "journal"] as const) {
      const want = at[kind];
      const got = (await api.getPage(name, kind))?.id ?? null;
      if (got !== want) diffs.push(`published getPage ${kind} [[${name}]]: live ${want}, published ${got}`);
    }
  }
  return diffs.sort();
}

describe("conceptual model conformance: the published site answers as the app", () => {
  it("differs from the live store exactly as conformance-differences.json records", async () => {
    const now: Record<string, string[]> = {};
    const files = readdirSync(PUBLISHED).filter((f) => f.endsWith(".json")).sort();
    expect(files.length).toBeGreaterThan(60);
    for (const file of files) {
      const name = file.slice(0, -".json".length);
      const published = JSON.parse(readFileSync(path.join(PUBLISHED, file), "utf8")) as PublishedSnapshot & { refused?: string };
      if (published.refused) {
        now[name] = [`published: the static export refuses: ${published.refused}`];
        continue;
      }
      const c = JSON.parse(readFileSync(path.join(FRONTEND, file), "utf8")) as Case;
      const diffs = await runCase(published, c);
      if (diffs.length) now[name] = diffs;
    }
    if (process.env.CM_REFRESH) {
      writeFileSync(EXPECTED, `${JSON.stringify(now, null, 2)}\n`);
      return;
    }
    const was = JSON.parse(readFileSync(EXPECTED, "utf8")) as Record<string, string[]>;
    expect(now, "the published site's differences changed; if intended, refresh with CM_REFRESH=1 and record the reason").toEqual(was);
  }, 120_000);
});

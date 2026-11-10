// The declared legacy reader (`publishedLegacy.ts`, I-12 exception): a
// snapshot without a recognized `answers` reads exactly as the published
// viewer read every snapshot before `answers` existed. The golden
// (`tests/fixtures/published-answers/legacy-golden.json`) was written by the
// viewer before this change (develop d86067791) over the frozen generation-1
// fixtures and the answer fixtures with `answers` removed or of an unknown
// `v`; this viewer must give every answer in it unchanged, except that the
// `inventory` also lists the snapshot's alias pairs as alias rows (HFh-eo,
// regenerated; the diff is those four alias rows, no `names` answer moved).
// `TINE_PUBLISHED_LEGACY_GOLDEN=write` rewrites it (only ever from that old
// viewer: the golden is the old behaviour, not this code's).
import { readFileSync, writeFileSync } from "node:fs";
import { expect, it, vi } from "vitest";
import type { PublishedSnapshot } from "./publishedBackend";

vi.mock("./backend", () => ({ backend: () => { throw new Error("no live backend in a published viewer"); } }));

const ROOT = new URL("../tests/fixtures/", import.meta.url);
const GOLDEN = new URL("published-answers/legacy-golden.json", ROOT);
const read = (path: string) => JSON.parse(readFileSync(new URL(path, ROOT), "utf8"));

function snapshots(): [string, PublishedSnapshot][] {
  const out: [string, PublishedSnapshot][] = [];
  for (const share of ["pages", "one-page"]) out.push([`g1/${share}`, read(`quick-share/g1/${share}/document.json`).snapshot]);
  for (const fixture of ["cases", "formats"]) {
    const snapshot = read(`published-answers/${fixture}.json`);
    delete snapshot.answers;
    out.push([`${fixture} without answers`, snapshot]);
    out.push([`${fixture} with answers v2`, { ...structuredClone(snapshot), answers: { v: 2, inventory: "not read" } }]);
  }
  return out;
}

/** Every name the snapshot spells, and a few it does not. */
function names(snapshot: PublishedSnapshot): string[] {
  const all = new Set<string>(["missing page", "2026-10-26", "2026_10_17", "proj", "PROJECT", "launch day", "Shared", "n", "ML"]);
  for (const page of snapshot.pages) all.add(page.name);
  for (const entry of snapshot.entries) all.add(entry.name);
  for (const [from, to] of snapshot.aliases) { all.add(from); all.add(to); }
  for (const key of Object.keys(snapshot.backlinks)) all.add(key);
  return [...all].sort();
}

async function answers(snapshot: PublishedSnapshot) {
  await (await import("./render/parse")).initParser();
  const { publishedBackend } = await import("./publishedBackend");
  const { resolvePublishedPermalink } = await import("./publishedPermalink");
  const api = publishedBackend(async () => snapshot);
  const loaded = await api.loadGraph("");
  const out: Record<string, unknown> = {
    formats: loaded.kind === "loaded" ? [loaded.meta.journal_page_title_format, loaded.meta.journal_file_name_format] : null,
    inventory: (await api.pageInventory()).entries,
    content_days: await api.journalContentDays(),
    feed: (await api.journalFeedPage(50, null)).pages.map((page) => page.id),
  };
  const byName: Record<string, unknown> = {};
  for (const name of names(snapshot)) {
    byName[name] = {
      page: (await api.getPage(name, "page"))?.id ?? null,
      journal: (await api.getPage(name, "journal"))?.id ?? null,
      resolve_page: await api.resolvePage(name, "page"),
      resolve_journal: await api.resolvePage(name, "journal"),
      group: await api.pageAliasGroup(name),
      backlinks: (await api.getBacklinks(name)).map((group) => group.page),
      permalink: resolvePublishedPermalink(snapshot, { kind: "page", page: name })?.page.path ?? null,
    };
  }
  out.names = byName;
  return out;
}

it("reads every snapshot without recognized answers exactly as the viewer before answers did", async () => {
  const actual: Record<string, unknown> = {};
  for (const [label, snapshot] of snapshots()) actual[label] = await answers(snapshot);
  if (process.env.TINE_PUBLISHED_LEGACY_GOLDEN === "write") writeFileSync(GOLDEN, `${JSON.stringify(actual, null, 2)}\n`);
  expect(actual).toEqual(JSON.parse(readFileSync(GOLDEN, "utf8")));
});

// Conceptual model step 2, batch 4 (census Q12, harness gap H1): favorites
// after a rename, a merge or a title:: change that renames a page.
//
// The scenarios and the one answer are shared with the Rust checks
// (`crates/tine-graph-features/tests/fixtures/cm-favorites/scenarios.json`;
// the Rust entry in `tests/cm_favorites_rename.rs`, the headless CLI owner in
// `src-tauri/src/owner/owner_tests/cm_favorites_tests.rs`). One answerer
// computes the favorites: the backend's config step (`pages::commit_config`).
// This test pins that the window is not a second computer (I-12): from a graph
// seeded as `graph.ts` seeds it (`seedFavorites(:favorites,
// :tine/favorites-page)`), it runs the window's real receive paths with the
// backend's answer (the scenario's target, which the Rust entries are pinned
// to produce) and records any config write the window makes
// (`backend().setFavorites`, expected: none) and what the sidebar shows.
// - rename/merge: graph.ts `refreshAfterRename` (navigation; must leave
//   favorites alone) then graphRewrite `applyOwnerPageRename` with a report
//   carrying `favorites`, as a CLI rename a window hears about does (an app
//   rename's own report takes the same `refreshAfterRename`).
// - retitle: graphRewrite `followPageIdentity(path, old)`, which the page
//   identity handler calls; it must ask the backend with exactly that path and
//   old name and adopt its answer.
//
// Differences from the target are compared with the `frontend` part of
// `expected-differences.json`; `CM_REFRESH=1` rewrites that part.
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { refreshAfterRename } from "./graph";
import { applyOwnerPageRename, followPageIdentity } from "./document/graphRewrite";
import { seedFavorites, favorites } from "./favorites";
import { allPages, installPageIndex, refreshPageIndex, resetPageIndex } from "./pageIndex";
import type { PageInventoryEntry } from "./types";

interface Scenario {
  id: string;
  config: string;
  op: { kind: "rename" | "merge" | "retitle"; old: string; new: string; into?: string; path?: string };
  target: { favorites: string[]; favoritesPage: string | null };
  frontendInventory?: PageInventoryEntry[];
}

const FIXTURES = path.resolve(__dirname, "../crates/tine-graph-features/tests/fixtures/cm-favorites");
const EXPECTED = path.join(FIXTURES, "expected-differences.json");
const scenarios = (JSON.parse(readFileSync(path.join(FIXTURES, "scenarios.json"), "utf8")) as { scenarios: Scenario[] }).scenarios;

/** The two config keys a scenario seeds, read from its fixture text. The app
 *  receives them parsed by the backend (`GraphMeta`); these fixtures write
 *  them in one plain shape (a vector of strings, a string). */
function seeded(config: string): { names: string[]; page: string | null } {
  const vector = /:favorites\s*\[([^\]]*)\]/.exec(config)?.[1] ?? "";
  const names = [...vector.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map((m) => JSON.parse(`"${m[1]}"`) as string);
  const page = /:tine\/favorites-page\s*"((?:[^"\\]|\\.)*)"/.exec(config)?.[1] ?? null;
  return { names, page };
}

const settle = async () => { for (let turn = 0; turn < 8; turn++) { for (let i = 0; i < 30; i++) await Promise.resolve(); await new Promise((r) => setTimeout(r, 0)); } };

function sorted(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(sorted);
  if (v && typeof v === "object") {
    return Object.fromEntries(Object.keys(v).sort().map((k) => [k, sorted((v as Record<string, unknown>)[k])]));
  }
  return v;
}

const showList = (names: string[]) => `[${names.map((n) => JSON.stringify(n)).join(", ")}]`;

async function runScenario(s: Scenario): Promise<string[]> {
  const api = backend();
  const written: { names: string[]; page: string | null }[] = [];
  vi.spyOn(api, "setFavorites").mockImplementation(async (names: string[], page?: string | null) => {
    written.push({ names: [...names], page: page ?? null });
  });
  // The arrangement page's contents are not this check's question (the
  // backend rewrites its links); an unread page leaves the seeded flat list.
  vi.spyOn(api, "getPage").mockResolvedValue(null);
  vi.spyOn(api, "pageInventory").mockResolvedValue({ rev: "1", entries: s.frontendInventory ?? [] } as never);
  installPageIndex();
  resetPageIndex();
  await refreshPageIndex();
  await vi.waitFor(() => expect(allPages()).toBeDefined());
  const config = seeded(s.config);
  seedFavorites(config.names, config.page);
  await settle();
  const answer = { names: s.target.favorites, page: s.target.favoritesPage };
  const moved = JSON.stringify(answer) !== JSON.stringify(config);
  const diffs: string[] = [];
  if (s.op.kind === "retitle") {
    const follow = vi.spyOn(api, "followPageIdentity").mockResolvedValue({ name: s.op.new, ...(moved ? { favorites: answer } : {}) });
    await followPageIdentity(s.op.path!, s.op.old);
    const asked = follow.mock.calls.map((c) => c.join(" <- "));
    if (JSON.stringify(asked) !== JSON.stringify([`${s.op.path} <- ${s.op.old}`])) {
      diffs.push(`frontend follow: asked ${JSON.stringify(asked)}`);
    }
  } else {
    // Navigation alone must not touch favorites.
    refreshAfterRename(s.op.old, s.op.new);
    await settle();
    const shownAfterNavigation = favorites().map((f) => f.name);
    if (JSON.stringify(shownAfterNavigation) !== JSON.stringify(config.names)) {
      diffs.push(`frontend navigation computed favorites: ${showList(shownAfterNavigation)}`);
    }
    await applyOwnerPageRename({
      old: s.op.old, name: s.op.new, kind: "page", path: null,
      report: { outcome: s.op.kind === "merge" ? "merged" : "renamed", touched: [], ...(moved ? { favorites: answer } : {}) },
    } as never);
  }
  await settle();
  if (written.length) diffs.push(`frontend wrote favorites: ${written.map((w) => showList(w.names)).join("; ")}`);
  const shown = favorites().map((f) => f.name);
  if (JSON.stringify(shown) !== JSON.stringify(s.target.favorites)) {
    diffs.push(`frontend sidebar: target ${showList(s.target.favorites)}, shown ${showList(shown)}`);
  }
  vi.restoreAllMocks();
  return diffs;
}

describe("conceptual model: favorites after a rename (frontend)", () => {
  it("records the frontend's answer against the target", async () => {
    const found: Record<string, string[]> = {};
    for (const s of scenarios) {
      const diffs = await runScenario(s);
      if (diffs.length) found[s.id] = diffs;
    }
    const all = JSON.parse(readFileSync(EXPECTED, "utf8")) as Record<string, unknown>;
    if (process.env.CM_REFRESH) {
      all.frontend = found;
      writeFileSync(EXPECTED, JSON.stringify(sorted(all), null, 2) + "\n");
      return;
    }
    expect(sorted(found)).toEqual(sorted(all.frontend ?? {}));
  }, 60_000);
});

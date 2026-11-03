// Conceptual model step 2, batch 4 (census Q12, harness gap H1): favorites
// after a rename or merge, as the frontend answers it today.
//
// The scenarios and the proposed one answer are shared with the Rust checks
// (`crates/tine-graph-features/tests/fixtures/cm-favorites/scenarios.json`;
// the Rust entry in `tests/cm_favorites_rename.rs`, the headless CLI owner in
// `src-tauri/src/owner/owner_tests/cm_favorites_tests.rs`). This test runs the
// frontend's real path after a backend rename: graph.ts `refreshAfterRename`
// -> ui.ts `renamePageInNavigation` -> favorites.ts `renameFavorite`, from a
// graph seeded as `graph.ts` seeds it (`seedFavorites(:favorites,
// :tine/favorites-page)`), and records the config write it makes
// (`backend().setFavorites`). For a merge the app passes the typed name, as
// `renamePageOnDisk(from, to, …, mergeInto)` does. Page files (the arrangement
// page's links) and `:default-home` are the backend's and are checked there.
//
// Differences from the target are compared with the `frontend` part of
// `expected-differences.json`; `CM_REFRESH=1` rewrites that part.
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { refreshAfterRename } from "./graph";
import { seedFavorites, favorites } from "./favorites";
import { allPages, installPageIndex, refreshPageIndex, resetPageIndex } from "./pageIndex";
import type { PageInventoryEntry } from "./types";

interface Scenario {
  id: string;
  config: string;
  op: { kind: "rename" | "merge"; old: string; new: string; into?: string };
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
const showPage = (page: string | null) => (page === null ? "None" : `Some(${JSON.stringify(page)})`);

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
  refreshAfterRename(s.op.old, s.op.new);
  await settle();
  const after = written.at(-1) ?? config;
  // `setFavorites(names, null)` leaves `:tine/favorites-page` as it was.
  const page = after.page ?? config.page;
  const diffs: string[] = [];
  if (JSON.stringify(after.names) !== JSON.stringify(s.target.favorites)) {
    diffs.push(`frontend favorites: target ${showList(s.target.favorites)}, got ${showList(after.names)}`);
  }
  if (page !== s.target.favoritesPage) {
    diffs.push(`frontend favorites-page: target ${showPage(s.target.favoritesPage)}, got ${showPage(page)}`);
  }
  // The in-memory list the sidebar shows must be what was written.
  const shown = favorites().map((f) => f.name);
  if (JSON.stringify(shown) !== JSON.stringify(after.names)) {
    diffs.push(`frontend sidebar: written ${showList(after.names)}, shown ${showList(shown)}`);
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

import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  rankLauncherItems,
  recordLauncherActivation,
  resetLauncherRanking,
  setLauncherRankingEnabled,
  type LauncherRankable,
} from "./launcherRanking";

const item = (id: string, adaptiveClass: LauncherRankable["adaptiveClass"]): LauncherRankable => ({
  adaptiveIdentity: id,
  adaptiveClass,
});

class TestStorage implements Storage {
  private values = new Map<string, string>();
  writes = 0;
  get length() { return this.values.size; }
  clear() { this.values.clear(); }
  getItem(key: string) { return this.values.get(key) ?? null; }
  key(index: number) { return [...this.values.keys()][index] ?? null; }
  removeItem(key: string) { this.values.delete(key); }
  setItem(key: string, value: string) { this.writes += 1; this.values.set(key, value); }
}

let storage: Storage;

beforeEach(() => {
  storage = new TestStorage();
  setLauncherRankingEnabled(true);
  vi.restoreAllMocks();
});

describe("bounded Ctrl+K frecency (GH #143)", () => {
  it("requires repetition and never crosses objective classes", () => {
    const items = [item("exact", "exact"), item("a", "prefix"), item("b", "prefix")];
    recordLauncherActivation("graph", "ti", "b", 100, storage);
    expect(rankLauncherItems("graph", "ti", items, 100, storage).map((x) => x.adaptiveIdentity))
      .toEqual(["exact", "a", "b"]);
    recordLauncherActivation("graph", "ti", "b", 101, storage);
    expect(rankLauncherItems("graph", "ti", items, 101, storage).map((x) => x.adaptiveIdentity))
      .toEqual(["exact", "b", "a"]);
  });

  it("uses favorite status only inside the same objective class", () => {
    const exact = item("exact", "exact");
    const ordinary = item("ordinary", "prefix");
    const favorite = { ...item("favorite", "prefix"), adaptiveFavorite: true };
    const weakerFavorite = { ...item("weaker-favorite", "substring"), adaptiveFavorite: true };
    expect(rankLauncherItems("graph", "ti", [exact, ordinary, favorite, weakerFavorite], 100, storage)
      .map((x) => x.adaptiveIdentity))
      .toEqual(["exact", "favorite", "ordinary", "weaker-favorite"]);
    // Defend the contract even if a future producer accidentally interleaves
    // shallow rows; the ranking comparator must remain transitive.
    expect(rankLauncherItems("graph", "ti", [ordinary, weakerFavorite, favorite, exact], 100, storage)
      .map((x) => x.adaptiveIdentity))
      .toEqual(["exact", "favorite", "ordinary", "weaker-favorite"]);
  });

  it("isolates queries and graphs, decays, resets, and obeys disable", () => {
    const items = [item("a", "body_evidence"), item("b", "body_evidence")];
    recordLauncherActivation("one", "query", "b", 100, storage);
    recordLauncherActivation("one", "query", "b", 101, storage);
    expect(rankLauncherItems("one", "other", items, 101, storage)[0].adaptiveIdentity).toBe("a");
    expect(rankLauncherItems("two", "query", items, 101, storage)[0].adaptiveIdentity).toBe("a");
    resetLauncherRanking("one", storage);
    expect(rankLauncherItems("one", "query", items, 101, storage)[0].adaptiveIdentity).toBe("a");
    setLauncherRankingEnabled(false);
    recordLauncherActivation("one", "query", "b", 102, storage);
    recordLauncherActivation("one", "query", "b", 103, storage);
    expect(rankLauncherItems("one", "query", items, 103, storage)[0].adaptiveIdentity).toBe("a");
  });

  it("keeps storage bounded and never writes while ranking", () => {
    for (let index = 0; index < 520; index += 1) {
      recordLauncherActivation("graph", `q${index}`, `id${index}`, index, storage);
    }
    const key = storage.key(0)!;
    const parsed = JSON.parse(storage.getItem(key)!) as { records: unknown[] };
    expect(parsed.records).toHaveLength(400);
    const writes = (storage as TestStorage).writes;
    rankLauncherItems(
      "graph",
      "q519",
      [item("id518", "fuzzy"), item("id519", "fuzzy")],
      520,
      storage,
    );
    expect((storage as TestStorage).writes).toBe(writes);
  });
});

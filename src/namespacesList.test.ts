import { describe, expect, it } from "vitest";
import { buildNamespaceTree, namespaceTreeFor } from "./components/Namespace";
import {
  allExpanded,
  defaultExpanded,
  indexNamespaces,
  matchingEntries,
  namespaceTotals,
  pathPrefix,
  visibleEntries,
} from "./namespacesList";
import { parsePersistedSession } from "./session";
import { routeTitle, sameRoute } from "./router";

// GH #335: the list logic of the Namespaces view. Pure: no backend, no DOM.

// The inventory's names (CM step 2 batch 10): it lists every namespace parent,
// so "Archive", "Archive/Old" and "Other" are listed although no file has them.
const NAMES = [
  "Project", // a page that is also a namespace
  "Project/Roadmap",
  "Project/Roadmap/2026",
  "Project/Notes",
  "Archive/Old/Notes",
  "Archive/Old/Plans",
  "Archive", // namespace parents: pages of their own (model rule 1)
  "Archive/Old",
  "plain",
  "Other/Notes",
  "Other",
];
/** `names` plus every namespace parent, as the inventory lists them. */
const withParents = (names: string[]) => {
  const all = new Set(names);
  for (const name of names) {
    const segs = name.split("/");
    for (let k = 1; k < segs.length; k++) all.add(segs.slice(0, k).join("/"));
  }
  return [...all];
};
const index = () => indexNamespaces(buildNamespaceTree(NAMES));
const fulls = (entries: ReturnType<typeof index>, rows: number[]) => rows.map((i) => entries[i].node.full);

describe("indexNamespaces", () => {
  it("lists the tree in path order with depth, parent and subtree end", () => {
    const entries = index();
    expect(entries.map((e) => [e.node.full, e.depth])).toEqual([
      ["Archive", 0], ["Archive/Old", 1], ["Archive/Old/Notes", 2], ["Archive/Old/Plans", 2],
      ["Other", 0], ["Other/Notes", 1],
      ["Project", 0], ["Project/Notes", 1], ["Project/Roadmap", 1], ["Project/Roadmap/2026", 2],
    ]);
    expect(entries.map((e) => e.end)).toEqual([4, 4, 3, 4, 6, 6, 10, 8, 10, 10]);
    expect(entries.map((e) => e.parent)).toEqual([-1, 0, 1, 1, -1, 4, -1, 6, 6, 8]);
  });

  it("counts the pages below each entry: a namespace parent is a page (model rule 1)", () => {
    const entries = index();
    const count = (full: string) => entries.find((e) => e.node.full === full)!.count;
    expect(count("Archive")).toBe(3); // Old (a namespace parent), Old/Notes and Old/Plans
    expect(count("Archive/Old")).toBe(2);
    expect(count("Project")).toBe(3); // itself is listed but not counted below itself
    expect(count("Project/Roadmap")).toBe(1);
    expect(count("Project/Notes")).toBe(0);
    expect(namespaceTotals(entries)).toEqual({ pages: 10, namespaces: 5 }); // "plain" has no namespace
  });

  it("builds the same tree for the same name list object once", () => {
    const names = ["a/b", "a/c"];
    expect(namespaceTreeFor(names)).toBe(namespaceTreeFor(names));
  });

  it("is iterative: a thousand-level name does not overflow the stack", () => {
    const deep = Array.from({ length: 2000 }, (_, i) => `s${i}`).join("/");
    const entries = indexNamespaces(buildNamespaceTree(withParents([deep])));
    expect(entries).toHaveLength(2000);
    expect(entries[1999].depth).toBe(1999);
    expect(entries[0].end).toBe(2000);
  });
});

describe("expand and collapse", () => {
  it("opens the top-level namespaces by default and shows only their children", () => {
    const entries = index();
    expect(fulls(entries, visibleEntries(entries, defaultExpanded(entries)))).toEqual([
      "Archive", "Archive/Old", "Other", "Other/Notes", "Project", "Project/Notes", "Project/Roadmap",
    ]);
  });

  it("collapse all leaves the top-level entries; expand all shows every entry", () => {
    const entries = index();
    expect(fulls(entries, visibleEntries(entries, new Set()))).toEqual(["Archive", "Other", "Project"]);
    expect(visibleEntries(entries, allExpanded(entries))).toEqual(entries.map((_, i) => i));
  });

  it("a collapsed parent hides its whole subtree even when a deeper entry is marked open", () => {
    const entries = index();
    expect(fulls(entries, visibleEntries(entries, new Set(["Project/Roadmap"])))).toEqual(["Archive", "Other", "Project"]);
  });
});

describe("matchingEntries", () => {
  it("matches the full path, case-insensitively, in tree order", () => {
    const entries = index();
    expect(fulls(entries, matchingEntries(entries, "NOTES"))).toEqual(["Archive/Old/Notes", "Other/Notes", "Project/Notes"]);
  });

  it("a match inside a parent path includes the entries below it", () => {
    const entries = index();
    expect(fulls(entries, matchingEntries(entries, "roadmap"))).toEqual(["Project/Roadmap", "Project/Roadmap/2026"]);
  });

  it("matches nothing for a blank filter and nothing for an absent name", () => {
    const entries = index();
    expect(matchingEntries(entries, "  ")).toEqual([]);
    expect(matchingEntries(entries, "zzz")).toEqual([]);
  });

  it("splits a row's path into the part above its own segment", () => {
    const entries = index();
    const leaf = entries.find((e) => e.node.full === "Project/Roadmap/2026")!;
    expect(pathPrefix(leaf)).toBe("Project/Roadmap/");
    expect(pathPrefix(entries[0])).toBe("");
  });
});

describe("the namespaces route", () => {
  it("is titled and compared like the other one-per-kind views", () => {
    expect(routeTitle({ kind: "namespaces" })).toBe("Namespaces");
    expect(sameRoute({ kind: "namespaces" }, { kind: "namespaces" })).toBe(true);
    expect(sameRoute({ kind: "namespaces" }, { kind: "all-pages" })).toBe(false);
  });

  it("survives a session restore", () => {
    const raw = JSON.stringify({ tabs: [{ history: [{ kind: "journals" }, { kind: "namespaces" }], pos: 1, pinned: false }], activeIndex: 0 });
    expect(parsePersistedSession(raw)?.snapshots.get("main")?.tabs[0].history).toEqual([{ kind: "journals" }, { kind: "namespaces" }]);
  });
});

describe("cost on a 30k-page graph", () => {
  // Not a timing assertion (machines differ): the bound is on work, and the measured time is printed
  // for the receipt. Filtering is one substring test per entry; expand/collapse walks visible rows.
  const names: string[] = [];
  for (let a = 0; a < 30; a++) for (let b = 0; b < 40; b++) for (let c = 0; c < 25; c++) names.push(`Area ${a}/Topic ${b}/Note ${c}`);
  it("indexes, filters and re-walks 30,000 namespaced pages", () => {
    const t0 = performance.now();
    const entries = indexNamespaces(buildNamespaceTree(withParents(names)));
    const t1 = performance.now();
    const hits = matchingEntries(entries, "topic 7/note 1");
    const t2 = performance.now();
    const rows = visibleEntries(entries, allExpanded(entries));
    const t3 = performance.now();
    const collapsed = visibleEntries(entries, new Set());
    console.log(`namespaces 30k: ${entries.length} entries; tree+index ${(t1 - t0).toFixed(0)} ms; filter ${(t2 - t1).toFixed(1)} ms (${hits.length} hits); expand-all walk ${(t3 - t2).toFixed(1)} ms; ${rows.length} rows; collapsed ${collapsed.length}`);
    expect(entries).toHaveLength(30 + 30 * 40 + 30_000);
    expect(hits.length).toBe(30 * 11); // "Topic 7/Note 1", "Note 10".."Note 19" = 11 per area
    expect(rows).toHaveLength(entries.length);
    expect(collapsed).toHaveLength(30);
  });
});

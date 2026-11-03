import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { backend } from "./backend";
import { captureBinding } from "./binding";
import { applyConfigDerivedState, applyGraphConfigChange } from "./graph";
import { graphEpoch, graphMeta, setGraphMeta } from "./graphSession";
import { favorites, seedFavorites } from "./favorites";
import { workflow } from "./ui";
import type { GraphMeta } from "./types";
import { initParser } from "./render/parse";
import { pageRefsInText } from "./render/pageRefs";

beforeAll(() => initParser());

const meta = {
  root: "/graph", preferred_workflow: "now", favorites: ["A"], favorites_page: null,
  journal_page_title_format: "MMM do, yyyy", default_home: "Home",
} as GraphMeta;

afterEach(() => { setGraphMeta(null); seedFavorites([]); });

it("applies an outside config edit in place and ignores an old binding", () => {
  setGraphMeta(meta);
  seedFavorites(["A"]);
  const binding = captureBinding();
  applyGraphConfigChange({ binding_generation: binding.backendGeneration + 1, meta: { ...meta, favorites: ["B"] } });
  expect(favorites().map((item) => item.name)).toEqual(["A"]);
  applyGraphConfigChange({ binding_generation: binding.backendGeneration, meta: { ...meta, favorites: ["B"], preferred_workflow: "todo" } });
  expect(graphMeta()?.favorites).toEqual(["B"]);
  expect(favorites().map((item) => item.name)).toEqual(["B"]);
  expect(workflow()).toBe("todo");
});

// Master's frontend half (docs/contracts/config-live-reload.md §4), ported in
// meaning: `applyConfigDerivedState` is the ONE producer of config-derived
// state that is not read reactively from `graphMeta` (I-12); graph open passes
// no previous meta and applies everything.
describe("config-derived state has one producer", () => {
  it("adopts a favorite added and one removed outside Tine", () => {
    seedFavorites(["Alpha", "Beta"]);
    applyConfigDerivedState({ ...meta, favorites: ["Beta", "Added"] }, { ...meta, favorites: ["Alpha", "Beta"] });
    expect(favorites().map((item) => item.name)).toEqual(["Beta", "Added"]);
  });

  it("does not re-seed favorites the user is already being shown", () => {
    const getPage = vi.spyOn(backend(), "getPage");
    seedFavorites(["Alpha", "Beta"]);
    applyConfigDerivedState(
      { ...meta, favorites: ["Alpha", "Beta"], favorites_page: "Favorites" },
      { ...meta, favorites: ["Zulu"], favorites_page: "Favorites" },
    );
    expect(favorites().map((item) => item.name)).toEqual(["Alpha", "Beta"]);
    expect(getPage).not.toHaveBeenCalled();
    getPage.mockRestore();
  });

  it("applies everything on a graph open, where there is no previous meta", () => {
    const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    applyConfigDerivedState({ ...meta, favorites: ["Alpha"], favorites_page: "Favorites", preferred_workflow: "todo" }, null);
    expect(favorites().map((item) => item.name)).toEqual(["Alpha"]);
    expect(workflow()).toBe("todo");
    expect(getPage).toHaveBeenCalledWith("Favorites", "page");
    getPage.mockRestore();
  });

  it("re-dates what is on screen when the journal title format moves outside Tine", () => {
    setGraphMeta(meta);
    const before = graphEpoch();
    applyGraphConfigChange({ binding_generation: captureBinding().backendGeneration, meta: { ...meta, journal_page_title_format: "yyyy-MM-dd" } });
    expect(graphEpoch()).toBeGreaterThan(before);
    const settled = graphEpoch();
    applyGraphConfigChange({ binding_generation: captureBinding().backendGeneration, meta: { ...meta, journal_page_title_format: "yyyy-MM-dd", macros: { hi: "x" } } as GraphMeta });
    expect(graphEpoch()).toBe(settled);
  });

  it("re-dates what is on screen when the journal file-name format moves outside Tine (D-20261102-JDL)", () => {
    setGraphMeta(meta);
    const before = graphEpoch();
    applyGraphConfigChange({ binding_generation: captureBinding().backendGeneration, meta: { ...meta, journal_file_name_format: "dd-MM-yyyy" } as GraphMeta });
    expect(graphEpoch()).toBeGreaterThan(before);
  });

  it("re-reads Org file links when the page file-name format moves outside Tine (census Q2)", () => {
    setGraphMeta(meta);
    const before = graphEpoch();
    applyGraphConfigChange({ binding_generation: captureBinding().backendGeneration, meta: { ...meta, file_name_format: "triple-lowbar" } as GraphMeta });
    expect(graphEpoch()).toBeGreaterThan(before);
    expect(pageRefsInText("[[file:./a.b.org][ab]]", "org")).toEqual(["a.b"]);
    applyGraphConfigChange({ binding_generation: captureBinding().backendGeneration, meta: { ...meta, file_name_format: "legacy" } as GraphMeta });
    expect(pageRefsInText("[[file:./a.b.org][ab]]", "org")).toEqual(["a/b"]);
  });

  it("graph open and live change share the producer (source guard, I-12)", () => {
    const source = readFileSync("src/graph.ts", "utf8");
    for (const setter of ["setWorkflow(", "setJournalFormats(", "setFileNameFormat(", "seedFavorites("]) {
      expect(source.split(setter).length - 1,
        `I-12: ${setter} must be called only from applyConfigDerivedState in src/graph.ts (master exemplar applyConfigDerivedState)`).toBe(1);
    }
    expect(source).toContain("applyConfigDerivedState(meta, null)");
  });
});

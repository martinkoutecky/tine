import { describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { buildVocabulary, displayFieldEntries, QueryVocabularyPicker } from "./QueryVocabularyPicker";
import type { RegistryRow } from "../editor/queryIr";

const row: RegistryRow = {
  normalized_name: "cost", observed_type: "number", cardinality: "one",
  count_blocks: 12, count_pages: 2, mismatch_count: 0, top_values: [["5", 4]],
};

describe("query vocabulary", () => {
  it("uses registry counts for each filter scope and labels types", () => {
    const entries = buildVocabulary({ anchor: "block", rows: [row] });
    expect(entries.filter((entry) => entry.label === "cost").map((entry) => [entry.count, entry.unit, entry.observed]))
      .toEqual([[12, "blocks", "number"], [2, "pages", "number"]]);
  });

  it("offers Page as a table column (GH #606)", () => {
    expect(displayFieldEntries({ slot: "column", rowKind: "block", rows: [], search: "page" })
      .some((entry) => entry.choice.kind === "field" && entry.choice.field === "page")).toBe(true);
  });

  it("keeps page and block field grammars distinct", () => {
    const block = displayFieldEntries({ slot: "group", rowKind: "block", rows: [row],
      formulas: ["margin"], search: "" });
    expect(block.some((entry) => entry.choice.kind === "field" && entry.choice.field === "formula:margin")).toBe(true);
    const page = displayFieldEntries({ slot: "column", rowKind: "page", rows: [
      row, { ...row, normalized_name: "page" }, { ...row, normalized_name: "block-only", count_pages: 0 },
    ], search: "" });
    expect(page.some((entry) => entry.choice.kind === "field" && entry.choice.field === "name")).toBe(false);
    expect(page.some((entry) => entry.label === "block-only")).toBe(false);
    expect(page.filter((entry) => entry.label === "page")).toHaveLength(0);
    expect(page.some((entry) => entry.choice.kind === "field" && entry.choice.field === "cost")).toBe(true);
  });

  it("keeps a large graph's mounted options bounded while the active ID exists", () => {
    const rows = Array.from({ length: 1000 }, (_, index): RegistryRow => ({
      ...row, normalized_name: `key-${index}`, count_blocks: 1, count_pages: 0,
    }));
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <QueryVocabularyPicker id="many-keys" anchor="block"
      rows={() => rows} onPick={() => {}} />, host);
    try {
      expect(host.querySelectorAll(".qs-vocab-option").length).toBeLessThanOrEqual(40);
      const active = host.querySelector<HTMLInputElement>("[aria-activedescendant]")?.getAttribute("aria-activedescendant");
      expect(active && document.getElementById(active)).not.toBeNull();
    } finally { dispose(); host.remove(); }
  });
});

import { describe, expect, it } from "vitest";
import { displayPropertyPatch, isLegacyBareColumnList, mergeQueryAggregateValue, normalizeQueryDisplayDraft } from "./queryViewProperties";

describe("query display persistence", () => {
  it("writes ordered sorts, columns, summaries and an explicit clear without touching schema", () => {
    const patch = displayPropertyPatch({
      view: "table", sort: [["priority", "desc"], ["page", "asc"]],
      group_by: "", columns: ["page", "status"],
      aggregates: [["", "count"], ["cost", "sum"]], sample: 0,
    });
    expect(patch).toEqual([
      ["tine.view", "table"], ["tine.sort", "priority desc;page asc"],
      ["tine.group-field", ""], ["tine.columns", "page;status"],
      ["tine.col-aggregates", "count;cost=sum"], ["tine.sample", "0"],
    ]);
    expect(patch.some(([key]) => key === "tine.fields")).toBe(false);
  });

  it("rejects unroundtrippable route drafts and keeps a valid draft detached", () => {
    expect(normalizeQueryDisplayDraft({ sort: [["a;b", "asc"]] })).toBeNull();
    const input = { columns: ["page"] };
    const draft = normalizeQueryDisplayDraft(input);
    input.columns.push("cost");
    expect(draft?.columns).toEqual(["page"]);
  });

  it("removes the legacy List override so the ADR 0030 switcher stays property driven", () => {
    expect(displayPropertyPatch({ view: "list" })[0]).toEqual(["tine.view", null]);
  });

  it("names page and block display facts independently", () => {
    expect(displayPropertyPatch({ view: "table", columns: ["name"] }, "page")).toContainEqual(["tine.page-columns", "name"]);
    expect(displayPropertyPatch({ view: "board", columns: ["page"] }, "block")).toContainEqual(["tine.block-columns", "page"]);
    expect(displayPropertyPatch({}, "page")).toContainEqual(["tine.page-display", "1"]);
  });

  it("keeps unsupported aggregate segments and typed schema bytes", () => {
    expect(mergeQueryAggregateValue("count; estimate=median ;cost=sum", [["", "count"], ["cost", "avg"]]))
      .toBe("count; estimate=median ;cost=avg");
    expect(mergeQueryAggregateValue("count; estimate=median ;cost=sum", [["", "count"], ["cost", "sum"]]))
      .toBeUndefined();
    expect(isLegacyBareColumnList("page;cost")).toBe(true);
    expect(isLegacyBareColumnList("cost=number;page")).toBe(false);
  });
});

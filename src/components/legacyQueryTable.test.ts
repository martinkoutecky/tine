import { describe, expect, it } from "vitest";
import { columnKey, compareCells, hostColumns, hostSort } from "./legacyQueryTable";

// OG query_table.cljs `get-sort-state` / `get-columns` / `locale-compare`.
describe("legacy query table host properties", () => {
  it("normalises column keys like OG property keys", () => {
    expect(columnKey(":Created_At")).toBe("created-at");
  });
  it("reads query-properties as an ordered EDN vector, deduplicated", () => {
    expect(hostColumns("[:block :page :Owner_Name :block]")).toEqual(["block", "page", "owner-name"]);
  });
  it("derives columns when the property is absent, empty or unreadable", () => {
    expect(hostColumns(null)).toBeNull();
    expect(hostColumns("[]")).toBeNull();
    expect(hostColumns(":block")).toBeNull();
  });
  it("sorts descending unless query-sort-desc is exactly false; no column means unsorted", () => {
    expect(hostSort("owner", null)).toEqual({ column: "owner", desc: true });
    expect(hostSort("owner", "true")).toEqual({ column: "owner", desc: true });
    expect(hostSort("owner", "false")).toEqual({ column: "owner", desc: false });
    expect(hostSort(null, "false")).toBeNull();
  });
  it("compares numbers numerically and text naturally", () => {
    expect(compareCells("9", "10")).toBeLessThan(0);
    expect(compareCells("item 9", "item 10")).toBeLessThan(0);
  });
});

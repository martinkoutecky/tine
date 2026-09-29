// Ported in meaning from master's queryDisplayDraft.test.ts (og E, family 3): the
// draft normalizer's bounds and all-or-nothing contract, the canonical grouping
// spelling, and the Friendly membership reader. The scoped `queryResultDisplaySettings`
// and singular `display` route field are master's shape; og carries page/block drafts
// on the route directly (`pageDisplay`/`blockDisplay`, tested in router/session tests).
import { describe, expect, it } from "vitest";
import {
  normalizeFriendlyPageMatchScope,
  normalizeQueryDisplayDraft,
  queryDisplaySettings,
  QUERY_DISPLAY_MAX_FIELD,
  QUERY_DISPLAY_MAX_LIST,
  QUERY_DISPLAY_MAX_SAMPLE,
  type QueryDisplayDraft,
} from "./queryDisplayDraft";
import type { ViewSettings } from "./queryIr";

const norm = normalizeQueryDisplayDraft;

describe("query display draft — the three states", () => {
  it("keeps an empty draft as an empty draft, distinct from no draft at all", () => {
    expect(norm({})).toEqual({});
    // A caller distinguishes the two by whether it has a draft to normalize; the
    // normalizer itself never invents one.
    expect(norm(undefined)).toBeNull();
    expect(norm(null)).toBeNull();
  });

  it("carries a full snapshot of every non-view setting", () => {
    const draft = {
      sort: [["priority", "asc"], ["deadline", "desc"]],
      group_by: "prop:state",
      columns: ["state", "prop:owner"],
      aggregates: [["prop:effort", "sum"], ["prop:hours", "avg"]],
      sample: 12,
    };
    expect(norm(draft)).toEqual(draft);
  });

  it("has no spelling for the view, which the route's presentation owns", () => {
    expect(norm({ view: "board", columns: ["state"] })).toEqual({ columns: ["state"] });
  });

  it("drops unsupported keys instead of carrying them forward", () => {
    expect(norm({ columns: ["state"], results: [1, 2, 3], filter: "alpha", tine: {} }))
      .toEqual({ columns: ["state"] });
  });

  it("treats an explicitly undefined member as absent rather than as a value", () => {
    expect(norm({ sort: undefined, columns: ["state"], sample: undefined }))
      .toEqual({ columns: ["state"] });
    expect(Object.hasOwn(norm({ sort: undefined })!, "sort")).toBe(false);
  });

  it("refuses anything that is not a plain object", () => {
    for (const bad of [[], ["columns"], "columns", 3, true]) expect(norm(bad)).toBeNull();
  });
});

describe("query display draft — list members", () => {
  it("preserves duplicates and the exact order the caller chose", () => {
    expect(norm({ columns: ["prop:b", "prop:a", "prop:b"] }))
      .toEqual({ columns: ["prop:b", "prop:a", "prop:b"] });
    expect(norm({ sort: [["deadline", "asc"], ["deadline", "desc"]] }))
      .toEqual({ sort: [["deadline", "asc"], ["deadline", "desc"]] });
  });

  it("accepts a list at the bound and refuses one past it", () => {
    const at = Array.from({ length: QUERY_DISPLAY_MAX_LIST }, (_, i) => `prop:c${i}`);
    expect(norm({ columns: at })).toEqual({ columns: at });
    expect(norm({ columns: [...at, "prop:over"] })).toBeNull();
    expect(norm({ sort: at.map((f) => [f, "asc"]) })).not.toBeNull();
    expect(norm({ sort: [...at, "prop:over"].map((f) => [f, "asc"]) })).toBeNull();
    expect(norm({ aggregates: [...at, "prop:over"].map((f) => [f, "count"]) })).toBeNull();
  });

  it("accepts a field at the length bound and refuses one past it", () => {
    const at = "p".repeat(QUERY_DISPLAY_MAX_FIELD);
    expect(norm({ columns: [at] })).toEqual({ columns: [at] });
    expect(norm({ columns: [`${at}p`] })).toBeNull();
  });

  it("refuses a field carrying the property grammar's own punctuation", () => {
    for (const bad of ["a;b", "a=b", "a\nb", "a\rb", "a\0b"]) {
      expect(norm({ columns: [bad] })).toBeNull();
      expect(norm({ sort: [[bad, "asc"]] })).toBeNull();
      expect(norm({ aggregates: [[bad, "sum"]] })).toBeNull();
    }
  });

  it("refuses an empty or padded field, which would read back as a different one", () => {
    for (const bad of ["", " owner", "owner ", " owner ", "\towner"]) {
      expect(norm({ columns: [bad] })).toBeNull();
      expect(norm({ sort: [[bad, "asc"]] })).toBeNull();
    }
  });

  it("refuses a malformed entry shape", () => {
    expect(norm({ sort: [["priority"]] })).toBeNull();
    expect(norm({ sort: [["priority", "asc", "extra"]] })).toBeNull();
    expect(norm({ sort: ["priority"] })).toBeNull();
    expect(norm({ sort: [{ field: "priority", dir: "asc" }] })).toBeNull();
    expect(norm({ columns: [null] })).toBeNull();
    expect(norm({ columns: [3] })).toBeNull();
    expect(norm({ columns: "state" })).toBeNull();
  });

  it("refuses a sort direction outside asc/desc", () => {
    expect(norm({ sort: [["priority", "asc"]] })).not.toBeNull();
    expect(norm({ sort: [["priority", "desc"]] })).not.toBeNull();
    for (const bad of ["ASC", "ascending", "up", "", 1, null, true]) {
      expect(norm({ sort: [["priority", bad]] })).toBeNull();
    }
  });

  it("refuses one bad entry by refusing the whole draft — never a partial list", () => {
    expect(norm({ columns: ["prop:ok", "bad;name"], sample: 5 })).toBeNull();
    expect(norm({ sort: [["priority", "asc"], ["deadline", "sideways"]] })).toBeNull();
  });
});

describe("query display draft — aggregates", () => {
  it("accepts the fieldless whole-result count", () => {
    expect(norm({ aggregates: [["", "count"]] })).toEqual({ aggregates: [["", "count"]] });
  });

  it("refuses an empty field for sum or avg, which have nothing to add up", () => {
    expect(norm({ aggregates: [["", "sum"]] })).toBeNull();
    expect(norm({ aggregates: [["", "avg"]] })).toBeNull();
  });

  it("accepts exactly the three aggregate functions", () => {
    for (const fn of ["count", "sum", "avg"]) {
      expect(norm({ aggregates: [["prop:effort", fn]] })).toEqual({ aggregates: [["prop:effort", fn]] });
    }
    for (const bad of ["COUNT", "total", "median", "", 1, null]) {
      expect(norm({ aggregates: [["prop:effort", bad]] })).toBeNull();
    }
  });
});

describe("query display draft — grouping identity", () => {
  it("reads the grouping with the canonical helper, not the legacy reader", () => {
    // `prop:state` is the ordinary property named "state"; the bare builtin
    // `state` is the task marker. The legacy reader would turn a bare `status`
    // into `prop:status`; the canonical one refuses it outright.
    expect(norm({ group_by: "prop:state" })).toEqual({ group_by: "prop:state" });
    expect(norm({ group_by: "state" })).toEqual({ group_by: "state" });
    expect(norm({ group_by: "formula:effort" })).toEqual({ group_by: "formula:effort" });
    expect(norm({ group_by: "status" })).toBeNull();
    expect(norm({ group_by: "prop:" })).toBeNull();
    expect(norm({ group_by: "formula:" })).toBeNull();
    expect(norm({ group_by: "formula.effort" })).toBeNull();
  });

  it("treats the empty string as the explicit clear", () => {
    expect(norm({ group_by: "" })).toEqual({ group_by: "" });
  });

  it("allows list punctuation inside a canonical scalar name", () => {
    // The scalar grammar is one field, not a list, so `;` and `=` are ordinary
    // bytes of a property name here — the rule the LIST members enforce must not
    // leak across.
    expect(norm({ group_by: "prop:a;b" })).toEqual({ group_by: "prop:a;b" });
    expect(norm({ group_by: "prop:a=b" })).toEqual({ group_by: "prop:a=b" });
    expect(norm({ group_by: "formula:a;b=c" })).toEqual({ group_by: "formula:a;b=c" });
    // …and the same name is still refused in a column list.
    expect(norm({ columns: ["prop:a;b"] })).toBeNull();
  });

  it("still refuses line breaks and NUL, and a name past the length bound", () => {
    for (const bad of ["prop:a\nb", "prop:a\rb", "prop:a\0b"]) {
      expect(norm({ group_by: bad })).toBeNull();
    }
    const at = `prop:${"p".repeat(QUERY_DISPLAY_MAX_FIELD - 5)}`;
    expect(norm({ group_by: at })).toEqual({ group_by: at });
    expect(norm({ group_by: `${at}p` })).toBeNull();
  });

  it("refuses a non-string grouping", () => {
    for (const bad of [null, 3, [], {}, true]) expect(norm({ group_by: bad })).toBeNull();
  });
});

describe("query display draft — sample", () => {
  it("accepts zero and the u32 maximum", () => {
    expect(norm({ sample: 0 })).toEqual({ sample: 0 });
    expect(norm({ sample: QUERY_DISPLAY_MAX_SAMPLE })).toEqual({ sample: QUERY_DISPLAY_MAX_SAMPLE });
  });

  it("refuses anything that is not a whole in-range number", () => {
    for (const bad of [
      -1, 0.5, QUERY_DISPLAY_MAX_SAMPLE + 1, Number.NaN, Number.POSITIVE_INFINITY,
      Number.MAX_SAFE_INTEGER + 2, null, "12", true, [],
    ]) {
      expect(norm({ sample: bad })).toBeNull();
    }
  });
});

describe("query display draft — size and aliasing", () => {
  it("refuses a draft whose normalized JSON exceeds the size bound", () => {
    // Within every per-field and per-list bound, yet far past the whole-draft
    // one: 64 columns of 512 units each is ~33k, and three such lists overflow.
    const big = Array.from({ length: QUERY_DISPLAY_MAX_LIST }, (_, i) =>
      `${String(i).padStart(3, "0")}${"p".repeat(QUERY_DISPLAY_MAX_FIELD - 3)}`);
    expect(norm({ columns: big })).not.toBeNull();
    expect(norm({
      columns: big,
      sort: big.map((f) => [f, "asc"]),
      aggregates: big.map((f) => [f, "count"]),
    })).toBeNull();
  });

  it("returns fresh lists and tuples, so a mutating caller cannot reach in", () => {
    const columns = ["prop:a"];
    const sort: [string, string][] = [["priority", "asc"]];
    const aggregates: [string, string][] = [["prop:effort", "sum"]];
    const normalized = norm({ columns, sort, aggregates })!;

    columns.push("prop:b");
    sort[0][1] = "desc";
    aggregates.push(["prop:x", "avg"]);

    expect(normalized).toEqual({
      columns: ["prop:a"],
      sort: [["priority", "asc"]],
      aggregates: [["prop:effort", "sum"]],
    });
    expect(normalized.columns).not.toBe(columns);
    expect(normalized.sort![0]).not.toBe(sort[0]);
  });
});

describe("queryDisplaySettings", () => {
  const parsed: ViewSettings = {
    view: "list",
    sort: [["deadline", "asc"]],
    group_by: "prop:area",
    columns: ["state"],
    sample: 7,
  };

  it("always takes the view from the route's presentation, never from either input", () => {
    expect(queryDisplaySettings(undefined, parsed, "board").view).toBe("board");
    expect(queryDisplaySettings({}, parsed, "table").view).toBe("table");
    expect(queryDisplaySettings({ columns: ["prop:a"] }, parsed, "search").view).toBe("search");
  });

  it("inherits every parsed non-view setting when there is no draft", () => {
    expect(queryDisplaySettings(undefined, parsed, "list")).toEqual({ ...parsed, view: "list" });
    expect(queryDisplaySettings(undefined, undefined, "list")).toEqual({ view: "list" });
  });

  it("clears every non-view setting for an empty draft", () => {
    expect(queryDisplaySettings({}, parsed, "list")).toEqual({ view: "list" });
  });

  it("replaces the parsed half wholesale rather than merging member by member", () => {
    // `columns` is stated, so the parsed `sort`, `group_by` and `sample` are
    // gone: a draft is a complete snapshot, not a patch.
    expect(queryDisplaySettings({ columns: ["prop:a"] }, parsed, "table"))
      .toEqual({ view: "table", columns: ["prop:a"] });
  });

  it("keeps an explicitly cleared grouping distinct from an absent one", () => {
    expect(queryDisplaySettings({ group_by: "" }, parsed, "board"))
      .toEqual({ view: "board", group_by: "" });
    expect(Object.hasOwn(queryDisplaySettings({}, parsed, "board"), "group_by")).toBe(false);
  });

  it("copies lists out of both inputs so the result can be edited freely", () => {
    const draft: QueryDisplayDraft = { columns: ["prop:a"], sort: [["priority", "asc"]] };
    const fromDraft = queryDisplaySettings(draft, parsed, "table");
    expect(fromDraft.columns).not.toBe(draft.columns);
    expect(fromDraft.sort![0]).not.toBe(draft.sort![0]);

    const inherited = queryDisplaySettings(undefined, parsed, "table");
    expect(inherited.columns).not.toBe(parsed.columns);
    expect(inherited.sort![0]).not.toBe(parsed.sort![0]);
  });
});

describe("Friendly page match scope", () => {
  it("accepts only the three exact membership modes", () => {
    for (const scope of ["names", "content", "both"] as const) {
      expect(normalizeFriendlyPageMatchScope(scope)).toBe(scope);
    }
    for (const bad of [undefined, null, "", "name", "all", "NAMES", 1, {}]) {
      expect(normalizeFriendlyPageMatchScope(bad)).toBeNull();
    }
  });
});

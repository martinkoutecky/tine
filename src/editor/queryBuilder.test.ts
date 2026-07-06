import { describe, it, expect } from "vitest";
import {
  parseQuery,
  toDsl,
  clauseLabel,
  addChild,
  removeAt,
  replaceAt,
  wrapAt,
  unwrapAt,
  setOp,
  type Clause,
} from "./queryBuilder";

// Round-trip helper: DSL -> tree -> DSL should be stable for supported forms.
const roundtrip = (dsl: string) => toDsl(parseQuery(dsl));

describe("parse + serialize round-trip", () => {
  it("page / tag refs", () => {
    expect(roundtrip("[[Foo]]")).toBe("[[Foo]]");
    expect(roundtrip("#bar")).toBe("[[bar]]"); // tag normalizes to page-ref (same predicate)
  });

  it("boolean operators", () => {
    expect(roundtrip("(and [[A]] [[B]])")).toBe("(and [[A]] [[B]])");
    expect(roundtrip("(or [[A]] [[B]])")).toBe("(or [[A]] [[B]])");
    expect(roundtrip("(not [[A]])")).toBe("(not [[A]])");
  });

  it("nested operators", () => {
    expect(roundtrip("(and [[A]] (or [[B]] [[C]]))")).toBe("(and [[A]] (or [[B]] [[C]]))");
  });

  it("task / priority / property / dates", () => {
    expect(roundtrip("(task TODO DOING)")).toBe("(task TODO DOING)");
    expect(roundtrip("(priority A B)")).toBe("(priority A B)");
    expect(roundtrip("(property type book)")).toBe("(property type book)");
    expect(roundtrip("(property public)")).toBe("(property public)");
    expect(roundtrip("(scheduled)")).toBe("(scheduled)");
    expect(roundtrip("(deadline)")).toBe("(deadline)");
    expect(roundtrip("(between [[Jan 1st, 2021]] [[Jan 1st, 2100]])")).toBe(
      "(between [[Jan 1st, 2021]] [[Jan 1st, 2100]])"
    );
  });

  it("new OG-parity filters round-trip", () => {
    expect(roundtrip("(page Project/Alpha)")).toBe("(page Project/Alpha)");
    expect(roundtrip("(namespace Project)")).toBe("(namespace Project)");
    expect(roundtrip("(page-property type book)")).toBe("(page-property type book)");
    expect(roundtrip("(page-property public)")).toBe("(page-property public)");
    expect(roundtrip("(page-tags research active)")).toBe("(page-tags research active)");
    expect(roundtrip('"full text"')).toBe('"full text"');
    // Relative/keyword/ISO bounds stay bare; journal titles keep their [[ ]].
    expect(roundtrip("(between -7d +7d)")).toBe("(between -7d +7d)");
    expect(roundtrip("(between today tomorrow)")).toBe("(between today tomorrow)");
    expect(roundtrip("(between 2026-01-01 2026-12-31)")).toBe("(between 2026-01-01 2026-12-31)");
  });

  it("between field selector and journal predicate round-trip", () => {
    expect(roundtrip("(journal)")).toBe("(journal)");
    expect(roundtrip("(between journal -30d today)")).toBe("(between journal -30d today)");
    expect(roundtrip("(between scheduled -7d +7d)")).toBe("(between scheduled -7d +7d)");
    expect(roundtrip("(between deadline today +14d)")).toBe("(between deadline today +14d)");
    // The motivating query.
    expect(roundtrip("(and (task TODO) (between journal -30d today))")).toBe(
      "(and (task TODO) (between journal -30d today))"
    );
  });

  it("quotes property values with spaces", () => {
    const t = parseQuery('(property title "War and Peace")');
    expect(toDsl(t)).toBe('(property title "War and Peace")');
  });

  it("escapes quotes/parens/backslashes in string values so they round-trip", () => {
    // Full-text content with an embedded quote — was serialized to `"foo "bar""`
    // and silently truncated to `foo `; now escaped + unescaped symmetrically.
    const content: Clause = { kind: "op", op: "and", children: [{ kind: "content", text: 'foo "bar"' }] };
    expect(toDsl(content)).toBe('"foo \\"bar\\""');
    expect(parseQuery(toDsl(content))).toEqual(content);

    // Property value with whitespace AND a quote.
    const prop: Clause = { kind: "op", op: "and", children: [{ kind: "property", key: "title", value: 'a "b" c' }] };
    expect(parseQuery(toDsl(prop))).toEqual(prop);

    // A `)` in a value must force quoting so it can't close the form early.
    const paren: Clause = { kind: "op", op: "and", children: [{ kind: "property", key: "note", value: "see (x)" }] };
    expect(toDsl(paren)).toBe('(property note "see (x)")');
    expect(parseQuery(toDsl(paren))).toEqual(paren);

    // A literal backslash round-trips (escaped on write, unescaped on read).
    const bs: Clause = { kind: "op", op: "and", children: [{ kind: "content", text: "a\\b" }] };
    expect(parseQuery(toDsl(bs))).toEqual(bs);

    // Generated value forcing quoting AND containing a backslash (Windows path).
    const winpath: Clause = { kind: "op", op: "and", children: [{ kind: "property", key: "path", value: "C:\\program files" }] };
    expect(parseQuery(toDsl(winpath))).toEqual(winpath);

    // Only `\"` and `\\` are escapes: a HAND-AUTHORED single backslash before a
    // normal char is kept literally, so `"C:\tmp"` doesn't become `C:tmp`.
    expect(parseQuery('"a\\q b"')).toEqual({
      kind: "op",
      op: "and",
      children: [{ kind: "content", text: "a\\q b" }],
    });
  });

  it("empty query is empty string", () => {
    expect(roundtrip("")).toBe("");
    expect(toDsl(parseQuery(""))).toBe("");
  });

  it("unknown form is preserved verbatim", () => {
    // (sample 5) isn't in Tine's runnable grammar — keep it rather than drop it.
    expect(roundtrip("(sample 5)")).toBe("(sample 5)");
  });

  it("preserves a NESTED unknown form and the clauses after it", () => {
    // The raw fallback must capture a BALANCED extent — a lazy stop at the first
    // ")" split `(custom (nested x))` at the inner ")", dropping [[B]] and writing
    // an unbalanced fragment back to the {{query}} block.
    const dsl = "(and [[A]] (custom (nested x)) [[B]])";
    expect(roundtrip(dsl)).toBe(dsl);
    expect(parseQuery(dsl)).toEqual({
      kind: "op",
      op: "and",
      children: [
        { kind: "page", name: "A" },
        { kind: "raw", text: "(custom (nested x))" },
        { kind: "page", name: "B" },
      ],
    });
  });
});

describe("tree shape", () => {
  it("bare clause is wrapped in an and-root", () => {
    const t = parseQuery("[[Foo]]");
    expect(t).toEqual({ kind: "op", op: "and", children: [{ kind: "page", name: "Foo" }] });
  });

  it("single-child and simplifies on serialize", () => {
    const root: Clause = { kind: "op", op: "and", children: [{ kind: "page", name: "X" }] };
    expect(toDsl(root)).toBe("[[X]]");
  });
});

describe("mutations", () => {
  it("addChild appends to the addressed op", () => {
    let t = parseQuery("[[A]]"); // and-root with one child
    t = addChild(t, [], { kind: "page", name: "B" });
    expect(toDsl(t)).toBe("(and [[A]] [[B]])");
  });

  it("removeAt deletes and re-simplifies", () => {
    let t = parseQuery("(and [[A]] [[B]])");
    t = removeAt(t, [1]);
    expect(toDsl(t)).toBe("[[A]]"); // back to single child -> simplified
  });

  it("removing the last child yields an empty query", () => {
    let t = parseQuery("[[A]]");
    t = removeAt(t, [0]);
    expect(toDsl(t)).toBe("");
  });

  it("replaceAt swaps a clause", () => {
    let t = parseQuery("(and [[A]] [[B]])");
    t = replaceAt(t, [1], { kind: "task", markers: ["TODO"] });
    expect(toDsl(t)).toBe("(and [[A]] (task TODO))");
  });

  it("wrapAt wraps a clause in a new operator", () => {
    let t = parseQuery("(and [[A]] [[B]])");
    t = wrapAt(t, [1], "or");
    expect(toDsl(t)).toBe("(and [[A]] (or [[B]]))");
    t = addChild(t, [1], { kind: "page", name: "C" });
    expect(toDsl(t)).toBe("(and [[A]] (or [[B]] [[C]]))");
  });

  it("unwrapAt promotes children and prunes empties", () => {
    let t = parseQuery("(and [[A]] (or [[B]] [[C]]))");
    t = unwrapAt(t, [1]);
    expect(toDsl(t)).toBe("(and [[A]] [[B]] [[C]])");
  });

  it("setOp flips and<->or on the root", () => {
    let t = parseQuery("(and [[A]] [[B]])");
    t = setOp(t, [], "or");
    expect(toDsl(t)).toBe("(or [[A]] [[B]])");
  });
});

describe("labels", () => {
  it("renders human-readable chips", () => {
    expect(clauseLabel({ kind: "page", name: "Foo" })).toBe("Foo");
    expect(clauseLabel({ kind: "task", markers: ["NOW", "LATER"] })).toBe("task: NOW | LATER");
    expect(clauseLabel({ kind: "property", key: "type", value: "book" })).toBe("type: book");
    expect(clauseLabel({ kind: "property", key: "public", value: null })).toBe("public: any");
    expect(clauseLabel({ kind: "between", field: "any", start: "-7d", end: "+7d" })).toBe(
      "between: -7d ~ +7d"
    );
    expect(clauseLabel({ kind: "between", field: "journal", start: "-30d", end: "today" })).toBe(
      "journal between: -30d ~ today"
    );
  });
});

describe("sort-by clause", () => {
  it("parses and round-trips", () => {
    expect(roundtrip("(sort-by priority desc)")).toBe("(sort-by priority desc)");
    expect(roundtrip("(sort-by page)")).toBe("(sort-by page asc)"); // default asc
    expect(roundtrip("(and (task TODO) (sort-by priority desc))")).toBe(
      "(and (task TODO) (sort-by priority desc))"
    );
  });
  it("has a readable label", () => {
    expect(clauseLabel({ kind: "sortBy", field: "priority", dir: "desc" })).toBe("sort: priority ↓");
  });
});

describe("aggregate + group-by directives", () => {
  it("parse and round-trip", () => {
    expect(roundtrip("(aggregate count)")).toBe("(aggregate count)");
    expect(roundtrip("(aggregate sum hours)")).toBe("(aggregate sum hours)");
    expect(roundtrip("(aggregate avg score)")).toBe("(aggregate avg score)");
    expect(roundtrip("(group-by page)")).toBe("(group-by page)");
    expect(roundtrip("(group-by status)")).toBe("(group-by status)");
    // Alongside filters + a group-by, all round-trip together.
    expect(roundtrip("(and (task TODO) (group-by page) (aggregate count))")).toBe(
      "(and (task TODO) (group-by page) (aggregate count))"
    );
    // `average` normalizes to `avg`.
    expect(roundtrip("(aggregate average score)")).toBe("(aggregate avg score)");
  });
  it("has readable labels", () => {
    expect(clauseLabel({ kind: "aggregate", agg: "count", field: null })).toBe("count");
    expect(clauseLabel({ kind: "aggregate", agg: "sum", field: "hours" })).toBe("sum of hours");
    expect(clauseLabel({ kind: "groupBy", field: "status" })).toBe("group by status");
  });
});

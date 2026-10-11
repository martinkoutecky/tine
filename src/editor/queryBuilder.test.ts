// The builder's model, over the IR.
//
// The old file in this place tested a parser and a printer that no longer exist
// here: `parseQuery`/`toDsl` round trips, `clauseToAdvanced` conversions, DSL
// quoting. Those were the frontend's second query language, and the tests were
// its specification — keeping them would have been keeping the twin (I-12, D-14).
// What is testable here now is what the builder actually owns: the IR SHAPE each
// picker emits, the phrase each node reads as, and the tree edits.

import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  ADVANCED_PHRASE,
  JOURNAL_ANY_RANGE,
  MAX_QUERY_BUILDER_DEPTH,
  anyTaskFilter,
  isAnyTaskStatus,
  OG_TASK_MARKERS,
  addChild,
  betweenFilter,
  builderLeafKind,
  builderRoot,
  contentFilter,
  currentAgg,
  currentGroup,
  currentSort,
  encodePropertyLeaf,
  escapeLike,
  filterChildren,
  filterLabel,
  filterValueLabel,
  filterPhrase,
  groupSelected,
  groupWithPrevious,
  isDisabledAt,
  isEmptyFilter,
  journalFilter,
  moveAcross,
  moveSibling,
  namespaceFilter,
  onPageFilter,
  pagePropertyFilter,
  pageRefFilter,
  pageTagsFilter,
  planningFilter,
  plainLikeSubstring,
  priorityFilter,
  propertyFilter,
  propertyLeafTest,
  propertyOperatorArity,
  propertyOperatorLabel,
  propertyOperators,
  querySentence,
  readLikePattern,
  removeAt,
  replaceAt,
  searchFilter,
  setOp,
  sortLabel,
  taskFilter,
  toggleDisabledAt,
  unwrapAt,
  withAgg,
  withGroup,
  withSort,
  wrapAt,
  type PropertyOperatorId,
} from "./queryBuilder";

it("treats an empty or as false, matching the engine", () => {
  expect(isEmptyFilter({ kind: "or", items: [] })).toBe(false);
  expect(isEmptyFilter({ kind: "and", items: [] })).toBe(true);
});
import {
  forEachFilter,
  type Cardinality,
  type Filter,
  type ObservedType,
  type ViewSettings,
} from "./queryIr";

const A = pageRefFilter("A");
const B = pageRefFilter("B");
const C = pageRefFilter("C");

// **The shapes are the contract with `crates/tine-core/src/query/og.rs`.**
// A chip added here and the same filter typed as OG text must be the SAME IR, or
// the round trip (add → print → re-parse) silently rewrites the user's query. So
// these assert the exact JSON, each naming the `og.rs` construction it mirrors.
describe("leaf constructors mirror the OG parser's IR", () => {
  it("`[[x]]` / `#x` is `refs any (name = x)` (Filter::page_ref, Q2)", () => {
    expect(pageRefFilter("Foo")).toEqual({
      kind: "leaf",
      leaf: {
        kind: "rel",
        rel: "refs",
        quant: "any",
        pred: { kind: "leaf", leaf: { kind: "attr", attr: "name", op: "eq", value: { kind: "text", text: "Foo" } } },
      },
    });
  });

  it("`(task …)` is `task in [...]`, and an empty list stays empty: OG's bare `(task)` adds no condition (D-18)", () => {
    expect(taskFilter(["TODO", "DOING"])).toEqual({
      kind: "leaf",
      leaf: {
        kind: "attr",
        attr: "task",
        op: "in",
        value: { kind: "list", items: [{ kind: "text", text: "TODO" }, { kind: "text", text: "DOING" }] },
      },
    });
    // og.rs reads a bare `(task)` as `task in []` (Filter::is_dropped_clause);
    // a TS default list here would be a second, different meaning (GH #422).
    const empty = taskFilter([]);
    expect(empty.kind === "leaf" && empty.leaf.kind === "attr" && empty.leaf.value).toEqual({ kind: "list", items: [] });
    expect(filterLabel(empty)).toBe("task: no markers");
  });

  it("`(priority …)` keeps its levels; an empty list is OG's bare `(priority)` (D-18)", () => {
    const empty = priorityFilter([]);
    expect(empty.kind === "leaf" && empty.leaf.kind === "attr" && empty.leaf.value).toEqual({ kind: "list", items: [] });
    expect(filterLabel(empty)).toBe("priority: no levels");
    expect(filterLabel(priorityFilter(["A"]))).toBe("priority: A");
  });

  it("`(property k v)` is `props any (key = k AND value = v)` (og.rs::property_leaf)", () => {
    expect(propertyFilter("type", "book")).toEqual({
      kind: "leaf",
      leaf: {
        kind: "rel",
        rel: "props",
        quant: "any",
        pred: {
          kind: "and",
          items: [
            { kind: "leaf", leaf: { kind: "attr", attr: "key", op: "eq", value: { kind: "text", text: "type" } } },
            { kind: "leaf", leaf: { kind: "attr", attr: "value", op: "eq", value: { kind: "text", text: "book" } } },
          ],
        },
      },
    });
    // `(property k)` with no value is the bare key test, not `value = ""`.
    expect(propertyFilter("public", null)).toEqual({
      kind: "leaf",
      leaf: {
        kind: "rel",
        rel: "props",
        quant: "any",
        pred: { kind: "leaf", leaf: { kind: "attr", attr: "key", op: "eq", value: { kind: "text", text: "public" } } },
      },
    });
  });

  it("page-row filters are read THROUGH the page relation (og.rs::through_page)", () => {
    const throughPage = (inner: unknown) => ({
      kind: "leaf",
      leaf: { kind: "rel", rel: "page", quant: "any", pred: inner },
    });
    expect(onPageFilter("Alpha")).toEqual(
      throughPage({ kind: "leaf", leaf: { kind: "attr", attr: "name", op: "eq", value: { kind: "text", text: "Alpha" } } }),
    );
    // OG `(namespace x)` is recursive membership: the name starts with `x/`.
    expect(namespaceFilter("Projects")).toEqual(
      throughPage({ kind: "leaf", leaf: { kind: "attr", attr: "name", op: "starts_with", value: { kind: "text", text: "Projects/" } } }),
    );
    // OG has no `(journal)`: "In a journal page" is OG's own `between` over a range wider than any journal date (GH #619).
    expect(journalFilter()).toEqual(
      throughPage({
        kind: "leaf",
        leaf: {
          kind: "attr",
          attr: "day",
          op: "between",
          value: {
            kind: "list",
            items: [
              { kind: "date", literal: JOURNAL_ANY_RANGE[0] },
              { kind: "date", literal: JOURNAL_ANY_RANGE[1] },
            ],
          },
        },
      }),
    );
    expect(pagePropertyFilter("fach", null)).toEqual(throughPage(propertyFilter("fach", null)));
  });

  it("`(page-tags …)` is the page's `tags` property with a set test", () => {
    expect(pageTagsFilter(["research"])).toEqual({
      kind: "leaf",
      leaf: {
        kind: "rel",
        rel: "page",
        quant: "any",
        pred: {
          kind: "leaf",
          leaf: {
            kind: "rel",
            rel: "props",
            quant: "any",
            pred: {
              kind: "and",
              items: [
                { kind: "leaf", leaf: { kind: "attr", attr: "key", op: "eq", value: { kind: "text", text: "tags" } } },
                { kind: "leaf", leaf: { kind: "attr", attr: "value", op: "in", value: { kind: "list", items: [{ kind: "text", text: "research" }] } } },
              ],
            },
          },
        },
      },
    });
  });

  it("`(scheduled)` / `(deadline)` are presence leaves, never a date", () => {
    expect(planningFilter("scheduled")).toEqual({
      kind: "leaf",
      leaf: { kind: "attr", attr: "scheduled", op: "is_set", value: { kind: "none" } },
    });
  });

  // og.rs::bounded — two bounds, one bound, no bounds are three DIFFERENT leaves.
  // The IR keeps the literal unresolved, so a cached query never pins a day.
  it("`(between …)` degrades exactly as og.rs::bounded does", () => {
    expect(betweenFilter("scheduled", "-7d", "+7d")).toEqual({
      kind: "leaf",
      leaf: {
        kind: "attr",
        attr: "scheduled",
        op: "between",
        value: { kind: "list", items: [{ kind: "date", literal: "-7d" }, { kind: "date", literal: "+7d" }] },
      },
    });
    expect(betweenFilter("deadline", "today", "")).toEqual({
      kind: "leaf",
      leaf: { kind: "attr", attr: "deadline", op: "ge", value: { kind: "date", literal: "today" } },
    });
    expect(betweenFilter("deadline", "", "+14d")).toEqual({
      kind: "leaf",
      leaf: { kind: "attr", attr: "deadline", op: "le", value: { kind: "date", literal: "+14d" } },
    });
    expect(betweenFilter("deadline", "", "")).toEqual({
      kind: "leaf",
      leaf: { kind: "attr", attr: "deadline", op: "is_set", value: { kind: "none" } },
    });
  });

  it("`(between any …)` expands to the faithful three-way or", () => {
    const any = betweenFilter("any", "-7d", "+7d");
    expect(any.kind).toBe("or");
    expect(any.kind === "or" && any.items).toEqual([
      betweenFilter("journal", "-7d", "+7d"),
      betweenFilter("scheduled", "-7d", "+7d"),
      betweenFilter("deadline", "-7d", "+7d"),
    ]);
    // The journal form goes through the page; the planning forms do not.
    expect(betweenFilter("journal", "-7d", "+7d")).toEqual({
      kind: "leaf",
      leaf: {
        kind: "rel",
        rel: "page",
        quant: "any",
        pred: {
          kind: "leaf",
          leaf: {
            kind: "attr",
            attr: "day",
            op: "between",
            value: { kind: "list", items: [{ kind: "date", literal: "-7d" }, { kind: "date", literal: "+7d" }] },
          },
        },
      },
    });
  });
});

// `og::escape_like` and its inverse `og::plain_like_substring`. The builder emits
// the pattern synchronously as the chip commits, so these two are transcribed
// rather than shared — and an unescaped `%` would make `50%` match everything
// after the `50`.
describe("LIKE escaping (transcribed from og::escape_like)", () => {
  it("treats the user's `%`, `_` and `\\` as data", () => {
    expect(escapeLike("50% _ok_ a\\b")).toBe("50\\% \\_ok\\_ a\\\\b");
  });

  it("a content chip carries the `%…%` substring pattern", () => {
    expect(contentFilter("100% done")).toEqual({
      kind: "leaf",
      leaf: { kind: "attr", attr: "content", op: "like", value: { kind: "text", text: "%100\\% done%" } },
    });
  });

  it("round-trips through the printer's inverse", () => {
    for (const text of ["plain", "100% done", "a_b", "back\\slash", "%%%"]) {
      expect(plainLikeSubstring(`%${escapeLike(text)}%`)).toBe(text);
    }
  });

  it("refuses a pattern that is not a plain substring, so the label never lies", () => {
    // An unescaped wildcard in the middle is a real pattern, not a literal.
    expect(plainLikeSubstring("%a%b%")).toBeNull();
    expect(plainLikeSubstring("no percent signs")).toBeNull();
  });
});

describe("builderLeafKind", () => {
  it("recognises every shape a picker can emit", () => {
    expect(builderLeafKind(pageRefFilter("A"))).toBe("page");
    expect(builderLeafKind(taskFilter(["TODO"]))).toBe("task");
    expect(builderLeafKind(priorityFilter(["A"]))).toBe("priority");
    expect(builderLeafKind(propertyFilter("k", "v"))).toBe("property");
    expect(builderLeafKind(propertyFilter("k", null))).toBe("property");
    expect(builderLeafKind(pagePropertyFilter("k", "v"))).toBe("pageProperty");
    expect(builderLeafKind(pageTagsFilter(["t"]))).toBe("pageTags");
    expect(builderLeafKind(planningFilter("scheduled"))).toBe("scheduled");
    expect(builderLeafKind(planningFilter("deadline"))).toBe("deadline");
    expect(builderLeafKind(journalFilter())).toBe("journal");
    expect(builderLeafKind(onPageFilter("P"))).toBe("onPage");
    expect(builderLeafKind(namespaceFilter("N"))).toBe("namespace");
    expect(builderLeafKind(contentFilter("x"))).toBe("content");
    expect(builderLeafKind(searchFilter("a or b"))).toBe("search");
    expect(builderLeafKind(betweenFilter("scheduled", "-7d", "+7d"))).toBe("between");
    expect(builderLeafKind(betweenFilter("journal", "-7d", "+7d"))).toBe("between");
  });

  it("returns null — not a wrong guess — for a shape no picker can re-collect", () => {
    // A typed comparison, a quantifier the pickers do not offer, and a boolean
    // node are all things the builder RENDERS but must not offer to "edit" with a
    // form that would rewrite them into something else.
    const typed: Filter = {
      kind: "leaf",
      leaf: { kind: "attr", attr: "content", op: "gt", value: { kind: "number", number: 3 } },
    };
    expect(builderLeafKind(typed)).toBeNull();
    const everyChild: Filter = {
      kind: "leaf",
      leaf: { kind: "rel", rel: "children", quant: "every", pred: taskFilter(["TODO"]) },
    };
    expect(builderLeafKind(everyChild)).toBeNull();
    expect(builderLeafKind({ kind: "and", items: [] })).toBeNull();
  });
});

// §7.5's anti-Jira property: "every query the parser accepts renders in the
// builder, invalid ones included". The golden wire fixture is the widest IR the
// two sides agree on, so labelling every node of it is the strongest cheap
// statement of totality available here.
describe("filterValueLabel: the row's value cell holds VALUES", () => {
  // The row names its field and its operator in two cells of its own, so the
  // third must not say them a third time: `Task marker ▾ | is any of ▾ | task:
  // TODO | DOING` is what the sentence says, not what a value cell says.
  it("drops the prose and keeps the operands", () => {
    expect(filterValueLabel(taskFilter(["NOW", "LATER"]))).toBe("NOW | LATER");
    expect(filterValueLabel(propertyFilter("type", "book"))).toBe("book");
    expect(filterValueLabel(onPageFilter("Alpha"))).toBe("Alpha");
    expect(filterValueLabel(namespaceFilter("Projects"))).toBe("Projects");
    expect(filterValueLabel(pageRefFilter("Foo"))).toBe("Foo");
    expect(filterValueLabel(contentFilter("100% done"))).toBe("100% done");
    expect(filterValueLabel(betweenFilter("scheduled", "-7d", "+7d"))).toBe("7 days ago to 7 days ahead");
  });

  it("keeps the whole phrase when a condition compares against nothing", () => {
    // An empty cell would be nothing to read and nothing to click.
    expect(filterValueLabel(journalFilter())).toBe("on journal page");
  });

  it("never says more than the sentence does", () => {
    for (const filter of [
      taskFilter(["NOW"]),
      propertyFilter("type", "book"),
      pageTagsFilter(["a", "b"]),
      pagePropertyFilter("fach", "x"),
      journalFilter(),
    ]) {
      expect(filterLabel(filter).length).toBeGreaterThanOrEqual(filterValueLabel(filter).length);
    }
  });
});

describe("filterLabel is total over the IR", () => {
  const fixture = JSON.parse(
    readFileSync(
      join(fileURLToPath(new URL("../..", import.meta.url)), "crates/tine-core/src/query/fixtures/query-ir/filter.json"),
      "utf8",
    ),
  ) as Filter;

  it("phrases every node of the golden filter fixture without throwing", () => {
    let nodes = 0;
    forEachFilter(fixture, (node) => {
      nodes += 1;
      const label = filterLabel(node);
      expect(typeof label).toBe("string");
      expect(label.length).toBeGreaterThan(0);
    });
    expect(nodes).toBeGreaterThan(20);
  });

  it("keeps an unparsed span's own text as its phrase (§4.3.2 R4)", () => {
    expect(filterLabel({ kind: "raw", text: "(frobnicate x)", diagnostic_kind: "unknown_head" }))
      .toBe("(frobnicate x)");
  });

  it("marks a disabled subtree rather than hiding it (§3.5, Q12)", () => {
    expect(filterLabel({ kind: "off", inner: pageRefFilter("A") })).toBe("A (off)");
  });

  it("reads the friendly shapes friendlily", () => {
    expect(filterLabel(pageRefFilter("Foo"))).toBe("Foo");
    expect(filterLabel(taskFilter(["NOW", "LATER"]))).toBe("task: NOW | LATER");
    expect(filterLabel(propertyFilter("type", "book"))).toBe("type: book");
    expect(filterLabel(propertyFilter("public", null))).toBe("public: any");
    expect(filterLabel(pagePropertyFilter("fach", "x"))).toBe("page fach: x");
    expect(filterLabel(pageTagsFilter(["a", "b"]))).toBe("page tags: a | b");
    expect(filterLabel(onPageFilter("Alpha"))).toBe("page: Alpha");
    expect(filterLabel(namespaceFilter("Projects"))).toBe("namespace: Projects");
    expect(filterLabel(journalFilter())).toBe("on journal page");
    expect(filterLabel(contentFilter("100% done"))).toBe('text: "100% done"');
    expect(filterLabel(betweenFilter("journal", "-30d", "today"))).toBe("journal date: last 30 days");
    expect(filterLabel(betweenFilter("scheduled", "-7d", "+7d"))).toBe("scheduled: 7 days ago to 7 days ahead");
  });
});

describe("builderRoot", () => {
  it("adopts a bare leaf so `+ add filter` has somewhere to add", () => {
    expect(builderRoot(A)).toEqual({ kind: "and", items: [A] });
  });
  it("keeps an `or` root as an `or`, so adding a filter does not change the query", () => {
    expect(builderRoot({ kind: "or", items: [A, B] })).toEqual({ kind: "or", items: [A, B] });
  });
  it("reads `true` as the empty query", () => {
    expect(builderRoot({ kind: "true" })).toEqual({ kind: "and", items: [] });
  });
});

describe("tree edits", () => {
  const root = (items: Filter[]): Filter => ({ kind: "and", items });

  it("addChild appends to the addressed boolean node", () => {
    expect(addChild(root([A]), [], B)).toEqual(root([A, B]));
    expect(addChild(root([{ kind: "or", items: [A] }]), [0], B)).toEqual(root([{ kind: "or", items: [A, B] }]));
  });

  it("addChild refuses a `not`, which holds exactly one child", () => {
    const tree = root([{ kind: "not", inner: A }]);
    expect(addChild(tree, [0], B)).toBe(tree);
  });

  it("removeAt deletes, and prunes the empty node it leaves behind", () => {
    expect(removeAt(root([A, B]), [1])).toEqual(root([A]));
    expect(removeAt(root([A]), [0])).toEqual(root([]));
    // The `or` becomes childless and goes with it, rather than staying as a
    // vacuous `(or)` that would match everything.
    expect(removeAt(root([A, { kind: "or", items: [B] }]), [1, 0])).toEqual(root([A]));
  });

  it("replaceAt swaps one node", () => {
    expect(replaceAt(root([A, B]), [1], taskFilter(["TODO"]))).toEqual(root([A, taskFilter(["TODO"])]));
  });

  it("wrapAt wraps in a new boolean node, `not` included", () => {
    expect(wrapAt(root([A, B]), [1], "or")).toEqual(root([A, { kind: "or", items: [B] }]));
    expect(wrapAt(root([A]), [0], "not")).toEqual(root([{ kind: "not", inner: A }]));
  });

  it("unwrapAt promotes children, through `not` and `off` too", () => {
    expect(unwrapAt(root([A, { kind: "or", items: [B, C] }]), [1])).toEqual(root([A, B, C]));
    expect(unwrapAt(root([{ kind: "not", inner: A }]), [0])).toEqual(root([A]));
    expect(unwrapAt(root([{ kind: "off", inner: A }]), [0])).toEqual(root([A]));
  });

  it("setOp flips the root and any inner node", () => {
    expect(setOp(root([A, B]), [], "or")).toEqual({ kind: "or", items: [A, B] });
    expect(setOp(root([{ kind: "and", items: [A, B] }]), [0], "or")).toEqual(
      root([{ kind: "or", items: [A, B] }]),
    );
  });

  it("a path that no longer addresses anything is a no-op, not a corruption", () => {
    // A popover can outlive the tree it was opened over; an edit at a stale `loc`
    // must leave the query alone rather than delete a neighbour.
    const tree = root([A]);
    expect(removeAt(tree, [7])).toBe(tree);
    expect(replaceAt(tree, [1, 2], B)).toBe(tree);
    expect(wrapAt(tree, [], "or")).toBe(tree);
    expect(unwrapAt(tree, [0])).toBe(tree); // a leaf has no children to promote
    expect(setOp(tree, [0], "or")).toBe(tree);
  });

  it("edits do not mutate the input tree", () => {
    const tree = root([A, { kind: "or", items: [B] }]);
    const before = structuredClone(tree);
    removeAt(tree, [1, 0]);
    addChild(tree, [1], C);
    wrapAt(tree, [0], "not");
    expect(tree).toEqual(before);
  });
});

// Presentation is a separate value (§3.1, Q15). It used to ride in the clause
// tree as fake filter children, which is why wrapping a sort in an OR silently
// disabled it.
describe("view settings edits", () => {
  it("sort holds at most the one key OG can express", () => {
    const view = withSort({}, { field: "priority", dir: "desc" });
    expect(view.sort).toEqual([["priority", "desc"]]);
    expect(currentSort(view)).toEqual({ field: "priority", dir: "desc" });
    expect(currentSort(withSort(view, null))).toBeNull();
    expect(withSort(view, null).sort).toBeUndefined();
  });

  it("a blank field clears rather than writing an empty sort key", () => {
    expect(withSort({}, { field: "   ", dir: "asc" }).sort).toBeUndefined();
  });

  it("`count` is the whole-result count, spelled with the empty field (X3)", () => {
    const view = withAgg({}, { agg: "count", field: null });
    expect(view.aggregates).toEqual([["", "count"]]);
    expect(currentAgg(view)).toEqual({ agg: "count", field: null });
  });

  it("sum/avg keep their field", () => {
    const view = withAgg({}, { agg: "sum", field: "hours" });
    expect(view.aggregates).toEqual([["hours", "sum"]]);
    expect(currentAgg(view)).toEqual({ agg: "sum", field: "hours" });
    expect(currentAgg(withAgg(view, null))).toBeNull();
  });

  it("group-by is independent of the aggregate", () => {
    let view: ViewSettings = withAgg({}, { agg: "count", field: null });
    view = withGroup(view, "page");
    expect(currentGroup(view)).toBe("page");
    expect(currentAgg(view)).toEqual({ agg: "count", field: null });
    expect(currentGroup(withGroup(view, null))).toBeNull();
  });

  it("a view edit does not mutate the view it was given", () => {
    const view: ViewSettings = { sort: [["page", "asc"]] };
    withSort(view, { field: "priority", dir: "desc" });
    withGroup(view, "status");
    expect(view).toEqual({ sort: [["page", "asc"]] });
  });

  it("sortLabel prefers a preset's words", () => {
    expect(sortLabel("modified", "desc")).toBe("newest first");
    expect(sortLabel("rating", "desc")).toBe("rating ↓");
  });
});

// ---------------------------------------------------------------------------
// Operator identities over the registry type (SPEC §7.4, design §2.4; P3 T3)
// ---------------------------------------------------------------------------

describe("propertyOperators: the identity menu for a registry type", () => {
  const ids = (type: ObservedType, cardinality: Cardinality = "one") =>
    propertyOperators({ type, cardinality }).map((o) => o.id);

  it("offers exactly the §7.4 table, identity for identity", () => {
    expect(ids("number")).toEqual([
      "is", "is_not", "gt", "ge", "lt", "le", "between", "is_set", "is_not_set", "is_blank",
    ]);
    // A date atom has no honest negation short of `not(…)`, so no "is not".
    expect(ids("date")).toEqual([
      "is", "before", "on_or_before", "after", "on_or_after", "between", "is_set", "is_not_set",
    ]);
    expect(ids("text")).toEqual([
      "contains", "does_not_contain", "is", "is_not", "starts_with", "ends_with",
      "is_set", "is_not_set", "is_blank",
    ]);
    expect(ids("ref")).toEqual(["references", "does_not_reference"]);
    expect(ids("checkbox")).toEqual(["is_checked", "is_unchecked", "is_not_set"]);
  });

  it("never offers P2's atom-level `!=` in the menu", () => {
    for (const type of ["text", "number", "date", "checkbox", "ref"] as ObservedType[]) {
      expect(ids(type)).not.toContain("has_other_value");
    }
  });

  it("gives the presence and checkbox identities no value cell", () => {
    for (const id of ["is_set", "is_not_set", "is_blank", "is_checked", "is_unchecked"] as const) {
      expect(propertyOperatorArity(id)).toBe(0);
    }
    expect(propertyOperatorArity("between")).toBe(2);
    expect(propertyOperatorArity("is")).toBe(1);
  });

  it("says `contains this value` for a many-valued key, keeping the same encoding", () => {
    expect(propertyOperatorLabel("is", "many")).toBe("contains this value");
    expect(propertyOperatorLabel("is_not", "many")).toBe("does not contain this value");
    expect(propertyOperatorLabel("is", "one")).toBe("is");
    // Only the words change: `value = 'x'` on a many-valued key ALREADY means
    // "one of this key's values is x".
    expect(encodePropertyLeaf({ id: "is", key: "tags", values: ["x"] })).toEqual(
      propertyFilter("tags", { op: "eq", operand: { kind: "text", text: "x" } }),
    );
  });

  it("builds the operand KIND the type calls for, not a string for everything", () => {
    const atomOf = (filter: Filter | null) =>
      ((filter as { leaf: { pred: { items: Filter[] } } }).leaf.pred.items[1] as {
        leaf: { value: unknown };
      }).leaf.value;
    expect(atomOf(encodePropertyLeaf({ id: "gt", key: "cost", values: ["3"], type: "number" })))
      .toEqual({ kind: "number", number: 3 });
    // A6/§4.2.3: the date operand is the unresolved literal. Resolving it here
    // would freeze "today" to the day the row was added.
    expect(atomOf(encodePropertyLeaf({ id: "before", key: "due", values: ["today"], type: "date" })))
      .toEqual({ kind: "date", literal: "today" });
    expect(atomOf(encodePropertyLeaf({ id: "between", key: "due", values: ["today", "+7d"], type: "date" })))
      .toEqual({
        kind: "list",
        items: [{ kind: "date", literal: "today" }, { kind: "date", literal: "+7d" }],
      });
    expect(atomOf(encodePropertyLeaf({ id: "is_checked", key: "done" })))
      .toEqual({ kind: "bool", bool: true });
    expect(atomOf(encodePropertyLeaf({ id: "is_unchecked", key: "done" })))
      .toEqual({ kind: "bool", bool: false });
    expect(atomOf(encodePropertyLeaf({ id: "is", key: "type", values: ["book"] })))
      .toEqual({ kind: "text", text: "book" });
  });

  it("refuses text that is not a value of the type, instead of filtering for nothing", () => {
    expect(encodePropertyLeaf({ id: "is", key: "cost", values: ["not a number"], type: "number" })).toBeNull();
    expect(encodePropertyLeaf({ id: "is", key: "cost", values: [""], type: "number" })).toBeNull();
    expect(encodePropertyLeaf({ id: "is_checked", key: "" })).toBeNull();
    // A half-filled range is not a range.
    expect(encodePropertyLeaf({ id: "between", key: "due", values: ["today", ""], type: "date" })).toBeNull();
  });

  it("puts the typed comparison on the VALUE attribute, leaving the key test alone", () => {
    const filter = propertyFilter("cost", { op: "gt", operand: { kind: "number", number: 100 } });
    expect(encodePropertyLeaf({ id: "gt", key: "cost", values: ["100"], type: "number" })).toEqual(filter);
    expect(encodePropertyLeaf({ id: "gt", key: "cost", values: ["100"], type: "number", throughPage: true }))
      .toEqual({ kind: "leaf", leaf: { kind: "rel", rel: "page", quant: "any", pred: filter } });
  });

  it("spells the three §3.3 presence shapes as three different leaves", () => {
    // is set: the bare key test, exactly what `(any value)` always built.
    expect(encodePropertyLeaf({ id: "is_set", key: "public" })).toEqual(propertyFilter("public", null));
    // is not set: the SAME predicate under the `none` quantifier.
    expect(encodePropertyLeaf({ id: "is_not_set", key: "public" })).toEqual({
      kind: "leaf",
      leaf: {
        kind: "rel", rel: "props", quant: "none",
        pred: { kind: "leaf", leaf: { kind: "attr", attr: "key", op: "eq", value: { kind: "text", text: "public" } } },
      },
    });
    // is blank: present, with no atoms.
    expect(encodePropertyLeaf({ id: "is_blank", key: "public" })).toEqual({
      kind: "leaf",
      leaf: {
        kind: "rel", rel: "props", quant: "any",
        pred: {
          kind: "and",
          items: [
            { kind: "leaf", leaf: { kind: "attr", attr: "key", op: "eq", value: { kind: "text", text: "public" } } },
            { kind: "leaf", leaf: { kind: "attr", attr: "atom_count", op: "eq", value: { kind: "number", number: 0 } } },
          ],
        },
      },
    });
  });

  it("wraps every negative identity around the POSITIVE leaf (§3.3)", () => {
    // The builder's *is not* is `not(prop('k') = v)`, which is TRUE for an owner
    // that has no such property at all. P2's atom-level `!=` is a different
    // predicate that is false for that owner, so it is no longer offered.
    expect(encodePropertyLeaf({ id: "is_not", key: "type", values: ["book"] })).toEqual({
      kind: "not",
      inner: propertyFilter("type", { op: "eq", operand: { kind: "text", text: "book" } }),
    });
    expect(encodePropertyLeaf({ id: "has_other_value", key: "type", values: ["book"] })).toEqual(
      propertyFilter("type", { op: "not_eq", operand: { kind: "text", text: "book" } }),
    );
    expect(encodePropertyLeaf({ id: "is_not", key: "type", values: ["book"] })).not.toEqual(
      encodePropertyLeaf({ id: "has_other_value", key: "type", values: ["book"] }),
    );
    expect(encodePropertyLeaf({ id: "does_not_reference", key: "owner", values: ["Avery"] })).toEqual({
      kind: "not",
      inner: propertyFilter("owner", { op: "eq", operand: { kind: "text", text: "Avery" } }),
    });
    expect(encodePropertyLeaf({ id: "does_not_contain", key: "note", values: ["x"] })).toEqual({
      kind: "not",
      inner: propertyFilter("note", { op: "like", operand: { kind: "text", text: "%x%" } }),
    });
  });
});

describe("propertyLeafTest: every property leaf reopens with its operator", () => {
  /** Each row of the §7.4 table, with the type its identity belongs to. */
  const TABLE: { id: PropertyOperatorId; type: ObservedType; values: string[] }[] = [
    { id: "is", type: "number", values: ["100"] },
    { id: "is_not", type: "number", values: ["100"] },
    { id: "gt", type: "number", values: ["100"] },
    { id: "ge", type: "number", values: ["100"] },
    { id: "lt", type: "number", values: ["100"] },
    { id: "le", type: "number", values: ["100"] },
    { id: "between", type: "number", values: ["1", "9"] },
    { id: "is_set", type: "number", values: [] },
    { id: "is_not_set", type: "number", values: [] },
    { id: "is_blank", type: "number", values: [] },
    { id: "is", type: "date", values: ["today"] },
    { id: "before", type: "date", values: ["today"] },
    { id: "on_or_before", type: "date", values: ["today"] },
    { id: "after", type: "date", values: ["today"] },
    { id: "on_or_after", type: "date", values: ["today"] },
    { id: "between", type: "date", values: ["-7d", "today"] },
    { id: "contains", type: "text", values: ["book"] },
    { id: "does_not_contain", type: "text", values: ["book"] },
    { id: "is", type: "text", values: ["book"] },
    { id: "is_not", type: "text", values: ["book"] },
    { id: "starts_with", type: "text", values: ["Proj"] },
    { id: "ends_with", type: "text", values: ["ing"] },
    { id: "references", type: "ref", values: ["Avery"] },
    { id: "does_not_reference", type: "ref", values: ["Avery"] },
    { id: "is_checked", type: "checkbox", values: [] },
    { id: "is_unchecked", type: "checkbox", values: [] },
    { id: "has_other_value", type: "text", values: ["book"] },
  ];

  it("is the exact inverse of encodePropertyLeaf on every row of the table", () => {
    for (const row of TABLE) {
      const filter = encodePropertyLeaf({ id: row.id, key: "k", values: row.values, type: row.type });
      expect(filter, `${row.id}/${row.type} must encode`).not.toBeNull();
      expect(propertyLeafTest(filter!, { type: row.type }), `${row.id}/${row.type}`).toEqual({
        id: row.id,
        key: "k",
        values: row.values,
        throughPage: false,
      });
    }
  });

  it("reads a page-property row back through its page hop", () => {
    const filter = encodePropertyLeaf({ id: "is", key: "fach", values: ["x"], throughPage: true })!;
    expect(propertyLeafTest(filter)).toEqual({ id: "is", key: "fach", values: ["x"], throughPage: true });
  });

  it("tells contains and ends-with apart, and keeps escaped %, _ and \\ as data", () => {
    const tricky = "50%_a\\b";
    const contains = encodePropertyLeaf({ id: "contains", key: "note", values: [tricky] })!;
    const endsWith = encodePropertyLeaf({ id: "ends_with", key: "note", values: [tricky] })!;
    expect(contains).not.toEqual(endsWith);
    expect(propertyLeafTest(contains)).toEqual({ id: "contains", key: "note", values: [tricky], throughPage: false });
    expect(propertyLeafTest(endsWith)).toEqual({ id: "ends_with", key: "note", values: [tricky], throughPage: false });
    expect(readLikePattern("%50\\%%")).toEqual({ shape: "contains", text: "50%" });
    expect(readLikePattern("%50\\%")).toEqual({ shape: "ends_with", text: "50%" });
    expect(readLikePattern("Proj%")).toEqual({ shape: "starts_with", text: "Proj" });
    // A pattern with an unescaped wildcard in the MIDDLE is not one this
    // builder wrote, and there is no identity that honestly re-collects it.
    expect(readLikePattern("%a%b%")).toBeNull();
  });

  it("decodes a `not(like)` as `does not contain`, never as a bare contains", () => {
    const filter: Filter = {
      kind: "not",
      inner: propertyFilter("note", { op: "like", operand: { kind: "text", text: "%x%" } }),
    };
    expect(propertyLeafTest(filter)?.id).toBe("does_not_contain");
  });

  it("reads `prop = 'x'` as `references` only for a ref key", () => {
    const filter = propertyFilter("owner", { op: "eq", operand: { kind: "text", text: "Avery" } });
    expect(propertyLeafTest(filter, { type: "ref" })?.id).toBe("references");
    expect(propertyLeafTest(filter, { type: "text" })?.id).toBe("is");
    expect(propertyLeafTest(filter)?.id).toBe("is");
  });

  it("reopens P2's atom-level `!=` without dropping its operator", () => {
    // P2 could construct this and P2's chip had no Edit affordance for it, so
    // reopening it dropped the operator. It is not offered any more, but it
    // must still come back as what it is.
    const inherited = propertyFilter("type", { op: "not_eq", operand: { kind: "text", text: "book" } });
    expect(propertyLeafTest(inherited)).toEqual({
      id: "has_other_value", key: "type", values: ["book"], throughPage: false,
    });
    expect(propertyOperatorLabel("has_other_value")).toBe("has a value other than");
  });

  it("says nothing about a leaf that is not a property test", () => {
    expect(propertyLeafTest(pageRefFilter("Foo"))).toBeNull();
    expect(propertyLeafTest(taskFilter(["TODO"]))).toBeNull();
    expect(propertyLeafTest(pageTagsFilter(["a"]))).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// The resting sentence and the depth cap (SPEC §7.2, §7.4; P3 T1/T3)
// ---------------------------------------------------------------------------

describe("filterPhrase / querySentence", () => {
  const said = (segments: { text: string }[]) => segments.map((s) => s.text).join("");

  it("is what filterLabel joins, segment for segment", () => {
    for (const filter of [
      pageRefFilter("Foo"),
      taskFilter(["NOW", "LATER"]),
      propertyFilter("type", "book"),
      contentFilter("100% done"),
      { kind: "off", inner: pageRefFilter("A") } as Filter,
      { kind: "raw", text: "(frobnicate x)", diagnostic_kind: "unknown_head" } as Filter,
    ]) {
      expect(said(filterPhrase(filter))).toBe(filterLabel(filter));
    }
  });

  it("marks values as chips and field names as fields", () => {
    expect(filterPhrase(pageRefFilter("Project X"))).toEqual([
      { kind: "value", text: "Project X" },
    ]);
    expect(filterPhrase(propertyFilter("type", "book"))).toEqual([
      { kind: "field", text: "type" },
      { kind: "text", text: ": " },
      { kind: "value", text: "book" },
    ]);
  });

  it("phrases a typed property leaf in the row's own words", () => {
    const filter = encodePropertyLeaf({ id: "gt", key: "cost", values: ["100"], type: "number" })!;
    expect(filterLabel(filter)).toBe("cost is more than 100");
  });

  it("reads an empty query as `All blocks` / `All pages`", () => {
    expect(said(querySentence({ anchor: "block", filter: { kind: "and", items: [] } })))
      .toBe("All blocks");
    expect(said(querySentence({ anchor: "page", filter: { kind: "true" } }))).toBe("All pages");
  });

  it("GH #619 item 9: in `Pages and blocks` mode the subject names both families, never `Blocks`", () => {
    expect(said(querySentence({ anchor: "block", filter: { kind: "and", items: [] }, both: true })))
      .toBe("All pages and blocks");
    const filter: Filter = { kind: "and", items: [taskFilter(["TODO"])] };
    const both = said(querySentence({ anchor: "block", filter, both: true }));
    expect(both).toMatch(/^Pages and blocks where /);
    expect(both).not.toMatch(/^Blocks where /);
    // The one-family reading is unchanged.
    expect(said(querySentence({ anchor: "block", filter, both: false }))).toMatch(/^Blocks where /);
  });

  it("reads a filter as one sentence whose subject is the anchor", () => {
    const filter: Filter = {
      kind: "and",
      items: [taskFilter(["TODO"]), pageRefFilter("Project X"), { kind: "not", inner: pageRefFilter("archive") }],
    };
    expect(said(querySentence({ anchor: "block", filter })))
      .toBe("Blocks where task: TODO, Project X, and not archive");
    expect(said(querySentence({ anchor: "page", filter: pageRefFilter("Alpha") })))
      .toBe("Pages where Alpha");
  });

  it("says `or` between the operands of an or group", () => {
    const filter: Filter = { kind: "or", items: [taskFilter(["TODO"]), taskFilter(["DONE"])] };
    expect(said(querySentence({ anchor: "block", filter })))
      .toBe("Blocks where task: TODO or task: DONE");
  });

  it("bounds a 64-deep hostile query to a short sentence (I-22)", () => {
    let deep: Filter = pageRefFilter("bottom");
    for (let i = 0; i < 64; i++) deep = { kind: "and", items: [pageRefFilter(`level${i}`), deep] };
    const sentence = querySentence({ anchor: "block", filter: deep });
    expect(sentence.length).toBeLessThan(24);
    expect(said(sentence)).toContain(ADVANCED_PHRASE);
    expect(said(sentence).length).toBeLessThan(240);

    // Relation predicates are levels too, so a deep `any(children, …)` chain
    // cannot recurse without bound either.
    let rel: Filter = pageRefFilter("bottom");
    for (let i = 0; i < 64; i++) {
      rel = { kind: "leaf", leaf: { kind: "rel", rel: "children", quant: "any", pred: rel } };
    }
    expect(filterLabel(rel).length).toBeLessThan(120);
    expect(filterLabel(rel)).toContain(ADVANCED_PHRASE);

    // And so is `off`.
    let off: Filter = pageRefFilter("bottom");
    for (let i = 0; i < 64; i++) off = { kind: "off", inner: off };
    expect(filterLabel(off).length).toBeLessThan(120);
  });

  it("caps the RENDERING at three levels while the language keeps 64", () => {
    expect(MAX_QUERY_BUILDER_DEPTH).toBe(3);
  });
});

// ---------------------------------------------------------------------------
// groupWithPrevious (SPEC §7.4, design §2.5; P3 T3)
// ---------------------------------------------------------------------------

describe("groupWithPrevious", () => {
  it("wraps a row and the sibling above it into one and-group, in place", () => {
    const root: Filter = { kind: "and", items: [A, B, C] };
    expect(groupWithPrevious(root, [2])).toEqual({
      kind: "and",
      items: [A, { kind: "and", items: [B, C] }],
    });
  });

  it("is a no-op on the first row, which has nothing above it", () => {
    const root: Filter = { kind: "and", items: [A, B] };
    expect(groupWithPrevious(root, [0])).toEqual(root);
    expect(groupWithPrevious(root, [])).toEqual(root);
  });

  it("is a no-op for a stale loc, exactly like every other edit here", () => {
    const root: Filter = { kind: "and", items: [A, B] };
    expect(groupWithPrevious(root, [7])).toEqual(root);
    expect(groupWithPrevious(root, [0, 3])).toEqual(root);
  });

  it("groups inside a nested group, not at the root", () => {
    const root: Filter = { kind: "and", items: [A, { kind: "or", items: [B, C, A] }] };
    expect(groupWithPrevious(root, [1, 2])).toEqual({
      kind: "and",
      items: [A, { kind: "or", items: [B, { kind: "and", items: [C, A] }] }],
    });
  });
});

// ---------------------------------------------------------------------------
// P6: grouping a selection, reordering siblings, and the enabled control
// (SPEC §7.4 remainder). The IR these produce is what a drag, a keyboard move
// and a menu all have to agree on, so it is pinned HERE and the mounted tests
// only have to show that each gesture arrives at it.
// ---------------------------------------------------------------------------

/** A `raw` leaf: the payload a re-enable must hand back byte for byte. */
const RAW: Filter = { kind: "raw", text: "task = 'TODO'", diagnostic_kind: "not_applicable" };
/** A subtree past the rendering cap — the `⟨advanced⟩` chip's node. */
const DEEP: Filter = {
  kind: "or",
  items: [{ kind: "and", items: [A, { kind: "or", items: [B, C] }] }, RAW],
};

describe("groupSelected", () => {
  it("groups a contiguous selection at the first selected position", () => {
    const root: Filter = { kind: "and", items: [A, B, C] };
    expect(groupSelected(root, [[0], [1]], "all")).toEqual({
      kind: "and",
      items: [{ kind: "and", items: [A, B] }, C],
    });
  });

  it("keeps the selected items in their original order however they were picked", () => {
    const root: Filter = { kind: "and", items: [A, B, C] };
    // Picked bottom-up: the group still reads A then B, not B then A.
    expect(groupSelected(root, [[1], [0]], "all")).toEqual({
      kind: "and",
      items: [{ kind: "and", items: [A, B] }, C],
    });
  });

  it("lands a non-contiguous selection where the topmost selected row was, leaving the rest in order", () => {
    const D = pageRefFilter("D");
    const root: Filter = { kind: "and", items: [A, B, C, D] };
    expect(groupSelected(root, [[1], [3]], "any")).toEqual({
      kind: "and",
      items: [A, { kind: "or", items: [B, D] }, C],
    });
  });

  it("spells `none of` as the not-over-or the group header already reads back", () => {
    const root: Filter = { kind: "and", items: [A, B, C] };
    expect(groupSelected(root, [[0], [2]], "none")).toEqual({
      kind: "and",
      items: [{ kind: "not", inner: { kind: "or", items: [A, C] } }, B],
    });
  });

  it("groups inside a nested list without touching anything outside it", () => {
    const root: Filter = { kind: "and", items: [A, { kind: "or", items: [B, C, A] }] };
    expect(groupSelected(root, [[1, 0], [1, 2]], "all")).toEqual({
      kind: "and",
      items: [A, { kind: "or", items: [{ kind: "and", items: [B, A] }, C] }],
    });
  });

  it("moves a whole group and its wrappers into the new group, not a copy of its rows", () => {
    const inner: Filter = { kind: "not", inner: { kind: "or", items: [B, C] } };
    const root: Filter = { kind: "and", items: [A, inner] };
    expect(groupSelected(root, [[0], [1]], "any")).toEqual({
      kind: "and",
      items: [{ kind: "or", items: [A, inner] }],
    });
  });

  it("carries a disabled row, its `off`, and an opaque `raw` payload through untouched", () => {
    const disabled: Filter = { kind: "off", inner: RAW };
    const root: Filter = { kind: "and", items: [disabled, DEEP, A] };
    const grouped = groupSelected(root, [[0], [1]], "all");
    expect(grouped).toEqual({
      kind: "and",
      items: [{ kind: "and", items: [disabled, DEEP] }, A],
    });
  });

  it("refuses a selection of fewer than two, so nothing is wrapped that was not asked for", () => {
    const root: Filter = { kind: "and", items: [A, B] };
    expect(groupSelected(root, [[0]], "all")).toEqual(root);
    expect(groupSelected(root, [], "all")).toEqual(root);
  });

  it("refuses locs from two different lists rather than moving conditions between them", () => {
    const root: Filter = { kind: "and", items: [A, { kind: "or", items: [B, C] }] };
    expect(groupSelected(root, [[0], [1, 0]], "all")).toEqual(root);
    expect(groupSelected(root, [[1, 0], [1, 1], [0]], "all")).toEqual(root);
  });

  it("refuses an ancestor selected together with its own descendant", () => {
    const root: Filter = { kind: "and", items: [A, { kind: "or", items: [B, C] }] };
    // `[1]` is the group; `[1, 1]` is inside it. They are not siblings.
    expect(groupSelected(root, [[1], [1, 1]], "all")).toEqual(root);
  });

  it("refuses a duplicate, a stale index and a leaf parent", () => {
    const root: Filter = { kind: "and", items: [A, B] };
    expect(groupSelected(root, [[1], [1]], "all")).toEqual(root);
    expect(groupSelected(root, [[0], [7]], "all")).toEqual(root);
    expect(groupSelected(root, [[0], [-1]], "all")).toEqual(root);
    // A `not` has one child and no list to group inside.
    const unary: Filter = { kind: "and", items: [{ kind: "not", inner: A }] };
    expect(groupSelected(unary, [[0, 0], [0, 1]], "all")).toEqual(unary);
  });
});

describe("groupWithPrevious", () => {
  it("is groupSelected over the two rows the menu names, with the same choices", () => {
    const root: Filter = { kind: "and", items: [A, B, C] };
    expect(groupWithPrevious(root, [2], "any")).toEqual(groupSelected(root, [[1], [2]], "any"));
    expect(groupWithPrevious(root, [2], "none")).toEqual(groupSelected(root, [[1], [2]], "none"));
    expect(groupWithPrevious(root, [2])).toEqual(groupSelected(root, [[1], [2]], "all"));
  });
});

describe("moveAcross (GH #619 item 6: drag a condition between groups)", () => {
  const D = pageRefFilter("D");
  const and = (...items: Filter[]): Filter => ({ kind: "and", items });
  const or = (...items: Filter[]): Filter => ({ kind: "or", items });

  it("moves a row into another group at the slot the drop line shows", () => {
    const root = and(A, or(B, C, D));
    expect(moveAcross(root, [0], [1], 0)).toEqual(and(or(A, B, C, D)));
    expect(moveAcross(root, [0], [1], 2)).toEqual(and(or(B, C, A, D)));
    expect(moveAcross(root, [0], [1], 3)).toEqual(and(or(B, C, D, A)));
  });

  it("moves a row out of a group into the root list, and a group left with two keeps its place", () => {
    const root = and(A, or(B, C, D));
    expect(moveAcross(root, [1, 2], [], 0)).toEqual(and(D, A, or(B, C)));
    expect(moveAcross(root, [1, 0], [], 2)).toEqual(and(A, or(C, D), B));
  });

  it("dissolves a group left with one condition into its parent, in place", () => {
    const root = and(A, or(B, C), D);
    // C leaves the `or`: B takes the group's place, between A and D.
    expect(moveAcross(root, [1, 1], [], 3)).toEqual(and(A, B, D, C));
    // Moving into the group's own sibling slot at the front: same dissolve.
    expect(moveAcross(root, [1, 0], [], 0)).toEqual(and(B, A, C, D));
  });

  it("dissolves inside the group's own wrapper, so `none of` stays a negation", () => {
    const none: Filter = { kind: "not", inner: or(B, C) };
    const root = and(A, none);
    expect(moveAcross(root, [1, 0, 1], [], 0)).toEqual(and(C, A, { kind: "not", inner: B }));
    const off: Filter = { kind: "off", inner: and(B, C) };
    expect(moveAcross(and(A, off), [1, 0, 0], [], 0)).toEqual(and(B, A, { kind: "off", inner: C }));
  });

  it("keeps a one-child group whose wrapper would end up holding another wrapper", () => {
    const negB: Filter = { kind: "not", inner: B };
    const none: Filter = { kind: "not", inner: or(negB, C) };
    const root = and(A, none);
    const moved = moveAcross(root, [1, 0, 1], [], 0);
    expect(moved).toEqual(and(C, A, { kind: "not", inner: or(negB) }));
  });

  it("dissolves a nested group into the group above it, not the root", () => {
    const root = and(A, or(B, and(C, D)));
    // D leaves the inner `and`: C takes its place inside the `or`.
    expect(moveAcross(root, [1, 1, 1], [], 0)).toEqual(and(D, A, or(B, C)));
  });

  it("moves a whole group, wrappers and all, into another group", () => {
    const group: Filter = { kind: "off", inner: or(B, C) };
    const root = and(A, group, and(D, A));
    expect(moveAcross(root, [1], [2], 1)).toEqual(and(A, and(D, group, A)));
  });

  it("does not move a group into itself or anything below it", () => {
    const root = and(A, or(B, and(C, D)));
    expect(moveAcross(root, [1], [1], 0)).toBe(root);
    expect(moveAcross(root, [1], [1, 1], 0)).toBe(root);
    const none: Filter = { kind: "not", inner: or(B, C) };
    const wrapped = and(A, none);
    expect(moveAcross(wrapped, [1], [1, 0], 0)).toBe(wrapped);
  });

  it("moving within one list is a reorder and dissolves nothing", () => {
    const root = and(A, or(B, C));
    expect(moveAcross(root, [1, 0], [1], 2)).toEqual(and(A, or(C, B)));
    expect(moveAcross(root, [1, 0], [1], 0)).toBe(root);
    expect(moveAcross(root, [1, 0], [1], 1)).toBe(root);
  });

  it("refuses stale paths, a leaf destination and an out-of-range slot", () => {
    const root = and(A, or(B, C));
    expect(moveAcross(root, [9], [], 0)).toBe(root);
    expect(moveAcross(root, [0], [0], 0)).toBe(root);
    expect(moveAcross(root, [0], [7], 0)).toBe(root);
    expect(moveAcross(root, [0], [1], 3)).toBe(root);
    expect(moveAcross(root, [0], [1], -1)).toBe(root);
    expect(moveAcross(root, [], [1], 0)).toBe(root);
  });

  it("prunes a group the move emptied, and leaves authored empty groups alone", () => {
    const solo = and(A, or(B), and());
    // The one-child `or` loses B: it empties and goes; the authored empty `and` stays.
    expect(moveAcross(solo, [1, 0], [], 0)).toEqual(and(B, A, and()));
  });

  it("never edits the input tree", () => {
    const root = and(A, or(B, C));
    const before = structuredClone(root);
    moveAcross(root, [1, 1], [], 0);
    expect(root).toEqual(before);
  });
});

describe("moveSibling", () => {
  it("moves a row up and down among its own siblings", () => {
    const root: Filter = { kind: "and", items: [A, B, C] };
    expect(moveSibling(root, [2], 1)).toEqual({ kind: "and", items: [A, C, B] });
    expect(moveSibling(root, [0], 2)).toEqual({ kind: "and", items: [B, C, A] });
  });

  it("refuses the boundary moves rather than saving an unchanged tree", () => {
    const root: Filter = { kind: "and", items: [A, B, C] };
    expect(moveSibling(root, [0], -1)).toBe(root);
    expect(moveSibling(root, [2], 3)).toBe(root);
    expect(moveSibling(root, [1], 1)).toBe(root);
  });

  it("refuses a stale or foreign path", () => {
    const root: Filter = { kind: "and", items: [A, B] };
    expect(moveSibling(root, [7], 0)).toBe(root);
    expect(moveSibling(root, [0, 1], 0)).toBe(root);
    expect(moveSibling(root, [], 0)).toBe(root);
  });

  it("moves a group's COMPLETE subtree and its wrappers, byte for byte", () => {
    const group: Filter = { kind: "off", inner: { kind: "not", inner: { kind: "or", items: [B, DEEP] } } };
    const root: Filter = { kind: "and", items: [A, group, C] };
    const moved = moveSibling(root, [1], 0);
    expect(moved).toEqual({ kind: "and", items: [group, A, C] });
    // Nothing inside the moved node was rebuilt into a different shape.
    expect((moved as { items: Filter[] }).items[0]).toEqual(group);
  });

  it("is atomic against the original tree: the destination is not recomputed after a prune", () => {
    // The mover is the ONLY child of its group. A remove-then-insert would
    // prune the emptied `or` first, which shifts every later index — the
    // destination `1` would then address a list that no longer exists.
    const root: Filter = {
      kind: "and",
      items: [{ kind: "or", items: [A] }, B],
    };
    expect(moveSibling(root, [0], 1)).toEqual({
      kind: "and",
      items: [B, { kind: "or", items: [A] }],
    });
    // And moving the inner row inside its one-child list is refused, not turned
    // into a cross-parent move.
    expect(moveSibling(root, [0, 0], 1)).toBe(root);
  });

  it("moves back: a pair of opposite moves returns the tree it started from", () => {
    const root: Filter = { kind: "and", items: [A, { kind: "or", items: [B, C] }, RAW, DEEP] };
    for (const [from, to] of [[0, 3], [3, 0], [1, 2], [2, 1]]) {
      const there = moveSibling(root, [from], to);
      expect(there).not.toEqual(root);
      expect(moveSibling(there, [to], from)).toEqual(root);
    }
  });
});

describe("toggleDisabledAt", () => {
  it("wraps and unwraps the node's own Off", () => {
    const root: Filter = { kind: "and", items: [A, B] };
    const off = toggleDisabledAt(root, [0]);
    expect(off).toEqual({ kind: "and", items: [{ kind: "off", inner: A }, B] });
    expect(isDisabledAt(off, [0])).toBe(true);
    expect(toggleDisabledAt(off, [0])).toEqual(root);
    expect(isDisabledAt(root, [0])).toBe(false);
  });

  it("keeps the row's Not, in either order it was stored", () => {
    // Disabling a negated row wraps the whole row, `not` included.
    const negated: Filter = { kind: "and", items: [{ kind: "not", inner: A }] };
    expect(toggleDisabledAt(negated, [0])).toEqual({
      kind: "and",
      items: [{ kind: "off", inner: { kind: "not", inner: A } }],
    });
    // And enabling gives the `not` back from either stored order — both spell
    // one negated, disabled row (§3.5 removes `Not(<removed>)` the same way).
    const offOverNot: Filter = { kind: "and", items: [{ kind: "off", inner: { kind: "not", inner: A } }] };
    const notOverOff: Filter = { kind: "and", items: [{ kind: "not", inner: { kind: "off", inner: A } }] };
    expect(toggleDisabledAt(offOverNot, [0])).toEqual(negated);
    expect(toggleDisabledAt(notOverOff, [0])).toEqual(negated);
    expect(isDisabledAt(notOverOff, [0])).toBe(true);
  });

  it("leaves a disabled DESCENDANT disabled when its group is enabled again", () => {
    const child: Filter = { kind: "off", inner: B };
    const group: Filter = { kind: "off", inner: { kind: "and", items: [A, child] } };
    const root: Filter = { kind: "and", items: [group, C] };
    const enabled = toggleDisabledAt(root, [0]);
    expect(enabled).toEqual({
      kind: "and",
      items: [{ kind: "and", items: [A, child] }, C],
    });
    // The child's own Off is still there, and still its own to toggle.
    expect(isDisabledAt(enabled, [0, 1])).toBe(true);
    expect(toggleDisabledAt(enabled, [0, 1])).toEqual({
      kind: "and",
      items: [{ kind: "and", items: [A, B] }, C],
    });
  });

  it("preserves a Raw payload and an opaque advanced subtree across a disable/enable pair", () => {
    const root: Filter = { kind: "and", items: [RAW, DEEP] };
    const off = toggleDisabledAt(toggleDisabledAt(root, [0]), [1]);
    expect(off).toEqual({
      kind: "and",
      items: [{ kind: "off", inner: RAW }, { kind: "off", inner: DEEP }],
    });
    const back = toggleDisabledAt(toggleDisabledAt(off, [1]), [0]);
    expect(back).toEqual(root);
    // The payload itself, not merely a tree that prints the same.
    expect(JSON.stringify(back)).toBe(JSON.stringify(root));
  });

  it("is a no-op on the root and on a stale loc", () => {
    const root: Filter = { kind: "and", items: [A] };
    expect(toggleDisabledAt(root, [])).toBe(root);
    expect(toggleDisabledAt(root, [4])).toEqual(root);
    expect(isDisabledAt(root, [4])).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// A group inside a `not`/`off` wrapper (P6). Its three actions address the
// `and`/`or` INSIDE the wrapper, and every one of them used to write a new but
// identical tree — a dead click that saved the block and pushed an empty step
// onto the undo stack. The `none of` half of this predates P6; the disabled half
// became reachable the moment a group could be switched off.
// ---------------------------------------------------------------------------

describe("editing a group that sits inside a unary wrapper", () => {
  const disabled = (): Filter => ({
    kind: "and",
    items: [{ kind: "off", inner: { kind: "and", items: [A, B] } }, C],
  });
  const negated = (): Filter => ({
    kind: "and",
    items: [{ kind: "not", inner: { kind: "or", items: [A, B] } }, C],
  });

  it("switches all of ↔ any of inside the wrapper, keeping the wrapper", () => {
    expect(setOp(disabled(), [0, 0], "or")).toEqual({
      kind: "and",
      items: [{ kind: "off", inner: { kind: "or", items: [A, B] } }, C],
    });
    expect(setOp(negated(), [0, 0], "and")).toEqual({
      kind: "and",
      items: [{ kind: "not", inner: { kind: "and", items: [A, B] } }, C],
    });
  });

  it("wraps the inner group in a Not without disturbing the Off around it", () => {
    expect(wrapAt(disabled(), [0, 0], "not")).toEqual({
      kind: "and",
      items: [
        { kind: "off", inner: { kind: "not", inner: { kind: "and", items: [A, B] } } },
        C,
      ],
    });
  });

  it("lifts a single child out, and refuses what would need a De Morgan rewrite", () => {
    const single: Filter = { kind: "and", items: [{ kind: "not", inner: { kind: "or", items: [A] } }] };
    expect(unwrapAt(single, [0, 0])).toEqual({ kind: "and", items: [{ kind: "not", inner: A }] });
    // Two children cannot be placed inside a unary wrapper, and neither
    // distributing the wrapper nor rewriting the group is the builder's to
    // decide (§3.5). The tree comes back UNCHANGED — identity, so the sheet can
    // tell a refusal from an edit and not save.
    const two = negated();
    expect(unwrapAt(two, [0, 0])).toBe(two);
    const off = disabled();
    expect(unwrapAt(off, [0, 0])).toBe(off);
  });

  it("keeps ungroup lossless: every wrapper the group HELD comes back out", () => {
    const child: Filter = { kind: "off", inner: { kind: "not", inner: B } };
    const kept: Filter = { kind: "raw", text: "task = 'TODO'", diagnostic_kind: "not_applicable" };
    const root: Filter = { kind: "and", items: [A, { kind: "or", items: [child, kept, DEEP] }, C] };
    expect(unwrapAt(root, [1])).toEqual({ kind: "and", items: [A, child, kept, DEEP, C] });
  });
});

it("tree edits preserve pre-existing empty boolean siblings", () => {
  const empty: Filter[] = [{ kind: "or", items: [] }, { kind: "and", items: [] }, { kind: "not", inner: { kind: "or", items: [] } }];
  const a: Filter = { kind: "true" };
  const tree: Filter = { kind: "and", items: [...empty, a] };
  expect(replaceAt(tree, [3], { kind: "false" })).toEqual({ kind: "and", items: [...empty, { kind: "false" }] });
  expect(removeAt(tree, [3])).toEqual({ kind: "and", items: empty });
  expect(addChild(tree, [], a)).toEqual({ kind: "and", items: [...empty, a, a] });
  expect(setOp(tree, [], "or")).toEqual({ kind: "or", items: [...empty, a] });
});


it("all query editing gestures retain authored empty siblings (I-4; queryBuilder.ts edit)", () => {
  const empty: Filter = { kind: "or", items: [] };
  const tree: Filter = { kind: "and", items: [empty, A, B, { kind: "and", items: [C] }] };
  for (const next of [
    wrapAt(tree, [1], "not"), toggleDisabledAt(tree, [1]),
    groupSelected(tree, [[1], [2]], "any"), groupWithPrevious(tree, [2]),
    moveSibling(tree, [1], 2), unwrapAt(tree, [3]), setOp(tree, [3], "or"),
    removeAt(tree, [3, 0]),
  ]) expect(filterChildren(next)?.[0]).toEqual(empty);
  expect(tree.items).toEqual([empty, A, B, { kind: "and", items: [C] }]);
});

// GH #619 (hestratos): task "Any status", "In a journal page" read-back, no "(advanced)" for builder shapes

describe("task Any status (GH #619 item 2)", () => {
  it("writes every status OG knows and reads back as `Any status`", () => {
    const filter = anyTaskFilter();
    expect(builderLeafKind(filter)).toBe("task");
    expect(filterLabel(filter)).toBe("task: Any status");
    expect(filterValueLabel(filter)).toBe("Any status");
    // OG's own marker set, not Tine's STARTED, so OG can read the text.
    expect(OG_TASK_MARKERS).not.toContain("STARTED");
    expect(OG_TASK_MARKERS).toHaveLength(10);
  });

  it("does not call a partial selection `Any status`", () => {
    expect(isAnyTaskStatus(["TODO", "DOING"])).toBe(false);
    expect(filterLabel(taskFilter(["TODO", "DOING"]))).toBe("task: TODO | DOING");
  });
});

describe("In a journal page (GH #619 item 3)", () => {
  it("is recognised as the journal condition, not as dates", () => {
    expect(builderLeafKind(journalFilter())).toBe("journal");
    expect(filterLabel(journalFilter())).toBe("on journal page");
    // A genuine date range stays a dates condition.
    expect(builderLeafKind(betweenFilter("journal", "-30d", "today"))).toBe("between");
  });
});

describe("no ⟨advanced⟩ for a shape the builder wrote (GH #619 item 5)", () => {
  const said = (segments: { text: string }[]) => segments.map((s) => s.text).join("");
  const everyBuilderShape: [string, Filter][] = [
    ["page ref", pageRefFilter("A")],
    ["task", taskFilter(["TODO"])],
    ["any task", anyTaskFilter()],
    ["priority", priorityFilter(["A"])],
    ["property", propertyFilter("type", "book")],
    ["scheduled", planningFilter("scheduled")],
    ["deadline", planningFilter("deadline")],
    ["journal", journalFilter()],
    ["journal dates", betweenFilter("journal", "today", "+7d")],
    ["scheduled dates", betweenFilter("scheduled", "today", "+7d")],
    ["deadline dates", betweenFilter("deadline", "-7d", "today")],
    ["scheduled from", betweenFilter("scheduled", "today", "")],
    ["content", contentFilter("x")],
    ["on page", onPageFilter("A")],
    ["namespace", namespaceFilter("A")],
    ["page property", pagePropertyFilter("fach", "x")],
    ["page tags", pageTagsFilter(["a"])],
  ];

  it("keeps every builder shape one plain condition two groups deep", () => {
    for (const [name, filter] of everyBuilderShape) {
      const nested: Filter = {
        kind: "and",
        items: [pageRefFilter("x"), { kind: "or", items: [filter, pageRefFilter("y")] }],
      };
      expect(said(querySentence({ anchor: "block", filter: nested })), name).not.toContain(ADVANCED_PHRASE);
    }
  });

  it("phrases a scheduled/deadline date in plain words", () => {
    expect(filterLabel(betweenFilter("scheduled", "today", "+7d"))).toBe("scheduled: next 7 days");
    expect(filterLabel(betweenFilter("deadline", "-7d", "today"))).toBe("deadline: last 7 days");
    expect(filterLabel(betweenFilter("scheduled", "today", ""))).toBe("scheduled: from today");
    expect(filterLabel(betweenFilter("journal", "-30d", "today"))).toBe("journal date: last 30 days");
  });

  it("still collapses a nested level the builder cannot re-collect", () => {
    let rel: Filter = pageRefFilter("bottom");
    for (let i = 0; i < 64; i++) {
      rel = { kind: "leaf", leaf: { kind: "rel", rel: "page", quant: "any", pred: rel } };
    }
    expect(filterLabel(rel)).toContain(ADVANCED_PHRASE);
    expect(filterLabel(rel).length).toBeLessThan(120);
  });
});

// I-12: the TypeScript readers below are twins of Rust functions; each reads
// the SAME golden file as its Rust test (query/tql_tests.rs, query/parse_tests.rs).
import likeGolden from "../../tests/fixtures/i12-like-escape-golden.json";
import propsGolden from "../../tests/fixtures/i12-props-reader-golden.json";
import { propsParts } from "./queryBuilder";
import type { Filter as IrFilter } from "./queryIr";

describe("twins agree with the native goldens", () => {
  it("escapeLike and plainLikeSubstring match escape_like_literal / plain_like_substring", () => {
    for (const [text, want] of likeGolden.escape as [string, string][]) expect(escapeLike(text), JSON.stringify(text)).toBe(want);
    for (const [pattern, want] of likeGolden.plain as [string, string | null][]) expect(plainLikeSubstring(pattern), JSON.stringify(pattern)).toBe(want);
  });
  it("propsParts matches Filter::props_key / props_atom_test", () => {
    for (const c of propsGolden.cases) {
      const parts = propsParts(c.pred as IrFilter);
      expect(parts?.key ?? null, c.name).toBe(c.key);
      if (c.key !== null) expect(parts?.atom ?? null, c.name).toEqual(c.atom);
    }
  });
});

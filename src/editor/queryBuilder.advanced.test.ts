import { describe, expect, it } from "vitest";
import {
  advancedToClause,
  clauseToAdvanced,
  clearSimpleForm,
  getSimpleForm,
  parseQuery,
  stashSimpleForm,
  toDsl,
} from "./queryBuilder";

function expectAdvancedRoundTrip(dsl: string): void {
  const conv = clauseToAdvanced(parseQuery(dsl));
  expect(conv.ok).toBe(true);
  if (!conv.ok) return;
  expect(conv.dropped).toEqual([]);

  const restored = advancedToClause(conv.dsl);
  expect(restored).not.toBeNull();
  expect(parseQuery(toDsl(restored!))).toEqual(parseQuery(dsl));
}

describe("advancedToClause", () => {
  it("round-trips every simple clause shape emitted by clauseToAdvanced", () => {
    for (const dsl of [
      "[[Roadmap]]",
      "(task TODO DOING)",
      "(task)",
      "(priority A B)",
      "(priority)",
      "(property status open)",
      "(property public)",
      '(property title "War and Peace")',
      "(page-property type book)",
      "(page-property public)",
      "(page-tags research active)",
      "(scheduled)",
      "(deadline)",
      "(journal)",
      "(page Project/Alpha)",
      "(namespace Project)",
      "(between journal -30d today)",
      "(between scheduled 2026-01-01 2026-01-31)",
      "(between deadline today +14d)",
      "(between today tomorrow)",
      "(and (or [[A]] (task TODO)) (not (priority A)))",
    ]) {
      expectAdvancedRoundTrip(dsl);
    }
  });

  it("returns null for non-round-trippable or unknown datalog", () => {
    expect(advancedToClause("[:find ?b :where [?b :block/refs ?r]]")).toBeNull();
    expect(advancedToClause('[:find (pull ?b [*]) :where (custom ?b "x")]')).toBeNull();
    expect(advancedToClause('[:find (pull ?b [*]) :where (task ?b "TODO") stray]')).toBeNull();
  });
});

describe("simple-form stash", () => {
  it("restores the pre-conversion DSL verbatim until cleared", () => {
    const id = "query-block";
    const dsl = "(and (task TODO) (sort-by priority desc))";

    stashSimpleForm(id, dsl);

    expect(getSimpleForm(id)).toBe(dsl);
    clearSimpleForm(id);
    expect(getSimpleForm(id)).toBeUndefined();
  });
});

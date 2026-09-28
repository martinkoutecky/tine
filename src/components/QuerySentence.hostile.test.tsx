import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { clearTransientLayersForTest } from "../transientLayers";
import { QueryBuilder, type BuilderSession } from "./QueryBuilder";
import {
  ADVANCED_PHRASE,
  MAX_QUERY_BUILDER_DEPTH,
  filterLabel,
  propertyFilter,
  querySentence,
  taskFilter,
} from "../editor/queryBuilder";
import type { Filter } from "../editor/queryIr";

// **I-22 for the two states of the builder.**
//
// The query in a `{{query}}` block is OUTSIDE CONTENT: whoever wrote the graph
// picked its shape, not us. The language accepts 64 levels of nesting
// (`QUERY_NESTING_MAX`) and must keep accepting them — a deep query still
// parses, still round-trips and still runs. What must NOT scale with the
// attacker's number is the DRAWING: the resting sentence stays a short line and
// the sheet stays a handful of rows, because both stop at
// `MAX_QUERY_BUILDER_DEPTH` and say `⟨advanced⟩` from there down.
//
// The cap is a presentation state, not a refusal: the whole query is still in
// the text pane below, still saved byte-for-byte, and removing the chip removes
// exactly that subtree.

function session(filter: Filter): BuilderSession {
  return { query: { anchor: "block", filter, source: { kind: "builder" } }, view: {} };
}

/** `depth` levels of one-item groups — the cheapest deep tree to write, and the
 *  one that used to cost a level of recursion for free. */
function chain(depth: number, leaf: Filter): Filter {
  let filter = leaf;
  for (let level = 0; level < depth; level += 1) filter = { kind: "and", items: [filter] };
  return filter;
}

/** `depth` levels alternating and/or/not/off around a two-item group, so the cap
 *  is exercised through every decoration the sheet peels rather than one. */
function tangle(depth: number, leaf: Filter): Filter {
  let filter: Filter = leaf;
  for (let level = 0; level < depth; level += 1) {
    if (level % 4 === 0) filter = { kind: "and", items: [filter, leaf] };
    else if (level % 4 === 1) filter = { kind: "or", items: [filter, leaf] };
    else if (level % 4 === 2) filter = { kind: "not", inner: filter };
    else filter = { kind: "off", inner: filter };
  }
  return filter;
}

function mountBuilder(filter: Filter) {
  const host = document.createElement("div");
  document.body.append(host);
  const [current, setCurrent] = createSignal<BuilderSession>(session(filter));
  const dispose = render(() => <QueryBuilder session={current} onChange={setCurrent} />, host);
  const open = (): HTMLElement => {
    host.querySelector<HTMLButtonElement>(".qs-gear")!.click();
    const sheet = document.querySelector<HTMLElement>(".qs-sheet");
    if (!sheet) throw new Error("the sheet did not open");
    return sheet;
  };
  return { host, open, source: () => JSON.stringify(current()), dispose };
}

beforeEach(() => {
  vi.spyOn(backend(), "queryFacets").mockResolvedValue([]);
  vi.spyOn(backend(), "printQuery").mockResolvedValue("(and (task TODO))");
});

afterEach(() => {
  clearTransientLayersForTest();
  resetSharedQueryResultsForTests();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe("I-22: a hostile query gets a bounded sentence and a bounded sheet", () => {
  it("reads a 64-deep single-item chain as a short line, not as 64 words", () => {
    const sentence = querySentence({ anchor: "block", filter: chain(64, taskFilter(["TODO"])) });
    expect(sentence.map((segment) => segment.text).join("")).toBe(`Blocks where ${ADVANCED_PHRASE}`);
    expect(sentence).toHaveLength(3);
  });

  it("bounds the phrase for every deep shape, however it is decorated", () => {
    for (const depth of [MAX_QUERY_BUILDER_DEPTH + 1, 16, 64]) {
      for (const build of [chain, tangle]) {
        const label = filterLabel(build(depth, taskFilter(["TODO"])));
        expect(label.length, `${build.name}(${depth}) drew ${label.length} characters`)
          .toBeLessThan(200);
      }
    }
  });

  it("mounts a 64-deep query as one line and a handful of rows", () => {
    const { host, open, source, dispose } = mountBuilder(tangle(64, propertyFilter("owner", "Ada")));
    try {
      const sentence = host.querySelector(".qs-sentence")!;
      expect(sentence.textContent).toContain(ADVANCED_PHRASE);
      expect(sentence.querySelectorAll(".qs-seg").length).toBeLessThanOrEqual(12);

      const before = source();
      const sheet = open();
      // Rows, groups and chips are all bounded by the same cap — nothing here
      // is a function of 64.
      expect(sheet.querySelectorAll(".qs-group").length).toBeLessThanOrEqual(MAX_QUERY_BUILDER_DEPTH);
      expect(sheet.querySelectorAll(".qs-row").length).toBeLessThanOrEqual(8);
      expect(sheet.querySelectorAll(".qs-row-advanced").length).toBeGreaterThanOrEqual(1);

      // …and drawing it changed nothing: the cap is presentation, not a rewrite.
      expect(source()).toBe(before);
    } finally {
      dispose();
    }
  });

  it("keeps a single absurdly long value on one line rather than growing the box", () => {
    const wide = "x".repeat(20_000);
    const { host, dispose } = mountBuilder(propertyFilter("owner", wide));
    try {
      const sentence = host.querySelector<HTMLElement>(".qs-sentence")!;
      // The text is shown in full — truncating a value would be a lie about
      // what the query says — but it is ONE segment on ONE clipped line, and
      // the segment count does not depend on its length.
      expect(sentence.querySelectorAll(".qs-seg").length).toBeLessThanOrEqual(8);
      expect(sentence.querySelectorAll("*").length).toBeLessThanOrEqual(8);
    } finally {
      dispose();
    }
  });
});

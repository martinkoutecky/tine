// The read-set marshal (outline-kernel DESIGN §5.3, §8): `outlineChange` sends
// each page as the verb's read set, not whole. Over random pages and verbs the
// kernel must answer exactly as it does with whole pages, without falling back.
import { beforeAll, describe, expect, it, vi } from "vitest";
import type { BlockDto } from "../types";

// Fresh keys from a fixed source, so both asks name new blocks alike.
vi.mock("./model", async (orig) => ({
  ...(await orig<typeof import("./model")>()),
  keySource: () => ({ prefix: "n", next: 0 }),
}));

import { initParser } from "../render/parse";
import { outlineChange, outlineMarshal, type OutlineVerb } from "./kernel";
import { doc, setDoc } from "./model";
import { loadFeed, resetStore } from "./workingSet";

function rng(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 2 ** 32;
  };
}

const RAWS = ["a", "two words", "x\ncollapsed:: true", "n\nlogseq.order-list-type:: number", "y\nid:: 6501b0f0-0000-4000-8000-00000000000a", ""];

function randomPage(r: () => number, name: string, size: number, prefix: string): BlockDto[] {
  let next = 0;
  const make = (depth: number): BlockDto => {
    const id = `${prefix}${next++}`;
    const block: BlockDto = { id, raw: `${RAWS[Math.floor(r() * RAWS.length)]}`, collapsed: false, children: [] };
    while (next < size && r() < 0.55 - depth * 0.08) block.children.push(make(depth + 1));
    return block;
  };
  const roots: BlockDto[] = [];
  while (next < size) roots.push(make(0));
  return roots;
}

function randomVerb(r: () => number, pages: string[]): OutlineVerb {
  const keysOf = (page: string) => Object.values(doc.byId).filter((n) => n.page === page).map((n) => n.id);
  const page = pages[Math.floor(r() * pages.length)];
  const keys = keysOf(page);
  const all = pages.flatMap(keysOf);
  const pick = (from: string[]) => from[Math.floor(r() * from.length)];
  const key = pick(keys);
  const node = doc.byId[key];
  const siblings = node.parent === null ? doc.pages.find((p) => p.name === page)!.roots : doc.byId[node.parent].children;
  const run = () => {
    const i = siblings.indexOf(key);
    return siblings.slice(i, i + 1 + Math.floor(r() * 3));
  };
  switch (Math.floor(r() * 8)) {
    case 0:
      return { verb: "split", key, raw_offset: Math.floor(r() * (node.raw.length + 1)), force_child: r() < 0.3, children_visible: r() < 0.7, keep_start_in_scope: r() < 0.2 };
    case 1:
      return { verb: "merge", survivor: pick(keys), absorbed: key, absorbed_referenced: false };
    case 2:
      return { verb: "indent", keys: r() < 0.6 ? [key] : run() };
    case 3:
      return { verb: "outdent", keys: r() < 0.6 ? [key] : run(), logical: r() < 0.5 };
    case 4:
      return { verb: "delete", keys: r() < 0.6 ? [key] : [key, pick(all), pick(keys)] };
    case 5:
      return { verb: "insert_empty_child", parent: key, at: Math.floor(r() * (node.children.length + 1)) };
    case 6: {
      const target = pick(all);
      const to = r() < 0.75
        ? { at: (["before", "after", "last_child"] as const)[Math.floor(r() * 3)], key: target }
        : { at: "slot" as const, parent: r() < 0.5 ? null : target, page: r() < 0.5 ? page : doc.byId[target].page, index: Math.floor(r() * 4) };
      return { verb: "move_to", keys: r() < 0.6 ? [key] : run(), to, inherit_from: r() < 0.5 ? target : null };
    }
    default:
      return r() < 0.6 ? { verb: "move_up_down", keys: [key], up: r() < 0.5 } : { verb: "move_up_down", keys: run(), up: r() < 0.5, within_siblings: true };
  }
}

beforeAll(async () => {
  await initParser();
});

describe("the read-set marshal", () => {
  it("answers as whole pages do, without falling back", () => {
    const r = rng(20261012);
    let partialSent = 0;
    for (let trial = 0; trial < 400; trial++) {
      resetStore();
      const pages = ["P", "Q"];
      loadFeed(pages.map((name, i) => ({ name, kind: "page" as const, title: name, pre_block: null, format: "md" as const, read_only: false, blocks: randomPage(r, name, 2 + Math.floor(r() * 40), i ? "q" : "p") })));
      if (r() < 0.3) setDoc("byId", doc.pages.find((p) => p.name === "P")!.roots[0], "originatedFromPageHeader", true);
      for (let step = 0; step < 5; step++) {
        const verb = randomVerb(r, pages);
        outlineMarshal.whole = true;
        const whole = outlineChange(verb, pages);
        outlineMarshal.whole = false;
        outlineMarshal.fallbacks = 0;
        const partial = outlineChange(verb, pages);
        const context = JSON.stringify({ trial, step, verb, whole });
        expect(partial, context).toEqual(whole);
        // A caller error the whole page refuses as incomplete is asked twice.
        const expected = "refused" in whole && whole.refused === "incomplete" ? 1 : 0;
        expect(outlineMarshal.fallbacks, context).toBe(expected);
        if (!expected) partialSent++;
      }
    }
    expect(partialSent).toBeGreaterThan(1500);
    resetStore();
  });
});

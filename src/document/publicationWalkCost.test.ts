import { afterEach, beforeAll, expect, it, vi } from "vitest";

// Count distinct blocks read across delivery, including navigation's tree walk.
// This also catches another owner reintroducing the unchanged-page traversal.
const cost = vi.hoisted(() => ({ counting: false, visited: new Set<string>() }));
vi.mock("./model", async original => {
  const model = await original<typeof import("./model")>();
  const byId = new Proxy({}, {
    get(_target, key) {
      if (cost.counting && typeof key === "string") cost.visited.add(key);
      return Reflect.get(model.doc.byId, key);
    },
    ownKeys: () => Reflect.ownKeys(model.doc.byId),
    getOwnPropertyDescriptor: (_target, key) => Reflect.getOwnPropertyDescriptor(model.doc.byId, key),
  });
  return { ...model, doc: new Proxy(model.doc, {
    get: (target, key) => key === "byId" ? byId : Reflect.get(target, key),
  }) };
});

import { createComputed, createRoot } from "solid-js";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import type { PageRead } from "../types";
import { applyGraphChange, applyGraphChangesBulk } from "./external";
import { doc, setDoc } from "./model";
import { visibleData } from "./tree";
import { installPageContent, loadFeed, resetStore } from "./workingSet";

beforeAll(initParser);
afterEach(() => { cost.counting = false; resetStore(); vi.restoreAllMocks(); });

const page = (name: string, size: number): PageRead => ({
  name, title: name, kind: "page", id: `pages/${name}.md`, rev: "1", pre_block: null,
  blocks: Array.from({ length: size }, (_, i) => ({ id: `${name}-${i}`, raw: `${name} ${i}`, collapsed: false, children: [] })),
});

it.each(["publication", "external", "host"] as const)("%s visits only the changed page and keeps navigation current", async entry => {
  const target = page("Target", 2000);
  loadFeed([...Array.from({ length: 128 }, (_, i) => page(`Other-${i}`, 256)), target]);
  const initial = visibleData();
  expect(initial.order).toHaveLength(34768);
  const next = { ...target, rev: "2", blocks: target.blocks.map(block => ({ ...block, raw: `${block.raw} changed` })) };
  // A topology change must patch the target's order too, even when nobody asks
  // for a flattened feed during delivery. Keep the same set of 2000 identities.
  next.blocks[0].children = [next.blocks.pop()!];
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue(next);
  vi.spyOn(backend(), "getPage").mockResolvedValue(next);
  cost.visited.clear(); cost.counting = true;
  if (entry === "publication") {
    await applyGraphChangesBulk({ changes: [], answers: { rev: "2", inventoryChanged: false, blockRefCounts: {},
      summary: { rev: "2", pages: ["target"], blocks: [], linksTo: [], props: {}, inventoryChanged: false, full: false } } });
  } else if (entry === "external") {
    await applyGraphChange({ name: target.name, kind: target.kind, path: target.id, created: false, removed: false }, true);
  } else {
    installPageContent(target.name, next, target.id);
  }
  const current = visibleData();
  expect(current.order.slice(-2000, -1997)).toEqual(["Target-0", "Target-1999", "Target-1"]);
  expect(current.index.get("Target-1999")).toBe(32769);
  expect(doc.byId["Target-0"].raw).toBe(next.blocks[0].raw);
  cost.counting = false;
  expect(cost.visited.size).toBe(2000);
  expect([...cost.visited].every(id => id.startsWith("Target-"))).toBe(true);
});

it("folds patch only their page and notify mounted navigation readers", () => {
  const target = page("Target", 2);
  target.blocks[0].children = [target.blocks.pop()!];
  loadFeed([...Array.from({ length: 8 }, (_, i) => page(`Other-${i}`, 256)), target]);
  let order: string[] = [];
  const dispose = createRoot(dispose => { createComputed(() => { order = visibleData().order; }); return dispose; });
  try {
    cost.visited.clear(); cost.counting = true;
    setDoc("byId", "Target-0", "collapsed", true);
    expect(order.at(-1)).toBe("Target-0");
    expect(order).not.toContain("Target-1");
    expect([...cost.visited].every(id => id.startsWith("Target-"))).toBe(true);
    setDoc("byId", "Target-0", "collapsed", false);
    expect(order.slice(-2)).toEqual(["Target-0", "Target-1"]);
  } finally { cost.counting = false; dispose(); }
});

it("text-only host delivery keeps mounted navigation idle and feed departure releases its tree", () => {
  const target = page("Target", 2000), other = page("Other", 256);
  target.blocks[0].children = [target.blocks.pop()!];
  loadFeed([target, other]);
  let renders = 0;
  const dispose = createRoot(dispose => { createComputed(() => { visibleData(); renders++; }); return dispose; });
  try {
    const initial = visibleData();
    installPageContent(target.name, { ...target, rev: "2", blocks: target.blocks.map(block => ({ ...block, raw: `${block.raw} changed` })) }, target.id);
    expect(visibleData()).toBe(initial);
    expect(renders).toBe(1);
    setDoc("feed", [other.name]);
    const remaining = visibleData();
    expect(remaining.order).toEqual(other.blocks.map(block => block.id));
    cost.visited.clear(); cost.counting = true;
    setDoc("byId", "Target-0", "collapsed", true);
    expect(visibleData()).toBe(remaining);
    expect(cost.visited.size).toBe(0);
    setDoc("feed", [other.name, target.name]);
    expect(visibleData().order.slice(-target.blocks.length)).toEqual(target.blocks.map(block => block.id));
  } finally { cost.counting = false; dispose(); }
});

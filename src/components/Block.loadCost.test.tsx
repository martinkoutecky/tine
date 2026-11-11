import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { DEV, For, batch } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { facetsOf, seedFacets } from "../render/facets";
import { resetStore } from "../document";
import { setDoc, type Node } from "../document/model";
import { bigPageBlocks } from "../mockScrollFixtures";
import { CommentIndexContext, createCommentIndex } from "../commentIndex";
import { resetNearObserverForTests } from "../lazyObserve";
import { Block } from "./Block";

const work = vi.hoisted(() => ({ ipc: 0 }));
vi.mock("../backend", async importOriginal => {
  const actual = await importOriginal<typeof import("../backend")>();
  return { ...actual, backend: () => new Proxy(actual.backend(), {
    get(target, key, receiver) {
      const value = Reflect.get(target, key, receiver);
      return typeof value === "function" ? (...args: unknown[]) => { work.ipc++; return value.apply(target, args); } : value;
    },
  }) };
});

beforeAll(initParser);
afterEach(() => { resetStore(); resetNearObserverForTests(); vi.unstubAllGlobals(); });

// Count the complete shell, including compiled JSX and nested feature helpers.
// Bodies remain outside the view: their existing near-parse test owns V's cost.
function load(n: number) {
  let serial = 0;
  const blocks = bigPageBlocks(raw => ({ id: `cost-${serial++}`, raw, children: [], collapsed: false }), n);
  const ids = blocks.map(b => b.id);
  for (const b of blocks) seedFacets(b.raw, "md", { ...facetsOf(b.raw, "md"), hasId: false });
  setDoc({ byId: Object.fromEntries(blocks.map(b => [b.id, { id: b.id, raw: b.raw, collapsed: false, children: [], page: "Big", parent: null } satisfies Node])),
    pages: [{ name: "Big", kind: "page", title: "Big", preBlock: null, roots: ids, format: "md", readOnly: false, guide: false }],
    feed: ["Big"], loaded: true });
  let owners = 0, signals = 0, observers = 0, observations = 0;
  vi.stubGlobal("IntersectionObserver", class {
    constructor() { observers++; }
    observe() { observations++; }
    unobserve() {}
    disconnect() {}
  });
  const hooks = DEV!.hooks;
  const created: { owner?: unknown; component?: unknown; fn?: unknown; pure?: boolean }[] = [];
  const previousOwner = hooks.afterCreateOwner, previousSignal = hooks.afterCreateSignal;
  hooks.afterCreateOwner = owner => { owners++; created.push(owner); };
  hooks.afterCreateSignal = () => { signals++; };
  const host = document.createElement("div");
  document.body.append(host);
  const stats = { calls: 0, hits: 0, misses: 0 };
  const bench = window as unknown as { __tineBench?: boolean; __tineParseStats?: typeof stats };
  bench.__tineBench = true; bench.__tineParseStats = stats; work.ipc = 0;
  let dispose: (() => void) | undefined;
  try {
    dispose = render(() => {
      const index = createCommentIndex(() => ids);
      return <CommentIndexContext.Provider value={index}><For each={ids}>{id => <Block id={id} />}</For></CommentIndexContext.Provider>;
    }, host);
    expect(host.querySelectorAll(".ls-block")).toHaveLength(n);
    expect(host.querySelectorAll(".ast-deferred")).toHaveLength(n);
    expect(stats.misses, "DTO facets and deferred bodies need no cold parse").toBe(0);
    expect(work.ipc, "row count must not multiply backend requests").toBe(0);
    return { owners, signals, observers, observations, elements: host.querySelectorAll("*").length,
      components: created.filter(owner => owner.component).length,
      memos: created.filter(owner => owner.fn && owner.pure && !owner.component).length,
      effects: created.filter(owner => owner.fn && !owner.pure).length };
  } finally {
    hooks.afterCreateOwner = previousOwner;
    hooks.afterCreateSignal = previousSignal;
    delete bench.__tineBench; delete bench.__tineParseStats;
    dispose?.(); host.remove(); resetNearObserverForTests(); resetStore();
  }
}

it("bounds all per-row reactive work on a big-page open (REG-BIGLOAD-ROW-WORK)", () => {
  const small = load(60), large = load(120);
  const perRow = Object.fromEntries(Object.keys(large).map(key => [key,
    (large[key as keyof typeof large] - small[key as keyof typeof small]) / 60]));
  const full = load(2000);
  console.log({ small, large, perRow, full });
  expect(perRow.owners).toBeLessThanOrEqual(195);
  expect(perRow.signals).toBeLessThanOrEqual(6);
  expect(large.observers).toBe(1);
  expect(perRow.observations).toBe(1);
  expect(perRow.elements).toBe(9);
  expect(full.owners).toBeLessThanOrEqual(390009);
  expect(full.elements).toBe(18000);
  expect(full.observers).toBe(1);
  expect(full.observations).toBe(2000);
});

it("starts and releases comment presentation when a leaf gains and loses children", () => {
  const raw = "A passage to discuss";
  const parent: Node = { id: "parent", raw, page: "Big", parent: null, children: [], collapsed: false };
  const comment: Node = { id: "comment", raw: "A reply\ntine.quote:: passage", page: "Big", parent: "parent", children: [], collapsed: false };
  setDoc({ byId: { parent }, pages: [{ name: "Big", kind: "page", title: "Big", preBlock: null,
    roots: ["parent"], format: "md", readOnly: false, guide: false }], feed: ["Big"], loaded: true });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => {
    const index = createCommentIndex(() => ["parent"]);
    return <CommentIndexContext.Provider value={index}><Block id="parent" /></CommentIndexContext.Provider>;
  }, host);
  try {
    expect(host.querySelector(".comment-quote-anchor")).toBeNull();
    batch(() => { setDoc("byId", "comment", comment); setDoc("byId", "parent", "children", ["comment"]); });
    expect(host.querySelector(".comment-quote-anchor")?.textContent).toBe("passage");
    expect(host.querySelector("[data-block-id=comment]")).toBeNull();
    setDoc("byId", "parent", "children", []);
    expect(host.querySelector(".comment-quote-anchor")).toBeNull();
    setDoc("byId", "parent", "children", ["comment"]);
    expect(host.querySelector(".comment-quote-anchor")?.textContent).toBe("passage");
    const row = host.querySelector<HTMLElement>("[data-block-id=parent]")!;
    const tints = () => [...row.classList].filter(name => name.startsWith("author-tint-"));
    setDoc("byId", "parent", "raw", `${raw}\ntine.author:: Someone`);
    expect(tints()).toHaveLength(1);
    setDoc("byId", "parent", "raw", `${raw}\ntine.author:: Another writer`);
    expect(tints()).toHaveLength(1);
    setDoc("byId", "parent", "raw", raw);
    expect(tints()).toEqual([]);
    expect(row.classList.contains("authored")).toBe(false);
  } finally { dispose(); host.remove(); }
});

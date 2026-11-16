import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { bumpGraphEpoch } from "../graphSession";
import { Block, CollapseSurfaceContext } from "./Block";
import { createQueryFold } from "./macroFold";
import { createRoot } from "solid-js";

const calls = vi.hoisted(() => vi.fn());
vi.mock("../backend", async (original) => {
  const actual = await original<typeof import("../backend")>();
  return { ...actual, backend: () => new Proxy(actual.backend(), {
    get(target, key, receiver) {
      const value = Reflect.get(target, key, receiver);
      return typeof value === "function" ? (...args: unknown[]) => {
        calls(String(key)); return value.apply(target, args);
      } : value;
    },
  }) };
});

it("a read-only query's header and bullet share a session fold without remembering a setting", () => {
  loadSingle({ name: "Query", title: "Query", kind: "page", pre_block: null, read_only: true,
    blocks: [{ id: "query", raw: "{{query (task TODO)}}\ncollapsed:: true", collapsed: true, children: [] }] });
  createRoot(dispose => {
    const remember = vi.fn();
    const fold = createQueryFold(null, () => false, () => "query", remember);
    try {
      expect(fold.collapsed()).toBe(true);
      fold.toggle();
      expect(fold.collapsed()).toBe(false);
      expect(doc.byId.query.collapsed).toBe(true);
      expect(remember).not.toHaveBeenCalled();
    } finally { dispose(); }
  });
});
beforeAll(() => initParser());
afterEach(() => { resetStore(); bumpGraphEpoch(); document.body.replaceChildren(); });

it.each([false, true])("read-only fold toggles both ways without document or backend writes (surface=%s)", async (secondary) => {
  loadSingle({ name: "Read only", title: "Read only", kind: "page", format: "md", pre_block: null, read_only: true,
    blocks: [{ id: "parent", raw: "Parent\ncollapsed:: true", collapsed: true,
      children: [{ id: "child", raw: "Hidden child", collapsed: false, children: [] }] }] });
  const before = JSON.stringify(doc);
  const host = document.createElement("div"); document.body.append(host);
  const surfaceToggle = vi.fn();
  const mount = () => render(() => secondary
    ? <CollapseSurfaceContext.Provider value={{ collapsed: (_id, stored) => stored, toggle: surfaceToggle, setMany: surfaceToggle }}><Block id="parent" /></CollapseSurfaceContext.Provider>
    : <Block id="parent" />, host);
  let dispose = mount();
  try {
    await new Promise(resolve => setTimeout(resolve, 0)); calls.mockClear();
    const toggle = () => host.querySelector<HTMLElement>(".collapse-toggle")!.click();
    expect(host.textContent).not.toContain("Hidden child");
    toggle();
    expect(host.textContent).toContain("Hidden child");
    toggle();
    expect(host.textContent).not.toContain("Hidden child");
    toggle();
    dispose(); dispose = mount(); // leaving and returning to a page retains the view fold
    expect(host.textContent).toContain("Hidden child");
    expect(JSON.stringify(doc)).toBe(before);
    expect(surfaceToggle).not.toHaveBeenCalled();
    expect(calls).not.toHaveBeenCalled();
    bumpGraphEpoch(); // a fresh graph session restores the authored state
    expect(host.textContent).not.toContain("Hidden child");
  } finally { dispose(); }
});

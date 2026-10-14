import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { loadSingle } from "../document/workingSet";
import { resetStore } from "../document";
import { initParser } from "../render/parse";
import type { BlockDto } from "../types";

beforeAll(() => initParser());
afterEach(() => { resetStore(); document.body.replaceChildren(); vi.restoreAllMocks(); });

it("offers each parent through the root without a depth cap (GH #643)", () => {
  let block: BlockDto = { id: "leaf", raw: "leaf", collapsed: false, children: [] };
  for (let depth = 10; depth >= 0; depth--) block = { id: `parent-${depth}`, raw: `parent ${depth}`, collapsed: false, children: [block] };
  loadSingle({ name: "Nested", title: "Nested", kind: "page", pre_block: null, blocks: [block] });
  const root = document.createElement("div"); root.className = "main-content"; document.body.append(root);
  const dispose = render(() => <Block id="parent-0" />, root);
  try {
    expect(root.querySelectorAll(".sticky-ancestor")).toHaveLength(11);
    expect(root.querySelector('[data-sticky-block="parent-10"] button')?.textContent).toContain("parent 10");
    expect(root.querySelector('[data-sticky-block="leaf"]')).toBeNull();
  } finally { dispose(); }
});

it("pins a zoom/sidebar root whose children are forced visible despite its saved fold", () => {
  loadSingle({ name: "Zoomed", title: "Zoomed", kind: "page", pre_block: null,
    blocks: [{ id: "folded", raw: "folded parent", collapsed: true, children: [{ id: "child", raw: "child", collapsed: false, children: [] }] }] });
  const root = document.createElement("div"); root.className = "main-content"; document.body.append(root);
  const dispose = render(() => <Block id="folded" forceExpanded />, root);
  try {
    expect(root.querySelector('[data-block-id="child"]')).not.toBeNull();
    expect(root.querySelector('[data-sticky-block="folded"]')).not.toBeNull();
  } finally { dispose(); }
});

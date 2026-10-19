import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { stickyZ } from "./StickyAncestor";
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

it("GH #643: a pinned parent shows its rendered first line and a bullet in the row's bullet column", () => {
  loadSingle({ name: "Marked", title: "Marked", kind: "page", pre_block: null,
    blocks: [{ id: "p", raw: "Rim **Taclepef** [[Some page]]\nsecond line", collapsed: false,
      children: [{ id: "c", raw: "child", collapsed: false, children: [] }] }] });
  const root = document.createElement("div"); root.className = "main-content"; document.body.append(root);
  const dispose = render(() => <Block id="p" />, root);
  try {
    const button = root.querySelector('[data-sticky-block="p"] button')!;
    expect(button.querySelector("strong")?.textContent).toBe("Taclepef");
    expect(button.textContent).not.toContain("**");
    expect(button.querySelector(".page-ref")?.textContent).toContain("Some page");
    expect(button.textContent).not.toContain("second line");
    expect(button.querySelector(".sticky-ancestor-toggle")).not.toBeNull();
    expect(button.querySelector(".sticky-ancestor-bullet > .sticky-ancestor-dot")).not.toBeNull();
  } finally { dispose(); }
});

it("GH #643: pins stay below menus, toolbars and modals at every depth", () => {
  for (const depth of [0, 1, 5, 10, 40]) {
    expect(stickyZ(depth)).toBeGreaterThan(8);
    expect(stickyZ(depth)).toBeLessThan(20);
  }
  expect(stickyZ(0)).toBeGreaterThan(stickyZ(1));
  let block: BlockDto = { id: "leaf", raw: "leaf", collapsed: false, children: [] };
  block = { id: "top", raw: "top", collapsed: false, children: [block] };
  loadSingle({ name: "Z", title: "Z", kind: "page", pre_block: null, blocks: [block] });
  const root = document.createElement("div"); root.className = "main-content"; document.body.append(root);
  const dispose = render(() => <Block id="top" />, root);
  try {
    expect(Number(root.querySelector<HTMLElement>('[data-sticky-block="top"]')!.style.zIndex)).toBeLessThan(20);
  } finally { dispose(); }
});

it("GH #619 c6: a query or embed host with children is not pinned, so its live macro is not mounted twice", () => {
  for (const raw of ["{{query (task TODO)}}", '{{tine-query @block and task in (\'TODO\')}}', "{{embed [[Other]]}}"]) {
    loadSingle({ name: "Hosts", title: "Hosts", kind: "page", pre_block: null,
      blocks: [{ id: "host", raw, collapsed: false, children: [{ id: "c", raw: "child", collapsed: false, children: [] }] }] });
    const root = document.createElement("div"); root.className = "main-content"; document.body.append(root);
    const dispose = render(() => <Block id="host" />, root);
    try {
      expect(root.querySelector('[data-sticky-block="host"]'), raw).toBeNull();
      expect(root.querySelector(".sticky-ancestor .query-block, .sticky-ancestor .embed-block"), raw).toBeNull();
      expect(root.querySelector('[data-block-id="c"]'), raw).not.toBeNull();
    } finally { dispose(); resetStore(); root.remove(); }
  }
});

it("GH #717: the pin keeps its whole 30 px in flow so it can never hang below its own block", () => {
  // jsdom has no layout; scripts/shot-sticky-overlap.mjs proves the geometry in a
  // real browser. A negative bottom margin on the pin shrinks the margin box that
  // `position: sticky` constrains to zero height and lets the pin cover the next
  // row at its own level (the reporter's `1a` over `1b` and `2`).
  const css = readFileSync(resolve(__dirname, "../styles/app.css"), "utf8");
  const pin = css.match(/\n\.sticky-ancestor \{([^}]*)\}/)![1];
  expect(pin).toContain("position: sticky");
  expect(pin).not.toMatch(/margin-bottom\s*:\s*-/);
  expect(css).toMatch(/\n\[data-sticky-parent\] > \.block-main \{ margin-top: -30px; \}/);
});

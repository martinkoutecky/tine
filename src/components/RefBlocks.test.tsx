import { afterEach, beforeAll, describe, it, expect } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { RefBlocks } from "./RefBlocks";
import { initParser } from "../render/parse";
import type { BlockDto } from "../types";
import { rightSidebar, setRightSidebar, setRightSidebarOpen } from "../ui";
import { layoutPaneIds, paneRouter, resetPaneLayoutToSingle } from "../panes";

// RefBlocks is the read-only renderer for query results / linked references / embeds
// (the lazy fallback in LiveRefGroup, and the permanent renderer for id-less result
// blocks whose generated uuid never resolves to a loaded page). It reads header facets
// straight off the shipped DTO. It used to render the marker but SILENTLY DROP the
// `[#A]` priority chip — so a priority-A todo surfaced by `(priority A)` showed without
// its priority in the query view, while the same block rendered by the live <Block>
// (Block.tsx) showed it. This guards that RefBlocks stays at parity with <Block>.

beforeAll(async () => {
  await initParser();
});

const booksSnapshot = () => ({
  tabs: [{
    history: [{ kind: "page" as const, name: "Books", pageKind: "page" as const }],
    pos: 0,
    pinned: false,
  }],
  activeIndex: 0,
});

afterEach(() => {
  setRightSidebar([]);
  setRightSidebarOpen(false);
  resetPaneLayoutToSingle(booksSnapshot());
  document.body.innerHTML = "";
});

function html(node: () => JSX.Element): { html: string; text: string } {
  const div = document.createElement("div");
  const dispose = render(() => node(), div);
  // innerHTML for class assertions; textContent for the chip glyph (Solid splices an
  // invisible <!----> marker around the interpolated `[#{priority}]`, so the raw HTML
  // string reads `[#A<!---->]` — the rendered text is `[#A]`, same as Block.tsx:583).
  const out = { html: div.innerHTML, text: div.textContent ?? "" };
  dispose();
  return out;
}

function pageRefLabels(markup: string): string[] {
  const div = document.createElement("div");
  div.innerHTML = markup;
  return [...div.querySelectorAll<HTMLAnchorElement>("a.page-ref")]
    .map((anchor) => anchor.textContent ?? "");
}

const dto = (over: Partial<BlockDto>): BlockDto => ({
  id: "x",
  raw: "TODO [#A] Ship the slice",
  collapsed: false,
  children: [],
  ...over,
});

describe("RefBlocks priority chip", () => {
  it("renders the backend's maximum 512-level DTO without overflowing", () => {
    let block = dto({ id: "leaf", raw: "leaf" });
    for (let depth = 1; depth < 512; depth++) block = dto({ id: `level-${depth}`, raw: `level ${depth}`, children: [block] });
    const out = html(() => RefBlocks({ blocks: [block] }));
    expect(out.text, "I-22: RefBlocks truncates a backend-valid 512-level tree before Solid overflows")
      .toContain("More nested blocks are available on the page");
    expect(out.text).toContain("level 448");
  });
  it("keeps a 64-level reference tree fully visible", () => {
    let block = dto({ id: "leaf", raw: "leaf" });
    for (let depth = 1; depth < 64; depth++) block = dto({ id: `level-${depth}`, raw: `level ${depth}`, children: [block] });
    const out = html(() => RefBlocks({ blocks: [block] }));
    expect(out.text).toContain("leaf");
    expect(out.text).not.toContain("More nested blocks are available on the page");
  });
  it("renders the [#A] chip from the DTO facet (parity with <Block>)", () => {
    const out = html(() => RefBlocks({ blocks: [dto({ marker: "TODO", priority: "A" })] }));
    expect(out.html).toContain("block-priority");
    expect(out.html).toContain("priority-A");
    expect(out.text).toContain("[#A]");
  });

  it("omits the chip when the block has no priority", () => {
    const out = html(() =>
      RefBlocks({ blocks: [dto({ raw: "TODO plain task", marker: "TODO", priority: undefined })] })
    );
    expect(out.html).not.toContain("block-priority");
  });
});

describe("RefBlocks task checkbox (parity with <Block>, OG block-checkbox)", () => {
  it("renders an UNCHECKED checkbox for an open task marker", () => {
    const out = html(() =>
      RefBlocks({ blocks: [dto({ raw: "TODO plain task", marker: "TODO", priority: undefined })] })
    );
    expect(out.html).toContain("block-task-checkbox");
    expect(out.html).toContain('aria-checked="false"');
  });

  it("renders a CHECKED checkbox for a DONE task", () => {
    const out = html(() =>
      RefBlocks({ blocks: [dto({ raw: "DONE plain task", marker: "DONE", priority: undefined })] })
    );
    expect(out.html).toContain("block-task-checkbox");
    expect(out.html).toContain("checked");
    expect(out.html).toContain('aria-checked="true"');
  });

  it("renders NO checkbox for a non-task block or a CANCELED marker", () => {
    const plain = html(() => RefBlocks({ blocks: [dto({ raw: "just a note", marker: undefined })] }));
    expect(plain.html).not.toContain("block-task-checkbox");
    const canceled = html(() =>
      RefBlocks({ blocks: [dto({ raw: "CANCELED dropped", marker: "CANCELED", priority: undefined })] })
    );
    expect(canceled.html).not.toContain("block-task-checkbox");
  });
});

describe("RefBlocks page-property references", () => {
  it("renders synthetic page-property backlinks as parsed keys and linkified values (GH #212)", () => {
    const out = html(() => RefBlocks({
      page: "Books",
      blocks: [dto({
        id: "page-property:Books",
        raw: "tags:: blah\nalias:: Reading\nowner:: [[Martin]]",
        page_property: true,
        marker: undefined,
        priority: undefined,
      })],
    }));
    expect(out.html).toContain("page-property-reference");
    expect(out.text).toContain("tags");
    expect(out.text).toContain("alias");
    expect(out.text).toContain("owner");
    expect(out.text).not.toContain("::");
    expect(pageRefLabels(out.html)).toEqual(["blah", "Reading", "[[Martin]]"]);
  });

  it("keeps an ordinary reference block on the existing inline-rendering path", () => {
    const out = html(() => RefBlocks({
      page: "Books",
      blocks: [dto({
        id: "ordinary-reference",
        raw: "Read [[blah]] next",
        marker: undefined,
        priority: undefined,
      })],
    }));
    expect(out.html).not.toContain("page-property-reference");
    expect(out.text).toBe("Read [[blah]] next");
    expect(pageRefLabels(out.html)).toEqual(["[[blah]]"]);
  });
});

describe("RefBlocks external identity", () => {
  it("keeps the runtime DOM id while exposing and opening the authored sidebar reference", () => {
    const runtimeId = "runtime-ref-block";
    const authoredId = "authored-ref-block";
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => RefBlocks({
      page: "Books",
      blocks: [dto({
        id: runtimeId,
        raw: "Read this",
        properties: [["ID", authoredId]],
      })],
    }), root);
    try {
      const row = root.querySelector<HTMLElement>(".ref-block")!;
      expect(row.getAttribute("data-block-id")).toBe(runtimeId);
      expect(row.getAttribute("data-block-ref")).toBe(authoredId);
      row.querySelector<HTMLElement>(".bullet-container")!.dispatchEvent(
        new MouseEvent("click", { bubbles: true, shiftKey: true })
      );
      expect(rightSidebar()).toEqual([{
        kind: "block", uuid: authoredId, page: "Books", pageKind: "page",
      }]);
    } finally {
      dispose();
    }
  });
});

// GH #456: the same modifier ladder as the live outline's bullet. A reference
// bullet answered only Shift, so Ctrl/Cmd and Alt did nothing here either.
describe("reference bullet modified clicks (GH #456)", () => {
  function mountRef(): { bullet: HTMLElement; dispose: () => void } {
    resetPaneLayoutToSingle(booksSnapshot());
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => RefBlocks({
      page: "Books",
      blocks: [dto({ id: "runtime", raw: "Read this", properties: [["ID", "authored-ref"]] })],
    }), root);
    return { bullet: root.querySelector<HTMLElement>(".bullet-container")!, dispose };
  }
  function click(el: HTMLElement, init: MouseEventInit) {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, ...init }));
  }

  it("opens a background tab on Ctrl/Cmd+click", () => {
    const { bullet, dispose } = mountRef();
    try {
      const before = paneRouter("main").tabs().length;
      click(bullet, { ctrlKey: true });
      const tabs = paneRouter("main").tabs();
      expect(tabs.length).toBe(before + 1);
      expect(tabs.some((t) => {
        const r = t.history[t.pos];
        return r.kind === "page" && r.name === "Books" && r.block === "authored-ref";
      })).toBe(true);
    } finally {
      dispose();
    }
  });

  it("opens the other pane on Alt+click", () => {
    const { bullet, dispose } = mountRef();
    try {
      expect(layoutPaneIds()).toEqual(["main"]);
      click(bullet, { altKey: true });
      expect(layoutPaneIds().length).toBe(2);
    } finally {
      dispose();
    }
  });

  it("suppresses the browser defaults those gestures replace (GH #207)", () => {
    const { bullet, dispose } = mountRef();
    try {
      const press = (init: MouseEventInit) => {
        const e = new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, ...init });
        bullet.dispatchEvent(e);
        return e.defaultPrevented;
      };
      expect(press({ shiftKey: true })).toBe(true);
      expect(press({ button: 1 })).toBe(true);
      expect(press({})).toBe(false);
    } finally {
      dispose();
    }
  });

  it("leaves an unmodified click alone — a read-only reference has no in-place zoom", () => {
    const { bullet, dispose } = mountRef();
    try {
      const before = paneRouter("main").tabs().length;
      click(bullet, {});
      expect(paneRouter("main").tabs().length).toBe(before);
      expect(layoutPaneIds()).toEqual(["main"]);
      expect(rightSidebar()).toEqual([]);
    } finally {
      dispose();
    }
  });
});

import { describe, it, expect, beforeAll } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { RefBlocks } from "./RefBlocks";
import { initParser } from "../render/parse";
import type { BlockDto } from "../types";

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

const dto = (over: Partial<BlockDto>): BlockDto => ({
  id: "x",
  raw: "TODO [#A] Ship the slice",
  collapsed: false,
  children: [],
  ...over,
});

describe("RefBlocks priority chip", () => {
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

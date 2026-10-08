import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { AstBody } from "./body";
import { initParser, parseBlock } from "./parse";
import { visibleBody } from "./block";
import { parseBody } from "./facets";

// GH #506 research fixture — RESEARCH-ONLY branch off base fc37445f.
//
// Report: a bare block reference `((uuid))` to a block whose content contains
// a Shift+Enter soft line break — on disk `- A\n  B` (plus `  id:: uuid`) —
// displays only "A"; the content after the soft break ("B") is missing.
//
// Measured layer chain on this exact base (full evidence in the batch report):
//   1. tine-core `doc::parse` folds the two-space continuation into ONE block
//      (raw `"A\nB\nid:: uuid"`, visible_text `"A\nB"`) — probe against the
//      crate at this commit; see the report for the command.
//   2. lsdoc `parseBlock` keeps both lines of the body in one parse.
//   3. The SOURCE block renders both lines (`A<br>B` via renderBlocks).
//   4. `BlockRefView` (src/render/inline.tsx, `visibleBody(targetRaw())[0]`)
//      takes only the FIRST line — this is the defect the report describes.
//   5. The `{{embed ((uuid))}}` sibling renders both lines and is NOT affected.
//
// The `CURRENT (GH #506)` assertions deliberately PIN the reported truncation so
// this branch stays green on the unfixed base while making the defect visible
// in the suite. The fix must FLIP the two marked assertions to the commented
// full-content expectations.

const ID = "66660600-0000-4000-8000-000000000506";

// NOTE: JSX attribute string literals do not interpret \n — pass multiline
// raws as JS expressions, not attribute literals.
const TARGET_RAW = `A\nB\nid:: ${ID}`;

function setup(targetRaw: string, refText: string) {
  const target = {
    id: ID,
    raw: targetRaw,
    collapsed: false,
    children: [],
    properties: [["id", ID]] as [string, string][],
  };
  loadSingle({
    kind: "page",
    name: "Reference source",
    title: "Reference source",
    pre_block: null,
    format: "md" as const,
    blocks: [target],
  });
  vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([
    { page: "Reference source", kind: "page" as const, blocks: [target] },
  ]);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <AstBody raw={refText} />, host);
  return { host, dispose };
}

describe("block reference to a soft-line-break block (GH #506)", () => {
  beforeAll(async () => {
    await initParser();
  });
  afterEach(() => {
    resetStore();
    vi.restoreAllMocks();
    document.body.replaceChildren();
  });

  it("lsdoc parses the soft-break body as one block parse retaining both lines", () => {
    // The parser layer is NOT the truncating layer: the body of one block with
    // a soft break parses to inline-flow nodes carrying BOTH lines.
    const blocks = parseBlock("A\nB", false);
    expect(blocks.map((b) => b.kind)).toEqual(["bullet", "paragraph"]);
    expect(blocks[0]).toMatchObject({ inline: [{ k: "plain", text: "A" }] });
    expect(blocks[1]).toMatchObject({ inline: [{ k: "plain", text: "B" }] });
    expect(parseBody("A\nB", "md")).toBe(blocks);
  });

  it("visibleBody keeps both soft-break lines (the extractor is not the truncating layer)", () => {
    expect(visibleBody(TARGET_RAW)).toEqual(["A", "B"]);
  });

  it("the SOURCE block itself renders both soft-break lines", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <AstBody raw={"A\nB"} />, host);
    try {
      expect(host.textContent).toContain("A");
      expect(host.textContent).toContain("B");
      expect(host.querySelectorAll("br").length).toBe(1);
    } finally {
      dispose();
    }
  });

  it("renders every visible line of a bare ((ref)) across a soft break", async () => {
    const { host, dispose } = setup(TARGET_RAW, `((${ID}))`);
    try {
      await vi.waitFor(() => expect(host.querySelector(".block-ref")).toBeTruthy());
      expect(host.querySelector(".block-ref")?.textContent).toBe("AB");
      expect(host.querySelector(".block-ref")?.textContent).toContain("B");
      expect(host.querySelectorAll(".block-ref br")).toHaveLength(1);
    } finally {
      dispose();
    }
  });

  it("sibling: {{embed ((uuid))}} renders both soft-break lines and is not affected", async () => {
    const { host, dispose } = setup(TARGET_RAW, `{{embed ((${ID}))}}`);
    try {
      // LiveRefGroup renders the embedded target's full body: A <br> B —
      // the DOM shape the inline ref should converge on (see the fixture note
      // in the CURRENT test above).
      await vi.waitFor(() => expect(host.querySelector(".embed-block .block-content")).toBeTruthy(), { timeout: 3000 });
      const text = host.querySelector(".embed-block")?.textContent ?? "";
      expect(text).toContain("A");
      expect(text).toContain("B");
      expect(host.querySelectorAll(".embed-block .block-content br").length).toBeGreaterThanOrEqual(1);
    } finally {
      dispose();
    }
  });
});

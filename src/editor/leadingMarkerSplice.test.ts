// C3 L14 (og c3s F3): a task marker after leading whitespace (a file line
// `-  TODO buy milk` keeps the extra space in the block raw) must be rewritten
// at its recognized offsets by every marker edit, never sliced from index 0.
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { node as docNode, resetStore } from "../document";
import { doc, setDoc, type FeedPage, type Node } from "../document/model";
import { setWorkflow } from "../ui";
import { toggleBlockCheckbox } from "../components/blockParts";
import { cycleField, writeField } from "../sheet/fields";
import { cycleMarkerSmart, rollRepeat, toggleTaskDone } from "./repeat";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  setWorkflow("todo");
});

function load(raw: string) {
  const page: FeedPage = {
    name: "P", kind: "page", title: "P", preBlock: null, roots: ["a"],
    format: "md", readOnly: false, guide: false,
  };
  const n: Node = { id: "a", raw, collapsed: false, parent: null, page: "P", children: [] };
  setWorkflow("todo");
  setDoc({ byId: { a: n }, pages: [page], feed: ["P"], loaded: true });
}

const REPEAT = "\nSCHEDULED: <2026-01-01 Thu +1d>";

describe("leading-whitespace task marker edits keep the title", () => {
  it("the block checkbox checks and unchecks without eating the title", () => {
    load(" TODO buy milk");
    toggleBlockCheckbox("a");
    expect(docNode("a").raw).toBe(" DONE buy milk");
    toggleBlockCheckbox("a");
    expect(docNode("a").raw).toBe(" TODO buy milk");
  });

  it("the block checkbox on a repeating task rolls it without eating the title", () => {
    load(` DOING water${REPEAT}`);
    toggleBlockCheckbox("a");
    expect(docNode("a").raw).toBe(" TODO water\nSCHEDULED: <2026-01-02 Fri +1d>");
  });

  it("a newline before the marker keeps the marker line intact", () => {
    expect(toggleTaskDone("\nTODO x", "todo", "md")).toBe("\nDONE x");
    expect(rollRepeat(`\nTODO x${REPEAT}`, "todo", "md")).toBe("\nTODO x\nSCHEDULED: <2026-01-02 Fri +1d>");
  });

  it("the sheet state field (cycle and set) keeps the title", () => {
    load(` DOING jog${REPEAT}`);
    expect(cycleField("a", "state")).toBe(true);
    expect(docNode("a").raw).toBe(" TODO jog\nSCHEDULED: <2026-01-02 Fri +1d>");
    load(" DOING jog");
    expect(writeField("a", "state", "DONE")).toBe(true);
    expect(docNode("a").raw).toBe(" DONE jog");
  });

  it("Ctrl+Enter's repeating roll reports the marker's own length change", () => {
    const out = cycleMarkerSmart(` DOING jog${REPEAT}`, "todo", "md");
    expect(out.raw).toBe(" TODO jog\nSCHEDULED: <2026-01-02 Fri +1d>");
    expect(out.delta).toBe(-1);
  });
});

void doc;

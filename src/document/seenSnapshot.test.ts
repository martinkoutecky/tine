import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { setGraphMeta } from "../graphSession";
import { resetStore, setRaw } from "./index";
import { loadSingle } from "./workingSet";
import { seenPageSnapshot } from "./seenSnapshot";
import type { PageDto } from "../types";

beforeAll(() => initParser());
beforeEach(() => { resetStore(); setGraphMeta(null); });

const block = (id: string, raw: string, children: PageDto["blocks"] = []) => ({ id, raw, collapsed: false, children });
const load = (over: Partial<PageDto> & { id?: string }) =>
  loadSingle({ name: "P", kind: "page", title: "P", pre_block: null, blocks: [], format: "md", id: "pages/P.md", ...over } as PageDto);

describe("the seen snapshot of a loaded page (ADR 0079)", () => {
  it("serialises the tree as the file would read and maps structural paths to runtime ids", () => {
    load({ blocks: [block("a", "A\nstatus:: open", [block("a1", "A1")]), block("b", "B")] });
    const snapshot = seenPageSnapshot("P")!;
    expect(snapshot.path).toBe("pages/P.md");
    expect(snapshot.text).toBe("- A\n  status:: open\n\t- A1\n- B\n");
    expect(snapshot.roots.map((r) => [r.id, r.children.map((c) => c.id)])).toEqual([["a", ["a1"]], ["b", []]]);
  });

  it("carries unsaved edits and the header before the first block", () => {
    load({ pre_block: "title:: P", blocks: [block("a", "A")] });
    setRaw("a", "A edited");
    const snapshot = seenPageSnapshot("P")!;
    expect(snapshot.text).toBe("title:: P\n\n- A edited\n");
    expect(snapshot.roots[0].raw).toBe("A edited");
  });

  it("writes an Org page with stars", () => {
    load({ name: "O", title: "O", id: "pages/O.org", format: "org", blocks: [block("a", "A", [block("a1", "A1")])] });
    expect(seenPageSnapshot("O")!.text).toBe("* A\n** A1\n");
  });

  it("is null for a page that is not loaded or has no file yet", () => {
    expect(seenPageSnapshot("Nope")).toBeNull();
    load({ id: undefined, blocks: [block("a", "A")] });
    expect(seenPageSnapshot("P")).toBeNull();
  });
});

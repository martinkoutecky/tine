import { afterEach, describe, expect, it } from "vitest";
import { splitProps, isBuiltinHidden, hideAll, rawOffsetToVisibleOffset } from "./properties";
import { insertOutlineAfter, splitBlock } from "../document/edits/blocks";
import { loadSingle, resetStore } from "../document/workingSet";
import { doc } from "../document/model";

afterEach(() => resetStore());

describe("OG-R4B: hidden properties use parser-owned regions", () => {
  const raw = "DONE Résumé\nCLOSED: [2026-09-30 Wed 09:00]\n:PROPERTIES:\n:id: own-id\n:END:\nbody";
  it("hides the own Org drawer after CLOSED planning", () => {
    expect(splitProps(raw, isBuiltinHidden, "org")).toEqual({
      visible: "DONE Résumé\nCLOSED: [2026-09-30 Wed 09:00]\nbody",
      hidden: ":id: own-id",
    });
    const at = rawOffsetToVisibleOffset(raw, raw.indexOf("body"), isBuiltinHidden, "org");
    expect(splitProps(raw, isBuiltinHidden, "org").visible.slice(at)).toBe("body");
  });

  it("splits an Org block without copying its own ID into the new block", () => {
    loadSingle({ name: "P", title: "P", kind: "page", format: "org", pre_block: null,
      blocks: [{ id: "b", raw, collapsed: false, children: [] }] });
    splitBlock("b", "DONE Résumé".length);
    const newId = doc.pages[0].roots[1];
    expect(newId).toBeDefined();
    expect(doc.byId[newId!].raw).not.toContain(":id: own-id");
    expect(doc.byId.b.raw).toContain(":id: own-id");
  });

  it("pastes an Org outline without duplicating a loaded own ID after CLOSED planning", () => {
    loadSingle({ name: "P", title: "P", kind: "page", format: "org", pre_block: null,
      blocks: [{ id: "b", raw, collapsed: false, children: [] }] });
    const inserted = insertOutlineAfter("b", [{ raw, children: [] }]);
    expect(inserted).toBeTruthy();
    expect(doc.byId[inserted!].raw).not.toContain(":id: own-id");
    expect(doc.byId[inserted!].raw).toContain("CLOSED: [2026-09-30 Wed 09:00]");
    expect(doc.byId.b.raw).toBe(raw);
  });

  it("maps multibyte source spans and CRLF transport to the same own-property lines", () => {
    const text = raw.replaceAll("\n", "\r\n");
    const split = splitProps(text, isBuiltinHidden, "org");
    expect(split.hidden).toBe(":id: own-id\r");
    expect(split.visible).toBe("DONE Résumé\r\nCLOSED: [2026-09-30 Wed 09:00]\r\nbody");
    const at = rawOffsetToVisibleOffset(text, text.indexOf("body"), isBuiltinHidden, "org");
    expect(split.visible.slice(at)).toBe("body");
  });

  it("keeps metadata-looking content in a body drawer and literal wrapper", () => {
    for (const text of [
      "Title\nBody\n:PROPERTIES:\n:id: body-id\n:END:",
      "#+BEGIN_SRC text\n:id: literal-id\n#+END_SRC",
    ]) expect(splitProps(text, hideAll, "org")).toEqual({ visible: text, hidden: "" });
  });
});

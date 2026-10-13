import { afterEach, expect, it } from "vitest";
import { loadSingle } from "./workingSet";
import { doc } from "./model";
import { resetStore, insertOutlineAfter, insertOutlineBefore, insertOutlineChildren, replaceEmptyBlockWithOutline, undo, redo } from ".";
import { pageToDto } from "./convert";

afterEach(() => resetStore());

for (const format of ["md", "org"] as const) {
  for (const insert of [insertOutlineAfter, insertOutlineBefore, insertOutlineChildren, replaceEmptyBlockWithOutline]) {
    it(`${insert.name} derives saved collapse from parser metadata (${format}) and keeps it through undo/redo`, () => {
      loadSingle({ name: "Target", title: "Target", kind: "page", format, pre_block: null,
        blocks: [{ id: "host", raw: "", collapsed: false, children: [] }] });
      const property = (value: string) => format === "md" ? `\ncollapsed:: ${value}` : `\n:PROPERTIES:\n:collapsed: ${value}\n:END:`;
      const leaf = (raw: string) => ({ raw, children: [] });
      expect(insert("host", [
        { raw: `Folded${property("true")}`, children: [{ raw: `Nested${property("true")}`, children: [leaf("child")] }] },
        { raw: `Expanded${property("false")}`, children: [leaf("child")] },
        { raw: "Default", children: [leaf("child")] },
        leaf(format === "md" ? "```\ncollapsed:: true\n```" : "#+BEGIN_SRC\n:collapsed: true\n#+END_SRC"),
      ])).not.toBeNull();
      const check = () => {
        const nodes = Object.values(doc.byId);
        expect(nodes.find((b) => b.raw.startsWith("Folded"))?.collapsed).toBe(true);
        expect(nodes.find((b) => b.raw.startsWith("Nested"))?.collapsed).toBe(true);
        expect(nodes.filter((b) => !b.raw.startsWith("Folded") && !b.raw.startsWith("Nested")).every((b) => !b.collapsed)).toBe(true);
        expect(JSON.stringify(pageToDto("Target"))).toContain("collapsed");
      };
      check();
      undo();
      expect(Object.values(doc.byId).map((b) => b.raw)).toEqual([""]);
      redo();
      check();
    });
  }
}

import { expect, it } from "vitest";
import { exportHtml } from "./exportHtml";
import { exportOpml } from "./exportOpml";
import { DEFAULT_EXPORT_OPTIONS, exportOutline } from "./exportText";

const options = { stripLinks: false, removeEmphasis: false, removeTags: false };
it.each(["md", "org"] as const)("source Text, HTML and OPML retain %s literal metadata and body drawers", format => {
  const raw = format === "md" ? "Title\n```\nid:: literal\n```\nid:: real" :
    "Title\n:PROPERTIES:\n:id: real\n:END:\n#+BEGIN_SRC\n:id: literal\n#+END_SRC\nBody\n:PROPERTIES:\n:note: prose\n:END:";
  const nodes = [{ raw, format, children: [] }];
  const exports = [exportHtml(nodes, options), exportOpml(nodes, options),
    exportOutline(nodes, { ...DEFAULT_EXPORT_OPTIONS, ...options, content: "source", removeProperties: true })];
  for (const output of exports) {
    expect(output).toContain(format === "md" ? "id:: literal" : ":id: literal");
    expect(output).not.toContain(format === "md" ? "id:: real" : ":id: real");
    if (format === "org") expect(output).toContain(":note: prose");
  }
});

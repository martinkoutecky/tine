import { readFileSync } from "node:fs";
import { beforeAll, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { DEFAULT_EXPORT_OPTIONS, exportOutline, type ExportNode } from "./exportText";
import { exportHtml } from "./exportHtml";
import { exportOpml } from "./exportOpml";
const a = "12345678-1234-1234-1234-123456789abc", b = "22345678-1234-1234-1234-123456789abc";
const options = {...DEFAULT_EXPORT_OPTIONS, content:"source" as const, indent:"no-indent" as const};
beforeAll(initParser);
it("keeps property separators in plain text (GH #407 sibling)", () => {
  expect(exportOutline([{raw:"body\nx:: xx",children:[]}], {...options,content:"rendered"})).toBe("body\nx:: xx");
});
it("resolves nested Unicode refs, cleans their markup and preserves literal refs", () => {
  const nodes = [{raw:`é ((${a})) \`((${a}))\`\n\`\`\`\n((${a}))\n{{embed [[P]]}}\n\`\`\`\nid:: ${a}\nx:: [[P]]`,children:[]}];
  const opts = {...options, resolveBlockRef:(id:string) => ({raw:id === a ? `**é** ((${b}))` : "*終*",format:"md" as const})};
  const output = exportOutline(nodes, opts);
  expect(output).toContain("é **é** *終*");
  expect(output).toContain(`\`((${a}))\``);
  expect(output).toContain(`\n((${a}))\n{{embed [[P]]}}\n`);
  expect(output).not.toContain("id::");
  expect(output).toContain("x:: [[P]]");
  expect(exportOutline([{raw:`((${a}))`,children:[]}], {...opts,removeEmphasis:true})).toBe("é 終");
});
it("splices page/block embed trees, applies depth after expansion, and caps cycles at five", () => {
  const embedded: ExportNode[] = [{raw:"**root**",children:[{raw:"child",children:[]}]}];
  const opts = {...options,indent:"dashes" as const,resolveEmbed:()=>embedded};
  const nodes = [{raw:"{{embed [[P]]}}",children:[]}];
  expect(exportOutline(nodes,opts)).toBe("- **root**\n\t- child");
  expect(exportOutline(nodes,{...opts,maxDepth:1})).toBe("- **root**");
  expect(exportHtml(nodes,opts)).toContain("<strong>root</strong>");
  expect(exportOpml(nodes,opts)).toContain('text="child"');
  const cycle = {...options,resolveEmbed:()=>nodes};
  expect(exportOutline(nodes,cycle)).toBe("{{embed [[P]]}}");
  expect(exportOutline([{raw:`((${a}))`,children:[]}], {...options,resolveBlockRef:()=>({raw:`((${a}))`,format:"md"})})).toBe(`((${a}))`);
});
it("preserves empty blocks and removes properties only on request", () => {
  expect(exportOutline([{raw:"",children:[]}], {...options,indent:"dashes"})).toBe("-");
  expect(exportOutline([{raw:"text\nx:: xx",children:[]}], {...options,removeProperties:true})).toBe("text");
});

it("the canonical onboarding Guide explains resolved exports and property controls", () => {
  const guide = readFileSync("crates/tine-core/src/templates/files-external-edits-backups.md", "utf8");
  for (const detail of ["resolve block references", "expand page and block embeds", "formatting preserved", "five nested embeds", "`key:: value`", "**Remove properties**"]) expect(guide).toContain(detail);
});
it("block refs splice target inlines rather than their heading or task prefix", () => {
  expect(exportOutline([{raw:`((${a}))`,children:[]}], {...options,resolveBlockRef:()=>({raw:"### **Heading**\nx:: value",format:"md"})})).toBe("**Heading**");
});

it("keeps prose around multiple embeds in reading order", () => {
  const nodes = [{raw:"before {{embed [[P]]}} between {{embed [[P]]}} after",children:[]}];
  const output = exportOutline(nodes,{...options,resolveEmbed:()=>[{raw:"**embedded**",children:[]}]});
  expect(output.split("\n").map(line=>line.trim()).join("\n")).toBe("before\n**embedded**\nbetween\n**embedded**\nafter");
});

it("bounds expansion fan-out and names truncated embed output", () => {
  const nodes = [{raw:"{{embed [[P]]}}",children:[]}];
  const output = exportOutline(nodes,{...options,resolveEmbed:()=>Array.from({length:2001},()=>({raw:"item",children:[]}))});
  expect(output.split("\n").filter(line=>line==="item")).toHaveLength(2000);
  expect(output).toContain("[embed expansion truncated]");
});

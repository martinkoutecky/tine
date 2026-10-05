import { expect, it } from "vitest";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { callsImported, importsFrom } from "./tests/sourceOwnership";
function regexes(path: string): string[] {
  const file = ts.createSourceFile(path, readFileSync(path,"utf8"), ts.ScriptTarget.Latest, true);
  const found: string[] = [];
  const visit = (node: ts.Node) => {
    if (ts.isRegularExpressionLiteral(node) || ts.isNewExpression(node) && node.expression.getText(file) === "RegExp") found.push(node.getText(file));
    ts.forEachChild(node,visit);
  };
  visit(file); return found;
}
it("I-12 D15: sole macro recognition belongs to standalone_macro::sole_macro", () => {
  const ui = readFileSync("src/components/Rendered.tsx","utf8");
  const detect = ui.slice(ui.indexOf("export function detectMacro"), ui.indexOf("// `Block`"));
  expect(callsImported("src/components/Rendered.tsx", "soleBlockMacro", "src/render/parse", "detectMacro"), "I-12: imitate soleBlockMacro in render/parse.ts; never strip property lines or recognize embed braces").toBe(true);
  expect(callsImported("src/components/Block.tsx", "detectMacro", "src/components/Rendered"), "I-12: Block recognizes macros through detectMacro").toBe(true);
  expect(readFileSync("src/components/Block.tsx","utf8"), "I-12: retain the containing format; a Markdown parse of Org adds per-keystroke work").toMatch(/detectMacro\(node\(\)\.raw,\s*fmt\(\)\)/);
  expect(detect).not.toMatch(/\.split\(|\.filter\(|\.exec\(/);
  const native = readFileSync("crates/tine-graph-features/src/render_sheets.rs","utf8");
  const sole = native.slice(native.indexOf("pub(super) fn sole_query_macro"),native.indexOf("/// FNV", native.indexOf("pub(super) fn sole_query_macro")));
  expect(sole, "I-12: static export must use standalone_macro::sole_macro, not recognize HTML").toContain("standalone_macro::sole_macro");
  expect(sole).not.toContain("render_html");
});
it("I-12 D16: export syntax belongs to the AST visitor in editor/exportMarkup.ts", () => {
  expect(regexes("src/editor/exportMarkup.ts"), "I-12: cleanInline parses AST spans, never regex syntax").toEqual(["/&/g", "/</g", "/>/g", '/"/g', "/'/g", "/\\t/g", "/\\r\\n?|\\n/g"]);
  const text = readFileSync("src/editor/exportText.ts","utf8");
  expect(callsImported("src/editor/exportText.ts", "cleanInline", "src/editor/exportMarkup"), "I-12: export text cleans inline syntax through the AST visitor").toBe(true);
  expect(text).not.toContain("function stripInline");
});
it("I-12 D17: journal format grammar belongs to native date::Format", () => {
  const source = readFileSync("src/journal.ts","utf8");
  expect(callsImported("src/journal.ts", "parse_journal_format_json", "src/render/wasm/lsdoc_wasm", "parseJournalWith"), "I-12: imitate parseJournalWith's parse_journal_format_json").toBe(true);
  expect(source).not.toMatch(/function daysInMonth|function ordinal|const MONTHS|const matchName/);
  expect(source).toContain("return journalParts(name) !== null");
});
it("I-12 D19: repeaters and schedule reads use parser planning.date; imitate editor/repeat.ts scheduleParts", () => {
  expect(regexes("src/editor/repeat.ts"), "I-12: read planning.date, never regex timestamp/cookie recognition").toEqual([]);
  const props = readFileSync("src/document/edits/properties.ts","utf8");
  const reader = props.slice(props.indexOf("export function readSchedule"),props.indexOf("/** Set or clear"));
  expect(callsImported("src/document/edits/properties.ts", "scheduleParts", "src/editor/repeat", "readSchedule"), "I-12: schedule reads go through the parser's planning.date").toBe(true);
  expect(reader).not.toContain(".exec(");
  const picker = readFileSync("src/components/DatePicker.tsx","utf8");
  expect(importsFrom("src/components/DatePicker.tsx", "parseRepeater", "src/editor/repeat"), "I-12: the picker imports parseRepeater from editor/repeat").toBe(true);
  expect(picker).not.toContain("function parseRepeater");
});

it("Guide describes literal export cleanup and timed repeater preservation", () => {
  expect(readFileSync("crates/tine-core/src/templates/files-external-edits-backups.md","utf8")).toContain("inline code and fenced code stays literal");
  expect(readFileSync("crates/tine-core/src/templates/journals-tasks-scheduling.md","utf8")).toContain("advances to the next date while keeping `09:00`");
});

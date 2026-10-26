import { expect, it } from "vitest";
import { t } from "./index";
// @ts-expect-error Plain ESM ratchet script.
import { scanSource } from "../../scripts/i18n-literals.mjs";

const files = import.meta.glob([
  "../components/{Block,BlockReferences,FormulaEditor,PdfExportDialog,PdfViewer,pdfHighlightState,pdfOutline,pdfViewerView}.ts*",
  "../editor/{autocomplete,dateExpr,linkDefault,properties,queryAggregate,searchQuery,selectionActions,templateVars}.ts",
  "../render/{PeekPopup,body,inline,rawHtml}.tsx",
  "../sheet/{conversions,mutations,selection,tableColumnResize}.ts*",
  "!../**/*.test.ts*",
], { query: "?raw", import: "default", eager: true }) as Record<string, string>;

it("editor, PDF and rendering UI text uses the catalog while graph syntax stays literal", () => {
  const sites = Object.entries(files).flatMap(([path, source]) =>
    scanSource(`src/${path.slice(3)}`, source).sites);
  const syntax = new Set(["TODO", "DOING", "LATER", "NOW", "DONE", "WAITING", "WAIT", "IN-PROGRESS", "CANCELED"]);
  const templateSyntax = new Set(["today", "yesterday", "tomorrow", "current page", "time"]);
  const remaining = sites.filter((site: { file: string; owner: string; text: string }) =>
    !(site.file === "src/editor/autocomplete.ts" && site.owner === "COMMAND_DEFINITIONS" && syntax.has(site.text)) &&
    !(site.file === "src/components/FormulaEditor.tsx" && (site.text === "formula." || site.text === "formula.{}")) &&
    !(site.file === "src/editor/templateVars.ts" && site.owner === "TEMPLATE_VARS" && templateSyntax.has(site.text)));
  expect(remaining).toEqual([]);
});

it("counted notices and reference labels preserve English sentences at singular, plural and large counts", () => {
  expect(t("editor-block-inserted-file", { count: 1 })).toBe("Inserted 1 file");
  expect(t("editor-block-inserted-file", { count: 3 })).toBe("Inserted 3 files");
  expect(t("editor-block-inserted-file", { count: 1001 })).toBe("Inserted 1001 files");
  expect(t("editor-references-linked-reference", { count: 1 })).toBe("1 Linked Reference");
  expect(t("editor-references-linked-reference", { count: 2 })).toBe("2 Linked References");
  expect(t("editor-references-linked-reference", { count: 1001 })).toBe("1001 Linked References");
  expect(t("editor-block-skipped-item-folders-and-byte-only-files-over-64-mib-aren-t-pasted", { count: 1 })).toBe("Skipped 1 item; folders and byte-only files over 64 MiB aren't pasted");
  expect(t("editor-references-references-from", { action: "expand", page: "作者" })).toBe("Expand references from 作者");
  expect(t("editor-references-references-from", { action: "collapse", page: "作者" })).toBe("Collapse references from 作者");
});

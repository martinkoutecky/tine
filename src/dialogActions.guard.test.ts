import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

/** D-20261018-UI1 / UI2: confirm and cancel sit on consistent, separate sides
 *  app-wide. The order lives in one primitive (`DialogActions`: aux, Cancel,
 *  primary, trailing end), the disk-vs-mine choice in `DiskOrMineChoice`, and
 *  every native confirm names its action. These scans keep a new dialog from
 *  hand-rolling its own button row. */

function sources(ext: RegExp, skipTests = true): string[] {
  const out: string[] = [];
  const pending = ["src"];
  while (pending.length > 0) {
    const dir = pending.pop()!;
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = `${dir}/${entry.name}`;
      if (entry.isDirectory()) pending.push(path);
      else if (ext.test(entry.name) && !(skipTests && /\.(test|spec)\./.test(entry.name))) out.push(path);
    }
  }
  return out;
}

/** Every `-foot` / `-actions` container class used in a frontend source that is
 *  not the primitive. Each is either an extra class on a `<DialogActions>` (its
 *  file uses the primitive) or a row that is not a confirm/cancel surface, with
 *  the reason. A new entry here is a decision, not a convenience. */
const NOT_A_CONFIRM_ROW: Record<string, string> = {
  // Extra classes handed to <DialogActions class=...>: sizing only, never order.
  "share-foot": "DialogActions extra class (ShareDialog)",
  "dp-foot": "DialogActions extra class (DatePicker); CalendarJump reuses the class for its single jump button",
  "qs-anchor-prompt-actions": "DialogActions extra class (anchor-switch prompt)",
  "query-advanced-actions": "DialogActions extra class (advanced query dialog)",
  "query-text-pane-actions": "DialogActions extra class (query text pane)",
  // Not confirm/cancel surfaces.
  "navigation-actions": "topbar navigation buttons",
  "capture-actions": "quick capture: one Capture button",
  "conflict-actions": "banner: DiskOrMineChoice or a single Review/Discard action",
  "page-conflict-foot": "hint text plus the single Apply resolution button",
  "sync-merge-toolbar-actions": "merge toolbar, not a dialog footer",
  "query-crossing-notice-actions": "notice: [Undo that change][Keep it]; Keep it (dismiss) trails",
  "diagnostics-actions": "settings action rows (Create report, Copy, Save, ...)",
  "embed-actions": "embed toolbar",
  "external-change-actions": "banner: DiskOrMineChoice",
  "region-failure-actions": "[Copy details][Retry]: auxiliary leading, primary trailing",
  "journal-conflict-actions": "list-row actions (Open, Merge, Rename, Trash)",
  "qc-panel-actions": "single Done button",
  "page-carry-actions": "page carry buttons, not a dialog",
  "page-title-actions": "page title toolbar",
  "pp-foot": "single Done button (page properties)",
  "qc-actions": "query block toolbar",
  "query-og-hint-actions": "hint actions, not a dialog",
  "query-selection-actions": "selection toolbar [All][None][Copy][Done]",
  "reference-excerpt-actions": "excerpt toolbar",
  "ref-export-actions": "selection toggles inside the chooser body",
  "rs-header-actions": "right sidebar header",
  "rs-item-actions": "right sidebar item",
  "settings-nav-foot": "settings navigation footer text",
  "welcome-actions": "welcome choices",
  "welcome-foot": "welcome footer text",
  "pdf-toolbar-actions": "PDF toolbar",
  "peek-page-actions": "peek popup menu button",
  "plugin-detail-actions": "plugin detail: a single Open source button",
  "welcome-recovery-actions": "welcome recovery: a single Retry",
};

describe("action rows go through one primitive (D-20261018-UI1)", () => {
  it("no frontend source hand-rolls a -foot/-actions row outside the reviewed list", () => {
    const found = new Map<string, Set<string>>();
    for (const file of sources(/\.tsx$/)) {
      const text = readFileSync(file, "utf8");
      for (const match of text.matchAll(/\bclass="([^"]*)"/g)) {
        for (const token of match[1].split(/\s+/)) {
          if (/-(foot|actions)$/.test(token) && token !== "dialog-actions") {
            (found.get(token) ?? found.set(token, new Set()).get(token)!).add(file);
          }
        }
      }
    }
    for (const [token, files] of found) {
      expect(NOT_A_CONFIRM_ROW, `new action row class \`${token}\` in ${[...files].join(", ")}: render confirm/cancel rows through <DialogActions> (src/components/DialogActions.tsx); if this is not a confirm/cancel surface, add it to NOT_A_CONFIRM_ROW with the reason`).toHaveProperty(token);
    }
    // Non-vacuity: the scan reaches the rows it is about.
    expect(found.has("qc-panel-actions")).toBe(true);
    expect(found.has("conflict-actions")).toBe(true);
  });

  it("every file that passes a class to <DialogActions> also imports it", () => {
    for (const file of sources(/\.tsx$/)) {
      const text = readFileSync(file, "utf8");
      if (/<DialogActions[\s>]/.test(text) && !file.endsWith("DialogActions.tsx")) {
        expect(text, `${file} renders <DialogActions> without importing it`).toMatch(/import \{ DialogActions \} from "\.\/DialogActions"/);
      }
    }
  });

  it("the dialogs this decision names render through the primitive", () => {
    const users = [
      "ReferenceExportChooser", "QueryExportDialog", "GraphNamePrompt", "ShareDialog", "PdfExportDialog", "ExportModal",
      "FormulaEditor", "QueryWorkspace", "DatePicker", "querySheetParts", "UnsavedRecovery", "CssEditor",
      "JournalConflictFileRow", "QueryBuilder",
    ];
    for (const name of users) {
      expect(readFileSync(`src/components/${name}.tsx`, "utf8"), `${name} must use <DialogActions>`).toContain("<DialogActions");
    }
  });

  it("DOM order is visual order: no row-reverse or order: on an action row", () => {
    for (const file of sources(/\.css$/)) {
      const css = readFileSync(file, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
      for (const block of css.split("}")) {
        const [selector, body] = block.split("{");
        if (body === undefined || !/(dialog-actions|-foot\b|-actions\b)/.test(selector)) continue;
        expect(body, `${file}: \`${selector.trim()}\` reorders an action row`).not.toMatch(/row-reverse|(^|[;\s])order\s*:/);
      }
    }
  });

  it("the old export-foot row and the date picker's Today button are gone", () => {
    for (const file of [...sources(/\.(tsx|css)$/)]) {
      expect(readFileSync(file, "utf8"), file).not.toMatch(/export-foot/);
    }
    expect(readFileSync("src/components/DatePicker.tsx", "utf8")).not.toMatch(/pickToday|date-picker-today/);
    expect(readFileSync("src/i18n/locales/en/date.ftl", "utf8")).not.toContain("date-date-picker-today");
  });
});

describe("one disk-vs-mine choice (D-20261018-UI2a)", () => {
  it("the labels have one owner and the three surfaces use it", () => {
    for (const file of sources(/\.tsx?$/)) {
      if (file.endsWith("DiskOrMineChoice.tsx") || file.endsWith(".gen.ts")) continue;
      expect(readFileSync(file, "utf8"), file).not.toMatch(/conflict-choice-(keep-mine|use-disk-version)/);
    }
    for (const file of ["ConflictBar", "ExternalChangeBar", "pdfViewerView"]) {
      expect(readFileSync(`src/components/${file}.tsx`, "utf8"), file).toContain("<DiskOrMineChoice");
    }
  });
});

describe("native confirms name their action (D-20261018-UI2c)", () => {
  it("every backend().confirm call passes okLabel and cancelLabel", () => {
    let calls = 0;
    for (const file of sources(/\.tsx?$/)) {
      const text = readFileSync(file, "utf8");
      for (const match of text.matchAll(/backend\(\)\.confirm\(/g)) {
        calls++;
        // The call ends at the first `));` / `)),` after its options object.
        const rest = text.slice(match.index!, match.index! + 900);
        const end = rest.search(/\}\)\)|\}\s*,?\s*\)\)/);
        expect(rest.slice(0, end < 0 ? rest.length : end + 3), `${file}: confirm without explicit labels`).toMatch(/okLabel:[\s\S]*cancelLabel:/);
      }
    }
    expect(calls).toBeGreaterThanOrEqual(18);
  });
});

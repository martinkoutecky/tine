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

/** Index just past the `}` matching the `{` at `open` (string literals skipped). */
function closeBrace(text: string, open: number): number {
  let depth = 0;
  for (let i = open; i < text.length; i++) {
    const c = text[i];
    if (c === '"' || c === "'" || c === "`") {
      for (i++; i < text.length && text[i] !== c; i++) if (text[i] === "\\") i++;
    } else if (c === "{") depth++;
    else if (c === "}" && --depth === 0) return i + 1;
  }
  return text.length;
}

/** The `<DialogActions ... />` elements of a source, as [start, end) ranges. */
function dialogActionRanges(text: string): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  for (const m of text.matchAll(/<DialogActions[\s>]/g)) {
    let i = m.index! + 1;
    while (i < text.length && !(text[i] === "/" && text[i + 1] === ">")) {
      if (text[i] === "{") i = closeBrace(text, i);
      else i++;
    }
    out.push([m.index!, i + 2]);
  }
  return out;
}

/** The `aux=` / `cancel=` / `primary=` expressions of one <DialogActions> element. */
function slotsOf(element: string): Record<"aux" | "cancel" | "primary", string | undefined> {
  const slots: Record<string, string | undefined> = {};
  for (let i = 1; i < element.length; i++) {
    if (element[i] === "{") {
      i = closeBrace(element, i) - 1;
      continue;
    }
    const m = /^(aux|cancel|primary)=\{/.exec(element.slice(i));
    if (m && /\s/.test(element[i - 1])) {
      const open = i + m[0].length - 1;
      const end = closeBrace(element, open);
      slots[m[1]] = element.slice(open + 1, end - 1);
      i = end - 1;
    }
  }
  return slots as Record<"aux" | "cancel" | "primary", string | undefined>;
}

/** Words that make a button a dismiss action rather than the confirming one. */
const DISMISS = /cancel|dismiss|keep-working|go-back|keep-editing|keep-open/i;
const DISMISS_OR_CLOSE = new RegExp(`${DISMISS.source}|close`, "i");

/** Outcome-level violations of the placement rule in one frontend source:
 *  a cancel button outside the primitive, or slot meanings that disagree with
 *  where the button lands (Cancel in `primary`, a confirming action in `cancel`). */
function actionRowViolations(file: string, text: string): string[] {
  const out: string[] = [];
  const ranges = file.endsWith("/DialogActions.tsx") ? [] : dialogActionRanges(text);
  for (const [a, b] of ranges) {
    const { cancel, primary } = slotsOf(text.slice(a, b));
    if (cancel !== undefined && !DISMISS_OR_CLOSE.test(cancel)) out.push(`${file}: the cancel slot holds a confirming action: ${cancel.trim().slice(0, 80)}`);
    if (primary !== undefined && DISMISS.test(primary)) out.push(`${file}: the primary slot holds a dismiss action: ${primary.trim().slice(0, 80)}`);
  }
  if (!file.endsWith("/DialogActions.tsx")) {
    for (const m of text.matchAll(/<button\b[\s\S]*?<\/button>/g)) {
      // The label is what follows the opening tag (`=>` inside the tag is not its end).
      const label = m[0].slice(m[0].search(/(?<!=)>/) + 1);
      if (!/cancel/i.test(label) || ranges.some(([a, b]) => m.index! >= a && m.index! < b)) continue;
      if (CANCEL_OUTSIDE_PRIMITIVE.some(([f, re]) => file.endsWith(f) && re.test(m[0]))) continue;
      out.push(`${file}: a Cancel button outside <DialogActions> (hand-rolled confirm row): ${m[0].replace(/\s+/g, " ").slice(0, 80)}`);
    }
  }
  return out;
}

/** CSS rules (innermost blocks, so a rule inside `@media` is seen) that reorder an action row. */
function actionRowCssViolations(file: string, source: string): string[] {
  const css = source.replace(/\/\*[\s\S]*?\*\//g, "");
  const out: string[] = [];
  const walk = (chunk: string) => {
    let i = 0;
    while (i < chunk.length) {
      const open = chunk.indexOf("{", i);
      if (open < 0) return;
      const close = closeBrace(chunk, open);
      const selector = chunk.slice(i, open).trim();
      const body = chunk.slice(open + 1, close - 1);
      if (body.includes("{")) walk(body);
      else if (/(dialog-actions|-foot\b|-actions\b)/.test(selector) && /row-reverse|(^|[;\s])order\s*:/.test(body)) {
        out.push(`${file}: \`${selector}\` reorders an action row`);
      }
      i = close;
    }
  };
  walk(css);
  return out;
}

/** Every `-foot` / `-actions` container class used in a frontend source that is
 *  not the primitive. Each is either an extra class on a `<DialogActions>` (its
 *  file uses the primitive) or a row that is not a confirm/cancel surface, with
 *  the reason. A new entry here is a decision, not a convenience. */
/** Cancel buttons that are not a dialog's confirm/cancel pair (file suffix, button text). */
const CANCEL_OUTSIDE_PRIMITIVE: Array<[string, RegExp]> = [
  // A choice list with no confirming action: [graph A][graph B]... then Cancel (census row 14, reviewed OK).
  ["/DeepLinkGraphChoice.tsx", /deep-link-graph-choice-cancel/],
  // Inline settings rows, not dialog footers (census: Settings rows): Cancel stops a running verification / closes the home-page picker.
  ["/GraphVerification.tsx", /graph-verification-cancel/],
  ["/HomePageSetting.tsx", /home-page-setting-cancel/],
];

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
  "quick-share-settings-actions": "Settings > Sharing: account and per-share row actions (Copy, Check, Delete, Forget), no Cancel",
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
          if (/-(foot|actions|buttons|btns)$/.test(token) && token !== "dialog-actions") {
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

  it("DOM order is visual order: no row-reverse or order: on an action row, in any media query", () => {
    for (const file of sources(/\.css$/)) {
      expect(actionRowCssViolations(file, readFileSync(file, "utf8"))).toEqual([]);
    }
  });

  it("Cancel and the primary action are one non-wrapping group (D-20261018-UI1)", () => {
    const css = readFileSync("src/styles/app.css", "utf8");
    expect(css).toMatch(/\.dialog-actions-main\s*\{[^}]*flex-wrap:\s*nowrap/);
    expect(readFileSync("src/components/DialogActions.tsx", "utf8")).toContain('class="dialog-actions-main"');
  });

  it("every <DialogActions> puts Cancel in `cancel` and the confirming action in `primary`; no Cancel button outside it", () => {
    let rows = 0;
    const violations: string[] = [];
    for (const file of sources(/\.tsx$/)) {
      const text = readFileSync(file, "utf8");
      rows += dialogActionRanges(text).length;
      violations.push(...actionRowViolations(file, text));
    }
    expect(violations).toEqual([]);
    expect(rows, "the scan reaches the migrated dialogs").toBeGreaterThanOrEqual(14);
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

describe("the guard fails on realistic violations (planted through the real scanners)", () => {
  it("a new dialog with a raw [Save][Cancel] row bypassing the primitive", () => {
    const planted = `export function NewDialog() { return (<div class="modal-overlay"><div class="confirm-buttons">
      <button onClick={save}>{t("new-dialog-save")}</button><button onClick={close}>{t("new-dialog-cancel")}</button></div></div>); }`;
    expect(actionRowViolations("src/components/NewDialog.tsx", planted).join("\n")).toMatch(/Cancel button outside <DialogActions>/);
  });

  it("slot meanings swapped in an existing dialog (the advanced-query dialog's Cancel and Apply)", () => {
    const file = "src/components/QueryWorkspace.tsx";
    const real = readFileSync(file, "utf8");
    expect(actionRowViolations(file, real)).toEqual([]);
    const [a, b] = dialogActionRanges(real).find(([x, y]) => /query-advanced-actions/.test(real.slice(x, y)))!;
    const element = real.slice(a, b);
    const { cancel, primary } = slotsOf(element);
    const swapped = element.replace(`cancel={${cancel}}`, "cancel={@@}").replace(`primary={${primary}}`, `primary={${cancel}}`).replace("cancel={@@}", `cancel={${primary}}`);
    expect(swapped).not.toBe(element);
    const violations = actionRowViolations(file, real.slice(0, a) + swapped + real.slice(b)).join("\n");
    expect(violations).toMatch(/cancel slot holds a confirming action/);
    expect(violations).toMatch(/primary slot holds a dismiss action/);
  });

  it("row-reverse on .dialog-actions inside a media query", () => {
    const real = readFileSync("src/styles/app.css", "utf8");
    expect(actionRowCssViolations("src/styles/app.css", real)).toEqual([]);
    const planted = `${real}\n@media (max-width: 600px) {\n  .modal-body { padding: 0; }\n  .dialog-actions { flex-direction: row-reverse; }\n}\n`;
    expect(actionRowCssViolations("src/styles/app.css", planted)).toEqual(["src/styles/app.css: `.dialog-actions` reorders an action row"]);
    expect(actionRowCssViolations("x.css", "@media (max-width:600px){.dp-foot{order:2}}")).toHaveLength(1);
  });
});

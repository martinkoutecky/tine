// D-20261119-YL, review round 3: the native core decides once per config
// whether the configured journal title format is used and publishes the
// decision in GraphMeta (`journal_title`). The graph window takes it as
// decided: a graph load and a config change call no wasm entry point at all,
// and the wasm module has no title check to call. A valid title with 1,000
// weekday names cost 1.8 s per cold check in wasm when the window checked it.
import { readFileSync } from "node:fs";
import path from "node:path";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

const wasm = vi.hoisted(() => ({ calls: [] as string[] }));
vi.mock("./render/wasm/lsdoc_wasm.js", async (importOriginal) => {
  const real = await importOriginal<Record<string, unknown>>();
  const loading = new Set(["default", "initSync", "__tineReinstantiate"]);
  return Object.fromEntries(Object.entries(real).map(([name, value]) => [
    name,
    typeof value === "function" && !loading.has(name) && /^[a-z]/.test(name)
      ? (...args: unknown[]) => { wasm.calls.push(name); return (value as (...a: unknown[]) => unknown)(...args); }
      : value,
  ]));
});

import { applyConfigDerivedState } from "./graph";
import { setGraphMeta } from "./graphSession";
import { journalTitle, parseJournalTitle, setJournalFormats } from "./journal";
import { initParser } from "./render/parse";
import { dismissToast, toasts } from "./toasts";
import type { GraphMeta } from "./types";

beforeAll(() => initParser());
afterEach(() => {
  setGraphMeta(null);
  setJournalFormats(null, null);
  for (const toast of toasts()) dismissToast(toast.id);
});

const LONG = `${"EEEE ".repeat(1000)}EEEE MMMM do y`;
const meta = (configured: string, effective: string, root = "/graph") => ({
  root, preferred_workflow: "now", favorites: [], favorites_page: null, file_name_format: "legacy",
  journal_page_title_format: configured, journal_file_name_format: "yyyy_MM_dd", default_home: null,
  journal_title: { effective, refusal: effective === configured ? null : { missing: "year" } },
} as unknown as GraphMeta);

describe("the title decision crosses the boundary once, natively", () => {
  it("a graph load and a config change call no wasm entry point, whatever the pattern", () => {
    wasm.calls.length = 0;
    const started = performance.now();
    applyConfigDerivedState(meta(LONG, LONG), null);
    applyConfigDerivedState(meta("MM-dd", "MMM do, yyyy"), meta(LONG, LONG));
    applyConfigDerivedState(meta("dd.MM.yyyy", "dd.MM.yyyy", "/other"), null);
    const elapsed = performance.now() - started;
    expect(wasm.calls).toEqual([]);
    expect(elapsed).toBeLessThan(200);
    // What was published is what reads and renders.
    expect(journalTitle(new Date(2026, 10, 2))).toBe("02.11.2026");
    expect(parseJournalTitle("02.11.2026")?.getDate()).toBe(2);
    expect(wasm.calls.every((name) => name === "format_journal_date" || name === "journal_day_json")).toBe(true);
  });

  it("the wasm module has no title check: it builds text routes from the effective format only", () => {
    const lib = readFileSync(path.resolve(__dirname, "../crates/lsdoc-wasm/src/lib.rs"), "utf8");
    expect(lib).not.toMatch(/JournalFormat::new|title_refusal|journal_title_status/);
    const exports = readFileSync(path.resolve(__dirname, "render/wasm/lsdoc_wasm.d.ts"), "utf8");
    expect(exports).not.toMatch(/title_status|title_refusal/);
  });
});

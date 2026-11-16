import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";
import { bumpDataRev, bumpGraphEpoch, graphMeta, setGraphMeta } from "../graphSession";
import { applyChangeSummary } from "../changeSummary";
import { applyConfigDerivedState } from "../graph";
import { backend } from "../backend";
import { type GraphMeta } from "../types";
import golden from "../../tests/fixtures/og-journal-formats.json";

afterEach(() => { closeSettings(); setGraphMeta(null); vi.restoreAllMocks(); vi.useRealTimers(); document.body.innerHTML = ""; });

it("GX4: the sections-page note follows marker changes, full and gaps while unrelated saves do not locate again", async () => {
  bumpGraphEpoch(); let rev = 0;
  const publish = (props: Record<string, string[]> = {}, full = false) => {
    applyChangeSummary({ rev: String(++rev), full, pages: [], blocks: [], linksTo: [], props, inventoryChanged: false });
    bumpDataRev(true);
  };
  publish();
  const read = vi.spyOn(backend(), "queryRun").mockResolvedValue({ anchor: "page", pages: [], diagnostics: [], report: { ran: [], ignored: [], supported: true }, total: 0, exceeded: false });
  setGraphMeta({ root: "/fixture", journal_config_diagnostics: [] } as unknown as GraphMeta);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("journals");
    await vi.waitFor(() => expect(read).toHaveBeenCalled()); read.mockClear();
    publish({ unrelated: ["icon"] }); await new Promise(resolve => setTimeout(resolve, 0));
    expect(read).not.toHaveBeenCalled();
    for (const reason of ["marker", "full", "gap"]) {
      if (reason === "gap") rev += 2;
      publish(reason === "marker" ? { sections: ["tine/journal-sections"] } : {}, reason === "full");
      await vi.waitFor(() => expect(read).toHaveBeenCalled()); read.mockClear();
    }
  } finally { dispose(); }
});

it("shows backend journal pattern notes by the format picker and follows fresh config snapshots", async () => {
  const notes = [
    ":journal/page-title-format uses unsupported week numbers ww",
    ":journal/file-name-format keeps quoted text 'w'",
  ];
  setGraphMeta({ root: "/fixture", journal_page_title_format: "yyyy-ww", journal_config_diagnostics: notes } as GraphMeta);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("journals");
    await new Promise(resolve => setTimeout(resolve, 0));
    const statuses = () => [...root.querySelectorAll('[role="status"]')].map(n => n.textContent);
    for (const note of notes) expect(statuses().some(text => text?.includes(note))).toBe(true);
    const field = root.querySelector(".config-diagnostic")!.closest(".settings-field");
    expect(field?.querySelector("select")).toBeTruthy();
    applyConfigDerivedState({ ...graphMeta()!, journal_page_title_format: "yyyy-MM-dd", journal_config_diagnostics: [notes[1]] }, graphMeta());
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(statuses().some(text => text?.includes(notes[0]))).toBe(false);
    expect(statuses().some(text => text?.includes(notes[1]))).toBe(true);
    applyConfigDerivedState({ ...graphMeta()!, journal_config_diagnostics: [] }, graphMeta());
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(root.querySelector(".config-diagnostic")).toBeNull();
  } finally { dispose(); }
});

it("offers the OG built-ins with the current custom format first and writes a selected pattern", async () => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2024, 0, 5));
  const saved = vi.spyOn(backend(), "setJournalTitleFormat").mockResolvedValue([]);
  setGraphMeta({ root: "/fixture", journal_page_title_format: "EE, yyyy-MM-dd", journal_config_diagnostics: ["old pattern note"] } as GraphMeta);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  openSettings("journals");
  await new Promise(resolve => setTimeout(resolve, 0));
  const select = [...root.querySelectorAll<HTMLSelectElement>("select")].find(s => s.value === "EE, yyyy-MM-dd")!;
  expect(select).toBeDefined();
  expect([...select.options].map(o => o.value)).toEqual(["EE, yyyy-MM-dd", ...golden.formats.map(f => f.pattern)]);
  for (const { pattern, title } of golden.formats) {
    expect([...select.options].find(o => o.value === pattern)?.text).toContain(title);
  }
  select.value = "E, yyyy/MM/dd";
  select.dispatchEvent(new Event("change", { bubbles: true }));
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(saved).toHaveBeenCalledWith("E, yyyy/MM/dd", ["rename-page"]);
  expect(select.value).toBe("E, yyyy/MM/dd");
  expect(root.textContent).not.toContain("old pattern note");
  expect([...select.options].map(o => o.value)).toEqual(["E, yyyy/MM/dd", ...golden.formats.map(f => f.pattern).filter(p => p !== "E, yyyy/MM/dd")]);
  dispose();
});

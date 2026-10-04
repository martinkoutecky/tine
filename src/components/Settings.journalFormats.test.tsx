import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";
import { setGraphMeta } from "../graphSession";
import { backend } from "../backend";
import { type GraphMeta } from "../types";
import golden from "../../tests/fixtures/og-journal-formats.json";

afterEach(() => { closeSettings(); setGraphMeta(null); vi.restoreAllMocks(); vi.useRealTimers(); document.body.innerHTML = ""; });

it("offers the OG built-ins with the current custom format first and writes a selected pattern", async () => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2024, 0, 5));
  const saved = vi.spyOn(backend(), "setJournalTitleFormat").mockResolvedValue();
  setGraphMeta({ root: "/fixture", journal_page_title_format: "EE, yyyy-MM-dd" } as GraphMeta);
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
  expect([...select.options].map(o => o.value)).toEqual(["E, yyyy/MM/dd", ...golden.formats.map(f => f.pattern).filter(p => p !== "E, yyyy/MM/dd")]);
  dispose();
});

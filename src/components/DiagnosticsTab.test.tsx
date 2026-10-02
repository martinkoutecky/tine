import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import * as reload from "../reloadOnFocus";
import { DIAGNOSTIC_PREVIEW_LIMIT, DiagnosticsTab, diagnosticReportPreview } from "./DiagnosticsTab";

async function flush() {
  for (let i = 0; i < 12; i += 1) await Promise.resolve();
}

function button(host: HTMLElement, label: string): HTMLButtonElement {
  const found = [...host.querySelectorAll("button")].find((candidate) => candidate.textContent === label);
  if (!found) throw new Error(`no ${label} button`);
  return found;
}

describe("Help & diagnostics (GH #343)", () => {
  afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; });

  it("creates a reviewable report, copies the complete text, and clears recorded events", async () => {
    const text = JSON.stringify({ schemaVersion: 1, sessions: { current: [{ event: "runtime.started" }] } });
    const report = vi.spyOn(backend(), "diagnosticReport").mockResolvedValue({ text, suggestedFileName: "tine-diagnostics-1.json" });
    const writeText = vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);
    const clear = vi.spyOn(backend(), "clearDiagnostics").mockResolvedValue(undefined);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <DiagnosticsTab />, host);
    expect(host.querySelector("h2")?.textContent).toBe("Help & diagnostics");
    expect(host.textContent).toContain("Help improve Tine's parser");

    button(host, "Create diagnostic report").click();
    await flush();
    expect(report).toHaveBeenCalledOnce();
    expect(host.querySelector<HTMLTextAreaElement>(".diagnostics-preview textarea")?.value).toBe(text);
    expect(host.textContent).toContain("tine-diagnostics-1.json");

    button(host, "Copy report").click();
    await flush();
    expect(writeText).toHaveBeenCalledWith(text);

    button(host, "Clear recorded events").click();
    await flush();
    expect(clear).toHaveBeenCalledOnce();
    expect(host.querySelector(".diagnostics-preview")).toBeNull();
    dispose();
  });

  it("shortens only the on-screen preview of a large report; Copy report keeps every byte", async () => {
    const text = `${"a".repeat(DIAGNOSTIC_PREVIEW_LIMIT)}middle${"z".repeat(9 * 1024)}`;
    const preview = diagnosticReportPreview(text);
    expect(preview.length).toBeLessThan(DIAGNOSTIC_PREVIEW_LIMIT + 200);
    expect(preview).toContain(`[Preview shortened: ${text.length - DIAGNOSTIC_PREVIEW_LIMIT} characters omitted.`);
    expect(preview.endsWith("z".repeat(8 * 1024))).toBe(true);
    expect(diagnosticReportPreview("small")).toBe("small");

    vi.spyOn(backend(), "diagnosticReport").mockResolvedValue({ text, suggestedFileName: "big.json" });
    const writeText = vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <DiagnosticsTab />, host);
    button(host, "Create diagnostic report").click();
    await flush();
    expect(host.querySelector<HTMLTextAreaElement>(".diagnostics-preview textarea")?.value).toBe(preview);
    expect(host.textContent).toContain("Large report: this preview is shortened");
    button(host, "Copy report").click();
    await flush();
    expect(writeText).toHaveBeenCalledWith(text);
    dispose();
  });

  // og ADR 0058 (master 271885b2): on desktop the user may save the report to
  // a file they pick; a cancelled dialog is not an error.
  it("saves a report where the user picks on desktop, and says nothing when cancelled", async () => {
    const save = vi.spyOn(backend(), "saveDiagnosticReport").mockResolvedValueOnce(true).mockResolvedValueOnce(false);
    const toasts = await import("../toasts");
    const toast = vi.spyOn(toasts, "pushToast");
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <DiagnosticsTab />, host);
    expect(host.textContent).toContain("this run and the previous one");
    button(host, "Save report…").click();
    await flush();
    expect(save).toHaveBeenCalledOnce();
    expect(toast).toHaveBeenCalledWith("Diagnostic report saved", "success");
    toast.mockClear();
    button(host, "Save report…").click();
    await flush();
    expect(save).toHaveBeenCalledTimes(2);
    expect(toast).not.toHaveBeenCalled();
    dispose();
  });

  // GH #623: the full stat diff on demand, with the time it finished.
  it("rescans the graph on demand and shows when it finished", async () => {
    const finished = new Date(2026, 9, 2, 13, 14, 15).getTime();
    let release!: () => void;
    const rescan = vi.spyOn(reload, "rescanGraphNowFromSettings").mockImplementation(
      () => new Promise((resolve) => { release = () => resolve(finished); }),
    );
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <DiagnosticsTab />, host);
    expect(host.textContent).not.toContain("Last rescan finished");
    button(host, "Rescan graph").click();
    await flush();
    expect(rescan).toHaveBeenCalledOnce();
    expect(button(host, "Rescanning…").disabled).toBe(true);
    release();
    await flush();
    expect(button(host, "Rescan graph").disabled).toBe(false);
    expect(host.textContent).toContain(`Last rescan finished at ${new Date(finished).toLocaleTimeString()}.`);
    dispose();
  });
});

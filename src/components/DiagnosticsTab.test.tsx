import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
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
});

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { readFileSync } from "node:fs";
import { backend } from "../backend";
import { initDebug, resetDebugForTests } from "../debug";
import { ERROR_TOAST_HISTORY_LIMIT, recordErrorToastText, resetErrorToastHistoryForTests } from "../errorToastHistory";
import { pushToast, setToasts } from "../toasts";
import { DiagnosticsTab } from "./DiagnosticsTab";

async function flush() {
  for (let i = 0; i < 12; i += 1) await Promise.resolve();
}

const rows = (host: HTMLElement) => [...host.querySelectorAll(".diagnostics-error-row")];
const textOf = (row: Element) => row.querySelector(".diagnostics-error-text")?.textContent;

describe("Diagnostics: recent error messages of this session (og-D D5)", () => {
  beforeEach(() => { resetDebugForTests(); resetErrorToastHistoryForTests(); setToasts([]); });
  afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; setToasts([]); });

  it("shows the last 20 of 21 real error toasts, newest first, with a time, and writes nothing", async () => {
    await initDebug(); // wires the one error-toast recorder hook, exactly as the app does
    // Everything the backend could persist: any call carrying an error text fails the test.
    const calls: string[] = [];
    const be = backend() as unknown as Record<string, unknown>;
    for (const key of Object.keys(be)) {
      const original = be[key];
      if (typeof original !== "function") continue;
      vi.spyOn(be as Record<string, (...a: unknown[]) => unknown>, key).mockImplementation((...args: unknown[]) => {
        calls.push(`${key}:${JSON.stringify(args)}`);
        return (original as (...a: unknown[]) => unknown)(...args);
      });
    }
    const setItem = vi.spyOn(Storage.prototype, "setItem");

    for (let n = 1; n <= 21; n += 1) pushToast(`Could not save Secret Page ${n}.`, "error");
    setToasts([]); // the toasts were closed: only the session list remembers them
    await flush();

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <DiagnosticsTab />, host);
    const shown = rows(host);
    expect(shown).toHaveLength(ERROR_TOAST_HISTORY_LIMIT);
    expect(shown.map(textOf)).toEqual(Array.from({ length: 20 }, (_, i) => `Could not save Secret Page ${21 - i}.`));
    expect(shown.some((row) => textOf(row)?.includes("Page 1."))).toBe(false); // the oldest fell off
    expect(shown[0]!.querySelector("time")?.textContent).toBeTruthy();

    // Nothing persisted: no backend call and no browser storage write saw a message text.
    await flush();
    expect(calls.filter((c) => c.includes("Secret Page"))).toEqual([]);
    expect(setItem.mock.calls.filter((c) => String(c[1]).includes("Secret Page"))).toEqual([]);
    dispose();
  });

  it("counts a repeated identical error as one row with ×N and moves it to the top", async () => {
    recordErrorToastText("Disk full", 1000);
    recordErrorToastText("Other", 2000);
    recordErrorToastText("Disk full", 3000);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <DiagnosticsTab />, host);
    const shown = rows(host);
    expect(shown.map(textOf)).toEqual(["Disk full", "Other"]);
    expect(shown[0]!.querySelector(".diagnostics-error-count")?.textContent).toBe("×2");
    expect(shown[1]!.querySelector(".diagnostics-error-count")).toBeNull();
    dispose();
  });

  it("copies exactly the row's text to the clipboard", async () => {
    recordErrorToastText("Could not read Secret Page.");
    const writeText = vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <DiagnosticsTab />, host);
    const copy = rows(host)[0]!.querySelector("button")!;
    expect(copy.textContent).toBe("Copy");
    copy.click();
    await flush();
    expect(writeText).toHaveBeenCalledWith("Could not read Secret Page.");
    dispose();
  });

  it("says so when no error message was shown, and the module can never write what it holds", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <DiagnosticsTab />, host);
    expect(host.textContent).toContain("No error messages this session.");
    dispose();
    // Structural pin (the flight recorder and debug log must stay free of page names):
    // the history module imports only solid-js, and never touches storage.
    const source = readFileSync("src/errorToastHistory.ts", "utf8");
    const imports = [...source.matchAll(/^import .* from "(.*)";/gm)].map((m) => m[1]);
    expect(imports).toEqual(["solid-js"]);
    expect(source).not.toMatch(/localStorage|sessionStorage|indexedDB|backend\(/);
  });
});

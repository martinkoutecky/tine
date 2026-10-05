import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { DiagnosticsTab } from "./DiagnosticsTab";

async function flush() {
  for (let i = 0; i < 12; i += 1) await Promise.resolve();
}

function button(host: HTMLElement, label: string): HTMLButtonElement {
  const found = [...host.querySelectorAll("button")].find((candidate) => candidate.textContent === label);
  if (!found) throw new Error(`no ${label} button`);
  return found;
}

const manifest = (digest: string, complete = true) => ({
  schemaVersion: 1,
  tool: "tine-graph-bytes",
  algorithm: "sha256",
  complete,
  generatedAtUnixMs: 1,
  files: [{ path: "journals/2026_06_19.md", length: 85, digest: digest.repeat(64) }],
  aggregateDigest: complete ? "b".repeat(64) : undefined,
  errors: complete ? [] : [{ detail: "changed during verification" }],
});
const reportOf = (m: object, complete = true) => ({
  text: JSON.stringify(m),
  suggestedFileName: "tine-graph-verification.json",
  totalFiles: 1,
  totalBytes: 85,
  aggregateDigest: "b".repeat(64),
  complete,
});

describe("Verify synchronized graph (master 749bfb2b1)", () => {
  afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; });

  async function mountWithReport(local: object, complete = true) {
    vi.spyOn(backend(), "createGraphVerification").mockResolvedValue(reportOf(local, complete));
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <DiagnosticsTab />, host);
    button(host, "Create graph verification report").click();
    await flush();
    return { host, dispose };
  }
  const paste = (host: HTMLElement, text: string) => {
    const areas = host.querySelectorAll("textarea");
    const input = areas[areas.length - 1] as HTMLTextAreaElement;
    input.value = text;
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
  };

  it("I-21: cancels the running verification when the tab closes", async () => {
    let id = "";
    vi.spyOn(backend(), "createGraphVerification").mockImplementation((operationId) => { id = operationId; return new Promise(() => {}); });
    const cancel = vi.spyOn(backend(), "cancelGraphVerification").mockResolvedValue(undefined);
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <DiagnosticsTab />, host);
    button(host, "Create graph verification report").click();
    await flush();
    expect(id).not.toBe("");
    expect(cancel).not.toHaveBeenCalled();
    dispose();
    expect(cancel).toHaveBeenCalledExactlyOnceWith(id);
  });

  it("does not cancel anything when the tab closes with no verification running", async () => {
    const cancel = vi.spyOn(backend(), "cancelGraphVerification").mockResolvedValue(undefined);
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <DiagnosticsTab />, host);
    await flush();
    dispose();
    expect(cancel).not.toHaveBeenCalled();
  });

  it("compares graph bytes and names the exact differing source path", async () => {
    const { host, dispose } = await mountWithReport(manifest("a"));
    expect(host.textContent).toContain("1 files");
    paste(host, JSON.stringify(manifest("c")));
    button(host, "Compare reports").click();
    await flush();
    expect(host.textContent).toContain("Different bytes");
    expect(host.textContent).toContain("journals/2026_06_19.md");
    expect(host.textContent).toContain("file paths and page names");
    dispose();
  });

  it("confirms a match only for identical complete reports", async () => {
    const { host, dispose } = await mountWithReport(manifest("a"));
    paste(host, JSON.stringify(manifest("a")));
    button(host, "Compare reports").click();
    await flush();
    expect(host.textContent).toContain("The source file sets and bytes match.");
    paste(host, JSON.stringify(manifest("a", false)));
    button(host, "Compare reports").click();
    await flush();
    expect(host.textContent).toContain("No match can be confirmed");
    expect(host.textContent).not.toContain("bytes match.");
    dispose();
  });

  it("refuses a pasted report that is not a graph verification manifest", async () => {
    const toasts = await import("../toasts");
    const toast = vi.spyOn(toasts, "pushToast");
    const { host, dispose } = await mountWithReport(manifest("a"));
    paste(host, "{\"schemaVersion\":1,\"tool\":\"other\"}");
    button(host, "Compare reports").click();
    await flush();
    expect(toast).toHaveBeenCalledWith(expect.stringContaining("unsupported format"), "error");
    expect(host.querySelector(".diagnostics-comparison")).toBeNull();
    dispose();
  });

  it("cancels a running verification and stays silent about the cancellation", async () => {
    const toasts = await import("../toasts");
    const toast = vi.spyOn(toasts, "pushToast");
    const failure = vi.spyOn(await import("../uiFailure"), "reportUiFailure");
    let reject!: (error: unknown) => void;
    vi.spyOn(backend(), "createGraphVerification").mockReturnValue(new Promise((_, r) => { reject = r; }));
    const cancel = vi.spyOn(backend(), "cancelGraphVerification").mockImplementation(async () => reject({ kind: "cancelled" }));
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <DiagnosticsTab />, host);
    button(host, "Create graph verification report").click();
    await flush();
    button(host, "Cancel").click();
    await flush();
    expect(cancel).toHaveBeenCalledOnce();
    expect(toast).not.toHaveBeenCalled();
    expect(failure).not.toHaveBeenCalled();
    expect(button(host, "Create graph verification report").disabled).toBe(false);
    expect(host.textContent).not.toContain("Complete ·");
    dispose();
  });
  it("reports a failure whose diagnostic prose contains cancelled", async () => {
    const failures = await import("../uiFailure");
    const report = vi.spyOn(failures, "reportUiFailure");
    vi.spyOn(backend(), "createGraphVerification").mockRejectedValue({ kind: "failed", message: "disk cancelled the read" });
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <DiagnosticsTab />, host);
    button(host, "Create graph verification report").click();
    await flush();
    expect(report).toHaveBeenCalledWith("graph-verification", expect.objectContaining({ kind: "failed" }));
    dispose();
  });

});

import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { PdfExportDialog } from "./PdfExportDialog";
import { openPdfExport, closePdfExport, pdfExportPage } from "../ui";
import { exportPagePdf } from "../print";

vi.mock("../print", () => ({ exportPagePdf: vi.fn(), DEFAULT_PRINT_OPTS: { include_comments: true } }));

let teardown: (() => void) | undefined;
afterEach(() => { teardown?.(); teardown = undefined; closePdfExport(); document.body.innerHTML = ""; localStorage.clear(); vi.mocked(exportPagePdf).mockClear(); });

function mountDialog() {
  openPdfExport("P");
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <PdfExportDialog />, host);
  teardown = () => { dispose(); host.remove(); };
  return host;
}
const enter = (target: Element) => {
  const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
  target.dispatchEvent(event);
  return event;
};

it("includes comments initially, passes an explicit exclusion to print, and resets the next export", () => {
  const host = mountDialog();
  const checkbox = () => [...host.querySelectorAll("label")].find(label => label.textContent?.includes("Include comments"))!
    .querySelector<HTMLInputElement>('input[type="checkbox"]')!;
  expect(checkbox().checked).toBe(true);
  checkbox().click();
  enter(host.querySelector(".export-opts")!);
  expect(exportPagePdf).toHaveBeenCalledWith("P", expect.objectContaining({ include_comments: false }));
  openPdfExport("P");
  expect(checkbox().checked).toBe(true);
});

it("Enter on the focused Cancel button is left to activate Cancel instead of exporting", () => {
  const host = mountDialog();
  const cancel = [...host.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Cancel")!;
  cancel.focus();
  const event = enter(cancel);
  expect(event.defaultPrevented, "the browser's own button activation must run").toBe(false);
  expect(exportPagePdf).not.toHaveBeenCalled();
  cancel.click(); // what the browser does next
  expect(exportPagePdf).not.toHaveBeenCalled();
  expect(pdfExportPage()).toBeFalsy();
});

it("Enter elsewhere in the dialog still exports (the dialog's shortcut)", () => {
  const host = mountDialog();
  const event = enter(host.querySelector(".export-opts")!);
  expect(event.defaultPrevented).toBe(true);
  expect(exportPagePdf).toHaveBeenCalledOnce();
  expect(pdfExportPage()).toBeFalsy();
});

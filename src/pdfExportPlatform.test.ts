import { afterEach, expect, it, vi } from "vitest";
import { openPdfExport, pdfExportPage, setPdfExportPage } from "./ui";
import { setToasts, toasts } from "./toasts";

vi.mock("./nativeChrome", async (original) => ({
  ...(await original<typeof import("./nativeChrome")>()),
  isMobilePlatform: true,
}));

afterEach(() => { setPdfExportPage(null); setToasts([]); });

it("explains that PDF export needs desktop instead of opening a dead mobile dialog", () => {
  openPdfExport("Alpha");
  expect(pdfExportPage()).toBeNull();
  expect(toasts().map((toast) => toast.message)).toEqual([
    expect.stringContaining("desktop app"),
  ]);
});

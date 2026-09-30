import { afterEach, expect, it, vi } from "vitest";
import { exportPagePdf, preparePrintHtml } from "./print";
import { backend } from "./backend";
import * as documentStore from "./document";
import { setToasts, toasts } from "./toasts";

afterEach(() => { vi.restoreAllMocks(); setToasts([]); document.querySelectorAll("iframe").forEach((frame) => frame.remove()); });
it("aborts the PDF action at the renderer's Print query limit without attaching a partial print frame", async () => {
  setToasts([]);
  vi.spyOn(documentStore, "flushAll").mockResolvedValue(true);
  vi.spyOn(backend(), "pagePrintHtml").mockResolvedValue('<html><body><div class="query query-too-large">Query has 20001 matches; narrow it before publishing.</div></body></html>');
  await exportPagePdf("Limited query");
  expect(document.querySelector("iframe")).toBeNull();
  expect(toasts().at(-1)?.message).toContain("Print query limit");
  expect(toasts().at(-1)?.message).toContain("20001 matches");
});
it.each([
  "Query source exceeds the 64 KiB publication limit.",
  "Query nesting is too deep to publish safely.",
  "Query has 20001 matches; narrow it before publishing.",
])("refuses a renderer-declared limit before optional print rendering: %s", async (detail) => {
  await expect(preparePrintHtml(`<div class="query query-too-large">${detail}</div>`)).rejects.toThrow(`Print query limit. ${detail}`);
});

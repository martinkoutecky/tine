import { afterEach, describe, expect, it, vi } from "vitest";
import { exportPagePdf, preparePrintHtml, PRINT_IFRAME_SANDBOX } from "./print";
import { backend } from "./backend";
import * as documentStore from "./document";
import { toasts, setToasts } from "./toasts";

describe("print document privilege boundary", () => {
  afterEach(() => {
    document.head.querySelectorAll("[data-print-test]").forEach((element) => element.remove());
  });

  it("refuses PDF export when pending page edits could not be saved", async () => {
    setToasts([]);
    const flush = vi.spyOn(documentStore, "flushAll").mockResolvedValue(false);
    const renderPage = vi.spyOn(backend(), "pagePrintHtml").mockResolvedValue("<html></html>");
    try {
      await exportPagePdf("Draft");
      expect(flush).toHaveBeenCalledOnce();
      expect(renderPage).not.toHaveBeenCalled();
      expect(toasts().some((toast) => toast.message.includes("could not be saved"))).toBe(true);
    } finally { vi.restoreAllMocks(); setToasts([]); }
  });

  it("coalesces concurrent export requests into one print frame", async () => {
    const flush = vi.spyOn(documentStore, "flushAll").mockResolvedValue(true);
    const render = vi.spyOn(backend(), "pagePrintHtml").mockResolvedValue("<html><body>Draft</body></html>");
    try {
      await Promise.all([exportPagePdf("Draft"), exportPagePdf("Draft")]);
      expect(flush).toHaveBeenCalledOnce();
      expect(render).toHaveBeenCalledOnce();
      expect(document.querySelectorAll('iframe[aria-hidden="true"]')).toHaveLength(1);
    } finally { document.querySelectorAll('iframe[aria-hidden="true"]').forEach((frame) => frame.remove()); vi.restoreAllMocks(); }
  });

  it("renders math and code locally while removing every executable or remote resource", async () => {
    expect(PRINT_IFRAME_SANDBOX.split(/\s+/)).not.toContain("allow-scripts");
    const local = document.createElement("link");
    local.rel = "stylesheet";
    local.href = "/assets/main-test.css";
    local.dataset.printTest = "local";
    document.head.appendChild(local);

    const remote = document.createElement("link");
    remote.rel = "stylesheet";
    remote.href = "https://example.invalid/graph-leak.css";
    remote.dataset.printTest = "remote";
    document.head.appendChild(remote);

    const result = await preparePrintHtml(`<!doctype html><html><head>
      <meta http-equiv="Content-Security-Policy" content="script-src 'none'">
      <link rel="stylesheet" href="https://cdn.example.invalid/print.css">
      <script src="https://cdn.example.invalid/print.js"></script>
    </head><body>
      <span class="math">\\(x^2\\)</span>
      <pre class="code-block"><code class="hljs language-rust">fn main() {}</code></pre>
    </body></html>`);

    const parsed = new DOMParser().parseFromString(result, "text/html");
    expect(parsed.querySelectorAll("script")).toHaveLength(0);
    expect(result).not.toContain("cdn.example.invalid");
    expect(result).not.toContain("example.invalid/graph-leak.css");
    expect(parsed.querySelector("meta[http-equiv='Content-Security-Policy']")?.getAttribute("content"))
      .toContain("script-src 'none'");
    expect(parsed.querySelector("span.math .katex")).not.toBeNull();
    expect(parsed.querySelector("code .hljs-keyword")?.textContent).toBe("fn");
    const styles = [...parsed.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"]')];
    expect(styles).toHaveLength(1);
    expect(new URL(styles[0].href).pathname).toBe("/assets/main-test.css");
  });
});

import { invalidateBinding } from "./binding";
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

  it("a newer PDF export supersedes pending preparation and ignores late HTML", async () => {
    setToasts([]);
    vi.spyOn(documentStore, "flushAll").mockResolvedValue(true);
    let finish!: (html: string) => void;
    const renderPage = vi.spyOn(backend(), "pagePrintHtml")
      .mockImplementationOnce(() => new Promise<string>((resolve) => { finish = resolve; }))
      .mockResolvedValueOnce("<html><body>Second</body></html>");
    try {
      const first = exportPagePdf("First");
      await vi.waitFor(() => expect(renderPage).toHaveBeenCalledOnce());
      await exportPagePdf("Second");
      await first;
      finish("<html><body>First late output</body></html>");
      await Promise.resolve();
      const frames = document.querySelectorAll<HTMLIFrameElement>('iframe[aria-hidden="true"]');
      expect(renderPage).toHaveBeenCalledTimes(2);
      expect(frames).toHaveLength(1);
      expect(frames[0].srcdoc).toContain("Second");
      expect(frames[0].srcdoc).not.toContain("First late output");
      expect(toasts()).toHaveLength(0);
    } finally { window.dispatchEvent(new Event("pagehide")); vi.restoreAllMocks(); }
  });

  it("page teardown cancels pending PDF preparation without a stale error", async () => {
    setToasts([]);
    vi.spyOn(documentStore, "flushAll").mockResolvedValue(true);
    const renderPage = vi.spyOn(backend(), "pagePrintHtml").mockImplementation(() => new Promise<string>(() => {}));
    try {
      const pending = exportPagePdf("Pending");
      await vi.waitFor(() => expect(renderPage).toHaveBeenCalledOnce());
      window.dispatchEvent(new Event("pagehide"));
      await pending;
      expect(document.querySelector("iframe")).toBeNull();
      expect(toasts()).toHaveLength(0);
    } finally { vi.restoreAllMocks(); }
  });

  it("graph retirement releases pending preparation and hides a late backend rejection", async () => {
    setToasts([]);
    vi.spyOn(documentStore, "flushAll").mockResolvedValue(true);
    let reject!: (error: Error) => void;
    const renderPage = vi.spyOn(backend(), "pagePrintHtml").mockImplementation(() => new Promise<string>((_resolve, fail) => { reject = fail; }));
    try {
      const pending = exportPagePdf("Old graph");
      await vi.waitFor(() => expect(renderPage).toHaveBeenCalledOnce());
      invalidateBinding();
      await pending;
      reject(new Error("late"));
      await Promise.resolve();
      expect(document.querySelector("iframe")).toBeNull();
      expect(toasts()).toHaveLength(0);
    } finally { vi.restoreAllMocks(); }
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

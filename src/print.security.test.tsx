import { afterEach, describe, expect, it } from "vitest";
import { preparePrintHtml, PRINT_IFRAME_SANDBOX } from "./print";

describe("print document privilege boundary", () => {
  afterEach(() => {
    document.head.querySelectorAll("[data-print-test]").forEach((element) => element.remove());
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

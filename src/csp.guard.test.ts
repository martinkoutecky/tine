import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

// I-22: outside content (a synced graph, pasted HTML, a macro) must never run script in the app
// origin. The CSP's job is script-src; other directives stay open where Logseq behaviour needs them
// (remote images, https iframes, custom.css themes/fonts, update check and plugin registry fetches).
// Exemplar: src-tauri/tauri.conf.json.
it("I-22 app CSP restricts scripts to the app, WASM and the YouTube iframe API", () => {
  const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
  const csp = config.app.security.csp as Record<string, string> | null;
  expect(csp).not.toBeNull();
  const scripts = csp!["script-src"].split(/\s+/);
  expect(scripts.sort()).toEqual(["'self'", "'wasm-unsafe-eval'", "https://www.youtube.com"].sort());
  expect(csp!["object-src"]).toBe("'none'");
  expect(csp!["connect-src"]).toContain("ipc:");
  expect(csp!["connect-src"]).toContain("https:"); // update check + plugin registry
  expect(csp!["img-src"]).toContain("tine-media:");
  expect(csp!["frame-src"]).toContain("https:"); // sandboxed https iframes, as Logseq
  expect(csp!["style-src"]).toContain("https:"); // custom.css @import themes, as Logseq
});

// Tauri adds a style nonce to style-src when index.html carries an inline <style>, and a nonce
// disables 'unsafe-inline' — every KaTeX/sanitized-HTML style attribute would then be blocked.
it("I-22 index.html has no inline <style>, so style-src 'unsafe-inline' stays effective", () => {
  expect(readFileSync("index.html", "utf8")).not.toMatch(/<style\b/i);
});


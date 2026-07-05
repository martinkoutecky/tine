import { describe, it, expect } from "vitest";
import { sanitizeRawHtml } from "./htmlSanitize";
import fixtures from "../../fixtures/html-sanitize-cases.json";

// The SAME fixtures the Rust/ammonia export runs (crates/tine-core/src/html_sanitize.rs).
// This asserts the app (DOMPurify) and the static-HTML export enforce the same
// allowlist — the "two renderers silently diverge" trap, closed by a shared contract.
// (This is a .tsx so it runs under the jsdom render config, where DOMPurify has a DOM.)
describe("sanitizeRawHtml — shared allowlist contract", () => {
  for (const c of fixtures.cases) {
    it(c.name, () => {
      const out = sanitizeRawHtml(c.input);
      for (const needle of c.mustContain) {
        expect(out, `expected ${JSON.stringify(out)} to contain ${JSON.stringify(needle)}`).toContain(needle);
      }
      for (const needle of c.mustNotContain) {
        expect(out, `expected ${JSON.stringify(out)} NOT to contain ${JSON.stringify(needle)}`).not.toContain(needle);
      }
    });
  }
});

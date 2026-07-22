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

  it("preserves bounded native media tags and playback attributes", () => {
    const root = document.createElement("div");
    root.innerHTML = sanitizeRawHtml(`
      <audio controls loop muted preload="metadata" src="https://media.example/audio.ogg">
        <source src="https://media.example/audio.opus" type="audio/ogg">
      </audio>
      <video controls loop muted preload="none" poster="https://media.example/poster.jpg"
             width="640" height="360" src="https://media.example/video.mp4">
        <source src="https://media.example/video.webm" type="video/webm">
      </video>
    `);

    const audio = root.querySelector("audio");
    const video = root.querySelector("video");
    const sources = root.querySelectorAll("source");
    expect(audio).not.toBeNull();
    expect(audio?.hasAttribute("controls")).toBe(true);
    expect(audio?.hasAttribute("loop")).toBe(true);
    expect(audio?.hasAttribute("muted")).toBe(true);
    expect(audio?.getAttribute("preload")).toBe("metadata");
    expect(audio?.getAttribute("src")).toBe("https://media.example/audio.ogg");
    expect(video).not.toBeNull();
    expect(video?.hasAttribute("controls")).toBe(true);
    expect(video?.hasAttribute("loop")).toBe(true);
    expect(video?.hasAttribute("muted")).toBe(true);
    expect(video?.getAttribute("preload")).toBe("none");
    expect(video?.getAttribute("poster")).toBe("https://media.example/poster.jpg");
    expect(video?.getAttribute("width")).toBe("640");
    expect(video?.getAttribute("height")).toBe("360");
    expect(video?.getAttribute("src")).toBe("https://media.example/video.mp4");
    expect(sources).toHaveLength(2);
    expect(sources[0]?.getAttribute("type")).toBe("audio/ogg");
    expect(sources[1]?.getAttribute("type")).toBe("video/webm");
  });

  it("keeps executable media and external-request primitives dead", () => {
    const clean = sanitizeRawHtml(`
      <script>steal()</script>
      <img src="https://media.example/x.png" onerror="steal()">
      <audio autoplay src="javascript:steal()"></audio>
      <video autoplay poster="data:image/png;base64,AAAA" src="data:video/mp4;base64,AAAA"></video>
      <source src="javascript:steal()" type="video/mp4">
      <iframe src="https://evil.example"></iframe>
      <object data="https://evil.example"></object>
      <embed src="https://evil.example">
    `);

    expect(clean).not.toContain("<script");
    expect(clean).not.toContain("onerror");
    expect(clean).not.toContain("autoplay");
    expect(clean).not.toContain("javascript:");
    // data: in media `src` survives — DOMPurify/OG allow it (DATA_URI_TAGS);
    // DOMPurify itself strips data: from `poster` (not a data-URI attribute),
    // which is also OG's behavior. Only executable primitives are stripped.
    expect(clean).not.toContain("data:image");
    expect(clean).toContain('src="data:video/mp4;base64,AAAA"');
    expect(clean).not.toContain("<iframe");
    expect(clean).not.toContain("<object");
    expect(clean).not.toContain("<embed");
  });
});

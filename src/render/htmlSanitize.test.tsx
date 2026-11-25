import { describe, it, expect, afterEach } from "vitest";
import { render } from "solid-js/web";
import { rawHtmlPresentation } from "./htmlSanitize";
import { InlineText } from "./inline";
import { color_only_style } from "./wasm/lsdoc_wasm.js";
import policy from "../../fixtures/html-sanitize-policy.json";

/** Safe HTML insertion string, without iframe dispatch. */
const sanitizeRawHtml = (html: string): string => rawHtmlPresentation(html).html;
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

  // The same cases' Hiccup, through the app's real render path (parse, transcribe, sanitize).
  afterEach(() => { document.body.innerHTML = ""; });
  for (const c of fixtures.cases) {
    const hiccup = (c as { hiccup?: string }).hiccup;
    if (hiccup === undefined) continue;
    it(`${c.name} (Hiccup)`, () => {
      const host = document.createElement("div");
      document.body.append(host);
      render(() => <InlineText text={hiccup} />, host);
      const shown = host.querySelector(".raw-html");
      expect(shown, host.innerHTML).not.toBeNull();
      const out = shown!.innerHTML;
      for (const needle of c.mustContain) expect(out, JSON.stringify(out)).toContain(needle);
      for (const needle of c.mustNotContain) expect(out, JSON.stringify(out)).not.toContain(needle);
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

  it("uses the preview scheme policy for media sources and posters", () => {
    const root = document.createElement("div");
    root.innerHTML = sanitizeRawHtml(`
      <img src="tel:+12025550120">
      <video poster="mailto:preview@example.test">
        <source src="https://example.test/preview.mp4" type="video/mp4">
      </video>
    `);
    expect(root.querySelector("img")?.getAttribute("src")).toBeNull();
    expect(root.querySelector("video")?.getAttribute("poster")).toBeNull();
    expect(root.querySelector("source")?.getAttribute("src")).toBe("https://example.test/preview.mp4");
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

// ORACLE, test-only, not exported: the app's colour rule as it was before the Rust
// rule replaced it (`colorOnlyStyle`, src/render/htmlSanitize.ts:64 at b184ca9a4),
// copied verbatim. The wasm `color_only_style` must give the same answer for every
// declaration, so a note renders the colours it rendered before.
const ORACLE_STYLE_PROPERTIES: readonly string[] = policy.browser.styleProperties;
const ORACLE_COLOR_VALUE = /^(?:#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})|[a-z]{3,30}|(?:rgb|rgba|hsl|hsla|hwb|lab|lch|oklab|oklch)\(\s*[-+0-9a-z.%/,\s]{1,80}\))$/i;
const ORACLE_COLOR_FUNCTION_BANNED = /(?:var|calc|url|env|attr|image|min|max|clamp)/i;
function oracleColorOnlyStyle(value: string): string {
  const kept: string[] = [];
  for (const declaration of value.split(";")) {
    const colon = declaration.indexOf(":");
    if (colon < 0) continue;
    const property = declaration.slice(0, colon).trim().toLowerCase();
    const v = declaration.slice(colon + 1).trim();
    if (!ORACLE_STYLE_PROPERTIES.includes(property) || !ORACLE_COLOR_VALUE.test(v)) continue;
    // A function body may contain letters (`from`, `none`, channel names) but never a nested call.
    if (v.includes("(") && ORACLE_COLOR_FUNCTION_BANNED.test(v.slice(v.indexOf("(")))) continue;
    kept.push(`${property}: ${v}`);
  }
  return kept.join("; ");
}

describe("color_only_style gives the app's former answers", () => {
  // Every Unicode White_Space code point, every ECMAScript WhiteSpace/LineTerminator,
  // and look-alikes that are neither (ZWSP, MONGOLIAN VOWEL SEPARATOR, WORD JOINER,
  // the C0 separators U+001C-U+001F, NUL).
  const spaces = [
    "\t", "\n", "\v", "\f", "\r", " ", "\u0085", "\u00a0", "\u1680",
    ..."\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a",
    "\u2028", "\u2029", "\u202f", "\u205f", "\u3000", "\ufeff",
    "\u200b", "\u180e", "\u2060", "\u001c", "\u001d", "\u001e", "\u001f", "\u0000",
  ];
  const properties = [
    "color", "background-color", "text-decoration-color", "COLOR", "Background-Color",
    "bac\u212aground-color", "\u017fcolor", "font-size", "position", "", "colour",
  ];
  const values = [
    "red", "RED", "#c00", "#C0C0C0", "#c00c", "#cc00cc00", "#cc", "rebeccapurple",
    "rgb(220, 38, 38)", "RGB(1 2 3 / 50%)", "oklch(70% 0.1 200)", "hsl(from red h s l)",
    "rgb(var(--x))", "rgb(calc(1) 2 3)", "url(https://t.example/x)", "red !important",
    "red!important", "blac\u212a", "\u017filver", "re", "", "rgb()", "rgb(1,\u00a02,\ufeff3)",
    "rgb(1,\u00852,3)", "rgb(\u20281 2 3)",
  ];
  const inputs: string[] = [
    // The review's inputs.
    "color:\ufeffred", "\ufeffcolor:red", "color\ufeff:red", "color:red\ufeff",
    "color:\u0085red", "\u0085color:red", "color\u0085:red", "color:red\u0085",
    "color:\ufeffred;background-color:#0c0",
    // Empty parts and odd splits.
    "", ";", ";;", ":", "color:", ":red", "color", "color:red;", ";color:red", "color:red;;background-color:#0c0",
    "color:red:blue", "color : red ; background-color : #0c0", "position:fixed;color:#c00",
  ];
  for (const p of properties) for (const v of values) inputs.push(`${p}:${v}`, `${p}: ${v}; color: red`);
  for (const w of spaces) {
    for (const [p, v] of [["color", "red"], ["Background-Color", "#0c0"], ["color", "rgb(1, 2, 3)"]]) {
      inputs.push(`${w}${p}:${v}`, `${p}${w}:${v}`, `${p}:${w}${v}`, `${p}:${v}${w}`, `${w}${w}${p}${w}:${w}${v}${w}${w}`);
    }
  }

  it(`agrees on ${inputs.length} generated declarations`, () => {
    const differ = inputs
      .map((input) => ({ input, oracle: oracleColorOnlyStyle(input), wasm: color_only_style(input) }))
      .filter((r) => r.oracle !== r.wasm);
    expect(differ).toEqual([]);
  });
});

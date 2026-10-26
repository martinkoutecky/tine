import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "./parse";
import { InlineText } from "./inline";
import { sanitizeRawHtml } from "./htmlSanitize";

// GH #243: Logseq-style colored spans. OG (security.cljs DOMPurify, default
// config) keeps `style` on hiccup/raw HTML, so `[:span {:style "color: red"} "x"]`
// shows red text. Tine keeps ONLY colour declarations (color, background-color,
// text-decoration-color) with a closed value grammar; layout, positioning and
// anything that can fetch (url(), var(), calc(), image) stays stripped.
beforeAll(async () => { await initParser(); });
afterEach(() => { document.body.innerHTML = ""; });

function mount(text: string): HTMLElement {
  const host = document.createElement("div");
  document.body.append(host);
  render(() => <InlineText text={text} />, host);
  return host;
}

describe("colored hiccup spans (GH #243)", () => {
  it("renders the text of a span with a color style in that color", () => {
    const span = mount('[:span {:style "color: red"} "urgent"]').querySelector("span.raw-html span") as HTMLElement;
    expect(span.textContent).toBe("urgent");
    expect(span.style.color).toBe("red");
  });

  it("keeps a background color and an underline color, hex and functional notations", () => {
    const u = mount('[:u {:style "background-color: #fde68a; text-decoration-color: rgb(220, 38, 38)"} "x"]').querySelector("u") as HTMLElement;
    expect(u.style.backgroundColor).not.toBe("");
    expect(u.style.textDecorationColor).not.toBe("");
  });

  it("renders ==x== and ^^x^^ as highlights, as OG does (mldoc Highlight)", () => {
    const host = mount("a ==one== b ^^two^^ c");
    expect([...host.querySelectorAll("mark")].map((m) => m.textContent)).toEqual(["one", "two"]);
  });

  it("still renders class-only spans (a custom.css `.red` rule can color them)", () => {
    const span = mount('[:span.red "x"]').querySelector("span.red");
    expect(span).not.toBeNull();
  });
});

describe("only colour declarations survive the sanitizer", () => {
  const style = (html: string) => {
    const host = document.createElement("div");
    host.innerHTML = sanitizeRawHtml(html);
    return (host.firstElementChild as HTMLElement).getAttribute("style") ?? "";
  };

  it("drops layout and positioning, keeps color", () => {
    const s = style('<span style="position: fixed; top: 0; color: red; font-size: 400px">t</span>');
    expect(s).toContain("color: red");
    for (const banned of ["position", "top", "font-size"]) expect(s).not.toContain(banned);
  });

  it.each([
    "color: url(https://t.example/x)",
    "background-color: url(https://t.example/x)",
    "color: var(--secret)",
    "color: calc(1px + 2px)",
    "color: red !important",
    "background: url(https://t.example/x) red",
    "background-image: url(https://t.example/x)",
    "color: expression(alert(1))",
    "color: red; background-color: image-set(url(x) 1x)",
  ])("never keeps %s", (decl) => {
    const s = style(`<span style="${decl}">t</span>`);
    expect(s).not.toMatch(/url|var\(|calc|expression|image|important|background:/);
  });

  it("removes the attribute entirely when nothing survives", () => {
    const host = document.createElement("div");
    host.innerHTML = sanitizeRawHtml('<span style="position:fixed">t</span>');
    expect(host.firstElementChild?.hasAttribute("style")).toBe(false);
  });
});

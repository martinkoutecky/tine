// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { forbidsEditEntry } from "./editTargets";

// jsdom does no layout, so the geometry of a code block with a classic
// horizontal scrollbar is stated directly: 200x60 box, 44 px of client height,
// 16 px scrollbar gutter beneath it, content 400 px wide.
function codeHost(zoom = 1) {
  const host = document.createElement("div");
  const code = document.createElement("div");
  code.className = "code-block";
  host.appendChild(code);
  document.body.appendChild(host);
  const def = (o: object, k: string, v: unknown) => Object.defineProperty(o, k, { value: v, configurable: true });
  def(code, "scrollWidth", 400);
  def(code, "clientWidth", 200);
  def(code, "clientHeight", 44);
  def(code, "clientTop", 0);
  def(code, "clientLeft", 0);
  def(code, "offsetWidth", 200);
  def(code, "offsetHeight", 60);
  code.getBoundingClientRect = () => ({ left: 10, top: 10, width: 200 * zoom, height: 60 * zoom, right: 0, bottom: 0, x: 10, y: 10, toJSON() {} }) as DOMRect;
  return { host, code };
}

function press(host: Element, target: Element, x: number, y: number): MouseEvent {
  const e = new MouseEvent("mousedown", { clientX: x, clientY: y, bubbles: true });
  Object.defineProperty(e, "target", { value: target });
  Object.defineProperty(e, "currentTarget", { value: host });
  return e;
}

describe("forbidsEditEntry: native scrollbar drag (master 8d404e9c1)", () => {
  it("does not arm edit entry on a code block's horizontal scrollbar", () => {
    const { host, code } = codeHost();
    // y = 10 + 50: inside the 16 px gutter below the 44 px client area.
    expect(forbidsEditEntry(press(host, code, 50, 60))).toBe(true);
  });

  it("still enters edit from the code text above the scrollbar", () => {
    const { host, code } = codeHost();
    expect(forbidsEditEntry(press(host, code, 50, 30))).toBe(false);
  });

  it("finds the gutter under a zoomed layout", () => {
    const { host, code } = codeHost(1.25);
    // rendered box is 250x75 at (10,10): the gutter is y in [10+55, 10+75).
    expect(forbidsEditEntry(press(host, code, 60, 10 + 70))).toBe(true);
    expect(forbidsEditEntry(press(host, code, 60, 10 + 25))).toBe(false);
  });

  it("a code block that does not overflow has no scrollbar to protect", () => {
    const { host, code } = codeHost();
    Object.defineProperty(code, "scrollWidth", { value: 200, configurable: true });
    expect(forbidsEditEntry(press(host, code, 50, 60))).toBe(false);
  });
});

describe("forbidsEditEntry: portalled surfaces (GH #619, master 2617ff194)", () => {
  it("never arms edit entry for a press that originates outside the block that owns the handler", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const sheet = document.createElement("button"); // portalled to <body>, NOT inside host
    document.body.appendChild(sheet);
    expect(forbidsEditEntry(press(host, sheet, 5, 5))).toBe(true);
  });

  it("still allows a press on the block's own text", () => {
    const host = document.createElement("div");
    const text = document.createElement("span");
    host.appendChild(text);
    document.body.appendChild(host);
    expect(forbidsEditEntry(press(host, text, 5, 5))).toBe(false);
  });
});

describe("forbidsEditEntry: mounted CodeMirror editors", () => {
  it.each(["cm-content", "cm-line", "cm-scroller"])("leaves a press on %s with its live editor", className => {
    const host = document.createElement("div");
    host.innerHTML = '<div data-block-editor="cm6"><div class="cm-scroller"><div class="cm-content" contenteditable="true"><div class="cm-line">task</div></div></div></div>';
    document.body.append(host);
    expect(forbidsEditEntry(press(host, host.querySelector(`.${className}`)!, 5, 5))).toBe(true);
  });
});

describe("forbidsEditEntry: a query's results surface (GH #1000)", () => {
  // host (query) > results region > result row > its own content host
  function queryHost() {
    const host = document.createElement("div");
    host.innerHTML = '<div class="query-header">Query</div><div class="query-results-region" data-block-surface="query">'
      + '<div class="ls-block"><div class="block-main"><div class="block-content-wrapper"><div class="block-content">result</div></div></div></div></div>';
    document.body.append(host);
    return { host, row: host.querySelector(".block-main")!, content: host.querySelector(".block-content")!, header: host.querySelector(".query-header")! };
  }

  it("a press on blank space in the results never edits the query host", () => {
    const { host, row } = queryHost();
    expect(forbidsEditEntry(press(host, row, 5, 5))).toBe(true);
    expect(forbidsEditEntry(press(host, host.querySelector(".query-results-region")!, 5, 5))).toBe(true);
  });

  it("the result row's own handler still takes a press on its text", () => {
    const { content } = queryHost();
    expect(forbidsEditEntry(press(content, content, 5, 5))).toBe(false);
  });

  it("the query's header still edits the query", () => {
    const { host, header } = queryHost();
    expect(forbidsEditEntry(press(host, header, 5, 5))).toBe(false);
  });
});

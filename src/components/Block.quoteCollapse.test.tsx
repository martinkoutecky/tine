import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore, toggleCollapse } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { rawWithCollapsed } from "../document/edits/properties";
import { joinProps, splitProps, BUILTIN_HIDE } from "../editor/properties";
import { Block } from "./Block";

beforeAll(() => initParser());
afterEach(() => { resetStore(); document.body.innerHTML = ""; });

it.each(["> quote", "> first\n> second", "> [!TIP]\n> helpful"])("collapse of %j keeps metadata outside the quote and preserves content (GH #684)", raw => {
  loadSingle({ name: "Quote", title: "Quote", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "quote", raw, collapsed: false, children: [{ id: "child", raw: "child", collapsed: false, children: [] }] }] });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <Block id="quote" />, host);
  try {
    toggleCollapse("quote");
    expect(doc.byId.quote.raw).toBe(`collapsed:: true\n${raw}`);
    expect(host.querySelector(".md-quote, .callout")?.textContent).not.toContain("collapsed::");
    const split = splitProps(doc.byId.quote.raw, BUILTIN_HIDE);
    expect(split.visible).toBe(raw);
    expect(joinProps(split.visible, split.hidden)).toBe(doc.byId.quote.raw);
    toggleCollapse("quote");
    expect(doc.byId.quote.raw).toBe(raw);
    expect(doc.byId.child.raw).toBe("child");
  } finally { dispose(); }
});

it("Org admonition collapse retains drawer placement", () => {
  const raw = "#+BEGIN_IMPORTANT\nbody\n#+END_IMPORTANT";
  const collapsed = rawWithCollapsed(raw, true, "org");
  expect(collapsed).toBe(`${raw}\n:PROPERTIES:\n:collapsed: true\n:END:`);
  expect(rawWithCollapsed(collapsed, false, "org")).toBe(raw);
});

it("a title above a quote keeps its property before the quoted body", () => {
  const raw = "Title\n> quoted body";
  const collapsed = rawWithCollapsed(raw, true, "md");
  expect(collapsed).toBe("Title\ncollapsed:: true\n> quoted body");
  expect(rawWithCollapsed(collapsed, false, "md")).toBe(raw);
});

it("quote collapse preserves CRLF content bytes", () => {
  const raw = "> first\r\n> second";
  const collapsed = rawWithCollapsed(raw, true, "md");
  expect(collapsed).toBe(`collapsed:: true\n${raw}`);
  expect(rawWithCollapsed(collapsed, false, "md")).toBe(raw);
});

it("collapse leaves an existing unrelated metadata line in its original slot", () => {
  const raw = "> quote\n\nid:: 11111111-1111-4111-8111-111111111111";
  const collapsed = rawWithCollapsed(raw, true, "md");
  // The new row joins the block's existing group (one group, as OG's insert-property
  // writes it); above the quote it started a second group (REG-HIDDEN-PROPS-STAY-IN-THE-GROUP).
  expect(collapsed).toBe(`${raw}\ncollapsed:: true`);
  expect(rawWithCollapsed(collapsed, true, "md")).toBe(collapsed);
  expect(rawWithCollapsed(collapsed, false, "md")).toBe(raw);
});

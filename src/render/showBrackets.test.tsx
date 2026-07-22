import { describe, it, expect, beforeAll, afterEach } from "vitest";
import { render } from "solid-js/web";
import { renderInlines } from "./inline";
import { initParser } from "./parse";
import { setGraphMeta } from "../ui";
import type { JSX } from "solid-js";
import type { Inline } from "./ast";

// OG parity: `settings.show-brackets` (`:ui/show-brackets?`, default true).
// When the setting is OFF, the `[[ ]]` brackets around a page reference are
// hidden (OG `frontend/components/block.cljs` page-reference gates the
// `.bracket` spans on `state/show-brackets?`). Tags and aliased refs never
// carry brackets in either app. Default is ON, so omitting the field renders
// brackets exactly as before.
beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  setGraphMeta(null);
});

function html(node: () => JSX.Element): string {
  const div = document.createElement("div");
  const dispose = render(() => node(), div);
  const out = div.innerHTML;
  dispose();
  return out;
}
const inl = (xs: Inline[]) => html(() => renderInlines(xs));
const pageRef = (name: string): Inline[] => [
  { k: "link", url: { type: "page_ref", v: name }, full: `[[${name}]]` } as unknown as Inline,
];

describe("show-brackets setting gates page-reference brackets", () => {
  it("default (unset) keeps brackets — no regression of current behavior", () => {
    const h = inl(pageRef("My Page"));
    expect(h).toContain('class="bracket"');
    expect(h).toContain("My Page");
  });

  it("show_brackets: true renders brackets", () => {
    setGraphMeta({ show_brackets: true } as never);
    const h = inl(pageRef("My Page"));
    expect(h).toContain('class="bracket"');
    expect(h).toContain("My Page");
  });

  it("show_brackets: false hides brackets but keeps the link + name", () => {
    setGraphMeta({ show_brackets: false } as never);
    const h = inl(pageRef("My Page"));
    expect(h).not.toContain('class="bracket"');
    expect(h).toContain('class="page-ref"');
    expect(h).toContain("My Page");
  });

  it("tags never carry brackets, regardless of the setting", () => {
    setGraphMeta({ show_brackets: true } as never);
    const h = inl([{ k: "tag", children: [{ k: "plain", text: "project" }] } as unknown as Inline]);
    expect(h).not.toContain('class="bracket"');
    expect(h).toContain("#");
    expect(h).toContain("project");
  });
});

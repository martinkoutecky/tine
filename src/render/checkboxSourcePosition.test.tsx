// F5 (C3 L15): clicking a rendered in-block checkbox must flip THAT item's own
// source line — never a lookalike inside literal content (Markdown `#+BEGIN_SRC`,
// `#+BEGIN_EXAMPLE`, `$$` math, a `:PROPERTIES:` drawer) and never a sibling item
// (checkboxes inside a `>` quote). The click is mapped by the item's lsdoc source
// span, the same answer the renderer drew, not by a second regex walk over raw.
import { describe, it, expect, beforeAll, beforeEach } from "vitest";
import { render } from "solid-js/web";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { setGraphMeta } from "../graphSession";

beforeAll(async () => {
  await initParser();
});
beforeEach(() => {
  resetStore();
  setGraphMeta(null);
});

async function click(raw: string, index: number, format: "md" | "org" = "md", bodyRaw = raw): Promise<string> {
  resetStore();
  loadSingle({ name: "Test", kind: "page", title: "Test", pre_block: null, blocks: [{ id: "body", raw, collapsed: false, children: [] }], format });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => AstBody({ raw: bodyRaw, blockId: "body", format, macroExpansion: bodyRaw !== raw }), host);
  await Promise.resolve();
  const boxes = host.querySelectorAll('[role="checkbox"]');
  expect(boxes.length, "the clicked checkbox renders").toBeGreaterThan(index);
  boxes[index]?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  dispose();
  host.remove();
  return doc.byId.body.raw;
}

describe("checkbox click maps by the item's source position", () => {
  for (const [label, literal] of [
    ["Markdown #+BEGIN_SRC", "#+BEGIN_SRC\n+ [ ] code\n#+END_SRC"],
    ["Markdown #+BEGIN_EXAMPLE", "#+BEGIN_EXAMPLE\n+ [ ] code\n#+END_EXAMPLE"],
    ["$$ display math", "$$\n+ [ ] code\n$$"],
    [":PROPERTIES: drawer", ":PROPERTIES:\n+ [ ] code\n:END:"],
  ] as const) {
    it(`skips a checkbox lookalike inside ${label}`, async () => {
      const raw = `head\n${literal}\n+ [ ] real`;
      expect(await click(raw, 0), `I-1: the click ticks the rendered item, not literal content (${label})`).toBe(
        `head\n${literal}\n+ [x] real`
      );
    });
  }

  it("ticks the clicked item inside a quote, not the next plain item", async () => {
    const raw = "head\n> + [ ] quoted\n+ [ ] plain";
    expect(await click(raw, 0)).toBe("head\n> + [x] quoted\n+ [ ] plain");
    expect(await click(raw, 1)).toBe("head\n> + [ ] quoted\n+ [x] plain");
  });

  it("maps nested, ordered, markup-leading and multibyte items", async () => {
    const raw = "hé ✓\n+ [ ] **b** x\n  + [x] nested ✓\n1. [ ] [[p]]";
    expect(await click(raw, 0)).toBe("hé ✓\n+ [x] **b** x\n  + [x] nested ✓\n1. [ ] [[p]]");
    expect(await click(raw, 1)).toBe("hé ✓\n+ [ ] **b** x\n  + [ ] nested ✓\n1. [ ] [[p]]");
    expect(await click(raw, 2)).toBe("hé ✓\n+ [ ] **b** x\n  + [x] nested ✓\n1. [x] [[p]]");
  });

  it("an Org block maps the same way", async () => {
    const raw = "* Task\n#+BEGIN_EXAMPLE\n+ [ ] code\n#+END_EXAMPLE\n+ [ ] real";
    expect(await click(raw, 0, "org")).toBe("* Task\n#+BEGIN_EXAMPLE\n+ [ ] code\n#+END_EXAMPLE\n+ [x] real");
  });

  it("a checkbox rendered from a macro expansion never edits the host block's raw", async () => {
    // The expansion's positions are not positions in the host's raw.
    const raw = "{{tpl}}\n+ [ ] host item";
    expect(await click(raw, 0, "md", "intro\n+ [ ] from the macro")).toBe(raw);
  });
});

import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { facetsOf } from "../render/facets";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { journalTemplatePage } from "../document/convert";
import type { BlockDto } from "../types";
import { Block } from "./Block";

beforeAll(() => initParser());

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

// The backend's template copy (`template_copy`) ships raw text, collapse state
// and children only: no id and no header facets. A page built from it in the
// window must show what its text says, as it does after a reload (GH #1036).
const copy = (raw: string, children: BlockDto[] = []): BlockDto => ({ id: "", raw, collapsed: false, children });

describe("a page built in the window shows its blocks' text-derived facets", () => {
  it("renders journal-template heading blocks as headings at once", () => {
    const title = "Nov 25th, 2026";
    loadSingle(journalTemplatePage(title, [copy("## Log"), copy("## Notes", [copy("TODO plan the day")])]));
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(
      () => <For each={pageByName(title)?.roots ?? []}>{(id) => <Block id={id} />}</For>,
      root,
    );
    try {
      const headings = [...root.querySelectorAll(".heading-text.h2")].map((h) => h.textContent);
      expect(headings).toEqual(["Log", "Notes"]);
      expect(facetsOf("TODO plan the day", "md").marker).toBe("TODO");
    } finally {
      dispose();
    }
  });

  it("leaves the template page's own loaded heading a heading when the journal copies it", () => {
    // The template page as the backend ships it: runtime id and projected facets.
    loadSingle({ name: "Daily", kind: "page", title: "Daily", pre_block: null, format: "md",
      blocks: [{ id: "daily-log", has_id: false, raw: "## Log", collapsed: false, children: [], heading_level: 2 }] });
    loadSingle(journalTemplatePage("Nov 25th, 2026", [copy("## Log")]));
    expect(facetsOf("## Log", "md").headingLevel).toBe(2);
  });
});

import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing, endEdit } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

// C3 L10 (og c3s F4): entering and leaving a whole-block code wrapper must not
// rewrite it. While the code editor shows only the payload, the exit-time
// planning normalization sees a fence-less body and would move a
// `SCHEDULED:`/`DEADLINE:` line of the CODE up to line 2 before the wrapper is
// re-attached — reordering code on disk with no edit by the user.
//
// Block.tsx's onBlur commit skips normalizePlanning while the body-only code
// editor is shown (`calcExit || codeShown() ? ref.value : …`).

beforeAll(async () => {
  await initParser();
});
afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
});

let seq = 0;
function mountPage(raw: string, format: "md" | "org"): { blockId: string; textarea: HTMLTextAreaElement; dispose: () => void } {
  const name = `CodePlan${++seq}`;
  const block: BlockDto = { id: `cp-${seq}`, raw, collapsed: false, children: [] };
  const page: PageDto = { name, kind: "page", title: name, pre_block: null, blocks: [block], format };
  loadSingle(page);
  startEditing(block.id, 0);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render((): JSX.Element => <For each={pageByName(name)?.roots ?? []}>{(id) => <Block id={id} />}</For>, root);
  return { blockId: block.id, textarea: root.querySelector("textarea.block-editor") as HTMLTextAreaElement, dispose };
}

describe("focus + blur of a whole-block code wrapper", () => {
  for (const [format, raw] of [
    ["md", "```\nline1\nline2\nSCHEDULED: <2026-07-06 Mon>\n```"],
    ["org", "#+BEGIN_SRC text\nline1\nline2\nDEADLINE: <2026-07-06 Mon>\n#+END_SRC"],
  ] as const) {
    it(`${format}: leaves the code bytes unchanged`, () => {
      const { blockId, textarea, dispose } = mountPage(raw, format);
      try {
        expect(textarea.value.startsWith("line1\nline2\n"), "the body-only code editor is shown").toBe(true);
        textarea.dispatchEvent(new FocusEvent("blur"));
        expect(doc.byId[blockId].raw, "I-1: entering and leaving a code block never reorders it").toBe(raw);
      } finally {
        dispose();
      }
    });
  }
});

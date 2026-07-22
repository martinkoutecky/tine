import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore } from "../store";
import { startEditing } from "../editorController";
import { calcSource } from "../editor/calc";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function blk(id: string, raw: string): BlockDto {
  return { id, raw, collapsed: false, children: [] };
}

function page(name: string, blocks: BlockDto[]): PageDto {
  return { name, kind: "page", title: name, pre_block: null, blocks };
}

// GH #57: a ```calc block must survive clicking outside (blur), and its exit
// commit must NOT run planning-normalization over the calc expressions (which
// would reorder a SCHEDULED:-looking line and mangle the block).
describe("calc block persistence on blur", () => {
  it("stays a calc block on blur and preserves expression order", () => {
    loadSingle(page("Calc", [blk("calc-1", "```calc\n1 + 1\n```")]));
    const id = pageByName("Calc")!.roots[0];

    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Calc")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));

    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement | null;
      expect(ta).not.toBeNull();
      // Editor shows the fence-stripped expressions.
      expect(ta!.value).toBe("1 + 1");

      // User edits: adds a line and a SCHEDULED line AFTER it (the order that
      // normalizePlanning would scramble by hoisting SCHEDULED to line 2).
      ta!.focus();
      ta!.value = "1 + 1\n2 + 2\nSCHEDULED: <2026-07-06 Mon>";
      ta!.dispatchEvent(new FocusEvent("blur"));

      const raw = doc.byId[id]?.raw ?? "";
      // Block still exists AND is still a ```calc fence (didn't vanish).
      expect(calcSource(raw)).not.toBeNull();
      // Expression order preserved — planning-normalization did NOT reorder it.
      expect(calcSource(raw)).toBe("1 + 1\n2 + 2\nSCHEDULED: <2026-07-06 Mon>");
    } finally {
      dispose();
    }
  });
});

describe("calculator slash-command activation", () => {
  it("enters live calculator mode immediately without a blur and refocus", async () => {
    loadSingle(page("Calc slash", [blk("calc-slash", "/Calculator")]));
    const id = pageByName("Calc slash")!.roots[0];

    startEditing(id, "/Calculator".length);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Calc slash")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));

    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement | null;
      expect(ta).not.toBeNull();
      ta!.setSelectionRange(ta!.value.length, ta!.value.length);
      ta!.dispatchEvent(new InputEvent("input", {
        bubbles: true,
        inputType: "insertText",
        data: "r",
      }));

      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete")).not.toBeNull());
      ta!.dispatchEvent(new KeyboardEvent("keydown", {
        key: "Enter",
        bubbles: true,
        cancelable: true,
      }));

      await vi.waitFor(() => expect(root.querySelector(".editor-wrap")?.classList.contains("calc-wrap")).toBe(true));
      expect(root.querySelector(".calc-gutter")).not.toBeNull();
      expect(root.querySelector(".calc-results")).not.toBeNull();
      expect(ta!.value).toBe("");
      expect(doc.byId[id].raw).toBe("```calc\n\n```");
    } finally {
      dispose();
    }
  });
});

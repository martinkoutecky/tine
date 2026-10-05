import { describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { pageByName } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { startEditing } from "../editorController";
import { calcSource } from "../editor/calc";
import { Block } from "./Block";
import { installBlockEditorLifecycle, mount, blk, page } from "../tests/blockEditorTestkit";

installBlockEditorLifecycle();

// GH #57: a ```calc block must survive clicking outside (blur), and its exit
// commit must NOT run planning-normalization over the calc expressions (which
// would reorder a SCHEDULED:-looking line and mangle the block).
describe("calc block persistence on blur", () => {
  it("retains ordinary block text after the calc fence when an expression changes", () => {
    const original = "```calc\n1 + 1\n```\nordinary text";
    loadSingle(page("Calc suffix", [blk("calc-suffix", original)]));
    const id = pageByName("Calc suffix")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => <Block id={id} />);
    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement;
      expect(ta.value).toBe("1 + 1");
      ta.focus();
      ta.value = "2 + 2";
      ta.dispatchEvent(new FocusEvent("blur"));
      expect(doc.byId[id].raw, "calc exit rule: Block keeps text after the closing fence").toBe("```calc\n2 + 2\n```\nordinary text");
    } finally {
      dispose();
    }
  });

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

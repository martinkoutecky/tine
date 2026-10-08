import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { startEditing } from "../editorController";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { setJournalTitleFormat } from "../journal";
import { Block } from "./Block";

beforeAll(async () => { await initParser(); });
afterEach(() => { setJournalTitleFormat(null); resetStore(); document.body.innerHTML = ""; });

it.each([
  ["journal", "2026-07-21", "[[Jul 21st, 2026]]"],
  ["page", "Notes", ""],
] as const)("/That day uses its containing %s's date", async (kind, name, expected) => {
  loadSingle({
    name, kind, title: name, pre_block: null, format: "md",
    blocks: [{ id: "thatday-host", raw: "/thatday", collapsed: false, children: [] }],
  });
  startEditing("thatday-host", 8);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <For each={pageByName(name)?.roots ?? []}>{(id) => <Block id={id} />}</For>, root);
  try {
    const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
    textarea.focus();
    textarea.value = "/thatday";
    textarea.setSelectionRange(8, 8);
    textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "y" }));
    await vi.waitFor(() => expect([...document.body.querySelectorAll(".autocomplete .ac-label")].map((el) => el.textContent)).toContain("That day"));
    const item = [...document.body.querySelectorAll<HTMLElement>(".autocomplete .ac-item")]
      .find((el) => el.querySelector(".ac-label")?.textContent === "That day")!;
    item.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(doc.byId["thatday-host"].raw.trimEnd()).toBe(expected));
  } finally {
    dispose();
  }
});
